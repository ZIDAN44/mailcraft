//! The store: message bodies, changes waiting for the server, the Outbox, and the drafts kept
//! until their account connects.

use crate::*;

mod filing;
#[cfg(test)]
mod tests;
mod waiting;
pub use filing::Filing;
pub use waiting::Waiting;

/// The end of the mark an Outbox message taken out leaves where its files couldn't go
/// ([`Store::outbox_remove`]).
const GONE: &str = ".gone";

/// The data files' commits that waited for the disk, timed: sealed ones and the others. For
/// measuring what sealing costs; nothing for other stores.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CommitTimes {
    pub sealed: Timed,
    pub plain: Timed,
}

/// How many, how long in all, and the longest.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Timed {
    pub count: u64,
    pub total: std::time::Duration,
    pub longest: std::time::Duration,
}

impl Timed {
    /// One more, taking `took`.
    pub fn add(&mut self, took: std::time::Duration) {
        self.count = self.count.saturating_add(1);
        self.total = self.total.saturating_add(took);
        self.longest = self.longest.max(took);
    }

    /// What was added since `before` (the longest is the whole run's).
    pub fn since(&self, before: &Timed) -> Timed {
        Timed { count: self.count.saturating_sub(before.count), total: self.total.saturating_sub(before.total), longest: self.longest }
    }
}

/// A long write to an account's data file ([`Store::long_write`]), over when dropped (on every
/// way out, an error's included).
#[must_use = "the long write is over when this is dropped"]
pub struct LongWrite<'a> {
    store: &'a Store,
    /// The account's name as its data file has it; none for other stores.
    account: Option<String>,
}

impl Drop for LongWrite<'_> {
    fn drop(&mut self) {
        match (&self.store.files.main, &self.account) {
            #[cfg(not(target_arch = "wasm32"))]
            (Files::Packed(x), Some(account)) => x.long_end(account),
            _ => {}
        }
    }
}

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

/// A message waiting in the Outbox (its source is stored next to it as `<id>.eml`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutboxItem {
    pub id: u64,
    /// Envelope sender and recipients (Bcc included: it isn't in the message).
    pub from: String,
    pub recipients: Vec<String>,
    /// Where a copy goes once sent.
    #[serde(default)]
    pub sent_folder: Option<String>,
    /// When it was queued (Unix seconds).
    #[serde(default)]
    pub queued: i64,
    #[serde(default)]
    pub attempts: u32,
    /// Why the last attempt failed.
    #[serde(default)]
    pub error: Option<String>,
    /// Held back until then (Unix seconds; 0 = send at once): Outlook's Delay Delivery.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub not_before: i64,
    /// The server refused it for good (not a connection that failed): it isn't tried again, and
    /// waits for the user to change or delete it.
    #[serde(default, skip_serializing_if = "is_false")]
    pub refused: bool,
}

/// How [`OutboxItem::error`] began for one refused for good, kept before
/// [`OutboxItem::refused`] was.
pub(crate) const OUTBOX_REFUSED: &str = "refused: ";

fn is_zero(n: &i64) -> bool {
    *n == 0
}

fn is_false(b: &bool) -> bool {
    !*b
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
    /// Taken by value: each source is moved into what's written, the batch never held twice.
    pub fn write_bodies(&self, bodies: Vec<(MessageKey, Vec<u8>)>) -> Result<()> {
        let prepared = bodies
            .into_iter()
            .map(|(k, raw)| {
                let packs = self.packs(&k.account);
                (k, Store::prepare(packs, raw))
            })
            .collect();
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
    /// document never names a source that isn't there. A data file let go of after a failed write
    /// since [`Store::data_file_reopened`] said `reopened` refuses it, writing nothing: the
    /// batches before it that didn't wait for the disk may be gone, and the documents would name
    /// them.
    pub fn write_batch(&self, account: &str, bodies: Vec<(MessageKey, Prepared)>, docs: Vec<(&str, Vec<u8>)>, reopened: u64) -> Result<()> {
        let dir = self.account_dir(account);
        let items = bodies.into_iter().map(|(k, p)| (self.body_path(&k), p)).collect();
        let docs = docs.into_iter().map(|(name, bytes)| (dir.join(safe_doc(name)), bytes)).collect();
        self.files.write_batch(&dir, items, docs, reopened)
    }

    /// How many times `account`'s data file was let go of after a write the disk failed (a full
    /// drive), to open again at its last durable commit: what was written without waiting for the
    /// disk before then may be gone. A writer that sees it change since it began (an import)
    /// writes nothing more that counts on that. The count stays while the account is kept online
    /// only (a writer going on across the switch compares with what it saw as it began). Always 0
    /// for other stores.
    pub fn data_file_reopened(&self, account: &str) -> u64 {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(x) = self.data_files() {
            return x.reopened(&safe_segment(account));
        }
        let _ = account;
        0
    }

    /// The drive of a store's data files full (`true`: growing one fails as on a full drive) or
    /// not, for tests. Other stores ignore it.
    #[doc(hidden)]
    #[cfg(feature = "test-drive")]
    pub fn fill_drive(&self, full: bool) {
        if let Files::Packed(x) = &self.files.main {
            x.full.store(full, std::sync::atomic::Ordering::Relaxed);
        }
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
        // The read marks handed over first: they're sealed with the rest.
        self.flush_marks();
        self.files.seal_all()
    }

    /// `account` takes a bulk write (an import): until [`Store::bulk_end`] its data file has a
    /// cache of `cache_bytes` ([`BULK_CACHE`]), so a transaction holds what it changes in memory
    /// instead of writing pages out and reading them back. The file is opened again with it at
    /// once if nobody's using it, or as soon as nobody is. Nothing to do for other stores.
    pub fn bulk_begin(&self, account: &str, cache_bytes: usize) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some((x, a)) = self.in_data_file(account) {
            x.bulk_begin(&a, cache_bytes);
        }
        let _ = (account, cache_bytes);
    }

    /// `account`'s bulk write is over: its data file's cache goes back to its usual size, letting
    /// go of the memory (a clean close and an open, at once if nobody's using it, otherwise as
    /// soon as nobody is). Ended however the account is kept now: one begun before it went
    /// online only ends too.
    pub fn bulk_end(&self, account: &str) {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(x) = self.data_files() {
            x.bulk_end(&safe_segment(account));
        }
        let _ = account;
    }

    /// `account` takes a long write (a local move of more than a batch of mail) until what this
    /// returns is dropped: each commit to its data file that waits for the disk is sealed
    /// meanwhile, and the file is sealed now if it was written since its last seal, so a crash
    /// part-way reopens it at once rather than checking the whole of it. The writer's batches
    /// don't wait for the disk (`write_prepared`), and its last one that counts on them is a
    /// sealed commit checked against [`Store::data_file_reopened`] (`write_batch`). Nothing to do
    /// for other stores.
    pub fn long_write(&self, account: &str) -> LongWrite<'_> {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some((x, a)) = self.in_data_file(account) {
            x.long_begin(&a);
            return LongWrite { store: self, account: Some(a) };
        }
        let _ = account;
        LongWrite { store: self, account: None }
    }

    /// The cache `account`'s data file is open with, in bytes (none: it isn't open, or the store
    /// keeps no data files). Whatever the account is kept as now: it's the file's own.
    #[doc(hidden)]
    pub fn data_file_cache(&self, account: &str) -> Option<usize> {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(x) = self.data_files() {
            return x.cache_of(&safe_segment(account));
        }
        let _ = account;
        None
    }

    /// How `account`'s data file's cache has been used since the store opened ([`CacheUse`]):
    /// for measuring, so it counts while the account was kept in it, online only now or not.
    /// Default for other stores.
    pub fn cache_use(&self, account: &str) -> CacheUse {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(x) = self.data_files() {
            return x.cache_use(&safe_segment(account));
        }
        let _ = account;
        CacheUse::default()
    }

    /// The data files' commits that waited for the disk since the store opened, sealed and not,
    /// timed ([`CommitTimes`]): for measuring. Nothing for other stores.
    pub fn commit_times(&self) -> CommitTimes {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(x) = self.data_files() {
            return x.commit_times();
        }
        CommitTimes::default()
    }

    pub fn remove_body(&self, key: &MessageKey) {
        let _ = self.files.remove_file(&self.body_path(key));
    }

    /// The sources of `keys` (`account`'s) removed together: in a data file, one transaction not
    /// waited for, where each was one of its own. One that isn't there is no matter.
    pub fn remove_bodies(&self, account: &str, keys: &[MessageKey]) {
        let paths: Vec<PathBuf> = keys.iter().filter(|k| k.account == account).map(|k| self.body_path(k)).collect();
        if paths.is_empty() {
            return;
        }
        if let Err(e) = self.files.replace(&self.account_dir(account), None, &paths, false) {
            log::warn!("{account}: {} sources not removed: {e}", paths.len());
        }
    }

    pub fn load_ops(&self, account: &str) -> Result<Vec<Op>> {
        self.read_json(&self.account_dir(account).join("ops.json"))
    }

    /// Append `op` to ops.json. One that can't be read stops it: written over, the changes
    /// waiting in it would be lost (a missing or damaged one reads as empty).
    pub fn add_op(&self, account: &str, op: Op) -> Result<()> {
        let _held = self.hold_changes();
        let mut ops = self.load_ops(account)?;
        ops.push(op);
        self.save_ops(account, &ops)
    }

    /// Delete the first entry equal to `op` from ops.json, if any.
    pub fn remove_op(&self, account: &str, op: &Op) -> Result<()> {
        let _held = self.hold_changes();
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
        // A mark an earlier message of the id left would keep this one out of the Outbox.
        let gone = dir.join(format!("{}{GONE}", item.id));
        match self.files.remove_file(&gone) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(io_err(&gone)(e)),
            _ => {}
        }
        self.files.write(&dir.join(format!("{}.eml", item.id)), raw)?;
        self.write_json(&dir.join(format!("{}.json", item.id)), item)
    }

    /// Update a queued message's bookkeeping (attempts, last error).
    pub fn outbox_update(&self, account: &str, item: &OutboxItem) -> Result<()> {
        self.write_json(&self.outbox_dir(account).join(format!("{}.json", item.id)), item)
    }

    /// Queued messages, oldest first: each with its source, and not taken out
    /// ([`Store::outbox_remove`]).
    pub fn outbox(&self, account: &str) -> Vec<OutboxItem> {
        let dir = self.outbox_dir(account);
        let files = self.files.list(&dir);
        let there = |name: &str| files.iter().any(|(n, _)| n == name);
        let mut items: Vec<OutboxItem> = files
            .iter()
            .filter(|(name, _)| name.ends_with(".json"))
            .filter_map(|(_, p)| self.files.read(p).ok())
            .filter_map(|b| serde_json::from_slice::<OutboxItem>(&b).ok())
            .filter(|i| there(&format!("{}.eml", i.id)) && !there(&format!("{}{GONE}", i.id)))
            .map(|mut i| {
                // One kept before `refused` was says so in its error's words.
                i.refused |= i.error.as_deref().is_some_and(|e| e.starts_with(crate::OUTBOX_REFUSED));
                i
            })
            .collect();
        items.sort_by_key(|i| (i.queued, i.id));
        items
    }

    pub fn outbox_raw(&self, account: &str, id: u64) -> Option<Vec<u8>> {
        self.files.read(&self.outbox_dir(account).join(format!("{id}.eml"))).ok()
    }

    /// A message taken out of the Outbox (sent, deleted, or taken back to be changed): never
    /// listed again, so never sent again. Its files are removed; one that can't be (another
    /// program holding it, a failing disk) leaves a mark beside it (`<id>.gone`) that keeps the
    /// message out of the list, and [`Store::outbox_sweep`] removes what's left later. An error
    /// only when the message is still in the Outbox: neither file went, nor could the mark be
    /// written.
    pub fn outbox_remove(&self, account: &str, id: u64) -> Result<()> {
        let dir = self.outbox_dir(account);
        let mut left = Vec::new();
        for name in [format!("{id}.json"), format!("{id}.eml")] {
            let path = dir.join(name);
            match self.files.remove_file(&path) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => left.push((path, e)),
                _ => {}
            }
        }
        let still = left.len() == 2;
        let Some((path, why)) = left.into_iter().next() else { return Ok(()) };
        match self.files.write(&dir.join(format!("{id}{GONE}")), b"") {
            Ok(()) => log::warn!("{account}: Outbox message {id}: {} couldn't be removed ({why}); marked as gone", path.display()),
            Err(e) if still => {
                log::warn!("{account}: Outbox message {id}: not marked as gone either ({e})");
                return Err(io_err(&path)(why));
            }
            // Without one of its files it isn't listed: what's left is litter, not mail to send.
            Err(e) => log::warn!("{account}: Outbox message {id}: {} couldn't be removed ({why}), nor marked ({e})", path.display()),
        }
        Ok(())
    }

    /// What [`Store::outbox_remove`] couldn't remove when it was asked, removed now where it can
    /// be: a marked message's files, then its mark.
    pub fn outbox_sweep(&self, account: &str) {
        let dir = self.outbox_dir(account);
        for (name, mark) in self.files.list(&dir) {
            let Some(id) = name.strip_suffix(GONE) else { continue };
            let removed = |ext: &str| match self.files.remove_file(&dir.join(format!("{id}{ext}"))) {
                Err(e) => e.kind() == io::ErrorKind::NotFound,
                Ok(()) => true,
            };
            // Both tried, each time.
            let (json, eml) = (removed(".json"), removed(".eml"));
            if json && eml {
                let _ = self.files.remove_file(&mark);
            }
        }
    }

    fn kept_drafts_dir(&self, account: &str) -> PathBuf {
        self.account_dir(account).join("drafts-kept")
    }

    /// Keep a draft's source on this computer until its account connects (`<id>.eml`, as the
    /// Outbox keeps its messages); which drafts are kept, and where each goes, is the caller's.
    pub fn kept_draft_add(&self, account: &str, id: u64, raw: &[u8]) -> Result<()> {
        self.files.write(&self.kept_drafts_dir(account).join(format!("{id}.eml")), raw)
    }

    /// A kept draft's source; `None` when there's none.
    pub fn kept_draft_raw(&self, account: &str, id: u64) -> Result<Option<Vec<u8>>> {
        let path = self.kept_drafts_dir(account).join(format!("{id}.eml"));
        match self.files.read(&path) {
            Ok(raw) => Ok(Some(raw)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(io_err(&path)(e)),
        }
    }

    /// A kept draft's source removed (one not there is no matter).
    pub fn kept_draft_remove(&self, account: &str, id: u64) -> Result<()> {
        let path = self.kept_drafts_dir(account).join(format!("{id}.eml"));
        match self.files.remove_file(&path) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(io_err(&path)(e)),
            _ => Ok(()),
        }
    }

    /// The ids of the kept drafts' sources there are.
    pub fn kept_draft_ids(&self, account: &str) -> Vec<u64> {
        self.files.list(&self.kept_drafts_dir(account)).into_iter().filter_map(|(n, _)| n.strip_suffix(".eml").and_then(|n| n.parse().ok())).collect()
    }

    /// The uids of the sources cached for one folder at one validity (in no set order).
    pub fn cached_bodies(&self, account: &str, folder: &str, validity: u32) -> Vec<u32> {
        let dir = self.folder_dir(account, folder).join(validity.to_string());
        self.files.list(&dir).into_iter().filter_map(|(n, _)| n.strip_suffix(".eml").and_then(|n| n.parse().ok())).collect()
    }
}
