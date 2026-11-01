//! The room left on the drive the account's data file is written to, asked as the import writes:
//! with its first batch, with every checkpoint and every [`ASK_EVERY`] between. Short of what the
//! next steps can take ([`LOW_SPACE`](space::LOW_SPACE), and as much again as the data file can
//! grow by in one go), the import stops itself as a pause does (what's in is kept and listed, the
//! checkpoint says where it got) and waits for the user, instead of failing every write after and
//! counting the messages as lost. A write the drive refuses all the same (something else filled it
//! meanwhile) stops it too: nothing more is written, and it goes on from its last checkpoint. So
//! does the data file being opened again after any write to it failed at the disk (the import's,
//! the window's, the seal's): it's back at its last durable commit, and what the import wrote
//! without waiting for the disk since may be gone, so no checkpoint may name it.

use std::path::PathBuf;

use super::super::super::space::{self, Drives, Space, data_dir};
use super::*;

/// Bytes handed to the store between two asks at most, besides those at the checkpoints: far
/// less than [`LOW_SPACE`](space::LOW_SPACE), so what's written between them can't take the
/// margin.
const ASK_EVERY: usize = 64 << 20;

pub(super) struct Room {
    space: Arc<dyn Space>,
    /// The folder the data file is in, or will be.
    dir: PathBuf,
    /// Bytes handed to the store since the room was last asked (none: not yet asked).
    since: Option<usize>,
    /// The room left when the import stopped itself for want of it.
    full: Option<u64>,
    /// A write was refused for want of room, or the data file was opened again after a failed
    /// write: nothing more is written in this run.
    refused: bool,
    /// How many times the data file had been opened again after a failed write as the import
    /// began ([`Store::data_file_reopened`]).
    reopened: u64,
    /// The messages in, and the items left out, as the last checkpoint written says.
    kept: (usize, usize),
}

impl Room {
    pub(super) fn new(store: &Store, account: &str, opts: &ImportOptions) -> Room {
        let space: Arc<dyn Space> = opts.space.clone().unwrap_or_else(|| Arc::new(Drives));
        let reopened = store.data_file_reopened(account);
        Room { space, dir: data_dir(store, account), since: None, full: None, refused: false, reopened, kept: (0, 0) }
    }

    /// The room left, when it's short of what writing on in the data file `file` needs.
    fn short(&self, file: Option<&Path>) -> Option<u64> {
        space::short(self.space.as_ref(), &self.dir, file)
    }

    pub(super) fn full(&self) -> Option<u64> {
        self.full
    }
}

/// Whether a write failed for want of room on the drive.
pub(super) fn for_want_of_room(e: &mailcraft_store::StoreError) -> bool {
    space::for_want_of_room(e)
}

impl Out<'_> {
    /// The room left before anything is read, when it's too little to begin.
    pub(in super::super) fn short_of_room(&self) -> Option<u64> {
        self.room.short(self.store.data_file(self.account).as_deref())
    }

    /// A batch of `bytes` about to be written (with a checkpoint when `checkpoint`): the room
    /// left is asked when it's due, and short of it the import is stopped as if paused. The
    /// batch in hand still goes in: nothing read is lost, and nothing is counted as left out.
    pub(super) fn mind_room(&mut self, bytes: usize, checkpoint: bool) {
        if self.room.full.is_some() {
            return;
        }
        let since = self.room.since.map_or(usize::MAX, |n| n.saturating_add(bytes));
        if !checkpoint && since < ASK_EVERY {
            self.room.since = Some(since);
            return;
        }
        self.room.since = Some(0);
        if let Some(free) = self.short_of_room() {
            log::warn!("import: {} MiB left on the drive; stopped until there's more", free >> 20);
            self.room.full = Some(free);
            self.stop.raise(StopHow::Pause);
        }
    }

    /// A write the drive refused for want of room. The store opens the data file again at its
    /// last durable commit, which the last checkpoint went with, so nothing more is written in
    /// this run (a checkpoint now would name what isn't there): the import stops as if paused,
    /// and goes on from its last checkpoint.
    pub(super) fn out_of_room(&mut self) {
        if self.room.refused {
            return;
        }
        let free = self.room.space.free(&self.room.dir).unwrap_or(0);
        log::warn!("import: the drive is full ({} MiB left); stopped until there's more", free >> 20);
        self.room.refused = true;
        self.room.full.get_or_insert(free);
        self.stop.raise(StopHow::Pause);
    }

    /// Whether writing has stopped: for want of room, or because the data file was opened again
    /// after a failed write since the import began (whoever's write it was). Then nothing more
    /// goes to the store, and the import stops as if paused, to go on from its last checkpoint
    /// (said as a full drive when the drive is short of room).
    pub(in super::super) fn writes_stopped(&mut self) -> bool {
        if !self.room.refused && self.store.data_file_reopened(self.account) != self.room.reopened {
            match self.short_of_room() {
                Some(_) => self.out_of_room(),
                None => {
                    log::warn!("import: its data file was opened again after a write failed; stopped, to go on from the last checkpoint");
                    self.room.refused = true;
                    self.stop.raise(StopHow::Pause);
                }
            }
        }
        self.room.refused
    }

    /// The data file's count of times opened again after a failed write as the import began: a
    /// checkpoint is written only while it's still that ([`Store::write_batch`]).
    pub(in super::super) fn reopened(&self) -> u64 {
        self.room.reopened
    }

    /// A checkpoint was written, or one gone on from: what it counts is what the run comes to
    /// when a write is refused.
    pub(in super::super) fn room_kept(&mut self) {
        self.room.kept = (self.imported, self.skipped);
    }

    /// The messages in and the items left out when writing stopped for want of room: as the
    /// last checkpoint counted them (what came after it goes again).
    pub(super) fn kept_when_refused(&self) -> Option<(usize, usize)> {
        self.room.refused.then_some(self.room.kept)
    }
}
