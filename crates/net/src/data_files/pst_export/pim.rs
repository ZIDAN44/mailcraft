//! The account's contacts, contact groups, notes and tasks as Outlook items, in the data file's
//! own Contacts, Notes and Tasks folders (MS-OXOCNTC, MS-OXONOTE, MS-OXOTASK; the folders recorded
//! as the file's, MS-OXOSFLD 2.2, so Outlook shows them as its own): a card as an `IPM.Contact`
//! or, a group's, an `IPM.DistList` ([`contact`]), a note as an `IPM.StickyNote`, a task as an
//! `IPM.Task` ([`task`]). A file added to keeps its own items, and those it records as its
//! special folders take the account's. Each item is marked with its uid and a digest of what went
//! in (`pst_pim::EXPORTED_ITEM`): added to again, an item the file has as it is now isn't written
//! twice, and one changed since goes in again (beside its earlier copy, as the writer can't take an
//! item out of a folder). What Outlook has no place for stays in the item's text, as the import
//! reads it back (`pst_pim::rest`).

use mailcraft_model::pim::{NOTE_COLORS, NOTE_YELLOW, Note, Pim};
use pst_rw::messaging::named_prop::PS_PUBLIC_STRINGS;
use pst_rw::{UnicodePstName, UnicodePstSpecialFolder as Special, UnicodePstValue};

use super::item::{PSETID_COMMON, filetime};
use super::*;
use pst_rw::ltp::prop_context::GuidValue;

/// PSETID_Address and PSETID_Note (MS-OXPROPS 1.3.2).
pub(super) const PSETID_ADDRESS: GuidValue = GuidValue::new(0x0006_2004, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
const PSETID_NOTE: GuidValue = GuidValue::new(0x0006_200E, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
/// A note's size in Outlook's window, as Outlook makes one (PidLidNoteWidth, PidLidNoteHeight).
const NOTE_SIZE: (i32, i32) = (200, 166);
/// Names tried for a new file's folder that the account's mail took already: "Notes (2)" and on.
const MAX_SUFFIX: usize = 100;

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

/// An item, ready to write.
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
        self.body.len().saturating_add(props.sum()).saturating_add(self.files.iter().map(|f| f.data.len()).sum())
    }

    /// The item marked as the account's item `uid`, with a digest of all that is written of it
    /// but when (a card's is when it's exported); unmarked without a uid.
    fn marked(mut self, uid: &str) -> Item {
        let uid = uid.replace(['\r', '\n'], "");
        if !uid.trim().is_empty() {
            let mark = format!("{uid}\n{:016x}", self.digest());
            self.props.push((Key::Exported, Value::Text(mark)));
        }
        self
    }

    /// FNV-1a (64 bits) over the item's class, subject, text, properties and files, each part
    /// with its length, so the same item always gives the same digest, whatever version wrote it.
    fn digest(&self) -> u64 {
        let mut h = Fnv(0xCBF2_9CE4_8422_2325);
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
        for f in &self.files {
            h.part(f.name.as_bytes());
            h.part(f.mime.as_bytes());
            h.part(&f.data);
            h.part(&[u8::from(f.picture)]);
        }
        h.0
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
    pub(super) contacts: usize,
    pub(super) notes: usize,
    pub(super) tasks: usize,
    pub(super) already: usize,
}

/// The account's items for each of the file's folders, those it has, each marked; `now` is when
/// the export runs (Unix seconds), a card's time.
fn items(pim: &Pim, now: i64) -> Vec<(Special, Vec<Item>)> {
    let contacts: Vec<Item> = pim.cards.iter().map(|c| super::contact::item(&c.card, now).marked(&c.card.uid)).collect();
    let notes = pim.notes.iter().map(|n| note(&n.note, now).marked(&n.note.uid)).collect();
    let tasks = pim.tasks.iter().map(|t| super::task::item(&t.task, now).marked(&t.task.uid)).collect();
    [(Special::Contacts, contacts), (Special::Notes, notes), (Special::Tasks, tasks)].into_iter().filter(|(_, items)| !items.is_empty()).collect()
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
    }
}

/// Write the account's contacts, notes and tasks (`pim`) into the data file at `path` (made when
/// there is none yet: `created`); `existing`: a file added to, whose special folders take them;
/// `taken`: where the account's mail went, so a new folder isn't one of those. The file has the
/// mail by then: one that can't record a folder as its own keeps the items, and the log says why.
pub(super) fn write(path: &Path, pim: &Pim, taken: &[Vec<String>], existing: bool, created: &mut bool, now: i64) -> Result<Written, String> {
    let mut done = Written::default();
    let all = items(pim, now);
    if all.is_empty() {
        return Ok(done);
    }
    if !*created {
        UnicodePstFile::create_empty(path).map_err(|e| e.to_string())?;
        *created = true;
    }
    for (role, items) in all {
        let (names, recorded) = folder_for(path, role, existing, taken);
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        // Added to again: an item the folder has as it is now isn't written twice.
        let had = if existing { already(path, &names) } else { HashSet::new() };
        let count = items.len();
        let items: Vec<Item> = items.into_iter().filter(|i| mark(i).is_none_or(|m| !had.contains(m))).collect();
        if items.len() < count {
            log::info!("{} of the account's {role:?} items are in the data file already", count - items.len());
            done.already += count - items.len();
        }
        let count = items.len();
        UnicodePstFile::create_folder(path, &names, role.container_class()).map_err(|e| e.to_string())?;
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
        match role {
            Special::Contacts => done.contacts = count,
            Special::Notes => done.notes = count,
            _ => done.tasks = count,
        }
        if !recorded && let Err(e) = UnicodePstFile::set_special_folder(path, role, &names) {
            log::info!("the data file's {role:?} can't be recorded: {e}");
        }
    }
    Ok(done)
}

/// The folder `role`'s items go in, and whether the file records it as such already: the one a
/// file added to records (a panic in the writer's reader costs only that), else the role's name,
/// or that name with a number when the account's mail took it, or a file added to has a folder of
/// another kind by that name (its mail folder `Notes`, say).
fn folder_for(path: &Path, role: Special, existing: bool, taken: &[Vec<String>]) -> (Vec<String>, bool) {
    if existing && let Ok(Ok(Some(names))) = std::panic::catch_unwind(|| UnicodePstFile::special_folder(path, role)) {
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

/// The file at `path` opened by the reader, its store and the store as the reader's trait.
fn open_store(path: &Path) -> io::Result<(Rc<UnicodeStore>, Rc<dyn PstStore>)> {
    let pst = outlook_pst::UnicodePstFile::open(path)?;
    pst.set_max_value_size(super::super::pst::MAX_VALUE);
    let store = UnicodeStore::read(Rc::new(pst))?;
    let any: Rc<dyn PstStore> = store.clone();
    Ok((store, any))
}

/// The folders at the top of the file at `path`: each one's name and container class (a folder
/// that says none holds mail, `IPF.Note`); none when it can't be read (a panic in the reader too).
fn top_folders(path: &Path) -> Vec<(String, String)> {
    let read = std::panic::catch_unwind(|| -> io::Result<Vec<(String, String)>> {
        let (_, any) = open_store(path)?;
        let top = any.open_folder(&any.properties().ipm_sub_tree_entry_id()?)?;
        let Some(table) = top.hierarchy_table() else { return Ok(Vec::new()) };
        let mut out = Vec::new();
        for row in table.rows_matrix() {
            let Ok(f) = any.properties().make_entry_id(NodeId::from(u32::from(row.id()))).and_then(|e| any.open_folder(&e)) else { continue };
            let Ok(name) = f.properties().display_name() else { continue };
            let class =
                f.properties().get(0x3613).and_then(crate::pst::text_of).filter(|c| !c.trim().is_empty()).unwrap_or_else(|| "IPF.Note".into());
            out.push((name, class));
        }
        Ok(out)
    });
    match read {
        Ok(Ok(found)) => found,
        _ => Vec::new(),
    }
}

/// The marks of the items in the folder at `names` of the file at `path` (those MailCraft
/// exported: the file's own have none, and are never taken for the account's); none when there
/// is no such folder, or it can't be read (a panic in the reader too).
fn already(path: &Path, names: &[&str]) -> HashSet<String> {
    let read = std::panic::catch_unwind(|| -> io::Result<HashSet<String>> {
        let (store, any) = open_store(path)?;
        let Some(id) = crate::pst_pim::Named::of(any.as_ref()).exported() else { return Ok(HashSet::new()) };
        let open = |row: u32| any.properties().make_entry_id(NodeId::from(row)).and_then(|e| any.open_folder(&e));
        let mut folder = any.open_folder(&any.properties().ipm_sub_tree_entry_id()?)?;
        for name in names {
            let Some(table) = folder.hierarchy_table() else { return Ok(HashSet::new()) };
            let next = table
                .rows_matrix()
                .filter_map(|row| open(u32::from(row.id())).ok())
                .find(|f| f.properties().display_name().is_ok_and(|n| n == *name));
            let Some(next) = next else { return Ok(HashSet::new()) };
            folder = next;
        }
        let mut out = HashSet::new();
        for row in folder.contents_table().map(|t| t.rows_matrix().collect::<Vec<_>>()).unwrap_or_default() {
            let Ok(entry) = any.properties().make_entry_id(NodeId::from(u32::from(row.id()))) else { continue };
            let Ok(m) = UnicodeMessage::read(store.clone(), &entry, Some(&[id])) else { continue };
            if let Some(mark) = m.properties().get(id).and_then(crate::pst::text_of) {
                out.insert(mark);
            }
        }
        Ok(out)
    });
    match read {
        Ok(Ok(found)) => found,
        _ => HashSet::new(),
    }
}

/// The multi-valued properties' lists of an item, in the order of its properties.
struct Lists<'a> {
    texts: Vec<Vec<&'a str>>,
    bins: Vec<Vec<&'a [u8]>>,
}

fn lists(item: &Item) -> Lists<'_> {
    let mut l = Lists { texts: Vec::new(), bins: Vec::new() };
    for (_, v) in &item.props {
        match v {
            Value::Texts(t) => l.texts.push(t.iter().map(String::as_str).collect()),
            Value::Bins(b) => l.bins.push(b.iter().map(Vec::as_slice).collect()),
            _ => {}
        }
    }
    l
}

/// An item's properties as the writer takes them.
fn properties<'a>(item: &'a Item, lists: &'a Lists<'a>) -> Vec<UnicodePstProperty<'a>> {
    let (mut texts, mut bins) = (lists.texts.iter(), lists.bins.iter());
    let mut out = Vec::new();
    for (key, value) in &item.props {
        let value = match value {
            Value::Int(n) => UnicodePstValue::Integer32(*n),
            Value::Bool(b) => UnicodePstValue::Boolean(*b),
            Value::Time(t) => UnicodePstValue::Time(*t),
            Value::Float(f) => UnicodePstValue::Floating64(*f),
            Value::Text(s) => UnicodePstValue::Unicode(s),
            Value::Bin(b) => UnicodePstValue::Binary(b),
            Value::Texts(_) => match texts.next() {
                Some(t) => UnicodePstValue::MultipleUnicode(t),
                None => continue,
            },
            Value::Bins(_) => match bins.next() {
                Some(b) => UnicodePstValue::MultipleBinary(b),
                None => continue,
            },
        };
        out.push(match key {
            Key::Tag(id) => UnicodePstProperty::tagged(*id, value),
            Key::Lid(set, id) => UnicodePstProperty::named(*set, UnicodePstName::Id(*id), value),
            Key::Keywords => UnicodePstProperty::named(PS_PUBLIC_STRINGS, UnicodePstName::String("Keywords"), value),
            Key::Exported => UnicodePstProperty::named(PS_PUBLIC_STRINGS, UnicodePstName::String(crate::pst_pim::EXPORTED_ITEM), value),
        });
    }
    out
}

fn write_batch(path: &Path, folder: &[&str], batch: &[Item]) -> Result<(), String> {
    // PidTagAttachmentContactPhoto, and hidden, as Outlook marks a contact's picture.
    let picture =
        [UnicodePstProperty::tagged(0x7FFF, UnicodePstValue::Boolean(true)), UnicodePstProperty::tagged(0x7FFE, UnicodePstValue::Boolean(true))];
    let lists: Vec<Lists<'_>> = batch.iter().map(lists).collect();
    let properties: Vec<Vec<UnicodePstProperty<'_>>> = batch.iter().zip(&lists).map(|(item, l)| properties(item, l)).collect();
    let files: Vec<Vec<UnicodePstAttachment<'_>>> = batch
        .iter()
        .map(|item| {
            item.files
                .iter()
                .map(|f| UnicodePstAttachment {
                    filename: &f.name,
                    mime_type: &f.mime,
                    data: &f.data,
                    properties: if f.picture { &picture } else { &[] },
                    ..Default::default()
                })
                .collect()
        })
        .collect();
    let messages: Vec<UnicodePstBatchMessage<'_>> = batch
        .iter()
        .zip(&properties)
        .zip(&files)
        .map(|((item, p), f)| UnicodePstBatchMessage {
            message: UnicodePstMessage {
                subject: &item.subject,
                sender_name: "",
                sender_email: "",
                recipients: &[],
                body: &item.body,
                html_body: None,
                message_id: "",
                delivery_time: item.modified,
                message_class: item.class,
                properties: p,
            },
            attachments: f,
        })
        .collect();
    append(path, folder, &messages)
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
