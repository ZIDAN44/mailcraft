//! The room left on the drive the account's data file is written to, asked as the import writes:
//! with its first batch, with every checkpoint and every [`ASK_EVERY`] between. Short of
//! [`LOW_SPACE`], the import stops itself as a pause does (what's in is kept and listed, the
//! checkpoint says where it got) and waits for the user, instead of failing every write after and
//! counting the messages as lost.

use std::path::PathBuf;

use super::super::super::space::{Drives, LOW_SPACE, Space};
use super::*;

/// Bytes handed to the store between two asks at most, besides those at the checkpoints: far
/// less than [`LOW_SPACE`], so the drive can't fill between them.
const ASK_EVERY: usize = 64 << 20;

pub(super) struct Room {
    space: Arc<dyn Space>,
    /// The folder the data file is in, or will be.
    dir: PathBuf,
    /// Bytes handed to the store since the room was last asked (none: not yet asked).
    since: Option<usize>,
    /// The room left when the import stopped itself for want of it.
    full: Option<u64>,
}

impl Room {
    pub(super) fn new(store: &Store, account: &str, opts: &ImportOptions) -> Room {
        let space: Arc<dyn Space> = opts.space.clone().unwrap_or_else(|| Arc::new(Drives));
        Room { space, dir: data_dir(store, account), since: None, full: None }
    }

    /// The room left, when it's short of [`LOW_SPACE`] (one that can't be told isn't).
    fn short(&self) -> Option<u64> {
        self.space.free(&self.dir).filter(|free| *free < LOW_SPACE)
    }

    pub(super) fn full(&self) -> Option<u64> {
        self.full
    }
}

/// Where `account`'s data file is, or goes: its folder, else the data files' folder, else the
/// store's own (a cache not kept in data files).
fn data_dir(store: &Store, account: &str) -> PathBuf {
    store
        .data_file(account)
        .and_then(|f| f.parent().map(Path::to_path_buf))
        .or_else(|| store.data_files_dir())
        .unwrap_or_else(|| store.root().to_path_buf())
}

impl Out<'_> {
    /// The room left before anything is read, when it's too little to begin.
    pub(in super::super) fn short_of_room(&self) -> Option<u64> {
        self.room.short()
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
        if let Some(free) = self.room.short() {
            log::warn!("import: {} MiB left on the drive; stopped until there's more", free >> 20);
            self.room.full = Some(free);
            self.stop.raise(StopHow::Pause);
        }
    }
}
