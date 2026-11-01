//! Changes to a server account's mail (flags, moves, folders) kept on a thread of the pool's
//! own: made in the cache and queued for the server there, in the order they were made, then the
//! account's thread woken to send them.
//!
//! They were made on the caller's thread, which is the window's: a read mark on a folder of ten
//! thousand messages read and wrote its whole list there, every Down that opened one. The
//! account's own thread can't do it, as it may be minutes into a full sync (and the change has
//! to be in the cache before a folder refresh reads it). One thread for every account keeps the
//! order they were made in; each change says `FolderChanged` once it's on disk, as before (the
//! session knows its own changes coming back, `session/mailboxes/echo.rs`).

use std::panic::{AssertUnwindSafe, catch_unwind};

use super::*;

/// How long [`Edits::flush`] waits for the changes before it to be on disk.
const FLUSH_WAIT: Duration = Duration::from_secs(10);

/// The thread each change was made on, by account (tests).
#[cfg(test)]
pub(super) static APPLIED_ON: Mutex<Vec<(String, Option<String>)>> = Mutex::new(Vec::new());

enum Edit {
    /// A change to make, and the account's thread to wake after (none when it isn't running).
    Apply { account: String, op: Op, wake: Option<Sender<Job>> },
    /// Answered once every change before it is on disk.
    Flush(Sender<()>),
}

/// The pool's thread for changes, started with the first one.
#[derive(Default)]
pub(super) struct Edits {
    tx: Option<Sender<Edit>>,
    thread: Option<JoinHandle<()>>,
}

impl Edits {
    /// Make `op` in the cache and queue it for the server, after the changes before it; on this
    /// thread when there's no other to make it on.
    pub(super) fn send(&mut self, store: &Store, sink: &Sink, account: String, op: Op, wake: Option<Sender<Job>>) {
        if self.tx.is_none() {
            self.start(store, sink);
        }
        let edit = Edit::Apply { account, op, wake };
        let edit = match self.tx.as_ref() {
            Some(tx) => match tx.send(edit) {
                Ok(()) => return,
                // Ended (it never should): started again with the next one.
                Err(mpsc::SendError(edit)) => {
                    self.tx = None;
                    edit
                }
            },
            None => edit,
        };
        if let Edit::Apply { account, op, wake } = edit {
            apply(store, sink, &account, op, wake);
        }
    }

    fn start(&mut self, store: &Store, sink: &Sink) {
        let (tx, rx) = mpsc::channel::<Edit>();
        let (store, sink) = (store.clone(), sink.clone());
        let spawned = std::thread::Builder::new().name("mail-edits".into()).spawn(move || {
            for edit in rx {
                match edit {
                    Edit::Apply { account, op, wake } => {
                        // A change that panics is lost, never the ones after it.
                        let folders = op.keys().iter().map(|k| k.folder.clone()).collect::<Vec<_>>();
                        if catch_unwind(AssertUnwindSafe(|| apply(&store, &sink, &account, op, wake))).is_err() {
                            log::error!("{account}: a change to mail failed unexpectedly");
                            echo_failed(&sink, &account, folders, "a change to mail failed unexpectedly".into());
                        }
                    }
                    Edit::Flush(done) => {
                        let _ = done.send(());
                    }
                }
            }
        });
        match spawned {
            Ok(t) => {
                self.tx = Some(tx);
                self.thread = Some(t);
            }
            Err(e) => log::warn!("changes to mail: {e}"),
        }
    }

    /// Wait (a while at most) until the changes made so far are on disk: before what reads the
    /// cache as a whole (an export).
    pub(super) fn flush(&self) {
        let Some(tx) = self.tx.as_ref() else { return };
        let (done, wait) = mpsc::channel();
        if tx.send(Edit::Flush(done)).is_ok() && wait.recv_timeout(FLUSH_WAIT).is_err() {
            log::warn!("changes to mail: still being saved after {} s", FLUSH_WAIT.as_secs());
        }
    }

    /// Every change made on disk before the app goes: the thread ends with its queue.
    pub(super) fn finish(&mut self) {
        drop(self.tx.take());
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Make the change in the cache, queue it for the server and wake the account's thread to send
/// it; each folder it changed is said to have changed.
fn apply(store: &Store, sink: &Sink, account: &str, op: Op, wake: Option<Sender<Job>>) {
    #[cfg(test)]
    if let Ok(mut on) = APPLIED_ON.lock() {
        on.push((account.to_string(), std::thread::current().name().map(str::to_string)));
    }
    match store.apply_op(account, &op) {
        Ok(changed) => {
            if op.is_folder_op() {
                let folders = store.load_folders(account).unwrap_or_default();
                sink.emit(Event::Folders { account: account.to_string(), folders });
            }
            if let Err(e) = store.add_op(account, op.clone()) {
                sink.emit(Event::Notice { account: account.to_string(), message: e.to_string() });
            }
            for folder in changed {
                sink.emit(Event::FolderChanged { account: account.to_string(), folder });
            }
        }
        Err(e) => {
            let folders = op.keys().iter().map(|k| k.folder.clone()).collect();
            echo_failed(sink, account, folders, e.to_string());
        }
    }
    if let Some(tx) = wake {
        let _ = tx.send(Job::Apply { account: account.to_string(), op });
    }
}

/// A change that didn't reach the cache: said, and its folders read again (the window has it
/// already, which the disk doesn't).
fn echo_failed(sink: &Sink, account: &str, mut folders: Vec<String>, message: String) {
    sink.emit(Event::Notice { account: account.to_string(), message });
    folders.sort();
    folders.dedup();
    for folder in folders {
        sink.emit(Event::FolderChanged { account: account.to_string(), folder });
    }
}
