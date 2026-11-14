//! The changes waiting for the server, as the account's thread replays them ([`Waiting`]): read
//! from `ops.json` once, then each taken off it as it's made, without reading them all again.

use std::collections::VecDeque;

use super::*;

/// The changes waiting for the server, oldest first, as `ops.json` had them when last read or
/// written here ([`Store::waiting_ops`]). Taking the first off ([`Store::op_made`]) writes the
/// rest at once, so a stop part-way leaves on disk just the ones not made yet; what another
/// wrote to the file meanwhile (a change queued by the window) is read in then, not lost.
#[derive(Debug, Default)]
pub struct Waiting {
    ops: VecDeque<Op>,
    /// Each change as written, for writing the rest without making them again.
    written: VecDeque<Vec<u8>>,
    /// `ops.json` as last read or written here; `None` when there was none.
    seen: Option<Vec<u8>>,
}

impl Waiting {
    /// The oldest change waiting.
    pub fn first(&self) -> Option<&Op> {
        self.ops.front()
    }

    /// The changes waiting, oldest first.
    pub fn iter(&self) -> impl Iterator<Item = &Op> {
        self.ops.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }
}

impl Store {
    /// The changes waiting for `account`'s server, to replay them oldest first. A damaged
    /// `ops.json` is set aside and reads as none, as [`Store::load_ops`].
    pub fn waiting_ops(&self, account: &str) -> Result<Waiting> {
        let _held = self.hold_changes();
        self.waiting_held(account)
    }

    /// [`Store::waiting_ops`], with the changes' lock held.
    fn waiting_held(&self, account: &str) -> Result<Waiting> {
        let path = self.account_dir(account).join("ops.json");
        let seen = match self.files.read(&path) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => return Err(io_err(&path)(e)),
        };
        let ops: Vec<Op> = match &seen {
            Some(bytes) => match serde_json::from_slice(bytes) {
                Ok(ops) => ops,
                // Set aside as any damaged cache file is, and read as none.
                Err(_) => return Ok(Waiting { seen: None, ..Waiting::default() }.with(self.load_ops(account)?)),
            },
            None => Vec::new(),
        };
        Ok(Waiting { seen, ..Waiting::default() }.with(ops))
    }

    /// The first of `waiting` was made on the server (or refused there): taken off `ops.json`,
    /// the rest written at once. When the file isn't as `waiting` last saw it (another wrote it
    /// meanwhile: a change queued, the account removed), it is read again: the first entry equal
    /// to the change made comes off, as [`Store::remove_op`] does, and `waiting` becomes what's
    /// left, the changes queued meanwhile with it.
    pub fn op_made(&self, account: &str, waiting: &mut Waiting) -> Result<()> {
        let _held = self.hold_changes();
        let path = self.account_dir(account).join("ops.json");
        let now = match self.files.read(&path) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => return Err(io_err(&path)(e)),
        };
        if now.is_some() && now == waiting.seen {
            // As seen: the rest are the file's, written without the first.
            let rest = waiting.written.len().saturating_sub(1);
            if rest == 0 {
                self.save_ops(account, &[])?;
                *waiting = Waiting::default();
                return Ok(());
            }
            let size = waiting.written.iter().skip(1).map(|w| w.len().saturating_add(1)).sum::<usize>().saturating_add(1);
            let mut bytes = Vec::with_capacity(size);
            bytes.push(b'[');
            for (i, w) in waiting.written.iter().skip(1).enumerate() {
                if i > 0 {
                    bytes.push(b',');
                }
                bytes.extend_from_slice(w);
            }
            bytes.push(b']');
            self.files.write(&path, &bytes)?;
            waiting.ops.pop_front();
            waiting.written.pop_front();
            waiting.seen = Some(bytes);
            return Ok(());
        }
        // Written by another meanwhile: as remove_op, then read again.
        if let Some(op) = waiting.ops.front() {
            let mut ops = self.load_ops(account)?;
            if let Some(i) = ops.iter().position(|o| o == op) {
                ops.remove(i);
                self.save_ops(account, &ops)?;
            }
        }
        *waiting = self.waiting_held(account)?;
        Ok(())
    }
}

impl Waiting {
    /// These changes, each as written.
    fn with(mut self, ops: Vec<Op>) -> Waiting {
        let mut written = VecDeque::with_capacity(ops.len());
        for op in &ops {
            // An `Op` always serializes; were one not to, nothing seen means the rest are
            // never written from these: the file is read again for each taken off.
            match serde_json::to_vec(op) {
                Ok(w) => written.push_back(w),
                Err(_) => {
                    self.seen = None;
                    written.push_back(Vec::new());
                }
            }
        }
        self.ops = ops.into();
        self.written = written;
        self
    }
}

#[cfg(test)]
mod tests;
