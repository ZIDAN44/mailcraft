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
use mailcraft_model::pim::{LOCAL, PIM_DOC, Pim};

use crate::*;

/// The account document noting what came into it from its data file, by identity: kept beside
/// the import's own note.
pub const SEEN_DOC: &str = "import-seen.json";
/// The most identities of each kind kept: a file of more has its newest read past it.
const SEEN_MAX: usize = 4_000_000;
/// Messages copied into the account together, at most (one write).
const BATCH: usize = 500;

/// What came into an account from its data file, by identity ([`message_id`], [`item_id`]).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct Seen {
    messages: Vec<u64>,
    items: Vec<u64>,
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
}

/// A message's identity across two reads of a file: its Message-ID, else when it was sent, by
/// whom and its subject.
fn message_id(s: &Summary) -> u64 {
    let id = s.message_id.trim();
    if id.is_empty() {
        fnv64(&format!("d:{}|{}|{}", s.date, s.from.email.to_lowercase(), s.subject))
    } else {
        fnv64(&format!("m:{}", id.to_lowercase()))
    }
}

/// A contact's, calendar item's, note's or task's identity: what it holds, but for its uid (an
/// import numbers them as it reads them, so the same item read again has another).
fn item_id<T: Serialize>(kind: &str, item: &T, uid: &str) -> u64 {
    let json = serde_json::to_string(item).unwrap_or_default();
    let json = if uid.is_empty() { json } else { json.replace(uid, "") };
    fnv64(&format!("{kind}:{json}"))
}

/// Every item of `pim` by identity, with where it is (its kind and place in its list).
fn items_of(pim: &Pim) -> Vec<(u64, u8, usize)> {
    let mut out = Vec::with_capacity(pim.cards.len() + pim.events.len() + pim.notes.len() + pim.tasks.len());
    out.extend(pim.cards.iter().enumerate().map(|(i, c)| (item_id("c", &c.card, &c.card.uid), 0, i)));
    out.extend(pim.events.iter().enumerate().map(|(i, e)| (item_id("e", &e.event, &e.event.uid), 1, i)));
    out.extend(pim.notes.iter().enumerate().map(|(i, n)| (item_id("n", &n.note, &n.note.uid), 2, i)));
    out.extend(pim.tasks.iter().enumerate().map(|(i, t)| (item_id("t", &t.task, &t.task.uid), 3, i)));
    out
}

impl Store {
    /// Note what `account` holds now as what came in from its data file: called as an import of
    /// one ends, so a later read of the file ([`Store::merge_from`]) knows it.
    pub fn note_imported(&self, account: &str) -> Result<()> {
        let mut seen = Seen::default();
        for f in self.load_folders(account)? {
            seen.messages.extend(self.load_folder(account, &f.path)?.messages.iter().map(|m| message_id(m)));
        }
        let pim: Pim = self.load_account_doc(account, PIM_DOC)?;
        seen.items = items_of(&pim).into_iter().map(|(id, ..)| id).collect();
        self.save_seen(account, seen)
    }

    fn save_seen(&self, account: &str, mut seen: Seen) -> Result<()> {
        for list in [&mut seen.messages, &mut seen.items] {
            list.sort_unstable();
            list.dedup();
            let excess = list.len().saturating_sub(SEEN_MAX);
            list.drain(..excess);
        }
        self.save_account_doc(account, SEEN_DOC, &seen)
    }

    /// Add to `account` what `from`'s `from_account` (the same data file, read afresh) has that it
    /// hasn't: each message neither in `account` (in any folder) nor brought in from the file
    /// before, into the folder of the same path (made when there's none), and each contact,
    /// calendar item, note and task the same way. Nothing in `account` changes but by being
    /// added to; its messages keep their numbers.
    pub fn merge_from(&self, account: &str, from: &Store, from_account: &str) -> Result<Merged> {
        let seen: Seen = self.load_account_doc(account, SEEN_DOC)?;
        let (mut seen_messages, mut seen_items): (HashSet<u64>, HashSet<u64>) =
            (seen.messages.iter().copied().collect(), seen.items.iter().copied().collect());
        let mut mine = self.load_folders(account)?;
        let mut have: HashSet<u64> = HashSet::new();
        for f in &mine {
            have.extend(self.load_folder(account, &f.path)?.messages.iter().map(|m| message_id(m)));
        }
        let mut merged = Merged::default();
        let mut theirs = from.load_folders(from_account)?;
        // Parents before the folders inside them.
        theirs.sort_by(|a, b| a.path.cmp(&b.path));
        for f in theirs {
            let list = from.load_folder(from_account, &f.path)?;
            let mut new = Vec::new();
            for m in &list.messages {
                let id = message_id(m);
                if seen_messages.insert(id) && have.insert(id) {
                    new.push(m.clone());
                }
            }
            if new.is_empty() {
                continue;
            }
            let at = mine.iter().position(|x| x.path == f.path);
            let mut st = if at.is_some() {
                self.load_folder(account, &f.path)?
            } else {
                FolderState { validity: list.validity.max(1), ..FolderState::default() }
            };
            let mut uid = st.free_uid(account, &f.path)?;
            for chunk in new.chunks(BATCH) {
                let mut bodies = Vec::with_capacity(chunk.len());
                let mut added = Vec::with_capacity(chunk.len());
                for m in chunk {
                    let Some(raw) = from.load_body(&m.key)? else { continue };
                    let key = MessageKey {
                        account: account.to_string(),
                        folder: f.path.clone(),
                        validity: st.validity,
                        uid: FolderState::take_uid(&mut uid, account, &f.path)?,
                    };
                    let mut s = Summary::clone(m);
                    s.key = key.clone();
                    bodies.push((key, raw));
                    added.push(SharedSummary::new(s));
                }
                // The sources first: a list never names one that isn't there.
                self.write_bodies(&bodies)?;
                merged.messages += added.len();
                st.messages.extend(added);
            }
            st.uid_next = st.uid_next.max(uid);
            sort_newest_first(&mut st.messages);
            self.save_folder(account, &f.path, &st)?;
            let (total, unread) = (u32::try_from(st.messages.len()).unwrap_or(u32::MAX), st.unread());
            match at.and_then(|i| mine.get_mut(i)) {
                Some(x) => (x.total, x.unread) = (total, unread),
                None => {
                    // A folder renamed or deleted here comes back for what's new in it; never as
                    // a second Inbox or Sent Items.
                    let role = if mine.iter().any(|x| x.role == f.role) { FolderRole::Other } else { f.role };
                    mine.push(Folder { account: account.to_string(), total, unread, role, ..f.clone() });
                }
            }
            merged.folders.push(f.path.clone());
        }
        mailcraft_model::sort_folders(&mut mine);
        self.save_folders(account, &mine)?;
        [merged.cards, merged.events, merged.notes, merged.tasks] = self.merge_pim(account, from, from_account, &mut seen_items)?;
        self.save_seen(account, Seen { messages: seen_messages.into_iter().collect(), items: seen_items.into_iter().collect() })?;
        Ok(merged)
    }

    /// The contacts, calendar items, notes and tasks of `from_account` that `account` hasn't and
    /// didn't have from the file before, added (each with a uid of its own here). How many of
    /// each.
    fn merge_pim(&self, account: &str, from: &Store, from_account: &str, seen: &mut HashSet<u64>) -> Result<[usize; 4]> {
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
                0 => {
                    if let Some(c) = theirs.cards.get(i) {
                        let mut c = c.clone();
                        c.card.uid = fresh();
                        c.book = LOCAL.into();
                        pim.cards.push(c);
                    }
                }
                1 => {
                    if let Some(e) = theirs.events.get(i) {
                        let mut e = e.clone();
                        e.event.uid = fresh();
                        if !pim.calendars.iter().any(|c| c.id == e.calendar) {
                            e.calendar.clone_from(&calendar);
                        }
                        pim.events.push(e);
                    }
                }
                2 => {
                    if let Some(x) = theirs.notes.get(i) {
                        let mut x = x.clone();
                        x.note.uid = fresh();
                        pim.notes.push(x);
                    }
                }
                _ => {
                    if let Some(t) = theirs.tasks.get(i) {
                        let mut t = t.clone();
                        t.task.uid = fresh();
                        pim.tasks.push(t);
                    }
                }
            }
            if let Some(n) = added.get_mut(usize::from(kind.min(3))) {
                *n += 1;
            }
        }
        if added.iter().any(|n| *n > 0) {
            self.save_account_doc(account, PIM_DOC, &pim)?;
        }
        Ok(added)
    }
}
