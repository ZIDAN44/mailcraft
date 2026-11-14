//! Bringing a folder's cache up to date from Microsoft Graph: what Graph says, for the sync the
//! Exchange transports share (`exchange::window`).

use std::collections::HashSet;

use super::*;
use crate::exchange::window::{self, Window};

/// Bring one folder's cache up to date.
pub fn sync_folder(c: &Client, st: &mut State, store: &Store, account: &str, folder: &str, opts: &SyncOptions) -> Result<SyncReport> {
    sync_folder_as(c, st, store, account, folder, opts, store.removals(account))
}

/// [`sync_folder`] for a worker that started when its account had been removed `seen` times: a
/// removal since, while the sync talks to the server, has it write nothing of the account again.
pub fn sync_folder_as(c: &Client, st: &mut State, store: &Store, account: &str, folder: &str, opts: &SyncOptions, seen: u64) -> Result<SyncReport> {
    window::sync_folder_as(c, st, store, account, folder, opts, seen)
}

impl Window for Client {
    type Item = Value;

    fn newest_in(&self, id: &str, window: usize) -> Result<(Vec<(String, Flags)>, (u32, u32), Vec<Value>)> {
        let (listed, counts) = self.newest(id, window)?;
        let newest = listed.iter().filter_map(|m| Some((m.get("id")?.as_str()?.to_string(), flags(m)))).collect();
        Ok((newest, counts, listed))
    }

    fn still_in_folder(&self, id: &str, ids: &[String]) -> Result<Vec<(String, Flags)>> {
        self.still_in(id, ids)
    }

    /// Graph says whether it's read, flagged (and done) or a draft; it has no answered flag or
    /// categories here, which stay as they are.
    fn merged(&self, kept: &Flags, server: &Flags) -> Flags {
        let mut next = Flags { seen: server.seen, flagged: server.flagged, draft: server.draft, ..kept.clone() };
        next.set_keyword(mailcraft_model::FOLLOW_UP_COMPLETE, server.follow_up_complete());
        next
    }

    /// A small gap: per-message GETs with internet headers. A big one: the listing's entries.
    fn headers_of(&self, missing: &[String], listed: Vec<Value>) -> Result<Vec<Value>> {
        if missing.len() <= FEW {
            return self.headers(missing);
        }
        let wanted: HashSet<&str> = missing.iter().map(String::as_str).collect();
        Ok(listed.into_iter().filter(|m| m["id"].as_str().is_some_and(|i| wanted.contains(i))).collect())
    }

    fn id_of(item: &Value) -> Option<&str> {
        item.get("id").and_then(Value::as_str)
    }

    fn summary(item: &Value, key: MessageKey) -> Summary {
        message_summary(item, key)
    }

    /// Graph lists no sizes: mail with attachments waits until it's opened.
    fn ahead(&self, m: &Summary, _opts: &SyncOptions) -> bool {
        !m.has_attachments
    }

    fn source(&self, id: &str, opts: &SyncOptions) -> Result<Option<Vec<u8>>> {
        let raw = self.download(id)?;
        Ok((raw.len() <= usize::try_from(opts.prefetch_max_bytes).unwrap_or(usize::MAX)).then_some(raw))
    }
}
