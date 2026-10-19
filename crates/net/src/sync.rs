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
            // 3: kept more since it was filled: the older mail too, once.
            if opts.window > st.window && st.messages.len() < opts.window {
                let oldest = st.messages.iter().map(|m| m.key.uid).min().unwrap_or(0);
                if oldest > 1 {
                    let older = c.uid_search(&format!("UID 1:{}", oldest - 1))?;
                    let want = opts.window - st.messages.len();
                    let take: Vec<u32> = older.iter().rev().take(want).rev().copied().collect();
                    fetch_headers(c, &mut st, account, folder, &take, &mut report)?;
                }
                st.window = opts.window;
            }
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
    let wanted: Vec<u32> =
        st.messages.iter().take(opts.prefetch).filter(|m| m.size <= opts.prefetch_max_bytes && !store.has_body(&m.key)).map(|m| m.key.uid).collect();
    if wanted.is_empty() {
        return Ok(());
    }
    for chunk in uid_chunks(&wanted, 10) {
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
}
