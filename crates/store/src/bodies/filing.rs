//! Mail filed in a folder kept on this computer, its sources written together ([`Filing`]).

use super::*;

/// A batch of sources is written once it holds this many, or this many bytes of them.
const BATCH: usize = 1000;
const BATCH_BYTES: usize = 16 << 20;

/// Mail being filed in one folder of an account kept on this computer: each message numbered
/// past the folder's list ([`Filing::key`]), its source written with others in batches
/// ([`Filing::add`]), and the list naming them all saved last ([`Filing::finish`]). A filing given
/// up (an error, or dropped unfinished) takes away the sources it wrote, and only those: no list
/// names them.
///
/// More than a batch of sources is a long write ([`Store::long_write`], handed back by
/// [`Filing::finish`] to keep until the rest of the change is saved): its batches don't wait for
/// the disk, and the last, which the list counts on, is a sealed commit that waits for it,
/// refused if the data file lost what didn't (opened again after a failed write) meanwhile. Each
/// batch waiting for the disk unsealed, a crash part-way checked the whole file as it opened
/// again (105 s for 37 GB); the batches before the last are lost to a crash, but no list names
/// them yet.
pub struct Filing<'s> {
    store: &'s Store,
    account: String,
    folder: String,
    list: FolderState,
    /// The next number this filing gives.
    next: u32,
    packs: bool,
    batch: Vec<(MessageKey, Prepared)>,
    bytes: usize,
    /// The long write, and how often its data file had been let go of after a failed write as
    /// it began.
    long: Option<(LongWrite<'s>, u64)>,
    /// The numbers of the sources written (or tried) that only the list, once saved, names.
    written: Vec<u32>,
    saved: bool,
}

impl<'s> Filing<'s> {
    /// Filing in `account`'s `folder`: its list read first (one that can't be read is an error,
    /// never taken as empty: saved so, the folder's mail was gone).
    pub fn new(store: &'s Store, account: &str, folder: &str) -> Result<Self> {
        let mut list = store.load_folder(account, folder)?;
        if list.validity == 0 {
            list.validity = 1;
        }
        let next = list.free_uid(account, folder)?;
        Ok(Filing {
            store,
            account: account.to_string(),
            folder: folder.to_string(),
            list,
            next,
            packs: store.packs(account),
            batch: Vec::new(),
            bytes: 0,
            long: None,
            written: Vec::new(),
            saved: false,
        })
    }

    /// The key of the next message filed; an error, before anything of it is written, once the
    /// folder has no numbers left.
    pub fn key(&mut self) -> Result<MessageKey> {
        let uid = FolderState::take_uid(&mut self.next, &self.account, &self.folder)?;
        Ok(MessageKey { account: self.account.clone(), folder: self.folder.clone(), validity: self.list.validity, uid })
    }

    /// File `m`, keyed by [`Filing::key`], with its source `raw` (one never downloaded is filed
    /// without it).
    pub fn add(&mut self, m: Summary, raw: Option<Vec<u8>>) -> Result<()> {
        if let Some(raw) = raw {
            // A full batch goes before this one, so the last batch is never empty.
            if !self.batch.is_empty() && (self.batch.len() >= BATCH || self.bytes.saturating_add(raw.len()) > BATCH_BYTES) {
                if self.long.is_none() {
                    let began = self.store.long_write(&self.account);
                    self.long = Some((began, self.store.data_file_reopened(&self.account)));
                }
                self.written.extend(self.batch.iter().map(|(key, _)| key.uid));
                self.store.write_prepared(std::mem::take(&mut self.batch), false)?;
                self.bytes = 0;
            }
            self.bytes = self.bytes.saturating_add(raw.len());
            self.batch.push((m.key.clone(), Store::prepare(self.packs, raw)));
        }
        self.list.messages.push(m.into());
        Ok(())
    }

    /// The last batch of sources written, then the list naming them saved. Returns the long
    /// write, if it became one, to keep until the rest of the change (deleting what was moved)
    /// is saved.
    pub fn finish(mut self) -> Result<Option<LongWrite<'s>>> {
        let batch = std::mem::take(&mut self.batch);
        if !batch.is_empty() {
            self.written.extend(batch.iter().map(|(key, _)| key.uid));
            match &self.long {
                Some((_, reopened)) => self.store.write_batch(&self.account, batch, Vec::new(), *reopened)?,
                None => self.store.write_prepared(batch, true)?,
            }
        }
        self.list.uid_next = self.next;
        sort_newest_first(&mut self.list.messages);
        self.store.save_folder(&self.account, &self.folder, &self.list)?;
        self.saved = true;
        Ok(self.long.take().map(|(l, _)| l))
    }
}

impl Drop for Filing<'_> {
    fn drop(&mut self) {
        if self.saved || self.written.is_empty() {
            return;
        }
        // Its own sources, not every one numbered past where it began: another writer's there
        // (an import going on past the list) went with them. Best effort: what stays is written
        // over by the next filing here (it takes the same numbers), and no list names it
        // meanwhile.
        let key = |uid: &u32| MessageKey { account: self.account.clone(), folder: self.folder.clone(), validity: self.list.validity, uid: *uid };
        let gone: Vec<PathBuf> = self.written.iter().map(|uid| self.store.body_path(&key(uid))).filter(|p| self.store.files.is_file(p)).collect();
        if !gone.is_empty() {
            let _ = self.store.files.replace(&self.store.account_dir(&self.account), None, &gone, false);
        }
    }
}

#[cfg(test)]
mod tests;
