//! The store: an Outlook data file read again into the copy MailCraft keeps of it (`beside.rs`),
//! once the file has changed since. The file is read afresh into a store of its own; what it has
//! that the copy hasn't is then added to the copy, and nothing of the copy is changed or taken
//! out, so whatever was done in MailCraft stays: a message read, flagged, moved or deleted there
//! stays so. What came from the file before is noted by its identity ([`SEEN_DOC`]), so a message
//! or a contact deleted in MailCraft isn't brought back by the next read. What the file changed in
//! an item MailCraft has already (a message's flags in Outlook, a contact edited there) isn't
//! merged: the item as changed is new to the copy, and comes in beside it (a message by its
//! Message-ID is the same message, so it doesn't).

use mailcraft_model::FolderRole;
use mailcraft_model::organize::{FOLLOW_UP_DOC, FOLLOW_UP_DONE_DOC, merge_follow_up};
use mailcraft_model::pim::{LOCAL, PIM_DOC, Pim};

use crate::*;

/// The account document noting what came into it from its data file, by identity: kept beside
/// the import's own note.
pub const SEEN_DOC: &str = "import-seen.json";
/// The most identities of each kind kept, the oldest noted let go first: past it, a message or
/// item of those deleted here would come back with the next read of the file.
const SEEN_MAX: usize = 4_000_000;
/// Messages copied into the account together, at most (one write).
const BATCH: usize = 500;

/// What came into an account from its data file, by identity ([`message_id`], [`item_id`]),
/// each list in the order it was noted (the oldest first).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct Seen {
    messages: Vec<u64>,
    items: Vec<u64>,
}

/// Identities noted, to look up and add to, keeping the order they were noted in.
#[derive(Debug, Default)]
struct Noted {
    set: HashSet<u64>,
    /// In the order noted; one taken back ([`Noted::remove`]) is left here and dropped at the end.
    order: Vec<u64>,
}

impl Noted {
    fn new(order: Vec<u64>) -> Noted {
        Noted { set: order.iter().copied().collect(), order }
    }

    /// Note `id`: whether it's new.
    fn insert(&mut self, id: u64) -> bool {
        let new = self.set.insert(id);
        if new {
            self.order.push(id);
        }
        new
    }

    fn remove(&mut self, id: u64) {
        self.set.remove(&id);
    }

    /// The identities noted, oldest first, each once.
    fn into_list(mut self) -> Vec<u64> {
        self.order.retain(|id| self.set.remove(id));
        self.order
    }
}

/// `list` (oldest first) with each identity once, where it was first noted, and no more than
/// `max` of them: the oldest go.
pub(crate) fn kept(list: Vec<u64>, max: usize) -> Vec<u64> {
    let mut once = HashSet::with_capacity(list.len());
    let mut list: Vec<u64> = list.into_iter().filter(|id| once.insert(*id)).collect();
    let excess = list.len().saturating_sub(max);
    if excess > 0 {
        log::warn!("more than {max} identities noted from a data file: the oldest {excess} let go of");
        list.drain(..excess);
    }
    list
}

/// What reading a data file again added to its copy ([`Store::merge_from`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Merged {
    pub messages: usize,
    /// The folders that had messages added (made new when the copy had none of that path).
    pub folders: Vec<String>,
    /// Contacts, calendar items, notes and tasks added.
    pub cards: usize,
    pub events: usize,
    pub notes: usize,
    pub tasks: usize,
    /// Messages left out because their source couldn't be read in the file read again: not
    /// noted as come in, so the next read tries them again.
    pub unreadable: usize,
}

/// A message's identity across two reads of a file: its Message-ID, else when it was sent, by
/// whom and its subject. A Message-ID the import made up for an item with none names the item's
/// node in the file, which is the identity then: the account and the file's tag it names too
/// change from one read to the next (the tag with every write to the file).
fn message_id(s: &Summary) -> u64 {
    let id = s.message_id.trim().to_lowercase();
    if let Some(node) = made_up_node(&id) {
        fnv64(&format!("n:{node}"))
    } else if id.is_empty() {
        fnv64(&format!("d:{}|{}|{}", s.date, s.from.email.to_lowercase(), s.subject))
    } else {
        fnv64(&format!("m:{id}"))
    }
}

/// The node (hex) named by a Message-ID the import made up
/// (`pst.<account>.<tag>.<node>@mailcraft.invalid`, lowercase), or `None` for any other.
fn made_up_node(id: &str) -> Option<&str> {
    let id = id.strip_prefix('<').unwrap_or(id);
    let id = id.strip_suffix('>').unwrap_or(id);
    let local = id.strip_prefix("pst.")?.strip_suffix("@mailcraft.invalid")?;
    let mut parts = local.rsplitn(3, '.');
    let node = parts.next()?;
    let (Some(tag), Some(account)) = (parts.next(), parts.next()) else { return None };
    let hex = |x: &str| !x.is_empty() && x.bytes().all(|b| b.is_ascii_hexdigit());
    (hex(node) && hex(tag) && !account.is_empty()).then_some(node)
}

/// A contact's, calendar item's, note's or task's identity: what it holds, but for its uid (an
/// import numbers them as it reads them, so the same item read again has another).
fn item_id<T: Serialize>(kind: &str, item: &T, uid: &str) -> u64 {
    let json = serde_json::to_string(item).unwrap_or_default();
    let json = if uid.is_empty() { json } else { json.replace(uid, "") };
    fnv64(&format!("{kind}:{json}"))
}

/// What an item of an account's [`Pim`] is: each kind's place in what
/// [`Store::merge_pim`] counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Card = 0,
    Event = 1,
    Note = 2,
    Task = 3,
}

/// Every item of `pim` by identity, with where it is (its kind and place in its list).
fn items_of(pim: &Pim) -> Vec<(u64, Kind, usize)> {
    let mut out = Vec::with_capacity(pim.cards.len() + pim.events.len() + pim.notes.len() + pim.tasks.len());
    out.extend(pim.cards.iter().enumerate().map(|(i, c)| (item_id("c", &c.card, &c.card.uid), Kind::Card, i)));
    out.extend(pim.events.iter().enumerate().map(|(i, e)| (item_id("e", &e.event, &e.event.uid), Kind::Event, i)));
    out.extend(pim.notes.iter().enumerate().map(|(i, n)| (item_id("n", &n.note, &n.note.uid), Kind::Note, i)));
    out.extend(pim.tasks.iter().enumerate().map(|(i, t)| (item_id("t", &t.task, &t.task.uid), Kind::Task, i)));
    out
}

impl Store {
    /// Note what `account` holds now as what came in from its data file, beside what was noted
    /// before: called as an import of one ends, and as it stops part-way (what's deleted here
    /// meanwhile stays noted), so a later read of the file ([`Store::merge_from`]) knows it.
    pub fn note_imported(&self, account: &str) -> Result<()> {
        let mut seen = match self.load_account_doc_bytes(account, SEEN_DOC)? {
            Some(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
                log::warn!("{account}: {SEEN_DOC} unreadable, noted afresh: {e}");
                Seen::default()
            }),
            None => Seen::default(),
        };
        for f in self.load_folders(account)? {
            seen.messages.extend(self.load_folder(account, &f.path)?.messages.iter().map(|m| message_id(m)));
        }
        let pim: Pim = self.load_account_doc(account, PIM_DOC)?;
        seen.items.extend(items_of(&pim).into_iter().map(|(id, ..)| id));
        self.save_seen(account, seen)
    }

    /// Whether `account` has a note of what came in from its data file ([`SEEN_DOC`]): without
    /// one, what its file has new can't be told from what was deleted here, and it isn't read
    /// again ([`Store::merge_from`] refuses).
    pub fn has_seen(&self, account: &str) -> bool {
        self.files.is_file(&self.account_dir(account).join(SEEN_DOC))
    }

    /// The note of what came in from `account`'s data file: an error when there's none, or it
    /// can't be read (left where it is), rather than nothing noted.
    fn seen_of(&self, account: &str) -> Result<Seen> {
        let path = format!("{account}/{SEEN_DOC}");
        let Some(bytes) = self.load_account_doc_bytes(account, SEEN_DOC)? else {
            let why = "there's no note of what came in from the file before, so what's new in it can't be told from what was deleted here";
            return Err(StoreError::Io { path, source: io::Error::other(why) });
        };
        serde_json::from_slice(&bytes).map_err(|source| StoreError::Json { path, source })
    }

    /// Keep the note of what came in (each list oldest first), each identity once and no more
    /// than [`SEEN_MAX`] of a kind.
    fn save_seen(&self, account: &str, seen: Seen) -> Result<()> {
        let seen = Seen { messages: kept(seen.messages, SEEN_MAX), items: kept(seen.items, SEEN_MAX) };
        self.save_account_doc(account, SEEN_DOC, &seen)
    }

    /// Add to `account` what `from`'s `from_account` (the same data file, read afresh) has that it
    /// hasn't: each message neither in `account` (in any folder) nor brought in from the file
    /// before, into the folder of the same path (made when there's none), and each contact,
    /// calendar item, note and task the same way. Nothing in `account` changes but by being
    /// added to; its messages keep their numbers, and what's marked while this runs stays (each
    /// list is read and written under the lists' lock). A message whose source `from` hasn't (or
    /// has damaged) is left out and not noted, so the next read tries it again
    /// ([`Merged::unreadable`]). An account with no note of what came in before is refused
    /// ([`Store::has_seen`]).
    pub fn merge_from(&self, account: &str, from: &Store, from_account: &str) -> Result<Merged> {
        let seen = self.seen_of(account)?;
        let (mut seen_messages, mut seen_items) = (Noted::new(seen.messages), Noted::new(seen.items));
        let mut have: HashSet<u64> = HashSet::new();
        for f in &self.load_folders(account)? {
            have.extend(self.load_folder(account, &f.path)?.messages.iter().map(|m| message_id(m)));
        }
        let mut merged = Merged::default();
        // The flagged (or completed) messages added: their follow-up ids in the file's store, and here.
        let mut flagged: Vec<(String, String)> = Vec::new();
        let mut theirs = from.load_folders(from_account)?;
        // Parents before the folders inside them.
        theirs.sort_by(|a, b| a.path.cmp(&b.path));
        for f in theirs {
            let list = from.load_folder(from_account, &f.path)?;
            let mut new = Vec::new();
            for m in &list.messages {
                let id = message_id(m);
                if seen_messages.insert(id) && have.insert(id) {
                    new.push((id, m));
                }
            }
            if new.is_empty() {
                continue;
            }
            let n = u32::try_from(new.len()).map_err(|_| no_uids_left(account, &f.path))?;
            let (validity, mut uid) = self.reserve(account, &f.path, list.validity.max(1), n)?;
            let mut added = Vec::with_capacity(new.len());
            let mut unreadable = 0usize;
            for chunk in new.chunks(BATCH) {
                let mut bodies = Vec::with_capacity(chunk.len());
                for &(id, m) in chunk {
                    let Some(raw) = from.load_body(&m.key)? else {
                        // Not come in: not noted as come in either, so the next read tries it
                        // again (and another copy of it in the file may come in now).
                        seen_messages.remove(id);
                        have.remove(&id);
                        unreadable += 1;
                        continue;
                    };
                    let key = MessageKey {
                        account: account.to_string(),
                        folder: f.path.clone(),
                        validity,
                        uid: FolderState::take_uid(&mut uid, account, &f.path)?,
                    };
                    let mut s = Summary::clone(m);
                    s.key = key.clone();
                    if s.flags.flagged || s.flags.follow_up_complete() {
                        flagged.push((m.follow_up_id(), s.follow_up_id()));
                    }
                    bodies.push((key, raw));
                    added.push(SharedSummary::new(s));
                }
                // The sources first: a list never names one that isn't there.
                self.write_bodies(bodies)?;
            }
            if unreadable > 0 {
                log::warn!("{account}/{}: {unreadable} message(s) of the file read again couldn't be read, left for the next read", f.path);
                merged.unreadable += unreadable;
            }
            merged.messages += added.len();
            self.append(account, &f, validity, added)?;
            merged.folders.push(f.path.clone());
        }
        [merged.cards, merged.events, merged.notes, merged.tasks] = self.merge_pim(account, from, from_account, &mut seen_items)?;
        for doc in [FOLLOW_UP_DOC, FOLLOW_UP_DONE_DOC] {
            if let Err(e) = self.merge_due(account, from, from_account, &flagged, doc) {
                log::warn!("{account}: the days of its new messages' follow-ups weren't kept: {e}");
            }
        }
        self.save_seen(account, Seen { messages: seen_messages.into_list(), items: seen_items.into_list() })?;
        Ok(merged)
    }

    /// Numbers for `n` messages coming into `account`'s `folder`, taken under the lists' lock:
    /// its list is written with its next number past them, so nothing else is given them while
    /// their sources are written. The folder's validity, and the first number. A folder the
    /// account's list hasn't is started afresh, of `validity`.
    fn reserve(&self, account: &str, folder: &str, validity: u32, n: u32) -> Result<(u32, u32)> {
        self.flush_marks_of(account, Some(folder));
        let _held = self.hold_changes();
        let listed = self.read_folders(account)?.iter().any(|f| f.path == folder);
        let mut st = if listed { self.list_from_disk(account, folder)? } else { FolderState { validity, ..FolderState::default() } };
        let first = st.free_uid(account, folder)?;
        st.uid_next = first.checked_add(n).ok_or_else(|| no_uids_left(account, folder))?;
        let (writes, stale) = self.list_writes(account, folder, &st, Some(&HashSet::new()))?;
        self.write_files(writes)?;
        self.remove_stale(stale);
        Ok((st.validity, first))
    }

    /// `added` (their sources written) put in `account`'s folder of `theirs`' path, as its list
    /// stands now, and its counts in the account's folder list as that stands now (`theirs`
    /// added to it when it has no folder of that path): both read and written under the lists'
    /// lock, after the marks handed over before, so no mark or count made meanwhile is lost.
    fn append(&self, account: &str, theirs: &Folder, validity: u32, added: Vec<SharedSummary>) -> Result<()> {
        let path = theirs.path.as_str();
        self.flush_marks_of(account, None);
        let _held = self.hold_changes();
        let mut st = self.list_from_disk(account, path)?;
        if st.validity != validity {
            let why = "the folder was made again while the file was read again";
            return Err(StoreError::Io { path: format!("{account}/{path}"), source: io::Error::other(why) });
        }
        st.messages.extend(added);
        sort_newest_first(&mut st.messages);
        let (writes, stale) = self.list_writes(account, path, &st, None)?;
        self.write_files(writes)?;
        self.remove_stale(stale);
        let (total, unread) = (u32::try_from(st.messages.len()).unwrap_or(u32::MAX), st.unread());
        let mut list = self.read_folders(account)?;
        match list.iter_mut().find(|x| x.path == path) {
            Some(x) => (x.total, x.unread) = (total, unread),
            None => {
                // A folder renamed or deleted here comes back for what's new in it; never as a
                // second Inbox or Sent Items.
                let role = if list.iter().any(|x| x.role == theirs.role) { FolderRole::Other } else { theirs.role };
                list.push(Folder { account: account.to_string(), total, unread, role, ..theirs.clone() });
                mailcraft_model::sort_folders(&mut list);
            }
        }
        self.write_folders(account, &list)
    }

    /// The days the file gave the flagged (or completed) messages added (`flagged`: each one's
    /// follow-up id in `from_account`, and in `account`), put in `account`'s `doc` (the due days'
    /// [`FOLLOW_UP_DOC`] or the done days' [`FOLLOW_UP_DONE_DOC`]) for the engine to take, beside
    /// any it hasn't taken yet.
    fn merge_due(&self, account: &str, from: &Store, from_account: &str, flagged: &[(String, String)], doc: &str) -> Result<()> {
        if flagged.is_empty() {
            return Ok(());
        }
        let theirs: BTreeMap<String, i64> = from.load_account_doc(from_account, doc)?;
        let carried: BTreeMap<String, i64> = flagged.iter().filter_map(|(there, here)| theirs.get(there).map(|day| (here.clone(), *day))).collect();
        if carried.is_empty() {
            return Ok(());
        }
        let mut ours: BTreeMap<String, i64> = self.load_account_doc(account, doc)?;
        if merge_follow_up(&mut ours, carried) > 0 {
            self.save_account_doc(account, doc, &ours)?;
        }
        Ok(())
    }

    /// The contacts, calendar items, notes and tasks of `from_account` that `account` hasn't and
    /// didn't have from the file before, added (each with a uid of its own here). How many of
    /// each, in [`Kind`]'s order.
    fn merge_pim(&self, account: &str, from: &Store, from_account: &str, seen: &mut Noted) -> Result<[usize; 4]> {
        let theirs: Pim = from.load_account_doc(from_account, PIM_DOC)?;
        let mut pim: Pim = self.load_account_doc(account, PIM_DOC)?;
        pim.ensure_defaults();
        let have: HashSet<u64> = items_of(&pim).into_iter().map(|(id, ..)| id).collect();
        let mut uids: HashSet<String> = pim.cards.iter().map(|c| c.card.uid.clone()).collect();
        uids.extend(pim.events.iter().map(|e| e.event.uid.clone()));
        uids.extend(pim.notes.iter().map(|n| n.note.uid.clone()));
        uids.extend(pim.tasks.iter().map(|t| t.task.uid.clone()));
        let mut n = uids.len();
        let mut fresh = || loop {
            n = n.saturating_add(1);
            let uid = format!("pst-{account}-{n}@mailcraft.invalid");
            if uids.insert(uid.clone()) {
                return uid;
            }
        };
        let calendar = pim.calendars.first().map_or_else(|| LOCAL.to_string(), |c| c.id.clone());
        let mut added = [0usize; 4];
        for (id, kind, i) in items_of(&theirs) {
            if !seen.insert(id) || have.contains(&id) {
                continue;
            }
            match kind {
                Kind::Card => {
                    if let Some(c) = theirs.cards.get(i) {
                        let mut c = c.clone();
                        c.card.uid = fresh();
                        c.book = LOCAL.into();
                        pim.cards.push(c);
                    }
                }
                Kind::Event => {
                    if let Some(e) = theirs.events.get(i) {
                        let mut e = e.clone();
                        e.event.uid = fresh();
                        if !pim.calendars.iter().any(|c| c.id == e.calendar) {
                            e.calendar.clone_from(&calendar);
                        }
                        pim.events.push(e);
                    }
                }
                Kind::Note => {
                    if let Some(x) = theirs.notes.get(i) {
                        let mut x = x.clone();
                        x.note.uid = fresh();
                        pim.notes.push(x);
                    }
                }
                Kind::Task => {
                    if let Some(t) = theirs.tasks.get(i) {
                        let mut t = t.clone();
                        t.task.uid = fresh();
                        pim.tasks.push(t);
                    }
                }
            }
            if let Some(n) = added.get_mut(kind as usize) {
                *n += 1;
            }
        }
        if added.iter().any(|n| *n > 0) {
            self.save_account_doc(account, PIM_DOC, &pim)?;
        }
        Ok(added)
    }
}
