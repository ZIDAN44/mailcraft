//! The store: message bodies, changes waiting for the server, and the Outbox.

use crate::*;

/// A message source made ready for the store away from its writer ([`Store::prepare`]): as a data
/// file keeps it (compressed when that helps), or as it is for other stores.
#[derive(Clone, Debug)]
pub struct Prepared {
    pub(crate) bytes: Vec<u8>,
    /// `bytes` are as a data file keeps them, not the source itself.
    pub(crate) kept: bool,
    raw_len: usize,
}

impl Prepared {
    /// The source's own size.
    pub fn raw_len(&self) -> usize {
        self.raw_len
    }

    /// What the store will keep of it.
    pub fn kept_len(&self) -> usize {
        self.bytes.len()
    }

    /// The source itself, read back out of what was made ready (to be changed and made ready
    /// again: rare, it undoes the compression).
    pub fn source(&self) -> io::Result<Vec<u8>> {
        self.clone().into_raw()
    }

    /// The source itself again, for a store that keeps files as they are.
    pub(crate) fn into_raw(self) -> io::Result<Vec<u8>> {
        if !self.kept {
            return Ok(self.bytes);
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            packed::unpack(self.bytes)
        }
        // Nothing is kept for a data file in the browser.
        #[cfg(target_arch = "wasm32")]
        {
            Err(io::Error::new(io::ErrorKind::InvalidData, "kept for a data file"))
        }
    }
}

impl Store {
    pub fn read_body(&self, key: &MessageKey) -> Option<Vec<u8>> {
        self.reads.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.files.read(&self.body_path(key)).ok()
    }

    /// A message's source, `None` when it isn't kept here (never downloaded, or damaged past
    /// reading: like a damaged cache file, never worth stopping over); an error when it is but
    /// couldn't be read now, so that what goes on to rewrite it stops rather than lose it.
    pub fn load_body(&self, key: &MessageKey) -> Result<Option<Vec<u8>>> {
        self.reads.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = self.body_path(key);
        match self.files.read(&path) {
            Ok(raw) => Ok(Some(raw)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) if e.kind() == io::ErrorKind::InvalidData => {
                log::warn!("{}: damaged, taken as missing: {e}", path.display());
                Ok(None)
            }
            Err(e) => Err(io_err(&path)(e)),
        }
    }

    /// How many message sources were read since the cache opened (clones count together): a
    /// search that needs none can be shown to read none.
    pub fn reads(&self) -> u64 {
        self.reads.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn has_body(&self, key: &MessageKey) -> bool {
        self.files.is_file(&self.body_path(key))
    }

    pub fn write_body(&self, key: &MessageKey, raw: &[u8]) -> Result<()> {
        self.files.write(&self.body_path(key), raw)
    }

    /// Many message sources at once: one transaction for all of them in a data file (importing
    /// hundreds of thousands, a commit each spent most of the time waiting for the disk). Each is
    /// compressed before the transaction starts, so the data file is held only to store them.
    pub fn write_bodies(&self, bodies: &[(MessageKey, Vec<u8>)]) -> Result<()> {
        let prepared = bodies.iter().map(|(k, raw)| (k.clone(), Store::prepare(self.packs(&k.account), raw.clone()))).collect();
        self.write_prepared(prepared, true)
    }

    /// Whether `account`'s sources go into a data file (so are worth preparing off its writer).
    pub fn packs(&self, account: &str) -> bool {
        self.files.packs(&self.account_dir(account))
    }

    /// `raw` made ready for the store, on any thread and without touching the store: compressed
    /// as a data file keeps it when `packs` ([`Store::packs`]), as it is otherwise.
    pub fn prepare(packs: bool, raw: Vec<u8>) -> Prepared {
        let raw_len = raw.len();
        #[cfg(not(target_arch = "wasm32"))]
        if packs {
            return Prepared { bytes: packed::pack_owned(raw), kept: true, raw_len };
        }
        let _ = packs;
        Prepared { bytes: raw, kept: false, raw_len }
    }

    /// Sources made ready with [`Store::prepare`], written as they are (not copied) in one
    /// transaction per data file. Not `durable`, the commit doesn't wait for the disk: the next
    /// durable one (any other write) or the data file's closing makes it so, and a crash before
    /// then loses it. An import's batches go so, and the folder lists naming them durably after.
    pub fn write_prepared(&self, bodies: Vec<(MessageKey, Prepared)>, durable: bool) -> Result<()> {
        let items = bodies.into_iter().map(|(k, p)| (self.body_path(&k), p)).collect();
        self.files.write_prepared(items, durable)
    }

    /// Sources made ready ([`Store::prepare`]) and `account`'s documents (`docs`: a plain file
    /// name and its bytes, as [`Store::save_account_doc`] keeps them) written together, the
    /// documents saying what the sources are: an import's checkpoint. In a data file it's one
    /// transaction, durable and sealed (it makes the batches before it durable too, and a crash
    /// after it reopens at once); elsewhere the sources go first and the documents last, so a
    /// document never names a source that isn't there.
    pub fn write_batch(&self, account: &str, bodies: Vec<(MessageKey, Prepared)>, docs: Vec<(&str, Vec<u8>)>) -> Result<()> {
        let dir = self.account_dir(account);
        let items = bodies.into_iter().map(|(k, p)| (self.body_path(&k), p)).collect();
        let docs = docs.into_iter().map(|(name, bytes)| (dir.join(safe_doc(name)), bytes)).collect();
        self.files.write_batch(&dir, items, docs)
    }

    /// Remove the sources kept for `folder` (as numbered under `validity`) numbered `uid_next` or
    /// above, in one change: what an import wrote past the checkpoint it goes on from, or past
    /// where it was stopped, that no list names. Numbers are read as numbers ("10" is above "9").
    /// Returns how many went.
    pub fn drop_bodies_from(&self, account: &str, folder: &str, validity: u32, uid_next: u32) -> Result<usize> {
        let dir = self.folder_dir(account, folder).join(validity.to_string());
        let gone: Vec<PathBuf> = self
            .files
            .list(&dir)
            .into_iter()
            .filter(|(name, _)| name.strip_suffix(".eml").and_then(|n| n.parse::<u32>().ok()).is_some_and(|uid| uid >= uid_next))
            .map(|(_, p)| p)
            .collect();
        if !gone.is_empty() {
            self.files.replace(&self.account_dir(account), None, &gone, false)?;
        }
        Ok(gone.len())
    }

    /// Make an account's data file reopen at once after a crash, as it is now: a durable commit
    /// that records where its free space is. Commits don't do that on their own (on a 37 GB file it
    /// tripled a small one's time), so a crash after one costs a full check of the file when it
    /// next opens. A data file is sealed when its writes go quiet for a moment and as MailCraft
    /// closes ([`Store::seal_all`]); an import's end, and a checkpoint, seal at once. Nothing to do
    /// for other stores.
    pub fn seal(&self, account: &str) -> Result<()> {
        self.files.seal(&self.account_dir(account))
    }

    /// MailCraft is closing: every data file sealed now, and every commit after this one sealed
    /// as it's made (slower, but a sync or an import still writing when the process ends leaves
    /// its file reopening at once). Called on the way out by the app and the CLI: the threads
    /// that also hold the store are let go rather than waited for, so the files aren't closed.
    pub fn seal_all(&self) -> Result<()> {
        self.files.seal_all()
    }

    /// `account` takes a bulk write (an import): until [`Store::bulk_end`] its data file has a
    /// cache of `cache_bytes` ([`BULK_CACHE`]), so a transaction holds what it changes in memory
    /// instead of writing pages out and reading them back. The file is opened again with it at
    /// once if nobody's using it, or as soon as nobody is. Nothing to do for other stores.
    pub fn bulk_begin(&self, account: &str, cache_bytes: usize) {
        if self.is_online_only(account) {
            return;
        }
        match &self.files.main {
            #[cfg(not(target_arch = "wasm32"))]
            Files::Packed(x) => x.bulk_begin(&safe_segment(account), cache_bytes),
            _ => {
                let _ = cache_bytes;
            }
        }
    }

    /// `account`'s bulk write is over: its data file's cache goes back to its usual size, letting
    /// go of the memory (a clean close and an open, at once if nobody's using it, otherwise as
    /// soon as nobody is).
    pub fn bulk_end(&self, account: &str) {
        match &self.files.main {
            #[cfg(not(target_arch = "wasm32"))]
            Files::Packed(x) => x.bulk_end(&safe_segment(account)),
            _ => {
                let _ = account;
            }
        }
    }

    /// The cache `account`'s data file is open with, in bytes (none: it isn't open, or the store
    /// keeps no data files).
    #[doc(hidden)]
    pub fn data_file_cache(&self, account: &str) -> Option<usize> {
        match &self.files.main {
            #[cfg(not(target_arch = "wasm32"))]
            Files::Packed(x) => x.cache_of(&safe_segment(account)),
            _ => {
                let _ = account;
                None
            }
        }
    }

    /// How `account`'s data file's cache has been used since the store opened ([`CacheUse`]):
    /// for measuring. Default for other stores.
    pub fn cache_use(&self, account: &str) -> CacheUse {
        match &self.files.main {
            #[cfg(not(target_arch = "wasm32"))]
            Files::Packed(x) => x.cache_use(&safe_segment(account)),
            _ => {
                let _ = account;
                CacheUse::default()
            }
        }
    }

    pub fn remove_body(&self, key: &MessageKey) {
        let _ = self.files.remove_file(&self.body_path(key));
    }

    pub fn load_ops(&self, account: &str) -> Result<Vec<Op>> {
        self.read_json(&self.account_dir(account).join("ops.json"))
    }

    /// Append `op` to ops.json. One that can't be read stops it: written over, the changes
    /// waiting in it would be lost (a missing or damaged one reads as empty).
    pub fn add_op(&self, account: &str, op: Op) -> Result<()> {
        let _held = self.changes.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut ops = self.load_ops(account)?;
        ops.push(op);
        self.save_ops(account, &ops)
    }

    /// Delete the first entry equal to `op` from ops.json, if any.
    pub fn remove_op(&self, account: &str, op: &Op) -> Result<()> {
        let _held = self.changes.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut ops = self.load_ops(account)?;
        if let Some(i) = ops.iter().position(|o| o == op) {
            ops.remove(i);
            self.save_ops(account, &ops)?;
        }
        Ok(())
    }

    pub fn save_ops(&self, account: &str, ops: &[Op]) -> Result<()> {
        let path = self.account_dir(account).join("ops.json");
        if ops.is_empty() {
            match self.files.remove_file(&path) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(io_err(&path)(e)),
                _ => return Ok(()),
            }
        }
        self.write_json(&path, ops)
    }

    pub(super) fn outbox_dir(&self, account: &str) -> PathBuf {
        self.account_dir(account).join("outbox")
    }

    /// Queue a message to send when the account is back online.
    pub fn outbox_add(&self, account: &str, item: &OutboxItem, raw: &[u8]) -> Result<()> {
        let dir = self.outbox_dir(account);
        self.files.write(&dir.join(format!("{}.eml", item.id)), raw)?;
        self.write_json(&dir.join(format!("{}.json", item.id)), item)
    }

    /// Update a queued message's bookkeeping (attempts, last error).
    pub fn outbox_update(&self, account: &str, item: &OutboxItem) -> Result<()> {
        self.write_json(&self.outbox_dir(account).join(format!("{}.json", item.id)), item)
    }

    /// Queued messages, oldest first.
    pub fn outbox(&self, account: &str) -> Vec<OutboxItem> {
        let dir = self.outbox_dir(account);
        let mut items: Vec<OutboxItem> = self
            .files
            .list(&dir)
            .into_iter()
            .filter(|(name, _)| name.ends_with(".json"))
            .filter_map(|(_, p)| self.files.read(&p).ok())
            .filter_map(|b| serde_json::from_slice::<OutboxItem>(&b).ok())
            .filter(|i| self.files.is_file(&dir.join(format!("{}.eml", i.id))))
            .collect();
        items.sort_by_key(|i| (i.queued, i.id));
        items
    }

    pub fn outbox_raw(&self, account: &str, id: u64) -> Option<Vec<u8>> {
        self.files.read(&self.outbox_dir(account).join(format!("{id}.eml"))).ok()
    }

    pub fn outbox_remove(&self, account: &str, id: u64) {
        let dir = self.outbox_dir(account);
        let _ = self.files.remove_file(&dir.join(format!("{id}.json")));
        let _ = self.files.remove_file(&dir.join(format!("{id}.eml")));
    }

    /// Cached sources across all folders of an account, as (key, path) pairs, for indexing.
    pub fn cached_bodies(&self, account: &str, folder: &str, validity: u32) -> Vec<u32> {
        let dir = self.folder_dir(account, folder).join(validity.to_string());
        self.files.list(&dir).into_iter().filter_map(|(n, _)| n.strip_suffix(".eml").and_then(|n| n.parse().ok())).collect()
    }
}
