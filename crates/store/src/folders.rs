//! The store: folders' lists of messages, and changes applied to them.

use mailcraft_model::Flags;

use crate::*;

mod lists;
use lists::page_hash;
pub use lists::{FolderState, ListTail, ListWriter, PAGE_MESSAGES, TAIL_PAGE_MAX, TAIL_PAGES_MAX};
pub(crate) use lists::{StateFile, no_uids_left};

impl Store {
    #[track_caller]
    pub fn load_folder(&self, account: &str, folder: &str) -> Result<FolderState> {
        let mut load = self.folder_load(account, folder)?;
        while !load.step(self)? {}
        Ok(load.finish())
    }

    /// Start reading a folder's list a page at a time ([`FolderLoad`]): its head is read now,
    /// once the read marks handed over before are on disk ([`Store::mark_later`]).
    #[track_caller]
    pub fn folder_load(&self, account: &str, folder: &str) -> Result<FolderLoad> {
        self.flush_marks_of(account, Some(folder));
        self.list_load(account, folder)
    }

    /// [`Store::load_folder`] without waiting for the marks (their own thread's).
    pub(crate) fn list_from_disk(&self, account: &str, folder: &str) -> Result<FolderState> {
        let mut load = self.list_load(account, folder)?;
        while !load.step(self)? {}
        Ok(load.finish())
    }

    fn list_load(&self, account: &str, folder: &str) -> Result<FolderLoad> {
        let dir = self.folder_dir(account, folder);
        let mut head: StateFile = self.read_json(&dir.join(HEAD))?;
        // A list in pages: the flags marked since its pages were written, put on them at the end.
        let marks: Vec<(u32, Flags)> = if head.pages.is_empty() { Vec::new() } else { self.read_json(&dir.join(MARKS))? };
        Ok(FolderLoad::from_head(account, folder, dir, &mut head, marks))
    }

    /// [`Store::list_from_disk`] from its head read already (`head`, in `dir`: its messages taken
    /// from it), with `marks` put on its pages: only the pages are read.
    pub(crate) fn list_from_head(
        &self,
        account: &str,
        folder: &str,
        dir: PathBuf,
        head: &mut StateFile,
        marks: Vec<(u32, Flags)>,
    ) -> Result<FolderState> {
        let mut load = FolderLoad::from_head(account, folder, dir, head, marks);
        while !load.step(self)? {}
        Ok(load.finish())
    }

    /// The numbers of a folder's messages, the rest of each one's summary not read: the numbers
    /// still in use, all that forgetting the others needs (a folder's whole list read for its
    /// numbers alone took four times as long, measured on 100,000 messages). In no order.
    pub fn folder_uids(&self, account: &str, folder: &str) -> Result<Vec<u32>> {
        let dir = self.folder_dir(account, folder);
        let head: UidsFile = self.read_json(&dir.join(HEAD))?;
        let mut uids: Vec<u32> = head.messages.iter().map(|m| m.key.uid).collect();
        for i in 0..head.pages.len() {
            let page: Vec<UidOnly> = self.read_json(&page_path(&dir, i))?;
            uids.extend(page.iter().map(|m| m.key.uid));
        }
        Ok(uids)
    }
}

/// A folder's list file ([`StateFile`]) read for its messages' numbers and its pages alone.
#[derive(Default, Deserialize)]
#[serde(default)]
struct UidsFile {
    messages: Vec<UidOnly>,
    pages: Vec<u64>,
}

/// A message's summary read for its number alone.
#[derive(Default, Deserialize)]
#[serde(default)]
struct UidOnly {
    key: UidOfKey,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct UidOfKey {
    uid: u32,
}

/// Beside a list kept in pages: the flags of the messages marked since its pages were written
/// (number, flags), so a message marked read writes a few bytes, not its page of thousands
/// (about 2 MB, and 24 to 90 ms of the window's frame in a folder of 10,941). The next save of
/// the whole list puts them into the pages and empties it.
pub(crate) const MARKS: &str = "state-marks.json";

/// A folder's list's head ([`StateFile`]): its sync state, and its messages or the pages they're
/// kept in.
pub(crate) const HEAD: &str = "state.json";

/// Page `i` of a list kept in pages, in its folder's directory `dir`.
pub(crate) fn page_path(dir: &Path, i: usize) -> PathBuf {
    dir.join(format!("state-{i}.json"))
}

/// The most messages kept marked beside a list's pages (about 50 KB): one more puts them into
/// their pages.
pub const MARKS_MAX: usize = 512;

/// A folder's list being read a page at a time ([`Store::folder_load`]), so whoever reads it can
/// do something else between pages (a search on the window's thread reads a big folder over
/// several frames): [`FolderLoad::step`] until it says it's done, then [`FolderLoad::finish`].
#[derive(Debug)]
pub struct FolderLoad {
    account: String,
    folder: String,
    dir: PathBuf,
    st: FolderState,
    /// Pages the list has, and the next to read.
    pages: usize,
    next: usize,
    /// The flags marked since the pages were written ([`MARKS`]).
    marks: Vec<(u32, Flags)>,
}

impl FolderLoad {
    /// A list's reading from its `head` (in `dir`; its messages taken from it), with `marks` to put
    /// on its pages.
    fn from_head(account: &str, folder: &str, dir: PathBuf, head: &mut StateFile, marks: Vec<(u32, Flags)>) -> FolderLoad {
        let st = FolderState {
            validity: head.validity,
            uid_next: head.uid_next,
            highest_modseq: head.highest_modseq,
            messages: std::mem::take(&mut head.messages),
            window: head.window,
        };
        FolderLoad { account: account.to_string(), folder: folder.to_string(), dir, st, pages: head.pages.len(), next: 0, marks }
    }

    /// Read the next page of the list; `true` once there is none left.
    pub fn step(&mut self, store: &Store) -> Result<bool> {
        if self.next >= self.pages {
            return Ok(true);
        }
        let page: Vec<Summary> = store.read_json(&page_path(&self.dir, self.next))?;
        self.st.messages.extend(page.into_iter().map(SharedSummary::new));
        self.next += 1;
        Ok(self.next >= self.pages)
    }

    /// Whether every page is read.
    pub fn done(&self) -> bool {
        self.next >= self.pages
    }

    /// Pages read so far, the head counted (tests).
    #[cfg(test)]
    pub(crate) fn pages_read(&self) -> usize {
        self.next.saturating_add(1)
    }

    /// The list as read: newest first, its keys its own.
    pub fn finish(self) -> FolderState {
        let mut st = self.st;
        // A big folder's list in pages (by number): put together, newest first again.
        if self.pages > 0 {
            sort_newest_first(&mut st.messages);
        }
        if !self.marks.is_empty() {
            let marks: HashMap<u32, Flags> = self.marks.into_iter().collect();
            for m in &mut st.messages {
                if let Some(f) = marks.get(&m.key.uid) {
                    SharedSummary::make_mut(m).flags.clone_from(f);
                }
            }
        }
        // Keys are rebuilt rather than trusted: a renamed or copied cache stays consistent.
        for m in &mut st.messages {
            // Just read: each its own, nothing copied.
            let m = SharedSummary::make_mut(m);
            m.key.account.clone_from(&self.account);
            m.key.folder.clone_from(&self.folder);
            m.key.validity = st.validity;
        }
        st
    }
}

impl Store {
    /// Keep a folder's list. A big one goes in pages of [`PAGE_MESSAGES`] by number, and only
    /// the pages that changed are written: one message marked read in a folder of hundreds of
    /// thousands rewrote all of it (hundreds of megabytes, held in memory twice over).
    pub fn save_folder(&self, account: &str, folder: &str, state: &FolderState) -> Result<()> {
        self.save_folder_counted(account, folder, state, None).map(drop)
    }

    /// [`Store::save_folder`] after only these messages' flags changed (the same messages, in
    /// number): only their pages are made again (a big folder's other pages aren't even turned
    /// into text to see they're the same).
    pub fn save_folder_changed(&self, account: &str, folder: &str, state: &FolderState, changed: &std::collections::HashSet<u32>) -> Result<()> {
        self.save_folder_counted(account, folder, state, Some(changed)).map(drop)
    }

    /// [`Store::save_folder`], saying how many files it wrote (the list and the pages that
    /// changed). Under the lock [`Store::set_has_files`] puts a list right under.
    pub(crate) fn save_folder_counted(
        &self,
        account: &str,
        folder: &str,
        state: &FolderState,
        changed: Option<&std::collections::HashSet<u32>>,
    ) -> Result<usize> {
        // A list written whole after the marks handed over before, never under them.
        self.flush_marks_of(account, Some(folder));
        let _held = self.hold_changes();
        let (writes, stale) = self.list_writes(account, folder, state, changed)?;
        // The list and its pages together (one transaction in a data file).
        let written = writes.len();
        self.write_files(writes)?;
        self.remove_stale(stale);
        Ok(written)
    }

    /// Whether the marks beside a list's pages ([`MARKS`]) have any.
    pub(crate) fn has_marks(&self, path: &Path) -> bool {
        self.files.read(path).is_ok_and(|b| b.as_slice() != b"[]")
    }

    /// Pages a list no longer has, gone once it's written without them.
    pub(crate) fn remove_stale(&self, stale: Vec<PathBuf>) {
        for p in stale {
            let _ = self.files.remove_file(&p);
        }
    }

    /// What keeping a folder's list writes (with the lock held), and the pages it no longer has.
    /// Told only flags changed, and with few marked since its pages were written: the marks alone
    /// ([`MARKS`]), with the head when its numbers changed. Otherwise the pages that changed and the head, and the marks emptied.
    pub(crate) fn list_writes(
        &self,
        account: &str,
        folder: &str,
        state: &FolderState,
        changed: Option<&std::collections::HashSet<u32>>,
    ) -> Result<(Vec<(PathBuf, Vec<u8>)>, Vec<PathBuf>)> {
        // Missing or damaged reads as none; a failed read is an error, not a list without pages
        // (which wrote every page again, and left the stale ones).
        let old: StateFile = self.read_json(&self.folder_dir(account, folder).join(HEAD))?;
        self.list_writes_over(account, folder, state, changed, &old, None)
    }

    /// [`Store::list_writes`] over the head read already from the disk (`old`), and the marks beside
    /// its pages when they were read too (`kept`; `None`: read when they're needed).
    pub(crate) fn list_writes_over(
        &self,
        account: &str,
        folder: &str,
        state: &FolderState,
        changed: Option<&std::collections::HashSet<u32>>,
        old: &StateFile,
        kept: Option<&[(u32, Flags)]>,
    ) -> Result<(Vec<(PathBuf, Vec<u8>)>, Vec<PathBuf>)> {
        let dir = self.folder_dir(account, folder);
        let head = dir.join(HEAD);
        let mut file = StateFile {
            validity: state.validity,
            uid_next: state.uid_next,
            highest_modseq: state.highest_modseq,
            messages: Vec::new(),
            window: state.window,
            pages: Vec::new(),
            count: 0,
        };
        let mut writes: Vec<(PathBuf, Vec<u8>)> = Vec::new();
        let marks_path = dir.join(MARKS);
        if state.messages.len() <= PAGE_MESSAGES {
            file.messages = state.messages.clone();
        } else {
            let n = state.messages.len();
            // The same messages as last time, some flags changed: the other pages stand.
            let same = changed.filter(|_| old.count == n && old.pages.len() == n.div_ceil(PAGE_MESSAGES));
            // Few marked since the pages were written: those kept beside them, nothing else.
            let mut folded: Option<std::collections::HashSet<u32>> = None;
            if let Some(changed) = same {
                let read: Vec<(u32, Flags)>;
                let kept = match kept {
                    Some(kept) => kept,
                    None => {
                        read = self.read_json(&marks_path)?;
                        &read
                    }
                };
                let mut marks: BTreeMap<u32, &Flags> = kept.iter().map(|(uid, f)| (*uid, f)).collect();
                for m in state.messages.iter().filter(|m| changed.contains(&m.key.uid)) {
                    marks.insert(m.key.uid, &m.flags);
                }
                if marks.len() <= MARKS_MAX {
                    let list: Vec<(u32, &Flags)> = marks.into_iter().collect();
                    let bytes = to_json(&marks_path, &list)?;
                    let mut writes = vec![(marks_path, bytes)];
                    // A head that says more than the one on disk (a newer modseq) written too,
                    // naming the same pages.
                    if (old.validity, old.uid_next, old.highest_modseq, old.window)
                        != (file.validity, file.uid_next, file.highest_modseq, file.window)
                    {
                        file.pages.clone_from(&old.pages);
                        file.count = old.count;
                        let bytes = to_json(&head, &file)?;
                        writes.push((head, bytes));
                    }
                    return Ok((writes, Vec::new()));
                }
                // Too many: their pages are written again (as the list has them now), and the
                // marks emptied.
                folded = Some(marks.into_keys().collect());
            }
            let same = folded.as_ref().or(same);
            let mut by_number: Vec<&Summary> = state.messages.iter().map(|m| &**m).collect();
            by_number.sort_by_key(|m| m.key.uid);
            file.count = n;
            for (i, page) in by_number.chunks(PAGE_MESSAGES).enumerate() {
                if let Some(changed) = same
                    && let Some(&hash) = old.pages.get(i)
                    && !page.iter().any(|m| changed.contains(&m.key.uid))
                {
                    file.pages.push(hash);
                    continue;
                }
                let path = page_path(&dir, i);
                let bytes = to_json(&path, page)?;
                let hash = page_hash(&bytes);
                if old.pages.get(i) != Some(&hash) || !self.files.is_file(&path) {
                    writes.push((path, bytes));
                }
                file.pages.push(hash);
            }
        }
        let stale = self.head_writes(&dir, &file, old, &mut writes)?;
        Ok((writes, stale))
    }

    /// `file` written as the head of the list in `dir` after its pages (in `writes`), with the
    /// lock held: the marks beside the pages emptied first when they have any (the pages have
    /// every flag now), then the head. The pages `old` (the head it replaces) named and `file`
    /// doesn't, to go once these are written.
    pub(crate) fn head_writes(&self, dir: &Path, file: &StateFile, old: &StateFile, writes: &mut Vec<(PathBuf, Vec<u8>)>) -> Result<Vec<PathBuf>> {
        let marks = dir.join(MARKS);
        if self.has_marks(&marks) {
            writes.push((marks, b"[]".to_vec()));
        }
        let head = dir.join(HEAD);
        let bytes = to_json(&head, file)?;
        writes.push((head, bytes));
        Ok((file.pages.len()..old.pages.len()).map(|i| page_path(dir, i)).collect())
    }

    /// Put right whether these messages carry files (what an index pass found their summaries
    /// to say otherwise about), in the folder's list as it stands now: only the pages holding
    /// one that changed are written again, with the list's head (which names its pages), in one
    /// transaction. It reads and writes under the lock every save of a list takes, so a flag
    /// marked meanwhile from the window isn't written over. The other way round it isn't kept
    /// from: a save that loaded the list before this (a sync, a flag from the window) writes the
    /// old flags back; the pass that put the folder right leaves it unmarked
    /// ([`Pass::unmark`](crate::search::Pass::unmark)), so the next looks at it again and puts it
    /// right once more. A list of another validity is left as it is. How many summaries changed.
    pub fn set_has_files(&self, account: &str, folder: &str, found: &crate::search::FilesFound) -> Result<usize> {
        if found.is_empty() {
            return Ok(0);
        }
        let with: std::collections::HashSet<u32> = found.with.iter().copied().collect();
        let without: std::collections::HashSet<u32> = found.without.iter().copied().collect();
        let wanted = with.len().saturating_add(without.len());
        // Whether `m` is one of them (`.0`), and whether it changed (`.1`).
        let set = |m: &mut Summary| {
            let files = if with.contains(&m.key.uid) {
                true
            } else if without.contains(&m.key.uid) {
                false
            } else {
                return (false, false);
            };
            let changed = m.has_attachments != files;
            m.has_attachments = files;
            (true, changed)
        };
        let _held = self.hold_changes();
        let dir = self.folder_dir(account, folder);
        let head = dir.join(HEAD);
        let mut file: StateFile = self.read_json(&head)?;
        if file.validity != found.validity {
            return Ok(0);
        }
        let (mut seen, mut changed) = (0usize, 0usize);
        for m in &mut file.messages {
            let (one, c) = set(SharedSummary::make_mut(m));
            seen += usize::from(one);
            changed += usize::from(c);
        }
        let mut writes: Vec<(PathBuf, Vec<u8>)> = Vec::new();
        for (i, hash) in file.pages.iter_mut().enumerate() {
            if seen >= wanted {
                break;
            }
            let path = page_path(&dir, i);
            let mut page: Vec<Summary> = self.read_json(&path)?;
            let mut here = 0usize;
            for m in &mut page {
                let (one, c) = set(m);
                seen += usize::from(one);
                here += usize::from(c);
            }
            if here == 0 {
                continue;
            }
            let bytes = to_json(&path, &page)?;
            *hash = page_hash(&bytes);
            writes.push((path, bytes));
            changed += here;
        }
        if changed == 0 {
            return Ok(0);
        }
        let bytes = to_json(&head, &file)?;
        writes.push((head, bytes));
        self.write_files(writes)?;
        Ok(changed)
    }

    /// Write a folder's list a page at a time (importing a data file, the whole list needn't be
    /// held): messages are handed over in rising number order, then [`ListWriter::finish`]. The
    /// pages come out as [`Store::save_folder`] makes them.
    pub fn list_writer(&self, account: &str, folder: &str) -> ListWriter {
        ListWriter { account: account.to_string(), folder: folder.to_string(), pages: Vec::new(), page: Vec::new(), total: 0, unread: 0 }
    }

    /// Go on writing a folder's list from where `tail` says it had got ([`ListWriter::tail`]):
    /// the pages it names are taken as written, and its messages not yet in one are handed over
    /// again (a page they fill is written now). A tail read back from a file is hostile: one that
    /// doesn't add up (pages that aren't full ones, more unread than messages, too many) is an
    /// error, and nothing is written.
    pub fn resume_list_writer(&self, account: &str, folder: &str, tail: ListTail) -> Result<ListWriter> {
        if !tail.is_sound() {
            let source = io::Error::new(io::ErrorKind::InvalidData, "a folder list's tail that doesn't add up");
            return Err(StoreError::Io { path: format!("{account}/{folder}"), source });
        }
        let ListTail { pages, in_pages, unread_in_pages, page } = tail;
        let total = usize::try_from(in_pages).unwrap_or(usize::MAX);
        let mut w = ListWriter { account: account.to_string(), folder: folder.to_string(), pages, page: Vec::new(), total, unread: unread_in_pages };
        w.push(self, page)?;
        Ok(w)
    }

    /// Several files at once (one transaction in a data file).
    pub(super) fn write_files(&self, items: Vec<(PathBuf, Vec<u8>)>) -> Result<()> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.files.write_many(items)
        }
        #[cfg(target_arch = "wasm32")]
        {
            for (p, bytes) in &items {
                self.files.write(p, bytes)?;
            }
            Ok(())
        }
    }

    /// Forget a folder's cache (UIDVALIDITY changed, or the folder was deleted on the server).
    pub fn clear_folder(&self, account: &str, folder: &str) -> Result<()> {
        let dir = self.folder_dir(account, folder);
        match self.files.remove_dir_all(&dir) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(io_err(&dir)(e)),
            _ => Ok(()),
        }
    }

    /// Apply a change to the cached folders at once (optimistic update). Returns the folders that
    /// changed. Moves only remove from the source: the destination learns the new UID at its
    /// next sync (or, kept only here, has it filed already). A source taken out of its folder,
    /// moved or deleted for good, goes with it: a moved one stayed in the data file for good,
    /// in no folder (2,453 moved to Deleted Items and emptied kept their 2 GB).
    pub fn apply_op(&self, account: &str, op: &Op) -> Result<Vec<String>> {
        if op.is_folder_op() {
            return self.apply_folder_op(account, op);
        }
        if let Op::EmptyFolder { path } = op {
            let mut st = self.load_folder(account, path)?;
            // The list first: one that can't be saved still has its messages' sources.
            let gone = std::mem::take(&mut st.messages);
            self.save_folder(account, path, &st)?;
            for m in &gone {
                self.remove_body(&m.key);
            }
            let mut list = self.load_folders(account)?;
            for f in list.iter_mut().filter(|f| &f.path == path) {
                f.total = 0;
                f.unread = 0;
            }
            self.save_folders(account, &list)?;
            return Ok(vec![path.clone()]);
        }
        // Every message kept read, and the folder's count with them (the server's mail past what's
        // kept is marked there, issue #197).
        if let Op::MarkFolderRead { path } = op {
            let mut st = self.load_folder(account, path)?;
            let mut changed = HashSet::new();
            for m in st.messages.iter_mut().filter(|m| !m.flags.seen) {
                changed.insert(m.key.uid);
                SharedSummary::make_mut(m).flags.seen = true;
            }
            if !changed.is_empty() {
                self.save_folder_changed(account, path, &st, &changed)?;
            }
            let mut list = self.load_folders(account)?;
            let mut counted = false;
            for f in list.iter_mut().filter(|f| &f.path == path && f.unread != 0) {
                f.unread = 0;
                counted = true;
            }
            if counted {
                self.save_folders(account, &list)?;
            }
            return Ok(if changed.is_empty() && !counted { Vec::new() } else { vec![path.clone()] });
        }
        let mut folders: Vec<String> = op.keys().iter().filter(|k| k.account == account).map(|k| k.folder.clone()).collect();
        folders.sort();
        folders.dedup();
        for folder in &folders {
            let mut st = self.load_folder(account, folder)?;
            let mut sources_gone: Vec<&MessageKey> = Vec::new();
            // Only these messages' flags changed: a big list's other pages aren't made again.
            let mut flags_of: Option<HashSet<u32>> = None;
            let in_folder = |k: &MessageKey| k.account == account && &k.folder == folder && k.validity == st.validity;
            match op {
                Op::SetFlag { keys, flag, on } => {
                    // A set: every message of a folder may change at once (looked up in a list, that was quadratic).
                    let uids: HashSet<u32> = keys.iter().filter(|k| in_folder(k)).map(|k| k.uid).collect();
                    for m in st.messages.iter_mut().filter(|m| uids.contains(&m.key.uid)) {
                        flag.set(&mut SharedSummary::make_mut(m).flags, *on);
                    }
                    flags_of = Some(uids);
                }
                Op::SetKeyword { keys, keyword, on } => {
                    // A set: every message of a folder may change at once (looked up in a list, that was quadratic).
                    let uids: HashSet<u32> = keys.iter().filter(|k| in_folder(k)).map(|k| k.uid).collect();
                    for m in st.messages.iter_mut().filter(|m| uids.contains(&m.key.uid)) {
                        SharedSummary::make_mut(m).flags.set_keyword(keyword, *on);
                    }
                    flags_of = Some(uids);
                }
                Op::SetFollowUp { keys, due } => {
                    let uids: HashSet<u32> = keys.iter().filter(|k| in_folder(k)).map(|k| k.uid).collect();
                    for m in st.messages.iter_mut().filter(|m| uids.contains(&m.key.uid)) {
                        SharedSummary::make_mut(m).flags.follow_up_due = due.map(|d| d.day);
                    }
                    flags_of = Some(uids);
                }
                Op::SetReminder { keys, at } => {
                    let uids: HashSet<u32> = keys.iter().filter(|k| in_folder(k)).map(|k| k.uid).collect();
                    for m in st.messages.iter_mut().filter(|m| uids.contains(&m.key.uid)) {
                        SharedSummary::make_mut(m).flags.reminder = *at;
                    }
                    flags_of = Some(uids);
                }
                Op::Move { keys, to } => {
                    if to == folder {
                        continue;
                    }
                    // A set: every message of a folder may change at once (looked up in a list, that was quadratic).
                    sources_gone = keys.iter().filter(|k| in_folder(k)).collect();
                    let uids: HashSet<u32> = sources_gone.iter().map(|k| k.uid).collect();
                    st.messages.retain(|m| !uids.contains(&m.key.uid));
                }
                Op::Expunge { keys } => {
                    sources_gone = keys.iter().filter(|k| in_folder(k)).collect();
                    let uids: HashSet<u32> = sources_gone.iter().map(|k| k.uid).collect();
                    st.messages.retain(|m| !uids.contains(&m.key.uid));
                }
                Op::CreateFolder { .. } | Op::RenameFolder { .. } | Op::DeleteFolder { .. } | Op::EmptyFolder { .. } | Op::MarkFolderRead { .. } => {}
            }
            match &flags_of {
                Some(changed) => self.save_folder_changed(account, folder, &st, changed)?,
                None => self.save_folder(account, folder, &st)?,
            }
            // Once the list no longer has them: one that can't be saved keeps their sources.
            let gone: Vec<MessageKey> = sources_gone.into_iter().cloned().collect();
            self.remove_bodies(account, &gone);
        }
        Ok(folders)
    }

    /// Folder-list changes: `folders.json` is updated and each folder's cache moves with it (or
    /// goes, for a deleted folder). Returns the folders whose contents should be shown afresh.
    pub(super) fn apply_folder_op(&self, account: &str, op: &Op) -> Result<Vec<String>> {
        let _held = self.hold_changes();
        let mut list = self.read_folders(account)?;
        let delimiter = list.iter().find_map(|f| f.delimiter);
        let changed = match op {
            Op::CreateFolder { path } => {
                if !list.iter().any(|f| &f.path == path) {
                    // Made in another mailbox (a shared one), it's in that mailbox too.
                    let parent = delimiter.and_then(|d| path.rsplit_once(d)).map(|(p, _)| p);
                    let mailbox = list.iter().find(|f| Some(f.path.as_str()) == parent).and_then(|f| f.mailbox.clone());
                    list.push(Folder {
                        account: account.to_string(),
                        path: path.clone(),
                        delimiter,
                        role: mailcraft_model::FolderRole::Other,
                        selectable: true,
                        mailbox,
                        ..Default::default()
                    });
                }
                vec![path.clone()]
            }
            Op::RenameFolder { from, to } => {
                let mut moved = Vec::new();
                for f in list.iter_mut() {
                    if let Some(new) = mailcraft_model::rebase_path(&f.path, from, to, f.delimiter.or(delimiter)) {
                        let (src, dst) = (self.folder_dir(account, &f.path), self.folder_dir(account, &new));
                        if self.files.exists(&src) {
                            let _ = self.files.remove_dir_all(&dst);
                            self.files.rename(&src, &dst).map_err(io_err(&src))?;
                        }
                        f.path = new.clone();
                        moved.push(new);
                    }
                }
                moved
            }
            Op::DeleteFolder { path } => {
                let gone: Vec<String> =
                    list.iter().filter(|f| mailcraft_model::is_within(&f.path, path, f.delimiter.or(delimiter))).map(|f| f.path.clone()).collect();
                for g in &gone {
                    self.clear_folder(account, g)?;
                }
                // A set: a folder with thousands of subfolders (an import's) was gone × folders.
                let gone: HashSet<String> = gone.into_iter().collect();
                list.retain(|f| !gone.contains(&f.path));
                Vec::new()
            }
            Op::SetFlag { .. }
            | Op::SetFollowUp { .. }
            | Op::SetReminder { .. }
            | Op::SetKeyword { .. }
            | Op::Move { .. }
            | Op::Expunge { .. }
            | Op::EmptyFolder { .. }
            | Op::MarkFolderRead { .. } => Vec::new(),
        };
        mailcraft_model::sort_folders(&mut list);
        self.write_folders(account, &list)?;
        Ok(changed)
    }

    /// Add `folder` to the account's list, or, when the list has a folder at its path, set only
    /// that one's counts: an import adds each folder as it finishes, and what the window changed
    /// meanwhile (a folder renamed, another's counts) stays. Nothing is ever taken out.
    pub fn upsert_folder(&self, account: &str, folder: &Folder) -> Result<()> {
        self.upsert_folders(account, std::slice::from_ref(folder))
    }

    /// [`Store::upsert_folder`] for each of `folders`, in one write of the list: an import's
    /// folders finished together cost one write, however many there are.
    pub fn upsert_folders(&self, account: &str, folders: &[Folder]) -> Result<()> {
        self.flush_marks_of(account, None);
        if folders.is_empty() {
            return Ok(());
        }
        let _held = self.hold_changes();
        let mut list = self.read_folders(account)?;
        let mut at: std::collections::HashMap<String, usize> = list.iter().enumerate().map(|(i, f)| (f.path.clone(), i)).collect();
        for folder in folders {
            match at.get(&folder.path).and_then(|i| list.get_mut(*i)) {
                Some(f) => {
                    f.unread = folder.unread;
                    f.total = folder.total;
                }
                None => {
                    at.insert(folder.path.clone(), list.len());
                    list.push(Folder { account: account.to_string(), ..folder.clone() });
                }
            }
        }
        self.write_folders(account, &list)
    }

    /// Set the counts (`path`, unread, total) of the account's folders that are in its list, as
    /// the list stands now: a list held elsewhere (the window's) written whole would drop the
    /// folders an import added since it was read. Says how many of them the list has.
    pub fn update_folder_counts(&self, account: &str, counts: &[(String, u32, u32)]) -> Result<usize> {
        self.flush_marks_of(account, None);
        let _held = self.hold_changes();
        let mut list = self.read_folders(account)?;
        let (found, changed) = set_counts(&mut list, counts);
        if changed {
            self.write_folders(account, &list)?;
        }
        Ok(found)
    }
}

/// Set the counts (`path`, unread, total) of the folders `list` has: how many it has, and
/// whether any count changed.
pub(crate) fn set_counts(list: &mut [Folder], counts: &[(String, u32, u32)]) -> (usize, bool) {
    let (mut found, mut changed) = (0, false);
    // Each path's first folder (a damaged list may name one twice), looked up once each: a search
    // of the list for each count was counts × folders (41 ms for an import's 5,000).
    let mut at: HashMap<&str, usize> = HashMap::with_capacity(list.len());
    for (i, f) in list.iter().enumerate() {
        at.entry(f.path.as_str()).or_insert(i);
    }
    let found_at: Vec<(usize, u32, u32)> =
        counts.iter().filter_map(|(path, unread, total)| Some((*at.get(path.as_str())?, *unread, *total))).collect();
    for (i, unread, total) in found_at {
        if let Some(f) = list.get_mut(i) {
            found += 1;
            changed |= f.unread != unread || f.total != total;
            f.unread = unread;
            f.total = total;
        }
    }
    (found, changed)
}
