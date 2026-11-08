//! Mail added to an Outlook data file (`.pst`) that's already there, as an export adds to one: the
//! file damaged any way, then one message with a file appended by the writer, and what it then
//! holds read back. Each message carries what an export gives one: tagged properties, and named
//! ones the writer looks up in the file's (damaged) name-to-ID map and adds to it; the one added
//! also a message attached to it, with a file of its own, and a file the writer reads from a source
//! as it writes it.
//!
//! Random bytes rarely get past a `.pst`'s header, so most inputs damage a good file instead:
//! one the writer made (an Inbox with a message and its attachment, and a folder beside it),
//! with the input read as five-byte patches (a little-endian offset, then the byte to put
//! there). An input that starts like a data file (`!BDN`) is used whole: the seeds have one
//! encoded as Outlook encodes its files (permute), so the writer's encoded paths are fuzzed too,
//! and one in Outlook 97–2002's ANSI format (an account exported into one), added to through
//! `PstFile` and read with the reader's ANSI types.
//!
//! Before the append, the special folders the file records are looked up, as an export does to
//! know where Sent, Trash, Drafts and Junk go: their entry IDs, the folders they name and the
//! hierarchy down to them, all from the damaged file.
//!
//! Then a message that read before is taken out again, as an export takes out the earlier copy
//! of an item changed since: the others still read as they did, and it no longer does.
//!
//! Every panic counts: the writer reads the file it adds to as its reader does, and a damaged file
//! is an error. So does an append that succeeds but loses mail: every message that read before it
//! (all of its properties, as they were) still has to, and the one added has to as well; and so
//! does a deletion that succeeds but takes another message with it or leaves the one it took.
#![no_main]

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::OnceLock;

use mailcraft_pst::{GuidValue, Message, PS_PUBLIC_STRINGS, PstFile, PstStore, write::{self, ADD_MAX_VALUE_SIZE, AttachmentContent, AttachmentSource, BatchMessage, EmbeddedMessage, Name, NewAttachment, NewMessage, NewProperty, NewValue as Value, PropertyKey as Key, SpecialFolder as Special}};
use libfuzzer_sys::fuzz_target;

/// PSETID_Common.
const COMMON: GuidValue = GuidValue::new(0x0006_2008, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);

/// An unread, flagged message in a category, as the export writes one.
const PROPERTIES: [NewProperty<'static>; 4] = [
    NewProperty { key: Key::Tag(0x0E07), value: Value::Integer32(0) },
    NewProperty { key: Key::Tag(0x1090), value: Value::Integer32(2) },
    NewProperty { key: Key::Named(COMMON, Name::Id(0x8530)), value: Value::Unicode("Follow up") },
    NewProperty { key: Key::Named(PS_PUBLIC_STRINGS, Name::String("Keywords")), value: Value::MultipleUnicode(&["Red"]) },
];

fn scratch(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("mailcraft-fuzz-{}-{name}", std::process::id()))
}

fn message(subject: &str) -> NewMessage<'_> {
    NewMessage {
        subject,
        sender_name: "Riley Moreno",
        sender_email: "riley@example.com",
        recipients: &[],
        body: "Hello there.",
        html_body: Some("<p>Hello <b>there</b>.</p>"),
        message_id: "<m1@example.com>",
        delivery_time: 133_485_408_000_000_000,
        message_class: "IPM.Note",
        properties: &PROPERTIES,
    }
}

fn notes() -> NewAttachment<'static> {
    NewAttachment { filename: "notes.txt", mime_type: "text/plain", content_id: None, data: b"remember", ..Default::default() }
}

/// A file of 20,000 bytes, read as it is written.
struct Source;

impl AttachmentSource for Source {
    fn size(&self) -> u64 {
        20_000
    }

    fn open(&self) -> std::io::Result<Box<dyn std::io::Read + '_>> {
        Ok(Box::new(std::io::Read::take(std::io::repeat(b'x'), 20_000)))
    }
}

/// A data file the writer made: Inbox with one message and its file, and Projects beside it.
fn good_file() -> Vec<u8> {
    let path = scratch("good.pst");
    let _ = std::fs::remove_file(&path);
    let _ = write::create_with(&path, &["Inbox"], &message("Plan"), &[notes()]);
    let _ = write::append_message(&path, &["Projects"], &message("Nested"), &[]);
    let data = std::fs::read(&path).unwrap_or_default();
    let _ = std::fs::remove_file(&path);
    assert!(data.starts_with(b"!BDN"), "the writer wrote no data file");
    data
}

static GOOD: OnceLock<Vec<u8>> = OnceLock::new();

/// The file at `path`'s store, read within the limit the writer reads a file it adds to with.
fn store(path: &Path) -> Option<Rc<PstStore>> {
    let pst = PstFile::open(path).ok()?;
    pst.set_max_value_size(ADD_MAX_VALUE_SIZE);
    PstStore::read(Rc::new(pst)).ok()
}

/// Every message the file's folders list that reads, by node, with its properties as text; read
/// within the limit the writer reads a file it adds to with.
fn messages(path: &Path) -> BTreeMap<u32, String> {
    let mut out = BTreeMap::new();
    let Some(store) = store(path) else { return out };
    let Ok(top) = store.ipm_subtree() else { return out };
    let mut stack = vec![(top, 0)];
    let mut seen = HashSet::new();
    while let Some((id, depth)) = stack.pop() {
        if depth > 32 || seen.len() > 1_000 || !seen.insert(id) {
            continue;
        }
        let Ok(folder) = store.open_folder(id) else { continue };
        for row in folder.contents_table().iter().flat_map(|table| table.rows_matrix()) {
            let node = row.id();
            let Ok(message) = Message::read(store.clone(), node) else { continue };
            let mut properties: Vec<_> = message.properties().iter().collect();
            properties.sort_by_key(|(id, _)| **id);
            out.insert(node, format!("{properties:?}"));
        }
        for row in folder.hierarchy_table().iter().flat_map(|table| table.rows_matrix()) {
            stack.push((row.id(), depth + 1));
        }
    }
    out
}

fuzz_target!(|data: &[u8]| {
    // Half the inputs with the checksums taken for right: such a file holds anything, so all it
    // has to do is give an error or a result, with no panic. The rest hold the checks below.
    let forged = data.last().is_some_and(|b| b & 1 == 1);
    mailcraft_pst::take_checksums_as_right(forged);
    let file = if data.starts_with(b"!BDN") {
        data.to_vec()
    } else {
        let mut file = GOOD.get_or_init(good_file).clone();
        for p in data.chunks_exact(5) {
            let at = u32::from_le_bytes([p[0], p[1], p[2], p[3]]) as usize % file.len();
            file[at] = p[4];
        }
        file
    };
    let path = scratch("existing.pst");
    if std::fs::write(&path, &file).is_err() {
        return;
    }
    let attached = NewAttachment {
        filename: "Forwarded",
        content: AttachmentContent::Message(Box::new(EmbeddedMessage { message: message("Forwarded"), attachments: vec![notes()] })),
        ..Default::default()
    };
    let read = NewAttachment { filename: "big.bin", content: AttachmentContent::Source(&Source), ..Default::default() };
    let attachments = [notes(), attached, read];
    let batch = [BatchMessage { message: message("Added"), attachments: &attachments }];
    for role in [Special::DeletedItems, Special::SentItems, Special::Outbox, Special::Drafts, Special::JunkEmail] {
        let _ = write::special_folder(&path, role);
    }
    let before = messages(&path);
    let added = write::append(&path, &["Inbox"], &batch).is_ok();
    let after = if added { messages(&path) } else { BTreeMap::new() };
    if added && !forged {
        // An append that succeeds keeps what the file held and adds what it says it wrote.
        let lost = before.iter().filter(|(node, was)| after.get(*node) != Some(*was)).count();
        assert_eq!(lost, 0, "{lost} of {} messages no longer read as they did", before.len());
        assert!(after.keys().any(|node| !before.contains_key(node)), "the added message doesn't read");
    }
    // A message that read before taken out again: the rest stay as they were.
    if let Some(&gone) = before.keys().next().filter(|_| added) {
        let ids = [gone];
        let deleted = write::delete_messages(&path, &ids).is_ok();
        if deleted && !forged {
            let left = messages(&path);
            assert!(!left.contains_key(&gone), "the message taken out still reads");
            let lost = after.iter().filter(|(node, was)| **node != gone && left.get(*node) != Some(*was)).count();
            assert_eq!(lost, 0, "{lost} of {} messages no longer read as they did once one was taken out", after.len());
        }
    }
    let _ = std::fs::remove_file(&path);
});
