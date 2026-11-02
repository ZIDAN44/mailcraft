//! What an import has written so far: folders, their lists, message sources and contacts, and
//! the messages' words in the account's search index (`indexing`).

use super::budget::Budget;
use super::item::Item;
use super::lanes::Built;
use super::resume::CheckState;
use super::*;

mod indexing;
mod pim;
mod room;
pub(super) mod sent_ids;

/// Message sources written to the cache together (one transaction): at most this many…
const BATCH_MESSAGES: usize = 2000;
/// …or this many bytes as kept: a transaction short enough that a write from the window (a
/// message marked read in a folder already in) waits a few milliseconds for it, not a tenth of a
/// second. They're committed without waiting for the disk; each folder list, written after the
/// sources it names, makes them durable.
const BATCH_BYTES: usize = mailcraft_store::BACKGROUND_COMMIT;
/// Finished folders go into the account's folder list together, at most this often: a write of
/// the list for each would cost an import of many folders a time that grows with their square.
const PUBLISH_EVERY: Duration = Duration::from_secs(1);

/// A folder being read.
pub(super) struct Open {
    pub(super) uid_next: u32,
    /// Messages whose sources may not be written yet (they go to the list once they are).
    pub(super) waiting: Vec<mailcraft_model::Summary>,
    pub(super) list: mailcraft_store::ListWriter,
}

/// What has been written so far.
pub(super) struct Out<'a> {
    pub(super) store: &'a Store,
    pub(super) account: &'a str,
    pub(super) folders: Vec<Folder>,
    /// The folders being read: their next number, and their list going out a page at a time
    /// (a folder of hundreds of thousands isn't held whole; written ones keep only their counts).
    pub(super) states: HashMap<String, Open>,
    pub(super) imported: usize,
    /// Items read from the file, whatever became of them.
    pub(super) read: usize,
    /// Items left out, in all and by reason (`skips` also counts messages that came in with part
    /// of them lost, and the files left out of the messages that came in, which `skipped` doesn't).
    pub(super) skipped: usize,
    pub(super) skips: BTreeMap<SkipReason, usize>,
    pub(super) body_not_converted: usize,
    /// Told how the import goes, and of each folder as it's done.
    sink: &'a mut dyn ImportSink,
    /// What the file holds (its items and their bytes), and the bytes of the items read so far.
    total: usize,
    bytes_total: u64,
    pub(super) bytes_done: u64,
    folders_total: usize,
    /// Contacts and calendar items read so far.
    pub(super) pim: Pim,
    /// Sources read (compressed already) but not yet written, and their size as kept.
    pub(super) pending: Vec<(MessageKey, mailcraft_store::Prepared)>,
    pub(super) pending_bytes: usize,
    /// The same sources' bytes as rebuilt, before compression, and what they hold of the budget
    /// (given back once they're written).
    pending_raw: usize,
    pub(super) pending_charge: u64,
    pub(super) budget: Arc<Budget>,
    /// Raised from elsewhere to stop reading, and whether the writer saw it before the end.
    stop: Stop,
    stopped: bool,
    /// When the import began, and where its time has gone since.
    pub(super) began: Instant,
    pub(super) tally: Arc<Tally>,
    /// The lanes that read the items.
    pub(super) lanes: u64,
    /// Folders finished but not yet in the account's folder list, and when the list was last
    /// written (none: not yet).
    finished: Vec<Folder>,
    published: Option<Instant>,
    /// The most a batch of sources holds as kept ([`BATCH_BYTES`] unless measuring).
    batch_bytes: usize,
    /// The account's search index, held for the import (none: it isn't indexing), and the
    /// messages put in it.
    index: Option<indexing::Index>,
    indexed: u64,
    /// Where the import has got, for its checkpoints (`resume`).
    pub(super) note: super::resume::Note,
    /// The replies and ids read, for the sent items read last (`sent_ids`).
    pub(super) sent: sent_ids::SentIds,
    /// The room left where the data file is written, as it's written.
    room: room::Room,
}

impl<'a> Out<'a> {
    /// Nothing written yet, into `account`'s cache.
    pub(super) fn new(store: &'a Store, account: &'a str, opts: &ImportOptions, sink: &'a mut dyn ImportSink) -> Self {
        Out {
            store,
            account,
            folders: Vec::new(),
            states: HashMap::new(),
            imported: 0,
            read: 0,
            skipped: 0,
            skips: BTreeMap::new(),
            body_not_converted: 0,
            sink,
            total: 0,
            bytes_total: 0,
            bytes_done: 0,
            folders_total: 0,
            pim: Pim::default(),
            pending: Vec::new(),
            pending_bytes: 0,
            pending_raw: 0,
            pending_charge: 0,
            budget: Arc::new(Budget::new(u64::try_from(opts.budget_bytes).unwrap_or(u64::MAX))),
            stop: opts.stop.clone(),
            stopped: false,
            began: Instant::now(),
            tally: Arc::default(),
            lanes: 1,
            finished: Vec::new(),
            published: None,
            batch_bytes: if opts.batch_bytes > 0 { opts.batch_bytes } else { BATCH_BYTES },
            index: None,
            indexed: 0,
            note: super::resume::Note::every(checkpoint_every(opts)),
            sent: sent_ids::SentIds::default(),
            room: room::Room::new(store, account, opts),
        }
    }
}

/// Folders the outline of what an import brings in names, at most (the rest come in unannounced).
const OUTLINE_MAX: usize = 2000;

/// A folder's name in the file as MailCraft names it: no `/` in it, and never empty.
fn folder_name(name: &str) -> String {
    let name = name.trim().replace('/', "∕");
    if name.is_empty() { "Folder".to_string() } else { name }
}

/// The path a folder named `name` (in the file) under `parent` gets beside `folders`, and its
/// role when it's a usual top-level one (each role once).
fn folder_path(folders: &[Folder], parent: Option<&str>, name: &str) -> (String, FolderRole) {
    let name = folder_name(name);
    let mut role = match parent {
        None => FolderRole::from_name(&name, None),
        Some(_) => FolderRole::Other,
    };
    if role != FolderRole::Other && folders.iter().any(|f| f.role == role) {
        role = FolderRole::Other;
    }
    let base = match (role, parent) {
        (FolderRole::Inbox, _) => "INBOX".to_string(),
        (_, Some(p)) => format!("{p}/{}", utf7::encode(&name)),
        (_, None) => utf7::encode(&name),
    };
    let mut path = base.clone();
    let mut n = 2;
    while folders.iter().any(|f| f.path.eq_ignore_ascii_case(&path)) {
        path = format!("{base} ({n})");
        n += 1;
    }
    (path, role)
}

/// How often the import's checkpoint is written ([`super::resume::CHECKPOINT_EVERY`], or as a
/// test says).
fn checkpoint_every(opts: &ImportOptions) -> Duration {
    #[cfg(test)]
    if let Some(every) = opts.checkpoint_every {
        return every;
    }
    let _ = opts;
    super::resume::CHECKPOINT_EVERY
}

impl Out<'_> {
    /// Whether the import was asked to stop: nothing more is read.
    pub(super) fn stopped(&mut self) -> bool {
        self.stopped |= self.stop.raised();
        self.stopped
    }

    /// How the import was stopped, when the writer saw it was: as asked by now (a pause may have
    /// become a discard since).
    fn stopped_as(&self) -> Option<StopHow> {
        if self.stopped { self.stop.how() } else { None }
    }

    /// Count `n` items left out for `reason`.
    pub(super) fn skip(&mut self, reason: SkipReason, n: usize) {
        self.skipped = self.skipped.saturating_add(n);
        let c = self.skips.entry(reason).or_default();
        *c = c.saturating_add(n);
    }

    /// Count `n` parts of an item that came in all the same (a recipient) left out for
    /// `reason`: in the report, not among the items left out.
    pub(super) fn lose(&mut self, reason: SkipReason, n: usize) {
        if n > 0 {
            let c = self.skips.entry(reason).or_default();
            *c = c.saturating_add(n);
        }
    }

    /// What the file holds (its items, their bytes, its mail folders), before any of it is
    /// read: the progress goes by it.
    pub(super) fn totals(&mut self, items: u64, bytes: u64, folders: usize) {
        self.total = usize::try_from(items).unwrap_or(usize::MAX);
        self.bytes_total = bytes;
        self.folders_total = folders;
    }

    /// One more item read from the file, of `size` bytes as the file gives it, whatever became of
    /// it (call it once the item is dealt with): how far the import has got is said every 200,
    /// so a run of items left out or of contacts moves the bar as mail does.
    pub(super) fn item_read(&mut self, size: u64) {
        self.read = self.read.saturating_add(1);
        self.bytes_done = self.bytes_done.saturating_add(size);
        if self.read.is_multiple_of(200) {
            let p = Progress {
                read: self.read,
                imported: self.imported,
                skipped: self.skipped,
                total: self.total,
                bytes_done: self.bytes_done,
                bytes_total: self.bytes_total,
                folders: self.folders_total,
                start_read: self.note.start_read,
                start_bytes: self.note.start_bytes,
                earlier_ms: self.note.earlier_ms,
            };
            self.sink.progress(p);
        }
    }

    /// A folder of a kind that isn't brought in, holding `items` (one holding nothing loses
    /// nothing): its items are left out, and the folder is in the report.
    pub(super) fn leave_out_folder(&mut self, items: usize) {
        if items > 0 {
            self.lose(SkipReason::FolderLeftOut, 1);
            self.skip(SkipReason::ItemInFolderLeftOut, items);
        }
    }

    /// A folder for `name` under `parent`, with a role when it's a usual top-level one.
    pub(super) fn add_folder(&mut self, parent: Option<&str>, name: &str) -> String {
        let (path, role) = folder_path(&self.folders, parent, name);
        self.folders.push(Folder {
            account: self.account.to_string(),
            path: path.clone(),
            delimiter: Some('/'),
            role,
            selectable: true,
            ..Default::default()
        });
        self.states.insert(path.clone(), Open { uid_next: 1, waiting: Vec::new(), list: self.store.list_writer(self.account, &path) });
        path
    }

    /// The plan's mail folders (`outline`, in the order they come in), each with the path it gets
    /// (where it went, going on, or as it will be made), told to the sink before any comes in.
    pub(super) fn outline(&mut self, outline: &[super::order::Outlined]) {
        let mut made = self.folders.clone();
        let mut paths: HashMap<usize, String> = HashMap::new();
        let mut planned = Vec::new();
        for f in outline {
            let parent = f.parent.and_then(|p| paths.get(&p)).cloned();
            let path = match self.known_path(f.node) {
                Some(p) => p.to_string(),
                None => {
                    let (path, role) = folder_path(&made, parent.as_deref(), &f.name);
                    made.push(Folder { path: path.clone(), role, ..Default::default() });
                    path
                }
            };
            if planned.len() < OUTLINE_MAX {
                planned.push(PlannedFolder { path: path.clone(), name: folder_name(&f.name), items: f.items });
            }
            paths.insert(f.index, path);
        }
        self.sink.planned(&planned);
    }

    /// A contact, contact group, calendar item, note or journal entry (anything else in their
    /// folders is left out).
    pub(super) fn add_pim(&mut self, it: Item) {
        let began = Instant::now();
        self.pim_of(it);
        self.pim_read();
        self.tally.since(Phase::Build, began);
    }

    /// An item made into a message (in a test, as a lane would), into the folder at `path`.
    #[cfg(test)]
    pub(super) fn add_message(&mut self, path: &str, it: Item) {
        if !super::lanes::is_mail(&it.class) {
            self.skip(SkipReason::ItemNotUsable, 1);
            return;
        }
        let fallback = format!("pst.{}.test.{}@mailcraft.invalid", self.account, self.read);
        let packs = self.store.packs(self.account);
        match super::lanes::build(it, &fallback, packs, self.indexing(), &self.tally) {
            super::lanes::Outcome::Built(b) => self.add_built(path, *b, 0),
            super::lanes::Outcome::NotRebuilt => self.skip(SkipReason::MessageNotRebuilt, 1),
            _ => {}
        }
    }

    /// A message a lane made (holding `charge` of the budget until it's written), into the
    /// folder at `path`, numbered next there. The batches it writes carry no checkpoint: its
    /// row isn't done until the writer says so, and one written here would read it again.
    pub(super) fn add_built(&mut self, path: &str, b: Built, charge: u64) {
        let Some(st) = self.states.get_mut(path) else {
            self.budget.release(charge);
            return;
        };
        let Built { mut summary, prepared, doc, raw_len, charset_unsupported, body_not_converted, without_address, left_out, .. } = b;
        let uid = st.uid_next;
        st.uid_next = uid.saturating_add(1);
        let key = MessageKey { account: self.account.to_string(), folder: path.to_string(), validity: 1, uid };
        summary.key = key.clone();
        let size = summary.size;
        let msgid = mailcraft_store::search::msgid_hash(&summary.message_id);
        st.waiting.push(summary);
        let page_full = st.waiting.len() >= mailcraft_store::PAGE_MESSAGES;
        if charset_unsupported {
            // A loss in the report, though the message comes in: not among the items left out.
            let c = self.skips.entry(SkipReason::CharsetUnsupported).or_default();
            *c = c.saturating_add(1);
        }
        if body_not_converted {
            self.body_not_converted += 1;
        }
        self.imported += 1;
        self.lose(SkipReason::RecipientWithoutAddress, without_address);
        // The message came in without these files: counted by reason, not as items skipped.
        for (reason, n) in &left_out {
            self.lose(*reason, *n);
        }
        // One that would take the batch past its size goes into the next (a big one alone).
        if self.pending_bytes.saturating_add(prepared.kept_len()) > self.batch_bytes {
            self.write_pending(false);
        }
        self.pending_bytes = self.pending_bytes.saturating_add(prepared.kept_len());
        self.pending_raw = self.pending_raw.saturating_add(raw_len);
        self.pending_charge = self.pending_charge.saturating_add(charge);
        self.tally.source(raw_len, prepared.kept_len(), self.pending_raw);
        self.index_later(&key, msgid, size, doc);
        self.pending.push((key, prepared));
        if page_full || self.pending.len() >= BATCH_MESSAGES || self.pending_bytes >= self.batch_bytes {
            self.write_pending(false);
        }
        // A page's worth whose sources are written: into the folder's list.
        if page_full {
            self.list_out(path);
        }
    }

    /// Write the sources read so far, together, with the checkpoint when one is due (called
    /// between items, where the checkpoint's note is whole).
    pub(super) fn flush(&mut self) {
        let due = self.checkpoint_due();
        self.write_pending(due);
    }

    /// Write the sources read so far, together, and the checkpoint with them when `checkpoint`;
    /// returns whether a checkpoint was written. A batch that can't be written leaves its
    /// messages out of their folders' lists (counted as skipped), as one failed write did; one
    /// refused for want of room stops the writing ([`Out::out_of_room`]) and isn't counted:
    /// its messages are read again going on, as are those of one whose failure had the data file
    /// opened again ([`Out::writes_stopped`]). Once writing has stopped, a batch goes nowhere.
    pub(super) fn write_pending(&mut self, checkpoint: bool) -> bool {
        if self.pending.is_empty() {
            return false;
        }
        if self.writes_stopped() {
            let batch = std::mem::take(&mut self.pending);
            self.pending_bytes = 0;
            self.pending_raw = 0;
            let keys: Vec<MessageKey> = batch.into_iter().map(|(k, _)| k).collect();
            self.unlist(&keys);
            self.index_written(false);
            let charge = std::mem::take(&mut self.pending_charge);
            self.budget.release(charge);
            return false;
        }
        self.mind_room(self.pending_bytes, checkpoint);
        let batch = std::mem::take(&mut self.pending);
        self.pending_bytes = 0;
        self.pending_raw = 0;
        let charge = std::mem::take(&mut self.pending_charge);
        let keys: Vec<MessageKey> = batch.iter().map(|(k, _)| k.clone()).collect();
        // Every so often the checkpoint goes with the batch, naming it.
        let began = Instant::now();
        let wrote = if checkpoint {
            let docs = self.checkpoint_docs();
            let reopened = self.reopened();
            let wrote = self.tally.time(Phase::Write, || self.store.write_batch(self.account, batch, docs, reopened));
            self.checkpointed(wrote.is_ok());
            wrote
        } else {
            self.tally.time(Phase::Write, || self.store.write_prepared(batch, false))
        };
        self.tally.wrote(began, checkpoint);
        // Their words go into the index once their sources are in.
        let ok = wrote.is_ok();
        self.index_written(ok);
        match wrote {
            Err(e) if room::for_want_of_room(&e) => {
                self.unlist(&keys);
                self.out_of_room();
            }
            Err(e) => {
                log::warn!("import: {e}");
                self.unlist(&keys);
                // The data file opened again at its last durable commit: they're read again going
                // on, from the last checkpoint.
                if !self.writes_stopped() {
                    self.imported = self.imported.saturating_sub(keys.len());
                    self.skip(SkipReason::NotWritten, keys.len());
                }
            }
            Ok(()) => {}
        }
        // Written (or not): what they held of the budget goes back.
        self.budget.release(charge);
        checkpoint && ok
    }

    /// Messages whose sources weren't written, kept out of their folders' lists.
    fn unlist(&mut self, keys: &[MessageKey]) {
        for key in keys {
            if let Some(st) = self.states.get_mut(&key.folder) {
                st.waiting.retain(|m| m.key.uid != key.uid);
            }
        }
    }

    /// Hand a folder's messages whose sources are written to its list (full pages go out).
    fn list_out(&mut self, path: &str) {
        if self.writes_stopped() {
            return;
        }
        let Some(st) = self.states.get_mut(path) else { return };
        let ready = std::mem::take(&mut st.waiting);
        if let Err(e) = self.tally.time(Phase::Commit, || st.list.push(self.store, ready)) {
            log::warn!("import: {e}");
            // Failed at the disk, the data file is opened again: nothing more is written.
            self.writes_stopped();
        }
    }

    /// Write a finished folder's list and counts, then let go of it (only the counts are needed
    /// from here). It goes into the account's folder list, with the others finished within a
    /// second of it, and the sink is told: the window can show it while the rest of the file
    /// comes in.
    pub(super) fn save_folder(&mut self, path: &str) {
        self.flush();
        // Nothing more is written: the folder is read again going on.
        if self.writes_stopped() {
            return;
        }
        self.list_out(path);
        let Some(st) = self.states.remove(path) else { return };
        let head = FolderState { validity: 1, uid_next: st.uid_next, ..Default::default() };
        let (unread, total) = match self.tally.time(Phase::Commit, || st.list.finish(self.store, &head)) {
            Ok(c) => {
                self.index_folder_done(path);
                c
            }
            Err(e) => {
                log::warn!("import: {e}");
                if self.writes_stopped() {
                    return;
                }
                (0, 0)
            }
        };
        let Some(f) = self.folders.iter_mut().find(|f| f.path == path) else { return };
        f.unread = unread;
        f.total = total;
        let f = f.clone();
        self.finished.push(f);
        self.mail_folder_done(path);
        self.publish(false);
    }

    /// The folders finished so far put in the account's folder list in one write, and said to
    /// the sink, unless the list was written less than [`PUBLISH_EVERY`] ago (`now`: whenever).
    /// A write that fails keeps them for the next, and says why. A checkpoint naming them done
    /// goes first: a folder shown is never read again, whatever the user did to it since.
    pub(super) fn publish(&mut self, now: bool) -> Option<String> {
        if self.finished.is_empty() || (!now && self.published.is_some_and(|t| t.elapsed() < PUBLISH_EVERY)) {
            return None;
        }
        self.published = Some(Instant::now());
        self.checkpoint();
        // The checkpoint naming them done wasn't written: they're read again going on.
        if self.writes_stopped() {
            return None;
        }
        match self.tally.time(Phase::Commit, || self.store.upsert_folders(self.account, &self.finished)) {
            Ok(()) => {
                for f in std::mem::take(&mut self.finished) {
                    self.sink.folder(&f);
                }
                None
            }
            Err(e) => {
                log::warn!("import: {e}");
                // Failed at the disk: the folders are read again going on, not said lost.
                (!self.writes_stopped()).then(|| e.to_string())
            }
        }
    }

    /// The rest written out, and the data file sealed whatever happened: what went in without
    /// waiting for the disk is on it, and a crash after the import doesn't cost a check of the file.
    pub(super) fn finish(mut self) -> Imported {
        // Thrown away: nothing more is written, its data file goes.
        if self.stopped_as() == Some(StopHow::Discard) {
            return self.thrown_away();
        }
        let mut done = self.write_out();
        self.seal();
        done.stats = self.tally.snapshot(self.began.elapsed(), self.lanes);
        done.stats.indexed = self.indexed;
        done.stats.ids_given = self.sent.given.saturating_sub(self.sent.taken_back);
        done.stats.ids_taken_back = self.sent.taken_back;
        log::info!("import: {}", serde_json::to_string(&done.stats).unwrap_or_default());
        done
    }

    /// What went in so far made durable, its words written to the search index, and the data
    /// file sealed: the end of every import, finished or given up on.
    pub(super) fn seal(&mut self) {
        // Writing stopped for want of room: the data file is opened again when it's next used,
        // and what the index holds unwritten goes (its messages are read again going on).
        if self.writes_stopped() {
            self.index = None;
            return;
        }
        self.index_end();
        if let Err(e) = self.tally.time(Phase::Commit, || self.store.seal(self.account)) {
            log::warn!("import: {e}");
        }
    }

    /// What a discarded import came to: what it had read, none of it written out.
    fn thrown_away(mut self) -> Imported {
        let stopped_in = self.part_way();
        self.budget.release(std::mem::take(&mut self.pending_charge));
        Imported {
            folders: Vec::new(),
            messages: self.imported,
            skipped: self.skipped,
            skips: std::mem::take(&mut self.skips),
            body_not_converted: self.body_not_converted,
            stopped: Some(StopHow::Discard),
            stopped_in,
            stats: self.tally.snapshot(self.began.elapsed(), self.lanes),
            ..Imported::default()
        }
    }

    /// The folder the import was part-way through, when it was stopped in one (the first, in
    /// the order the folders came).
    fn part_way(&self) -> Option<String> {
        self.stopped_as()?;
        // Not the Sent Items folder waiting to be read last.
        let deferred = self.deferred_path();
        self.folders.iter().find(|f| self.states.contains_key(&f.path) && Some(f.path.as_str()) != deferred).map(|f| f.path.clone())
    }

    fn write_out(&mut self) -> Imported {
        self.flush();
        let stopped = self.stopped_as();
        let stopped_in = self.part_way();
        if stopped == Some(StopHow::Pause) {
            // Paused: a folder part-way through stays out of the list, to be gone on with from the
            // checkpoint; it was never shown, so nothing in it was changed meanwhile.
            self.ending(CheckState::Running);
            let last_save = self.publish(true);
            self.checkpoint();
            // Going on, one not reached again yet is part-way through still.
            let mut part_way: Vec<String> = self.states.keys().cloned().collect();
            part_way.extend(self.part_way_path());
            part_way.extend(self.deferred_path().map(str::to_string));
            self.states.clear();
            let folders = self.folders.iter().filter(|f| !part_way.contains(&f.path)).cloned().collect();
            return self.came_to(folders, stopped, stopped_in, last_save);
        }
        if stopped == Some(StopHow::Cancel) {
            // Cancelled: the folder part-way through is finished as it is, and gone on with when
            // asked, from where it stopped.
            self.ending(CheckState::Cancelled);
        }
        if !self.folders.iter().any(|f| f.role == FolderRole::Inbox) {
            // An Inbox to open, even when the file had none.
            let path = self.add_folder(None, "Inbox");
            self.note.invented = Some(path.clone());
            self.save_folder(&path);
        }
        // Folders left part-way (the file ended early, or the import was cancelled) still get
        // their list.
        let unsaved: Vec<String> = self.states.keys().cloned().collect();
        for p in unsaved {
            self.save_folder(&p);
        }
        // A last save that fails doesn't take back what came in before it: it's said, with the
        // counts, and the account is kept. The folders not in the list yet (one whose write
        // failed as it finished, too) are added as they stand: what the window changed meanwhile
        // stays.
        let mut last_save = self.publish(true);
        if !self.writes_stopped() && self.pim.cards.len() + self.pim.events.len() + self.pim.notes.len() > 0 {
            self.pim.ensure_defaults();
            if let Err(e) = self.tally.time(Phase::Commit, || self.store.save_account_doc(self.account, PIM_DOC, &self.pim)) {
                last_save.get_or_insert(e.to_string());
            }
        }
        if stopped != Some(StopHow::Cancel) {
            self.ending(CheckState::Done);
        }
        self.checkpoint();
        if self.body_not_converted > 0 {
            log::info!("{} messages kept their body only as Rich Text that couldn't be read: they came in without one", self.body_not_converted);
        }
        let folders = std::mem::take(&mut self.folders);
        self.came_to(folders, stopped, stopped_in, last_save)
    }

    /// What the import came to, with `folders`.
    fn came_to(&mut self, folders: Vec<Folder>, stopped: Option<StopHow>, stopped_in: Option<String>, last_save: Option<String>) -> Imported {
        // Writing stopped for want of room: the run comes to its last checkpoint, whose folders
        // done are listed already.
        let kept = self.kept_when_refused();
        let folders = if kept.is_some() { Vec::new() } else { folders };
        // Stopped as it ended (its last writes refused): it's paused all the same.
        let stopped = if kept.is_some() { self.stopped_as().or(stopped) } else { stopped };
        let (imported, skipped) = kept.unwrap_or((self.imported, self.skipped));
        Imported {
            folders,
            messages: imported,
            contacts: self.pim.cards.len(),
            events: calendar_items(&self.pim),
            notes: self.pim.notes.len(),
            skipped,
            skips: std::mem::take(&mut self.skips),
            body_not_converted: self.body_not_converted,
            stopped,
            stopped_in,
            stats: ImportStats::default(),
            last_save,
            runs: self.note.runs.saturating_add(1),
            earlier_ms: self.note.earlier_ms,
            // Stopped for want of room (and not asked to stop further meanwhile): it waits for more.
            blocked: self.room.full().filter(|_| stopped == Some(StopHow::Pause)).map(|free| mailcraft_model::job::Blocked::DiskFull { free }),
        }
    }
}
