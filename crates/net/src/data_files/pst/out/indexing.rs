//! The import's messages put in the account's search index as their sources are written, so
//! no message is read again for search once it's done. The index is held for the whole import:
//! a pass running for the account is asked to stop as the import begins, and given [`WAIT`] to
//! let go of it. Nothing that goes wrong with the index (held still, a write that fails) costs
//! the import anything: the messages it hasn't got are found by the next pass, which reads them.

use std::collections::HashSet;

use mailcraft_store::StoreError;
use mailcraft_store::search::{self, Ready, Writer};

use super::*;

/// How long an index held by a pass asked to stop is waited for.
#[cfg(not(test))]
const WAIT: Duration = Duration::from_secs(5);
#[cfg(test)]
const WAIT: Duration = Duration::from_millis(300);

/// The account's index, held for the import.
pub(super) struct Index {
    writer: Writer,
    /// The words of the messages whose sources wait to be written, in the order they came.
    waiting: Vec<(MessageKey, u64, u32, Ready)>,
    /// Messages added since the index was last written.
    added: u64,
    /// Folders with a message the index hasn't got: not marked as brought up to the mail, so the
    /// next pass looks at them.
    missed: HashSet<String>,
}

impl Out<'_> {
    /// Whether the messages go into the search index as they're written.
    pub(in super::super) fn indexing(&self) -> bool {
        self.index.is_some()
    }

    /// Hold the account's index for the import, waiting up to [`WAIT`] for a pass to let go of
    /// it. Held still, or not to be read, the import goes on without it.
    pub(in super::super) fn open_index(&mut self) {
        let began = Instant::now();
        loop {
            match Writer::open(self.store, self.account) {
                Ok(writer) => {
                    self.index = Some(Index { writer, waiting: Vec::new(), added: 0, missed: HashSet::new() });
                    break;
                }
                Err(StoreError::Busy { .. }) if began.elapsed() < WAIT => std::thread::sleep(Duration::from_millis(20)),
                Err(e) => {
                    log::info!("import: the messages are indexed after it, not as they come in: {e}");
                    break;
                }
            }
        }
        self.tally.since(Phase::Index, began);
    }

    /// A message's words (none: they couldn't be made), to go into the index once its source is
    /// written.
    pub(super) fn index_later(&mut self, key: &MessageKey, msgid_hash: u64, size: u32, doc: Option<Ready>) {
        let Some(ix) = &mut self.index else { return };
        match doc {
            Some(doc) => ix.waiting.push((key.clone(), msgid_hash, size, doc)),
            None => {
                ix.missed.insert(key.folder.clone());
            }
        }
    }

    /// The sources waiting were written (`wrote`), or not: their words go into the index, or
    /// with them. The index is written when it holds enough.
    pub(super) fn index_written(&mut self, wrote: bool) {
        let Some(ix) = &mut self.index else { return };
        let waiting = std::mem::take(&mut ix.waiting);
        // Not written: not in their folders' lists either.
        if !wrote {
            return;
        }
        let began = Instant::now();
        let mut broken = None;
        for (key, msgid_hash, size, doc) in waiting {
            match ix.writer.add_ready(&key, msgid_hash, size, doc) {
                Ok(_) => ix.added = ix.added.saturating_add(1),
                // No room left (ids, folders): it's to be made again.
                Err(e) if search::is_damaged(&e) => {
                    broken = Some(e);
                    break;
                }
                // Its folder's name is too long to keep.
                Err(e) => {
                    log::info!("import: a message isn't indexed: {e}");
                    ix.missed.insert(key.folder);
                }
            }
        }
        let due = ix.writer.should_flush();
        self.tally.since(Phase::Index, began);
        match broken {
            Some(e) => self.let_index_go(&e),
            None if due => self.index_flush(),
            None => {}
        }
    }

    /// A folder is done, its list written: marked as brought up to the mail as it now is, so the
    /// next pass passes it over, unless a message of it isn't in the index.
    pub(super) fn index_folder_done(&mut self, path: &str) {
        let Some(ix) = &mut self.index else { return };
        if ix.missed.remove(path) {
            return;
        }
        let began = Instant::now();
        if let Some(revision) = search::revision(self.store, self.account, path)
            && let Err(e) = ix.writer.mark(path, revision)
        {
            log::info!("import: a folder is left for the next pass to look at: {e}");
        }
        self.tally.since(Phase::Index, began);
    }

    /// What the index holds written, in short transactions with its meta last. One that fails
    /// lets the index go: the rest of the import goes in without it.
    fn index_flush(&mut self) {
        let Some(ix) = &mut self.index else { return };
        let began = Instant::now();
        let wrote = ix.writer.flush(self.store);
        self.tally.since(Phase::Index, began);
        match wrote {
            Ok(_) => self.indexed = self.indexed.saturating_add(std::mem::take(&mut ix.added)),
            Err(e) => self.let_index_go(&e),
        }
    }

    fn let_index_go(&mut self, e: &StoreError) {
        log::warn!("import: the search index is brought up to the mail after the import: {e}");
        self.index = None;
    }

    /// The end of the import: what the index holds written, and the index let go.
    pub(super) fn index_end(&mut self) {
        self.index_flush();
        self.index = None;
    }
}
