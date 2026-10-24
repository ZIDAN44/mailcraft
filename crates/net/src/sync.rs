//! Folder synchronisation (RFC 4549 §4.3, with CONDSTORE from RFC 7162).
//!
//! For each folder, the cache keeps UIDVALIDITY, UIDNEXT, HIGHESTMODSEQ and the summaries.
//! A sync then costs a handful of round trips however large the folder:
//!
//! 1. `SELECT`. A different UIDVALIDITY means every cached UID is meaningless: drop the cache.
//! 2. **New mail**: headers for `UID FETCH <cached UIDNEXT>:*`. On the first sync, only the newest
//!    [`SyncOptions::window`] messages are fetched (found with `UID SEARCH ALL`).
//! 3. **Flag changes**: with CONDSTORE, `UID FETCH … (FLAGS) (CHANGEDSINCE <modseq>)` returns only
//!    what changed, and nothing at all when HIGHESTMODSEQ hasn't moved. Without it, the flags of
//!    the cached range are fetched.
//! 4. **Deletions**: `UID SEARCH UID <cached range>` lists the survivors; the rest are gone.
//! 5. The newest few message sources are downloaded so they can be read (and searched) offline.
//!
//! With QRESYNC (RFC 7162 §3.2) steps 3 and 4 collapse into the `SELECT`: the server lists the
//! UIDs expunged since the cached HIGHESTMODSEQ (`VANISHED (EARLIER)`) and the changed flags, so
//! a large folder no longer needs a UID search over everything cached.

use std::io::{Read, Write};

use mailcraft_mime as mime;
use mailcraft_model::{Flags, MessageKey, Summary, sort_newest_first};
use mailcraft_store::{FolderState, Store};

use crate::imap::{Client, Fetched, uid_chunks, uid_set};
use crate::{NetError, Result};

pub use mailcraft_jmap::{SyncOptions, SyncReport, fill_from_source};

/// Downloading ahead, one message at a time: whether to go on after one failed. A message the
/// server won't give (gone meanwhile, or failing on its side) is left for when it's opened, and
/// the rest still come down; a refused sign-in, or several failures running (the connection,
/// then), stop it.
pub struct AheadFailures {
    running: u32,
}

impl AheadFailures {
    pub fn new() -> AheadFailures {
        AheadFailures { running: 0 }
    }

    /// One went well.
    pub fn ok(&mut self) {
        self.running = 0;
    }

    /// One failed: `Ok` to go on with the next, or the error to stop with.
    pub fn failed(&mut self, what: &str, e: NetError) -> Result<()> {
        self.running += 1;
        if matches!(e, NetError::Auth(_)) || self.running >= 3 {
            return Err(e);
        }
        log::info!("{what}: left for when it's opened: {e}");
        Ok(())
    }
}

impl Default for AheadFailures {
    fn default() -> AheadFailures {
        AheadFailures::new()
    }
}

const HEADER_ITEMS: &str = "UID FLAGS RFC822.SIZE INTERNALDATE BODY.PEEK[HEADER]";

fn summary(account: &str, folder: &str, validity: u32, f: &Fetched) -> Summary {
    let key = MessageKey { account: account.to_string(), folder: folder.to_string(), validity, uid: f.uid };
    let flags = Flags::from_imap(f.flags.iter().flatten().map(String::as_str));
    let raw = f.body.as_deref().or(f.header.as_deref()).unwrap_or_default();
    let mut s = mime::summary_from_header(raw, key, flags, f.size, f.internal_date.unwrap_or(0));
    s.modseq = f.modseq;
    s
}

/// Bring one folder's cache up to date.
pub fn sync_folder<S: Read + Write>(c: &mut Client<S>, store: &Store, account: &str, folder: &str, opts: &SyncOptions) -> Result<SyncReport> {
    let mut st = store.load_folder(account, folder)?;
    let cached_max = st.messages.iter().map(|m| m.key.uid).max().unwrap_or(0);
    let cached_min = st.messages.iter().map(|m| m.key.uid).min().unwrap_or(0);
    // QRESYNC: expunges and flag changes arrive with the SELECT itself (RFC 7162 §3.2.5).
    let quick = c.qresync() && st.validity != 0 && st.highest_modseq > 0 && cached_max > 0;
    let (info, quick) = if quick {
        let (info, vanished, changed) = c.select_qresync(folder, st.validity, st.highest_modseq, &format!("{cached_min}:{cached_max}"))?;
        (info, Some((vanished, changed)))
    } else {
        (c.select(folder, false)?, None)
    };
    let mut report = SyncReport::default();

    if st.validity != info.validity && (st.validity != 0 || !st.messages.is_empty()) {
        log::info!("{account}/{folder}: UIDVALIDITY {} → {}, dropping the cache", st.validity, info.validity);
        store.clear_folder(account, folder)?;
        st = FolderState::default();
        report.reset = true;
    }
    let quick = if report.reset { None } else { quick };
    st.validity = info.validity;

    if info.exists == 0 {
        report.removed = st.messages.len();
        st.messages.clear();
    } else if let Some((vanished, changed)) = quick {
        // 3 + 4 from what the server just said.
        for f in changed {
            if let (Some(m), Some(fl)) = (st.find_mut(f.uid), &f.flags) {
                let flags = Flags::from_imap(fl.iter().map(String::as_str));
                if m.flags != flags {
                    m.flags = flags;
                    report.changed += 1;
                }
                m.modseq = m.modseq.max(f.modseq);
            }
        }
        let (kept, gone): (Vec<Summary>, Vec<Summary>) =
            std::mem::take(&mut st.messages).into_iter().partition(|m| !vanished.iter().any(|r| r.contains(&m.key.uid)));
        for m in &gone {
            store.remove_body(&m.key);
        }
        report.removed = gone.len();
        st.messages = kept;
        new_mail(c, &mut st, account, folder, cached_max, &mut report)?;
        keep_more(c, &mut st, account, folder, opts.window, &mut report)?;
    } else {
        let known_max = st.messages.iter().map(|m| m.key.uid).max().unwrap_or(0);
        let known_min = st.messages.iter().map(|m| m.key.uid).min().unwrap_or(0);

        // 3 + 4: changes to what we have (before adding new mail, so only cached UIDs are asked).
        if known_max > 0 {
            let range = format!("{known_min}:{known_max}");
            let condstore = c.has("CONDSTORE") && st.highest_modseq > 0 && info.highest_modseq > 0;
            let changed = if condstore {
                if info.highest_modseq == st.highest_modseq {
                    Vec::new()
                } else {
                    c.uid_fetch(&range, "UID FLAGS", &format!("(CHANGEDSINCE {})", st.highest_modseq))?
                }
            } else {
                c.uid_fetch(&range, "UID FLAGS", "")?
            };
            for f in changed {
                if let (Some(m), Some(fl)) = (st.find_mut(f.uid), &f.flags) {
                    let flags = Flags::from_imap(fl.iter().map(String::as_str));
                    if m.flags != flags {
                        m.flags = flags;
                        report.changed += 1;
                    }
                    m.modseq = m.modseq.max(f.modseq);
                }
            }
            let alive = c.uid_search(&format!("UID {range}"))?;
            let (kept, gone): (Vec<Summary>, Vec<Summary>) =
                std::mem::take(&mut st.messages).into_iter().partition(|m| alive.binary_search(&m.key.uid).is_ok());
            for m in &gone {
                store.remove_body(&m.key);
            }
            report.removed = gone.len();
            st.messages = kept;
        }

        // 2: new mail.
        if st.uid_next == 0 && known_max == 0 {
            let all = c.uid_search("ALL")?;
            let skip = all.len().saturating_sub(opts.window);
            let first: Vec<u32> = all.into_iter().skip(skip).collect();
            fetch_headers(c, &mut st, account, folder, &first, &mut report)?;
            st.window = opts.window;
        } else {
            new_mail(c, &mut st, account, folder, known_max, &mut report)?;
            keep_more(c, &mut st, account, folder, opts.window, &mut report)?;
        }
    }

    sort_newest_first(&mut st.messages);
    let max_uid = st.messages.iter().map(|m| m.key.uid).max().unwrap_or(0);
    st.uid_next = if info.uid_next > 0 { info.uid_next } else { max_uid.saturating_add(1) };
    st.highest_modseq = info.highest_modseq;
    store.save_folder(account, folder, &st)?;

    // 5: prefetch the newest sources (in small batches, saving as we go).
    prefetch(c, store, account, folder, &mut st, opts)?;

    report.total = info.exists;
    report.unread = if info.exists == 0 { 0 } else { u32::try_from(c.uid_search("UNSEEN")?.len()).unwrap_or(u32::MAX) };
    Ok(report)
}

/// An argument of an IMAP `SEARCH`: a search key, or a string it takes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SearchArg {
    Atom(String),
    Str(String),
}

/// The search box's query as IMAP `SEARCH` keys (RFC 3501 §6.4.4), for mail past what's kept
/// (issue #4): words anywhere are `TEXT`, `from:` `FROM` and so on, `is:unread` `UNSEEN`, a
/// leading `-` `NOT`. What IMAP can't ask (attachments, importance) is left to the list; `None`
/// when nothing is left to ask.
pub fn imap_criteria(query: &str) -> Option<Vec<SearchArg>> {
    use mailcraft_model::search::{Field, Query, Term};
    let q = Query::parse(query);
    let mut out = Vec::new();
    let atom = |s: &str| SearchArg::Atom(s.to_string());
    for c in q.clauses.iter().take(16) {
        let not = |out: &mut Vec<SearchArg>| {
            if c.negated {
                out.push(atom("NOT"));
            }
        };
        let key = |f: &Field| match f {
            Field::Any => "TEXT",
            Field::From => "FROM",
            Field::To => "TO",
            Field::Subject => "SUBJECT",
            Field::Body => "BODY",
        };
        match &c.term {
            Term::Words(f, words) => {
                for w in words.iter().filter(|w| !w.is_empty()).take(8) {
                    not(&mut out);
                    out.extend([atom(key(f)), SearchArg::Str(w.chars().take(200).collect())]);
                }
            }
            Term::Phrase(f, p) if !p.is_empty() => {
                not(&mut out);
                out.extend([atom(key(f)), SearchArg::Str(p.chars().take(200).collect())]);
            }
            Term::Unread(on) => out.push(atom(if *on != c.negated { "UNSEEN" } else { "SEEN" })),
            Term::Flagged(on) => out.push(atom(if *on != c.negated { "FLAGGED" } else { "UNFLAGGED" })),
            Term::Size(larger, bytes) => {
                not(&mut out);
                out.extend([atom(if *larger { "LARGER" } else { "SMALLER" }), atom(&bytes.to_string())]);
            }
            // `from:ana,jun`: OR ana OR jun …, when each is one key (a word or a phrase).
            Term::AnyOf(any) => {
                let keys: Vec<(&str, String)> = any
                    .iter()
                    .filter_map(|t| match t {
                        Term::Words(f, w) if w.len() == 1 => w.first().map(|w| (key(f), w.chars().take(200).collect())),
                        Term::Phrase(f, p) if !p.is_empty() => Some((key(f), p.chars().take(200).collect())),
                        _ => None,
                    })
                    .take(16)
                    .collect();
                if keys.len() == any.len() && !keys.is_empty() {
                    not(&mut out);
                    let last = keys.len() - 1;
                    for (i, (k, v)) in keys.into_iter().enumerate() {
                        if i < last {
                            out.push(atom("OR"));
                        }
                        out.extend([atom(k), SearchArg::Str(v)]);
                    }
                }
            }
            Term::Keyword(k)
                if !k.is_empty() && k.len() < 100 && k.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '$' | '_' | '-' | '.')) =>
            {
                not(&mut out);
                out.extend([atom("KEYWORD"), atom(k)]);
            }
            _ => {}
        }
    }
    (!out.is_empty()).then_some(out)
}

/// The search box's query for Exchange's searches (issue #4): the words as KQL (Microsoft
/// Graph's `$search`) or AQS (EWS's `QueryString`), which read the same for what MailCraft
/// asks, and the read, flag and attachment conditions, which neither searches: those are asked
/// as a filter when there are no words, and checked on what comes back.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WordQuery {
    /// `from:ana subject:"short film" NOT budget`; empty when the query has no words.
    pub text: String,
    pub unread: Option<bool>,
    pub flagged: Option<bool>,
    pub attachment: Option<bool>,
}

impl WordQuery {
    /// Whether a message's flags and attachments meet the query's conditions.
    pub fn admits(&self, flags: &Flags, has_attachments: bool) -> bool {
        self.unread.is_none_or(|u| u != flags.seen)
            && self.flagged.is_none_or(|f| f == flags.flagged)
            && self.attachment.is_none_or(|a| a == has_attachments)
    }
}

/// [`WordQuery`] for a search box query; `None` when nothing is left to ask.
pub fn word_query(query: &str) -> Option<WordQuery> {
    use mailcraft_model::search::{Field, Query, Term};
    let q = Query::parse(query);
    let mut out = WordQuery::default();
    let mut parts: Vec<String> = Vec::new();
    let prefix = |f: &Field| match f {
        Field::Any => "",
        Field::From => "from:",
        Field::To => "to:",
        Field::Subject => "subject:",
        Field::Body => "body:",
    };
    // A word as written when it's plain (a prefix match, as the list's own search), quoted
    // otherwise; quotes and backslashes can't be asked for and are dropped.
    let word = |w: &str| {
        let w: String = w.chars().filter(|c| !matches!(c, '"' | '\\') && !c.is_control()).take(200).collect();
        let plain =
            w.chars().all(|c| c.is_alphanumeric() || matches!(c, '@' | '.' | '_' | '-' | '\'')) && !matches!(w.as_str(), "AND" | "OR" | "NOT");
        if plain && !w.starts_with('-') { w } else { format!("\"{w}\"") }
    };
    for c in q.clauses.iter().take(16) {
        let not = if c.negated { "NOT " } else { "" };
        match &c.term {
            Term::Words(f, words) => {
                for w in words.iter().filter(|w| !w.is_empty()).take(8) {
                    parts.push(format!("{not}{}{}", prefix(f), word(w)));
                }
            }
            Term::Phrase(f, p) if !p.is_empty() => {
                let p: String = p.chars().filter(|c| !matches!(c, '"' | '\\') && !c.is_control()).take(200).collect();
                parts.push(format!("{not}{}\"{p}\"", prefix(f)));
            }
            Term::Unread(on) => out.unread = Some(*on != c.negated),
            Term::Flagged(on) => out.flagged = Some(*on != c.negated),
            Term::HasAttachment(on) => out.attachment = Some(*on != c.negated),
            _ => {}
        }
    }
    // Words that were nothing but quotes.
    parts.retain(|p| !p.ends_with("\"\"") && !p.ends_with(':') && !p.trim_start_matches("NOT ").is_empty());
    out.text = parts.join(" ");
    (!out.text.is_empty() || out.unread.is_some() || out.flagged.is_some() || out.attachment.is_some()).then_some(out)
}

/// How many messages one `SEARCH` of a folder covers at first; it grows while the server answers
/// quickly and shrinks when it's slow (a server without a text index reads every message).
const SEARCH_STEP: u32 = 500;
const SEARCH_STEP_MAX: u32 = 20_000;

/// The messages of `folder` the server finds for `criteria`, the newest `cap` of them: those not
/// kept yet come into its list (issue #4). The folder is searched newest first, a range of
/// message numbers at a time (`UID SEARCH 9501:10000 TEXT …`), and what each range finds goes to
/// `found` with how many came into the list, as it's found; `found` answers whether to go on
/// (the search can be stopped between two ranges). How many were found in all. A folder never
/// synced (or whose UIDVALIDITY changed) is left to its sync.
pub fn find_on_server<S: Read + Write>(
    c: &mut Client<S>,
    store: &Store,
    account: &str,
    folder: &str,
    criteria: &[SearchArg],
    cap: usize,
    found: &mut dyn FnMut(Vec<MessageKey>, usize) -> bool,
) -> Result<usize> {
    let info = c.select(folder, true)?;
    let mut st = store.load_folder(account, folder)?;
    if st.validity == 0 || st.validity != info.validity {
        return Ok(0);
    }
    let (mut hi, mut step, mut total) = (info.exists, SEARCH_STEP, 0usize);
    while hi > 0 && total < cap {
        let lo = hi.saturating_sub(step - 1).max(1);
        let began = std::time::Instant::now();
        let mut args = vec![SearchArg::Atom(format!("{lo}:{hi}"))];
        args.extend(criteria.iter().cloned());
        let mut uids = c.uid_search_args(&args)?;
        // How long the server took to answer: what the next range's size follows (not the
        // headers fetched after, nor what the caller does with the finds).
        let took = began.elapsed();
        uids.sort_unstable();
        let uids = uids.split_off(uids.len().saturating_sub(cap - total));
        total += uids.len();
        let missing: Vec<u32> = uids.iter().filter(|u| st.find(**u).is_none()).copied().collect();
        let mut report = SyncReport::default();
        fetch_headers(c, &mut st, account, folder, &missing, &mut report)?;
        if report.new > 0 {
            sort_newest_first(&mut st.messages);
            store.save_folder(account, folder, &st)?;
        }
        let keys = uids.iter().filter_map(|u| st.find(*u).map(|m| m.key.clone())).collect();
        if !found(keys, report.new) {
            break;
        }
        hi = lo - 1;
        step = if took < std::time::Duration::from_secs(1) {
            step.saturating_mul(2).min(SEARCH_STEP_MAX)
        } else if took > std::time::Duration::from_secs(4) {
            (step / 2).max(100)
        } else {
            step
        };
    }
    Ok(total)
}

/// Kept more since the folder was filled (Options, or Show more on the server): the newest of
/// what isn't kept yet, once. Not only what's older than the oldest kept: a message found by
/// searching the server can be older than the ones skipped. Both ways of syncing come here (the
/// QRESYNC one too, issue #4: Dovecot's Show more did nothing).
fn keep_more<S: Read + Write>(
    c: &mut Client<S>,
    st: &mut FolderState,
    account: &str,
    folder: &str,
    window: usize,
    report: &mut SyncReport,
) -> Result<()> {
    if window <= st.window || st.messages.len() >= window {
        return Ok(());
    }
    let all = c.uid_search("ALL")?;
    let want = window - st.messages.len();
    let take: Vec<u32> = all.iter().rev().filter(|u| st.find(**u).is_none()).take(want).copied().collect();
    fetch_headers(c, st, account, folder, &take, report)?;
    st.window = window;
    Ok(())
}

/// Headers for every message at or above the cached UIDNEXT.
fn new_mail<S: Read + Write>(
    c: &mut Client<S>,
    st: &mut FolderState,
    account: &str,
    folder: &str,
    known_max: u32,
    report: &mut SyncReport,
) -> Result<()> {
    let from = st.uid_next.max(known_max.saturating_add(1)).max(1);
    // `n:*` always returns at least the last message (RFC 3501 §6.4.8): filter.
    let uids: Vec<u32> = c.uid_search(&format!("UID {from}:*"))?.into_iter().filter(|u| *u >= from).collect();
    fetch_headers(c, st, account, folder, &uids, report)
}

fn fetch_headers<S: Read + Write>(
    c: &mut Client<S>,
    st: &mut FolderState,
    account: &str,
    folder: &str,
    uids: &[u32],
    report: &mut SyncReport,
) -> Result<()> {
    for chunk in uid_chunks(uids, 200) {
        for f in c.uid_fetch(&uid_set(&chunk), HEADER_ITEMS, "")? {
            if st.find(f.uid).is_none() {
                st.messages.push(summary(account, folder, st.validity, &f));
                report.new += 1;
            }
        }
    }
    Ok(())
}

fn prefetch<S: Read + Write>(c: &mut Client<S>, store: &Store, account: &str, folder: &str, st: &mut FolderState, opts: &SyncOptions) -> Result<()> {
    let wanted: Vec<(u32, u32)> = st
        .messages
        .iter()
        .take(opts.prefetch)
        .filter(|m| m.size <= opts.prefetch_max_bytes && !store.has_body(&m.key))
        .map(|m| (m.key.uid, m.size))
        .collect();
    if wanted.is_empty() {
        return Ok(());
    }
    for chunk in body_chunks(&wanted) {
        for f in c.uid_fetch(&uid_set(&chunk), "UID BODY.PEEK[]", "")? {
            let Some(raw) = &f.body else { continue };
            let Some(m) = st.find_mut(f.uid) else { continue };
            store.write_body(&m.key, raw)?;
            fill_from_source(m, raw);
        }
        store.save_folder(account, folder, st)?;
    }
    Ok(())
}

/// Prefetch chunks: ten messages at most, and no more than one literal's worth of sources
/// together, keeping each `UID FETCH` well inside the per-command cap ([`crate::imap::Limits`]).
fn body_chunks(wanted: &[(u32, u32)]) -> Vec<Vec<u32>> {
    let mut out: Vec<Vec<u32>> = Vec::new();
    let mut bytes = 0usize;
    for &(uid, size) in wanted {
        let size = usize::try_from(size).unwrap_or(usize::MAX);
        match out.last_mut() {
            Some(last) if last.len() < 10 && bytes.saturating_add(size) <= crate::imap::MAX_LITERAL => {
                last.push(uid);
                bytes = bytes.saturating_add(size);
            }
            _ => {
                out.push(vec![uid]);
                bytes = size;
            }
        }
    }
    out
}

/// Download one message's source (and cache it), updating its summary.
pub fn fetch_body<S: Read + Write>(c: &mut Client<S>, store: &Store, key: &MessageKey) -> Result<Vec<u8>> {
    if let Some(raw) = store.read_body(key) {
        return Ok(raw);
    }
    let info = if c.selected() == Some(key.folder.as_str()) { None } else { Some(c.select(&key.folder, false)?) };
    if let Some(i) = &info
        && i.validity != key.validity
    {
        return Err(crate::NetError::No("the folder changed on the server; refresh and try again".into()));
    }
    let fetched = c.uid_fetch(&key.uid.to_string(), "UID BODY.PEEK[]", "")?;
    let raw = fetched
        .into_iter()
        .find(|f| f.uid == key.uid)
        .and_then(|f| f.body)
        .ok_or_else(|| crate::NetError::No("the message is no longer on the server".into()))?;
    store.write_body(key, &raw)?;
    let mut st = store.load_folder(&key.account, &key.folder)?;
    if st.validity == key.validity
        && let Some(m) = st.find_mut(key.uid)
    {
        fill_from_source(m, &raw);
        store.save_folder(&key.account, &key.folder, &st)?;
    }
    Ok(raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Downloading ahead goes past a message that fails, but not a refused sign-in, nor three
    /// failures running.
    #[test]
    fn ahead_failures_go_on_until_they_mean_the_connection() {
        let mut f = AheadFailures::new();
        assert!(f.failed("m1", NetError::No("gone".into())).is_ok());
        f.ok();
        assert!(f.failed("m2", NetError::Protocol("x".into())).is_ok());
        assert!(f.failed("m3", NetError::Closed("HTTP 500".into())).is_ok());
        assert!(f.failed("m4", NetError::Closed("HTTP 500".into())).is_err(), "three running");
        assert!(AheadFailures::new().failed("m", NetError::Auth("expired".into())).is_err());
    }

    /// Prefetch chunks stay small enough for the per-command cap, whatever the sizes.
    #[test]
    fn prefetch_chunks_fit_the_command_cap() {
        let small: Vec<(u32, u32)> = (1..=25).map(|u| (u, 1000)).collect();
        assert_eq!(body_chunks(&small).iter().map(Vec::len).collect::<Vec<_>>(), vec![10, 10, 5]);
        let big: Vec<(u32, u32)> = (1..=5).map(|u| (u, 40 << 20)).collect();
        assert_eq!(body_chunks(&big).len(), 5);
        assert_eq!(body_chunks(&[(1, u32::MAX), (2, 1)]), vec![vec![1], vec![2]]);
        assert!(body_chunks(&[]).is_empty());
    }
}
