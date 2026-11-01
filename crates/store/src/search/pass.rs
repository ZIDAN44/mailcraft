//! One pass bringing an account's index up to its mail ([`Pass`]): what the indexer runs, on its
//! threads in the app and a few messages a frame in the browser. The pass plans, makes the
//! changes that read nothing, and is then handed each message to add as its source is read and
//! its text taken out (the reading is the caller's, so it can be spread over threads or frames).
//! It writes a generation whenever it holds enough; one stopped partway keeps what it added, and
//! the next plan finds the rest. Summaries that say otherwise than the index about a message's
//! files are gathered as it goes ([`Pass::take_files`]), for the caller to put right.

use std::collections::BTreeMap;

use mailcraft_model::job::Work;
use mailcraft_model::search::IndexDoc;

use super::*;

/// An account's index held for one pass: let go when dropped.
#[derive(Debug)]
pub struct Pass {
    writer: Writer,
    plan: Plan,
    /// The plan's messages to add, shared with the threads that read them.
    adds: Arc<[Message]>,
    done: u64,
    bytes_done: u64,
    bytes_total: u64,
    failed: u64,
    /// Summaries found to say otherwise about files, not yet taken.
    files: BTreeMap<String, FilesFound>,
}

impl Pass {
    /// Hold `account`'s index ([`StoreError::Busy`] while another holds it), plan what brings
    /// `folders` up to the mail (none: every folder), and make the changes that read nothing.
    pub fn start(store: &Store, account: &str, folders: &[String]) -> Result<Pass> {
        let mut writer = Writer::open(store, account)?;
        let mut plan = plan(store, &writer, account, folders)?;
        plan.apply(&mut writer)?;
        let adds: Arc<[Message]> = std::mem::take(&mut plan.add).into();
        let bytes_total = adds.iter().map(|m| u64::from(m.size)).fold(0, u64::saturating_add);
        let files = std::mem::take(&mut plan.files);
        Ok(Pass { writer, plan, adds, done: 0, bytes_done: 0, bytes_total, failed: 0, files })
    }

    /// The messages to read and add, newest first: each given back by its place here. Shared,
    /// so threads reading them need no copy (a big file's plan holds hundreds of thousands).
    pub fn adds(&self) -> &Arc<[Message]> {
        &self.adds
    }

    /// How far it has come: messages and their bytes handed back, of those to add.
    pub fn work(&self) -> Work {
        let total = u64::try_from(self.adds.len()).unwrap_or(u64::MAX);
        Work { done: self.done, total, bytes_done: self.bytes_done, bytes_total: self.bytes_total }
    }

    /// Messages that couldn't be read for search.
    pub fn failed(&self) -> u64 {
        self.failed
    }

    /// The messages whose summaries say otherwise than the index about carrying files, by
    /// folder, found since this was last asked: the plan's (messages indexed before), then each
    /// message's as it's added.
    pub fn take_files(&mut self) -> BTreeMap<String, FilesFound> {
        std::mem::take(&mut self.files)
    }

    /// The index's generation as last written.
    pub fn generation(&self) -> u64 {
        self.writer.generation()
    }

    /// [`adds`](Pass::adds)`[i]` read: what search keeps of it, or `None` when its source has
    /// gone since the plan (not added: the next plan settles it). Writes a generation when
    /// enough is held, and returns it. An error is the index's: stop the pass.
    pub fn add(&mut self, store: &Store, i: usize, doc: Option<&IndexDoc>) -> Result<Option<u64>> {
        let Some(m) = self.adds.get(i) else { return Ok(None) };
        self.done = self.done.saturating_add(1);
        self.bytes_done = self.bytes_done.saturating_add(u64::from(m.size));
        if let Some(doc) = doc {
            if let Some(files) = files_known(u16::try_from(doc.files).unwrap_or(u16::MAX), doc_bits(doc, false)).filter(|f| *f != m.has_files) {
                // A folder's messages all come from one reading of its list: one validity.
                let found =
                    self.files.entry(m.key.folder.clone()).or_insert_with(|| FilesFound { validity: m.key.validity, ..FilesFound::default() });
                found.note(m.key.uid, files);
            }
            match self.writer.add(&m.key, m.msgid_hash, m.size, doc) {
                Ok(_) => {}
                Err(e) if is_damaged(&e) => return Err(e),
                // Its folder's name is too long to keep: that message isn't indexed.
                Err(e) => {
                    log::warn!("{}: not indexed: {e}", m.key.folder);
                    self.failed = self.failed.saturating_add(1);
                }
            }
        }
        if self.writer.should_flush() { self.writer.flush(store) } else { Ok(None) }
    }

    /// [`adds`](Pass::adds)`[i]` couldn't be read for search (taking its text out failed): kept
    /// as a message with no words and something left out, so a search says it may be missing
    /// rather than read it again, and counted.
    pub fn add_unreadable(&mut self, store: &Store, i: usize) -> Result<Option<u64>> {
        self.failed = self.failed.saturating_add(1);
        self.add(store, i, Some(&IndexDoc { truncated: true, parts_cut: true, ..IndexDoc::default() }))
    }

    /// Write what's held. `complete`: every message to add was handed back, so the folders
    /// planned are marked as brought up (a pass stopped partway marks none: the next plans them
    /// again and finds only what's left). The new generation, if anything was written.
    pub fn finish(&mut self, store: &Store, complete: bool) -> Result<Option<u64>> {
        if complete {
            for (folder, revision) in std::mem::take(&mut self.plan.marks) {
                self.writer.mark(&folder, revision)?;
            }
        }
        self.writer.flush(store)
    }

    /// One [merge step](Writer::merge_step) (`now`: Unix seconds), each written in background
    /// transactions; whether another is due.
    pub fn merge_step(&mut self, store: &Store, now: u64) -> Result<bool> {
        self.writer.merge_step(store, BACKGROUND_COMMIT, now)
    }
}
