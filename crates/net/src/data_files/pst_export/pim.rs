//! The account's contacts, contact groups, notes and tasks as Outlook items, in the data file's
//! own Calendar, Contacts, Notes and Tasks folders (MS-OXOCAL, MS-OXOCNTC, MS-OXONOTE, MS-OXOTASK;
//! the folders recorded as the file's, MS-OXOSFLD 2.2, so Outlook shows them as its own): an event
//! as an `IPM.Appointment` ([`calendar`]), a card as an `IPM.Contact` or, a group's, an
//! `IPM.DistList` ([`contact`]), a note as an `IPM.StickyNote`, a task as an `IPM.Task`
//! ([`task`]). A file added to keeps its own items, and those it records as its
//! special folders take the account's. Each item is marked with its uid, the account's and a
//! digest of what went in (`pst_pim::EXPORTED_ITEM`): added to again, an item the file has as it
//! is now isn't written twice, and one changed since goes in again in place of the account's
//! earlier copy, which is taken out once the new one is in (so a failure between the two leaves
//! both, never neither). Another account's copy of the same uid stays (a meeting is in its
//! organizer's calendar and its guest's, as each has it), as do items the file has of a uid the
//! account no longer has: the export can't tell one the user deleted from one the account never
//! had here. What Outlook has no place for stays in the item's text, as the import reads it back
//! (`pst_pim::rest`).

use mailcraft_model::pim::{NOTE_COLORS, NOTE_YELLOW, Note, Pim};
use mailcraft_pst::{
    GuidValue, PS_PUBLIC_STRINGS,
    write::{self, Name, NewValue, SpecialFolder as Special},
};

use super::item::{PSETID_COMMON, filetime};
use super::*;
use std::collections::HashMap;

mod batch;
use batch::write_batch;
pub(super) use batch::{Lists, lists, properties};

/// PSETID_Address and PSETID_Note (MS-OXPROPS 1.3.2).
pub(super) const PSETID_ADDRESS: GuidValue = GuidValue::new(0x0006_2004, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
const PSETID_NOTE: GuidValue = GuidValue::new(0x0006_200E, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
/// A note's size in Outlook's window, as Outlook makes one (PidLidNoteWidth, PidLidNoteHeight).
const NOTE_SIZE: (i32, i32) = (200, 166);
/// Names tried for a new file's folder that the account's mail took already: "Notes (2)" and on.
pub(super) const MAX_SUFFIX: usize = 100;

/// A property's value.
pub(super) enum Value {
    Int(i32),
    Bool(bool),
    /// 100 ns since 1601.
    Time(i64),
    Float(f64),
    Text(String),
    Bin(Vec<u8>),
    Texts(Vec<String>),
    Bins(Vec<Vec<u8>>),
}

/// Which property: tagged, named by a number in a property set, the categories (`Keywords`), or
/// the item's mark (`pst_pim::EXPORTED_ITEM`).
pub(super) enum Key {
    Tag(u16),
    Lid(GuidValue, u32),
    Keywords,
    Exported,
}

pub(super) fn tag(id: u16, value: Value) -> (Key, Value) {
    (Key::Tag(id), value)
}

pub(super) fn lid(set: GuidValue, id: u32, value: Value) -> (Key, Value) {
    (Key::Lid(set, id), value)
}

/// A file of an item; `picture`: a contact's picture (PidTagAttachmentContactPhoto).
pub(super) struct File {
    pub(super) name: String,
    pub(super) mime: String,
    pub(super) data: Vec<u8>,
    pub(super) picture: bool,
}

/// Someone an item is to: an appointment's organizer or one of its attendees, with its
/// PidTagRecipientFlags and PidTagRecipientTrackStatus when it has them (MS-OXOCAL 2.2.4.10).
#[derive(Clone)]
pub(super) struct Person {
    pub(super) name: String,
    pub(super) email: String,
    pub(super) kind: RecipientType,
    pub(super) flags: Option<i32>,
    pub(super) track: Option<i32>,
}

/// An item attached to another, hidden: an appointment's changed occurrence (MS-OXOCAL 2.2.10.1),
/// with the attachment's own properties (tagged).
pub(super) struct Attached {
    pub(super) item: Item,
    pub(super) props: Vec<(u16, Value)>,
}

/// An item, ready to write.
#[derive(Default)]
pub(super) struct Item {
    pub(super) class: &'static str,
    pub(super) subject: String,
    /// Its text, with Outlook's line ends.
    pub(super) body: String,
    /// When it last changed (100 ns since 1601): the writer's delivery time, which is its
    /// PidTagLastModificationTime too.
    pub(super) modified: i64,
    pub(super) props: Vec<(Key, Value)>,
    pub(super) files: Vec<File>,
    /// Who it's from (name, address): an appointment's organizer.
    pub(super) sender: (String, String),
    pub(super) people: Vec<Person>,
    pub(super) attached: Vec<Attached>,
    /// An occurrence of a series whose rule the pattern can't say, written on its own (not part
    /// of what it is: counted only).
    pub(super) occurrence: bool,
}

impl Item {
    fn bytes(&self) -> usize {
        let props = self.props.iter().map(|(_, v)| match v {
            Value::Text(s) => s.len(),
            Value::Bin(b) => b.len(),
            Value::Texts(t) => t.iter().map(String::len).sum(),
            Value::Bins(b) => b.iter().map(Vec::len).sum(),
            _ => 8,
        });
        let attached = self.attached.iter().map(|a| a.item.bytes()).fold(0usize, usize::saturating_add);
        self.body.len().saturating_add(props.sum()).saturating_add(self.files.iter().map(|f| f.data.len()).sum()).saturating_add(attached)
    }

    /// The item marked as the item `uid` of the account `owner` names ([`owner_key`]), with a
    /// digest of all that is written of it but when (a card's is when it's exported); unmarked
    /// without a uid.
    fn marked(mut self, uid: &str, owner: &str) -> Item {
        let uid = uid.replace(['\r', '\n'], "");
        if !uid.trim().is_empty() {
            let mark = format!("{uid}\n{owner}\n{:016x}", self.digest());
            self.props.push((Key::Exported, Value::Text(mark)));
        }
        self
    }

    /// FNV-1a (64 bits) over the item's class, subject, text, properties and files, each part
    /// with its length, so the same item always gives the same digest, whatever version wrote it.
    fn digest(&self) -> u64 {
        let mut h = Fnv(0xCBF2_9CE4_8422_2325);
        self.digest_into(&mut h);
        h.0
    }

    fn digest_into(&self, h: &mut Fnv) {
        h.part(self.class.as_bytes());
        h.part(self.subject.as_bytes());
        h.part(self.body.as_bytes());
        for (key, value) in &self.props {
            match key {
                Key::Tag(id) => h.part(&[&[0][..], &id.to_le_bytes()].concat()),
                Key::Lid(set, id) => {
                    let set = [&set.data1().to_le_bytes()[..], &set.data2().to_le_bytes(), &set.data3().to_le_bytes(), set.data4()].concat();
                    h.part(&[&[1][..], &set, &id.to_le_bytes()].concat());
                }
                Key::Keywords => h.part(&[2]),
                Key::Exported => h.part(&[3]),
            }
            value_digest(h, value);
        }
        for f in &self.files {
            h.part(f.name.as_bytes());
            h.part(f.mime.as_bytes());
            h.part(&f.data);
            h.part(&[u8::from(f.picture)]);
        }
        // An appointment's organizer, guests and changed occurrences (none: nothing more, so the
        // digest of an item without them is what it was).
        if !self.sender.1.is_empty() || !self.people.is_empty() || !self.attached.is_empty() {
            h.part(self.sender.0.as_bytes());
            h.part(self.sender.1.as_bytes());
            for p in &self.people {
                let n = |v: Option<i32>| {
                    v.map_or([0; 5], |n| {
                        let b = n.to_le_bytes();
                        [1, b[0], b[1], b[2], b[3]]
                    })
                };
                h.part(&[p.name.as_bytes(), &[0], p.email.as_bytes(), &[0, p.kind as u8], &n(p.flags), &n(p.track)].concat());
            }
            for a in &self.attached {
                a.item.digest_into(h);
                for (id, v) in &a.props {
                    h.part(&id.to_le_bytes());
                    value_digest(h, v);
                }
            }
        }
    }
}

/// A value into a digest being made.
fn value_digest(h: &mut Fnv, value: &Value) {
    match value {
        Value::Int(n) => h.part(&[&[0][..], &n.to_le_bytes()].concat()),
        Value::Bool(b) => h.part(&[1, u8::from(*b)]),
        Value::Time(t) => h.part(&[&[2][..], &t.to_le_bytes()].concat()),
        Value::Float(f) => h.part(&[&[3][..], &f.to_bits().to_le_bytes()].concat()),
        Value::Text(s) => h.part(&[&[4][..], s.as_bytes()].concat()),
        Value::Bin(b) => h.part(&[&[5][..], b].concat()),
        Value::Texts(t) => {
            h.part(&[6]);
            t.iter().for_each(|s| h.part(s.as_bytes()));
        }
        Value::Bins(b) => {
            h.part(&[7]);
            b.iter().for_each(|x| h.part(x));
        }
    }
}

/// An FNV-1a hash being made.
struct Fnv(u64);

impl Fnv {
    fn part(&mut self, bytes: &[u8]) {
        for b in u64::try_from(bytes.len()).unwrap_or(u64::MAX).to_le_bytes().iter().chain(bytes) {
            self.0 = (self.0 ^ u64::from(*b)).wrapping_mul(0x0000_0100_0000_01B3);
        }
    }
}

/// Text with Outlook's line ends (CR LF).
pub(super) fn crlf(s: &str) -> String {
    s.replace("\r\n", "\n").replace('\n', "\r\n")
}

/// What went into the data file, and what a file added to had already (as it is now).
#[derive(Default)]
pub(super) struct Written {
    pub(super) events: usize,
    /// Occurrences of series whose rule Outlook's pattern can't say, each written as an
    /// appointment of its own (among `events`).
    pub(super) occurrences: usize,
    pub(super) contacts: usize,
    pub(super) notes: usize,
    pub(super) tasks: usize,
    pub(super) already: usize,
    /// Items changed since the file had them, written in place of their earlier copies (among
    /// the counts above).
    pub(super) replaced: usize,
}

/// What a mark says of the account whose item it is: a digest of its id (16 hex digits), so two
/// accounts exported into one file each know their own copies and never take out the other's.
fn owner_key(account: &str) -> String {
    let mut h = Fnv(0xCBF2_9CE4_8422_2325);
    h.part(account.as_bytes());
    format!("{:016x}", h.0)
}

/// The account's items for each of the file's folders, those it has, each marked as `account`'s;
/// `now` is when the export runs (Unix seconds), a card's time; `owner`: whose calendar it is.
fn items(pim: &Pim, now: i64, owner: &super::calendar::Owner, account: &str) -> Vec<(Special, Vec<Item>)> {
    let key = owner_key(account);
    let events: Vec<&mailcraft_model::ical::CalEvent> = pim.events.iter().map(|e| &e.event).collect();
    let calendar = super::calendar::items(&events, now, owner).into_iter().map(|(uid, item)| item.marked(&uid, &key)).collect();
    let contacts: Vec<Item> = pim.cards.iter().map(|c| super::contact::item(&c.card, now).marked(&c.card.uid, &key)).collect();
    let notes = pim.notes.iter().map(|n| note(&n.note, now).marked(&n.note.uid, &key)).collect();
    let tasks = pim.tasks.iter().map(|t| super::task::item(&t.task, now).marked(&t.task.uid, &key)).collect();
    let all = [(Special::Calendar, calendar), (Special::Contacts, contacts), (Special::Notes, notes), (Special::Tasks, tasks)];
    all.into_iter().filter(|(_, items)| !items.is_empty()).collect()
}

/// A note (MS-OXONOTE 2.2.1): its text the body, its first line the subject, its colour, and its
/// files as the item's attachments (as a task's go).
fn note(n: &Note, now: i64) -> Item {
    let color = if n.color < NOTE_COLORS { n.color } else { NOTE_YELLOW };
    Item {
        class: "IPM.StickyNote",
        subject: n.title().to_string(),
        body: crlf(&n.text),
        modified: filetime(if n.modified > 0 { n.modified } else { now }),
        props: vec![
            lid(PSETID_NOTE, 0x8B00, Value::Int(i32::from(color))),
            lid(PSETID_NOTE, 0x8B02, Value::Int(NOTE_SIZE.0)),
            lid(PSETID_NOTE, 0x8B03, Value::Int(NOTE_SIZE.1)),
        ],
        files: n.files.iter().filter_map(|l| super::task::file(l)).collect(),
        ..Default::default()
    }
}

/// Write the account's calendar, contacts, notes and tasks (`pim`) into the data file at `path` (made when
/// there is none yet: `created`); `existing`: a file added to, whose special folders take them;
/// `taken`: where the account's mail went, so a new folder isn't one of those; `owner`: whose
/// calendar it is; `account`: whose items they are, so its earlier copies are told from another
/// account's. The file has the mail by then: one that can't record a folder as its own keeps the
/// items, and the log says why.
pub(super) fn write(
    path: &Path,
    pim: &Pim,
    taken: &[Vec<String>],
    existing: bool,
    created: &mut bool,
    now: i64,
    owner: &super::calendar::Owner,
    account: &str,
) -> Result<Written, String> {
    let mut done = Written::default();
    let all = items(pim, now, owner, account);
    let key = owner_key(account);
    if all.is_empty() {
        return Ok(done);
    }
    if !*created {
        write::create_empty(path, mailcraft_pst::Format::Unicode).map_err(|e| e.to_string())?;
        *created = true;
    }
    for (role, items) in all {
        let (names, recorded) = folder_for(path, role, existing, taken);
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        // Added to again: an item the folder has as it is now isn't written twice, and the
        // copies it has of an item as it was before (its uid, another digest) go once the item
        // is in as it is now.
        let had = if existing { already(path, &names) } else { Vec::new() };
        let earlier = earlier_copies(&had, &items, &key);
        let marks: HashSet<&str> = had.iter().map(|(m, _)| m.as_str()).collect();
        let count = items.len();
        let items: Vec<Item> = items.into_iter().filter(|i| mark(i).is_none_or(|m| !has(&marks, m))).collect();
        if items.len() < count {
            log::info!("{} of the account's {role:?} items are in the data file already", count - items.len());
            done.already += count - items.len();
        }
        let count = items.len();
        let replaced = items.iter().filter_map(mark).filter(|m| earlier.uids.contains(uid_of(m))).count();
        write::create_folder(path, &names, role.container_class()).map_err(|e| e.to_string())?;
        let mut start = 0;
        while start < items.len() {
            let (mut end, mut bytes) = (start, 0usize);
            while end < items.len() && end - start < BATCH && (end == start || bytes < BATCH_BYTES) {
                bytes = bytes.saturating_add(items.get(end).map_or(0, Item::bytes));
                end += 1;
            }
            write_batch(path, &names, items.get(start..end).unwrap_or_default())?;
            start = end;
        }
        // The earlier copies, now that the items are in: one that can't be taken out stays
        // beside its new copy, as before the writer could, and the log says why.
        if !earlier.ids.is_empty() {
            match write::delete_messages(path, &earlier.ids) {
                Ok(()) => done.replaced += replaced,
                Err(e) => log::info!("{} earlier copies of the account's {role:?} items stay in the data file: {e}", earlier.ids.len()),
            }
        }
        match role {
            Special::Calendar => {
                done.events = count;
                done.occurrences = items.iter().filter(|i| i.occurrence).count();
            }
            Special::Contacts => done.contacts = count,
            Special::Notes => done.notes = count,
            _ => done.tasks = count,
        }
        if !recorded && let Err(e) = write::set_special_folder(path, role, &names) {
            log::info!("the data file's {role:?} can't be recorded: {e}");
        }
    }
    Ok(done)
}

/// The folder `role`'s items go in, and whether the file records it as such already: the one a
/// file added to records, else the role's name, or that name with a number when the account's mail
/// took it, or a file added to has a folder of another kind by that name (its mail folder `Notes`,
/// say).
fn folder_for(path: &Path, role: Special, existing: bool, taken: &[Vec<String>]) -> (Vec<String>, bool) {
    if existing && let Ok(Some(names)) = write::special_folder(path, role) {
        return (names, true);
    }
    let other = if existing { top_folders(path) } else { Vec::new() };
    let other: Vec<&str> = other.iter().filter(|(_, class)| !class.eq_ignore_ascii_case(role.container_class())).map(|(n, _)| n.as_str()).collect();
    let free = |name: &str| {
        !taken.iter().any(|t| t.first().is_some_and(|f| f.eq_ignore_ascii_case(name))) && !other.iter().any(|o| o.eq_ignore_ascii_case(name))
    };
    let name = role.default_name();
    let name = std::iter::once(name.to_string())
        .chain((2..=MAX_SUFFIX).map(|n| format!("{name} ({n})")))
        .find(|n| free(n))
        .unwrap_or_else(|| name.to_string());
    (vec![name], false)
}

/// The mark an item carries (`pst_pim::EXPORTED_ITEM`), when it has a uid.
fn mark(item: &Item) -> Option<&str> {
    item.props.iter().find_map(|(k, v)| match (k, v) {
        (Key::Exported, Value::Text(m)) => Some(m.as_str()),
        _ => None,
    })
}

/// The folders at the top of the file at `path`: each one's name and container class (a folder
/// that says none holds mail, `IPF.Note`); none when it can't be read.
fn top_folders(path: &Path) -> Vec<(String, String)> {
    let read = || -> io::Result<Vec<(String, String)>> {
        let any = store_of(path)?;
        let top = any.open_folder(any.ipm_subtree()?)?;
        let Some(table) = top.hierarchy_table() else { return Ok(Vec::new()) };
        let mut out = Vec::new();
        for row in table.rows_matrix() {
            let Ok(f) = any.open_folder(row.id()) else { continue };
            let Ok(name) = f.properties().display_name() else { continue };
            let class =
                f.properties().get(0x3613).and_then(crate::pst::text_of).filter(|c| !c.trim().is_empty()).unwrap_or_else(|| "IPF.Note".into());
            out.push((name, class));
        }
        Ok(out)
    };
    read().unwrap_or_default()
}

/// What a mark says: the item's uid, the account whose it is ([`owner_key`]; none in a mark an
/// earlier MailCraft wrote, two lines, whose account can't be known) and the digest, each on a
/// line of its own (a uid has no line ends).
fn parts(mark: &str) -> (&str, Option<&str>, &str) {
    let Some((rest, digest)) = mark.rsplit_once('\n') else { return (mark, None, "") };
    match rest.rsplit_once('\n') {
        Some((uid, owner)) => (uid, Some(owner), digest),
        None => (rest, None, digest),
    }
}

/// The uid a mark names.
fn uid_of(mark: &str) -> &str {
    parts(mark).0
}

/// Whether a folder whose items carry `marks` has the item marked `mark` as it is now: marked so,
/// or by an earlier MailCraft with its uid and digest and no account.
fn has(marks: &HashSet<&str>, mark: &str) -> bool {
    let (uid, _, digest) = parts(mark);
    marks.contains(mark) || marks.contains(format!("{uid}\n{digest}").as_str())
}

/// The copies a folder has of the account's items as they were before.
struct Earlier {
    ids: Vec<NodeId>,
    /// The uids of those items.
    uids: HashSet<String>,
}

/// The items of `had` (a folder's, by mark) that are the account's copies (marked with `owner`,
/// [`owner_key`]) of one of `items` as it was before: marked with its uid and a digest none of the
/// account's items of that uid has now (a series' occurrences written one by one share its uid).
/// Another account's copy of an item with the same uid (a meeting in the organizer's calendar and
/// a guest's) stays, as does one whose mark names no account.
fn earlier_copies(had: &[(String, NodeId)], items: &[Item], owner: &str) -> Earlier {
    let mut now: HashMap<&str, HashSet<&str>> = HashMap::new();
    for m in items.iter().filter_map(mark) {
        now.entry(uid_of(m)).or_default().insert(m);
    }
    let mut earlier = Earlier { ids: Vec::new(), uids: HashSet::new() };
    for (m, id) in had {
        let (uid, whose, _) = parts(m);
        if whose == Some(owner) && now.get(uid).is_some_and(|marks| !marks.contains(m.as_str())) {
            earlier.ids.push(*id);
            earlier.uids.insert(uid.to_string());
        }
    }
    earlier
}

/// The marks of the items in the folder at `names` of the file at `path` (those MailCraft
/// exported: the file's own have none, and are never taken for the account's), each with its
/// node; none when there is no such folder, or it can't be read.
fn already(path: &Path, names: &[&str]) -> Vec<(String, NodeId)> {
    let read = || -> io::Result<Vec<(String, NodeId)>> {
        let any = store_of(path)?;
        let Some(id) = crate::pst_pim::Named::of(any.as_ref()).exported() else { return Ok(Vec::new()) };
        let open = |row: u32| any.open_folder(row);
        let mut folder = any.open_folder(any.ipm_subtree()?)?;
        for name in names {
            let Some(table) = folder.hierarchy_table() else { return Ok(Vec::new()) };
            let next = table.rows_matrix().filter_map(|row| open(row.id()).ok()).find(|f| f.properties().display_name().is_ok_and(|n| n == *name));
            let Some(next) = next else { return Ok(Vec::new()) };
            folder = next;
        }
        let mut out = Vec::new();
        for row in folder.contents_table().map(|t| t.rows_matrix().collect::<Vec<_>>()).unwrap_or_default() {
            let node = row.id();
            let Ok(props) = any.message_properties(node, &[id]) else { continue };
            if let Some(mark) = props.get(id).and_then(crate::pst::text_of) {
                out.push((mark, node));
            }
        }
        Ok(out)
    };
    read().unwrap_or_default()
}

/// The categories, as an item's `Keywords`.
pub(super) fn keywords(categories: &[String]) -> Option<(Key, Value)> {
    let names: Vec<String> = categories.iter().map(|c| c.trim().to_string()).filter(|c| !c.is_empty()).collect();
    (!names.is_empty()).then_some((Key::Keywords, Value::Texts(names)))
}

/// A reminder at `at` (Unix seconds), as a task's (PidLidReminderSet, PidLidReminderDelta,
/// PidLidReminderTime and PidLidReminderSignalTime).
pub(super) fn reminder(at: i64) -> [(Key, Value); 4] {
    [
        lid(PSETID_COMMON, 0x8503, Value::Bool(true)),
        lid(PSETID_COMMON, 0x8501, Value::Int(0)),
        lid(PSETID_COMMON, 0x8502, Value::Time(filetime(at))),
        lid(PSETID_COMMON, 0x8560, Value::Time(filetime(at))),
    ]
}
