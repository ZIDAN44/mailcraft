//! Bringing a folder's cache up to date from the Exchange server: what EWS says, for the sync the
//! Exchange transports share (`exchange::window`).

use super::*;
use crate::exchange::window::{self, Window};

/// Bring one folder's cache up to date.
pub fn sync_folder(c: &Client, st: &mut State, store: &Store, account: &str, folder: &str, opts: &SyncOptions) -> Result<SyncReport> {
    sync_folder_as(c, st, store, account, folder, opts, store.removals(account))
}

/// [`sync_folder`] for a worker that started when its account had been removed `seen` times: a
/// removal since, while the sync talks to the server, has it write nothing of the account again.
pub fn sync_folder_as(c: &Client, st: &mut State, store: &Store, account: &str, folder: &str, opts: &SyncOptions, seen: u64) -> Result<SyncReport> {
    let id = st.mailbox(folder)?.to_string();
    c.in_folder(&id, || window::sync_folder_as(c, st, store, account, folder, opts, seen))
}

impl Window for Client {
    type Item = El;

    fn newest_in(&self, id: &str, window: usize) -> Result<(Vec<(String, Flags)>, (u32, u32), Vec<El>)> {
        let newest = self.newest(id, window)?;
        Ok((newest, self.counts(id)?, Vec::new()))
    }

    fn still_in_folder(&self, id: &str, ids: &[String]) -> Result<Vec<(String, Flags)>> {
        self.still_in(id, ids)
    }

    /// EWS says whether it's read, flagged or a draft; answered and categories stay as they are.
    fn merged(&self, kept: &Flags, server: &Flags) -> Flags {
        Flags { seen: server.seen, flagged: server.flagged, draft: server.draft, ..kept.clone() }
    }

    fn headers_of(&self, missing: &[String], _listed: Vec<El>) -> Result<Vec<El>> {
        self.headers(missing)
    }

    fn id_of(item: &El) -> Option<&str> {
        item.child("ItemId").and_then(|i| i.attr("Id"))
    }

    fn summary(item: &El, key: MessageKey) -> Summary {
        item_summary(item, key)
    }

    fn ahead(&self, m: &Summary, opts: &SyncOptions) -> bool {
        u64::from(m.size) <= u64::from(opts.prefetch_max_bytes)
    }

    fn source(&self, id: &str, _opts: &SyncOptions) -> Result<Option<Vec<u8>>> {
        self.download(id).map(Some)
    }
}
