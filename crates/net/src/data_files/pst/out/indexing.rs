//! The import's messages put in the account's search index as their sources are written, so
//! no message is read again for search once it's done. The index is held for the whole import:
//! a pass running for the account is asked to stop as the import begins, and given [`WAIT`] to
//! let go of it. Nothing that goes wrong with the index (held still, a write that fails) costs
//! the import anything: the messages it hasn't got are found by the next pass, which reads them.
//!
//! The index is written on a thread of its own, from what the writer hands it, in the order the
//! writer wrote it: the writer is what an import waits on, and putting the words in the index and
//! writing them out took it 7 to 8% of a 41 GB file's import.

use std::collections::HashSet;
use std::sync::atomic::AtomicU64;
use std::sync::mpsc::{Receiver, SyncSender};
use std::thread::JoinHandle;

use mailcraft_store::StoreError;
use mailcraft_store::search::{self, Ready, Writer};

use super::*;

/// How long an index held by a pass asked to stop is waited for.
#[cfg(not(test))]
const WAIT: Duration = Duration::from_secs(5);
#[cfg(test)]
const WAIT: Duration = Duration::from_millis(300);

/// What the writer hands the index's thread at most before it waits for it: the words of a
/// batch of sources each, or a folder done or missed.
const QUEUED: usize = 32;

/// A message's words, to go into the index: its key, its id's hash and its size.
type Doc = (MessageKey, u64, u32, Ready);

/// What the index's thread is handed.
enum Job {
    /// Messages whose sources are written.
    Add(Vec<Doc>),
    /// A folder done, its list written as of this revision (none: not to be read): marked as
    /// brought up to the mail, unless a message of it isn't in the index.
    Done(String, Option<u64>),
    /// A folder with a message the index hasn't got.
    Missed(String),
    /// The import's end: what the index holds is written.
    End,
}

/// What the writer and the index's thread both see.
#[derive(Default)]
struct Common {
    /// Let go of without an end: the thread does nothing more, not even the jobs it was handed.
    dropped: AtomicBool,
    /// The messages put in the index and written so far.
    indexed: AtomicU64,
}

/// The account's index, held for the import on a thread of its own.
pub(super) struct Index {
    jobs: Option<SyncSender<Job>>,
    thread: Option<JoinHandle<()>>,
    common: Arc<Common>,
    /// The words of the messages whose sources wait to be written, in the order they came.
    waiting: Vec<Doc>,
}

impl Index {
    /// The thread let go of: with an `end`, once it has written what the index holds; without,
    /// as soon as it's done with the job in hand, the jobs queued thrown away. The messages it
    /// put in the index and wrote.
    fn close(&mut self, end: bool) -> u64 {
        if !end {
            self.common.dropped.store(true, Ordering::Relaxed);
        }
        if let Some(jobs) = self.jobs.take()
            && end
        {
            let _ = jobs.send(Job::End);
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        self.common.indexed.swap(0, Ordering::Relaxed)
    }

    fn send(&mut self, job: Job) {
        // The thread gone (it let the index go): nothing more goes to it.
        if self.jobs.as_ref().is_some_and(|jobs| jobs.send(job).is_err()) {
            self.jobs = None;
        }
    }
}

impl Drop for Index {
    /// Let go of without an end (writing stopped, or the import cut off): what the index holds
    /// unwritten goes, as its messages are read again going on.
    fn drop(&mut self) {
        self.close(false);
    }
}

/// The index's thread: the writer's jobs, until the end (what the index holds written) or until
/// the writer lets it go without one (nothing more written, the jobs queued or not). A write that
/// fails, or an index with no room left, lets the index go. The messages it put in the index and
/// wrote are counted in `common` as they're written.
fn run(mut writer: Writer, store: Store, jobs: Receiver<Job>, tally: Arc<Tally>, common: Arc<Common>) {
    let mut added = 0u64;
    let mut missed: HashSet<String> = HashSet::new();
    let dropped = || common.dropped.load(Ordering::Relaxed);
    let flush = |writer: &mut Writer, added: &mut u64| -> bool {
        if dropped() {
            return false;
        }
        let began = Instant::now();
        let wrote = writer.flush(&store);
        tally.since(Phase::Index, began);
        match wrote {
            Ok(_) => {
                common.indexed.fetch_add(std::mem::take(added), Ordering::Relaxed);
                true
            }
            Err(e) => {
                let_go(&e);
                false
            }
        }
    };
    while let Ok(job) = jobs.recv() {
        if dropped() {
            return;
        }
        let began = Instant::now();
        match job {
            Job::Add(docs) => {
                for (key, msgid_hash, size, doc) in docs {
                    match writer.add_ready(&key, msgid_hash, size, doc) {
                        Ok(_) => added = added.saturating_add(1),
                        // No room left (ids, folders): it's to be made again.
                        Err(e) if search::is_damaged(&e) => {
                            let_go(&e);
                            return;
                        }
                        // Its folder's name is too long to keep.
                        Err(e) => {
                            log::info!("import: a message isn't indexed: {e}");
                            missed.insert(key.folder);
                        }
                    }
                }
                tally.since(Phase::Index, began);
                if writer.should_flush() && !flush(&mut writer, &mut added) {
                    return;
                }
            }
            Job::Missed(path) => {
                missed.insert(path);
            }
            Job::Done(path, revision) => {
                if !missed.remove(&path)
                    && let Some(revision) = revision
                    && let Err(e) = writer.mark(&path, revision)
                {
                    log::info!("import: a folder is left for the next pass to look at: {e}");
                }
                tally.since(Phase::Index, began);
            }
            Job::End => {
                flush(&mut writer, &mut added);
                break;
            }
        }
    }
}

fn let_go(e: &StoreError) {
    log::warn!("import: the search index is brought up to the mail after the import: {e}");
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
        let writer = loop {
            match Writer::open(self.store, self.account) {
                Ok(writer) => break Some(writer),
                Err(StoreError::Busy { .. }) if began.elapsed() < WAIT => std::thread::sleep(Duration::from_millis(20)),
                Err(e) => {
                    log::info!("import: the messages are indexed after it, not as they come in: {e}");
                    break None;
                }
            }
        };
        self.tally.since(Phase::Index, began);
        let Some(writer) = writer else { return };
        let (jobs, rx) = std::sync::mpsc::sync_channel(QUEUED);
        let (store, tally, common) = (self.store.clone(), self.tally.clone(), Arc::new(Common::default()));
        let theirs = common.clone();
        match std::thread::Builder::new().name("import-index".into()).spawn(move || run(writer, store, rx, tally, theirs)) {
            Ok(thread) => self.index = Some(Index { jobs: Some(jobs), thread: Some(thread), common, waiting: Vec::new() }),
            Err(e) => log::info!("import: the messages are indexed after it, not as they come in: {e}"),
        }
    }

    /// A message's words (none: they couldn't be made), to go into the index once its source is
    /// written.
    pub(super) fn index_later(&mut self, key: &MessageKey, msgid_hash: u64, size: u32, doc: Option<Ready>) {
        let Some(ix) = &mut self.index else { return };
        match doc {
            Some(doc) => ix.waiting.push((key.clone(), msgid_hash, size, doc)),
            None => ix.send(Job::Missed(key.folder.clone())),
        }
    }

    /// The sources waiting were written (`wrote`), or not: their words go to the index's thread,
    /// or with them.
    pub(super) fn index_written(&mut self, wrote: bool) {
        let Some(ix) = &mut self.index else { return };
        let waiting = std::mem::take(&mut ix.waiting);
        // Not written: not in their folders' lists either.
        if wrote && !waiting.is_empty() {
            let began = Instant::now();
            ix.send(Job::Add(waiting));
            self.tally.since(Phase::Index, began);
        }
    }

    /// The folder at `path` may hold messages the index hasn't got (going on with it after a
    /// crash: what was written past the index's last write isn't in it): not marked when done.
    pub(in super::super) fn index_missed(&mut self, path: &str) {
        if let Some(ix) = &mut self.index {
            ix.send(Job::Missed(path.to_string()));
        }
    }

    /// A folder is done, its list written: marked as brought up to the mail as it now is, so the
    /// next pass passes it over, unless a message of it isn't in the index.
    pub(super) fn index_folder_done(&mut self, path: &str) {
        let Some(ix) = &mut self.index else { return };
        let revision = search::revision(self.store, self.account, path);
        ix.send(Job::Done(path.to_string(), revision));
    }

    /// Writing stopped for want of room: the index let go of at once, nothing more written to it
    /// (what it holds unwritten goes, as its messages are read again going on).
    pub(super) fn index_drop(&mut self) {
        if let Some(mut ix) = self.index.take() {
            self.indexed = self.indexed.saturating_add(ix.close(false));
        }
    }

    /// The end of the import: what the index holds written, and the index let go.
    pub(super) fn index_end(&mut self) {
        if let Some(mut ix) = self.index.take() {
            let began = Instant::now();
            self.indexed = self.indexed.saturating_add(ix.close(true));
            self.tally.since(Phase::Index, began);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::sync_channel;

    use mailcraft_model::search::IndexDoc;
    use mailcraft_store::search::Reader;

    use super::*;

    /// The words of messages `uids`, in a job for the index's thread.
    fn docs(uids: std::ops::Range<u32>) -> Job {
        Job::Add(
            uids.map(|uid| {
                let key = MessageKey { account: "a".into(), folder: "INBOX".into(), validity: 1, uid };
                let doc = IndexDoc { text: "agreed".into(), ..IndexDoc::default() };
                (key, u64::from(uid), 100, Ready::of(&doc))
            })
            .collect(),
        )
    }

    /// The index's thread run over `jobs` (written after every batch), let go of with an end
    /// or not; what it counted as written.
    fn thread_over(store: &Store, jobs: Vec<Job>, dropped: bool) -> u64 {
        let mut writer = Writer::open(store, "a").unwrap();
        writer.hold_at_most(1);
        let (tx, rx) = sync_channel(QUEUED);
        for job in jobs {
            tx.send(job).unwrap();
        }
        drop(tx);
        let common = Arc::new(Common::default());
        common.dropped.store(dropped, Ordering::Relaxed);
        run(writer, store.clone(), rx, Arc::new(Tally::default()), common.clone());
        common.indexed.load(Ordering::Relaxed)
    }

    /// Let go of without an end (writing stopped for want of room), the index's thread writes
    /// nothing more, not even for the jobs it was handed before; what it wrote is counted all
    /// the same.
    #[test]
    fn an_index_let_go_writes_nothing_more() {
        let store = Store::memory();
        assert_eq!(thread_over(&store, vec![docs(0..3)], false), 3);
        assert_eq!(thread_over(&store, vec![docs(3..10), docs(10..12)], true), 0);
        let r = Reader::open(&store, "a").unwrap().expect("an index");
        assert_eq!(r.live(), 3, "written after it was let go");
    }
}
