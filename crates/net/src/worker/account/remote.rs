//! The connection to an account's server, and the changes made here carried out on it: what
//! each kind of server does its own way is matched here once, not by every caller.

use super::*;
use crate::sync::{SearchArg, WordQuery};
use mailcraft_model::Flags;
use mailcraft_model::Summary;
use mailcraft_model::job::Vacation;

/// The most of a message's own keywords given a server with it.
const KEYWORDS_MAX: usize = 32;
/// The most messages one IMAP command names (a set of UIDs some servers take only so long).
const UID_CHUNK: usize = 500;

/// The connection to an account's server.
pub(crate) enum Conn {
    Imap(Client<Stream>),
    Jmap(jmap::Client),
    Graph(crate::graph::Client),
    Ews(Box<crate::ews::Client>),
    /// POP3: signed in once to check the password; every download is a session of its own.
    Pop,
}

/// A search of the server as each kind asks it: IMAP's `SEARCH` keys, JMAP's filter, and the
/// words Graph and Exchange search for. `None` where that kind has nothing to ask.
pub(crate) struct ServerQuery {
    imap: Option<Vec<SearchArg>>,
    filter: Option<serde_json::Value>,
    words: Option<WordQuery>,
}

impl ServerQuery {
    /// `query` asked now: its dates (`before:`, `received:today`) as days around this moment,
    /// in UTC, which the period a day wider each side takes in whatever the zone.
    pub(crate) fn new(query: &str) -> ServerQuery {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0));
        let period = mailcraft_model::search::Query::parse(query).server_period(&mailcraft_model::dates::Clock::new(now, 0));
        ServerQuery {
            imap: crate::sync::imap_criteria(query, period),
            filter: mailcraft_jmap::search_filter(query, period),
            words: crate::sync::word_query(query, period),
        }
    }
}

impl Conn {
    /// The whole of one message from the server, kept, with its summary filled from it when the
    /// cache's lacks what only the message says; not kept once the account was removed since it
    /// had been removed `seen` times (an error then). POP mail is downloaded whole: what isn't
    /// here isn't anywhere.
    pub(crate) fn fetch_body(&mut self, jstate: &jmap::State, store: &Store, key: &MessageKey, seen: u64) -> Result<(Vec<u8>, Option<Summary>)> {
        match self {
            Conn::Imap(c) => crate::sync::fetch_body(c, store, key, seen),
            Conn::Jmap(c) => jmap::fetch_body(c, jstate, store, key, seen),
            Conn::Graph(c) => crate::graph::fetch_body(c, jstate, store, key, seen),
            Conn::Ews(c) => crate::ews::fetch_body(c, jstate, store, key, seen),
            Conn::Pop => Err(NetError::No("this message isn't on this computer".into())),
        }
    }

    /// A message's text first, when the server gives it apart from its files and the message is
    /// big enough for that to be quicker (issue #24): `None` otherwise (JMAP and POP: never).
    pub(crate) fn fetch_text(&mut self, jstate: &jmap::State, store: &Store, key: &MessageKey) -> Result<Option<Vec<u8>>> {
        match self {
            Conn::Imap(c) => crate::sync::fetch_text(c, store, key),
            Conn::Graph(c) => crate::graph::fetch_text(c, jstate, store, key),
            Conn::Ews(c) => crate::ews::fetch_text(c, jstate, store, key),
            Conn::Jmap(_) | Conn::Pop => Ok(None),
        }
    }

    /// Carry out one change on the server, with the account's Trash (permanent deletes on Gmail
    /// need it). POP: changes are made here only (the cache has them already).
    pub(crate) fn apply_remote(&mut self, jstate: &jmap::State, op: &Op, trash: Option<&str>) -> Result<()> {
        match self {
            Conn::Imap(c) => apply_remote_with(c, op, trash),
            Conn::Jmap(c) => jmap::apply_remote(c, jstate, op),
            Conn::Graph(c) => crate::graph::apply_remote(c, jstate, op),
            Conn::Ews(c) => crate::ews::apply_remote(c, jstate, op),
            Conn::Pop => Ok(()),
        }
    }

    /// The automatic replies a server keeps itself (JMAP, Graph, Exchange), first changed to
    /// `set` when given; POP has none. `None` for IMAP: its go through ManageSieve, on a
    /// connection of their own.
    pub(crate) fn vacation(&self, set: Option<&Vacation>) -> Option<Result<Vacation>> {
        let r = match self {
            Conn::Jmap(c) => set.map_or(Ok(()), |v| c.set_vacation(v)).and_then(|()| c.vacation()),
            Conn::Graph(c) => set.map_or(Ok(()), |v| c.set_vacation(v)).and_then(|()| c.vacation()),
            Conn::Ews(c) => set.map_or(Ok(()), |v| c.set_vacation(v)).and_then(|()| c.vacation()),
            Conn::Pop => Err(NetError::No("POP accounts have no automatic replies on the server".into())),
            Conn::Imap(_) => return None,
        };
        Some(r)
    }

    /// File a draft in `folder` on a server reached through its API (JMAP, Graph, Exchange),
    /// then delete the copy it replaces. `None` for IMAP and POP, whose drafts the worker files
    /// itself (a Drafts folder made, MailCraft's own copies looked for; the cache).
    pub(crate) fn file_api_draft(&mut self, jstate: &jmap::State, folder: &str, raw: &[u8], replaces: Option<&MessageKey>) -> Option<Result<()>> {
        let gone = replaces.map(|old| Op::Expunge { keys: vec![old.clone()] });
        let r = match self {
            Conn::Jmap(c) => jmap::append(c, jstate, folder, raw, &["$draft", "$seen"])
                .and_then(|_| gone.as_ref().map_or(Ok(()), |op| jmap::apply_remote(c, jstate, op))),
            Conn::Graph(c) => crate::graph::append(c, jstate, folder, raw)
                .and_then(|_| gone.as_ref().map_or(Ok(()), |op| crate::graph::apply_remote(c, jstate, op))),
            Conn::Ews(c) => {
                crate::ews::append(c, jstate, folder, raw).and_then(|_| gone.as_ref().map_or(Ok(()), |op| crate::ews::apply_remote(c, jstate, op)))
            }
            Conn::Imap(_) | Conn::Pop => return None,
        };
        Some(r)
    }

    /// Put message `raw` into `folder` as received, marked with `flags`; for IMAP, `date` is when
    /// it came (`None`: now, as the server sees it). A POP account's folders are on this computer:
    /// `account`'s, in `store`.
    pub(crate) fn put(
        &mut self,
        jstate: &jmap::State,
        store: &Store,
        account: &str,
        folder: &str,
        raw: &[u8],
        flags: &Flags,
        date: Option<i64>,
    ) -> Result<()> {
        match self {
            // A server that won't keep one of its keywords still takes the message: asked
            // again without them when it refuses, and only then; then without the day it came
            // too (a server may take neither). Nothing is asked twice word for word. After no
            // answer (a slow server, a connection gone) the message may well be there: asked
            // again, it was there twice, and a moved one was then deleted where it had been.
            Conn::Imap(c) => {
                let (all, plain) = (imap_flags(flags, true), imap_flags(flags, false));
                let mut asked: Vec<(&str, Option<i64>)> = Vec::new();
                let mut r = Ok(None);
                for ask in [(all.as_str(), date), (plain.as_str(), date), (plain.as_str(), None)] {
                    if asked.contains(&ask) {
                        continue;
                    }
                    asked.push(ask);
                    r = c.append_dated(folder, ask.0, ask.1, raw);
                    if !matches!(r, Err(NetError::No(_) | NetError::Bad(_))) {
                        break;
                    }
                }
                r.map(|_| ())
            }
            Conn::Jmap(c) => jmap::append(c, jstate, folder, raw, &jmap_keywords(flags)).map(|_| ()),
            // Exchange: EWS takes it as received (not a draft); Graph only makes drafts of what
            // it's given whole.
            Conn::Ews(c) => c.create_received(raw, jstate.mailbox(folder)?).map(|_| ()),
            Conn::Graph(c) => crate::graph::append(c, jstate, folder, raw).map(|_| ()),
            Conn::Pop => crate::pop::file_locally(store, account, folder, raw, Flags { deleted: false, ..flags.clone() }, now()).map(|_| ()),
        }
    }

    /// Whether this kind of server has anything to ask for a search (POP: never).
    pub(crate) fn asks(&self, q: &ServerQuery) -> bool {
        match self {
            Conn::Imap(_) => q.imap.is_some(),
            Conn::Jmap(_) => q.filter.is_some(),
            Conn::Graph(_) | Conn::Ews(_) => q.words.is_some(),
            Conn::Pop => false,
        }
    }

    /// Search one folder on the server (issue #4): what's found goes to `sent` with how many came
    /// into the folder's list, as it's found (IMAP, a range at a time) or at the end. `None` when
    /// this kind of server has nothing to ask ([`Conn::asks`]). Nothing found is kept once the
    /// account was removed since it had been removed `seen` times.
    pub(crate) fn find_on_server(
        &mut self,
        jstate: &mut jmap::State,
        store: &Store,
        (account, seen): (&str, u64),
        folder: &str,
        q: &ServerQuery,
        sent: &mut dyn FnMut(Vec<MessageKey>, usize) -> bool,
    ) -> Option<Result<()>> {
        let mut found = |r: Result<(Vec<MessageKey>, usize)>| {
            r.map(|(keys, new)| {
                sent(keys, new);
            })
        };
        let r = match (self, &q.imap, &q.filter, &q.words) {
            (Conn::Imap(c), Some(q), _, _) => crate::sync::find_on_server(c, store, (account, seen), folder, q, FOUND_MAX, sent).map(|_| ()),
            (Conn::Jmap(c), _, Some(q), _) => found(jmap::find_on_server(c, jstate, store, (account, seen), folder, q, FOUND_MAX)),
            (Conn::Graph(c), _, _, Some(q)) => found(crate::graph::find_on_server(c, jstate, store, (account, seen), folder, q, FOUND_MAX)),
            (Conn::Ews(c), _, _, Some(q)) => found(crate::ews::find_on_server(c, jstate, store, (account, seen), folder, q, FOUND_MAX)),
            _ => return None,
        };
        Some(r)
    }
}

/// The flags an IMAP `APPEND` gives a copy: how it was marked, and (`keywords`) its categories
/// and other keywords, those a server can take as they are.
fn imap_flags(flags: &Flags, keywords: bool) -> String {
    let mut out: Vec<&str> = Vec::new();
    for (on, name) in [(flags.seen, "\\Seen"), (flags.flagged, "\\Flagged"), (flags.answered, "\\Answered"), (flags.draft, "\\Draft")] {
        if on {
            out.push(name);
        }
    }
    if keywords {
        if flags.forwarded {
            out.push("$Forwarded");
        }
        // Any a server takes: MailCraft's own for a category (`to=20do` for "To do") has a
        // `=` in it, and was left behind with the message's categories.
        out.extend(flags.keywords.iter().filter(|k| mailcraft_model::is_keyword(k)).take(KEYWORDS_MAX).map(String::as_str));
    }
    out.join(" ")
}

/// The keywords a JMAP server gives a copy: how it was marked, and its own.
fn jmap_keywords(flags: &Flags) -> Vec<&str> {
    let mut out: Vec<&str> = Vec::new();
    for (on, name) in
        [(flags.seen, "$seen"), (flags.flagged, "$flagged"), (flags.answered, "$answered"), (flags.draft, "$draft"), (flags.forwarded, "$forwarded")]
    {
        if on {
            out.push(name);
        }
    }
    out.extend(flags.keywords.iter().filter(|k| mailcraft_model::is_keyword(k)).take(KEYWORDS_MAX).map(String::as_str));
    out
}

/// Group keys by folder, select each folder, and skip keys whose UIDVALIDITY is stale.
pub(crate) fn by_folder<S: std::io::Read + std::io::Write>(
    c: &mut Client<S>,
    keys: &[MessageKey],
    mut f: impl FnMut(&mut Client<S>, &str, &str) -> Result<()>,
) -> Result<()> {
    let mut folders: Vec<&str> = keys.iter().map(|k| k.folder.as_str()).collect();
    folders.sort_unstable();
    folders.dedup();
    for folder in folders {
        let info = c.select(folder, false)?;
        let uids: Vec<u32> = keys.iter().filter(|k| k.folder == folder && k.validity == info.validity).map(|k| k.uid).collect();
        if uids.is_empty() {
            continue;
        }
        for chunk in uids.chunks(UID_CHUNK) {
            f(c, folder, &uid_set(chunk))?;
        }
    }
    Ok(())
}

/// Permanent delete. On Gmail (`X-GM-EXT-1`) an EXPUNGE in a label folder merely archives to
/// All Mail, so those messages are first moved into Trash (COPYUID gives their new UIDs) and
/// expunged in Trash.
pub(crate) fn expunge_keys<S: std::io::Read + std::io::Write>(c: &mut Client<S>, keys: &[MessageKey], trash: Option<&str>) -> Result<()> {
    let to_trash = trash.filter(|_| c.has("X-GM-EXT-1"));
    let mut in_trash: Vec<u32> = Vec::new();
    by_folder(c, keys, |c, folder, set| match to_trash {
        Some(t) if folder != t => {
            let uids = c.uid_move_to(set, t)?;
            if uids.is_empty() {
                log::info!("{folder}: moved to {t} for deleting, but the server didn't say where");
            }
            in_trash.extend(uids);
            Ok(())
        }
        _ => {
            c.uid_store(set, true, Flag::Deleted.imap())?;
            c.expunge(set)
        }
    })?;
    if let (Some(t), false) = (to_trash, in_trash.is_empty()) {
        c.select(t, false)?;
        for chunk in in_trash.chunks(UID_CHUNK) {
            let set = uid_set(chunk);
            c.uid_store(&set, true, Flag::Deleted.imap())?;
            c.expunge(&set)?;
        }
    }
    Ok(())
}

/// Apply one change on the server, without the account's Trash: the tests' way to change a
/// server's mail.
#[cfg(test)]
pub(crate) fn apply_remote<S: std::io::Read + std::io::Write>(c: &mut Client<S>, op: &Op) -> Result<()> {
    apply_remote_with(c, op, None)
}

/// Apply one change on the server, with the account's Trash path, which permanent deletes on
/// Gmail need.
pub fn apply_remote_with<S: std::io::Read + std::io::Write>(c: &mut Client<S>, op: &Op, trash: Option<&str>) -> Result<()> {
    match op {
        Op::SetFlag { keys, flag, on } => by_folder(c, keys, |c, _, set| c.uid_store(set, *on, flag.imap())),
        // A keyword that isn't an atom would make the command malformed: the engine never sends one.
        Op::SetKeyword { keys, keyword, on } if mailcraft_model::is_keyword(keyword) => by_folder(c, keys, |c, folder, set| {
            // A folder that can't keep a new keyword (no `\*` in its PERMANENTFLAGS) takes the
            // STORE and forgets it: refused here instead, so the user is told and the change
            // undone, not seen to vanish later (issue #28). Taking one off is the same: Dovecot
            // answers OK to a reader's `-FLAGS` and leaves the keyword on.
            if c.seen(folder).is_some_and(|i| !i.keeps_keyword(keyword)) {
                return Err(NetError::No(format!("{folder} can't keep categories (the keyword {keyword}) on its mail")));
            }
            c.uid_store(set, *on, keyword)
        }),
        // IMAP keeps no follow-up days (MailCraft keeps them).
        Op::SetKeyword { .. } | Op::SetFollowUp { .. } | Op::SetReminder { .. } => Ok(()),
        Op::Move { keys, to } => by_folder(c, keys, |c, folder, set| if folder == to { Ok(()) } else { c.uid_move(set, to) }),
        Op::Expunge { keys } => expunge_keys(c, keys, trash),
        Op::CreateFolder { path } => c.create(path),
        Op::RenameFolder { from, to } => c.rename(from, to),
        Op::DeleteFolder { path } => c.delete(path),
        Op::EmptyFolder { path } => c.empty(path),
        Op::MarkFolderRead { path } => c.mark_read(path),
    }
}
