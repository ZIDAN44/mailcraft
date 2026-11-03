//! Mail added to an Outlook data file (`.pst`) that's already there, as an export adds to one:
//! the file damaged any way, then one message with a file appended by the writer (MailCraft's
//! fork of outlook-pst-rw), and what it then holds read back. Each message carries what an export
//! gives one: tagged properties, and named ones the writer looks up in the file's (damaged)
//! name-to-ID map and adds to it; the one added also a message attached to it, with a file of its
//! own, and a file the writer reads from a source as it writes it.
//!
//! Random bytes rarely get past a `.pst`'s header, so most inputs damage a good file instead:
//! one the writer made (an Inbox with a message and its attachment, and a folder beside it),
//! with the input read as five-byte patches (a little-endian offset, then the byte to put
//! there). An input that starts like a data file (`!BDN`) is used whole: the seeds have one
//! encoded as Outlook encodes its files (permute), so the writer's encoded paths are fuzzed too.
//!
//! Before the append, the special folders the file records are looked up, as an export does to
//! know where Sent, Trash, Drafts and Junk go: their entry IDs, the folders they name and the
//! hierarchy down to them, all from the damaged file.
//!
//! Every panic counts: the writer reads the file it adds to as its reader does, and a damaged
//! file is an error (its fork's PATCHES.md), so there is no fence here, unlike `pst`'s. So does
//! an append that succeeds but loses mail: every message that read before it (all of its
//! properties, as they were) still has to, and the one added has to as well.
#![no_main]

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::OnceLock;

use libfuzzer_sys::fuzz_target;
use outlook_pst::ltp::prop_context::GuidValue;
use outlook_pst::messaging::named_prop::PS_PUBLIC_STRINGS;
use outlook_pst::messaging::store::{Store as _, UnicodeStore};
use outlook_pst::ndb::node_id::NodeId;
use outlook_pst::{
    ADD_MAX_VALUE_SIZE, UnicodePstAttachment, UnicodePstAttachmentContent, UnicodePstAttachmentSource, UnicodePstBatchMessage,
    UnicodePstEmbeddedMessage, UnicodePstFile, UnicodePstMessage, UnicodePstName, UnicodePstProperty, UnicodePstPropertyKey as Key,
    UnicodePstSpecialFolder as Special, UnicodePstValue as Value,
};

/// PSETID_Common.
const COMMON: GuidValue = GuidValue::new(0x0006_2008, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);

/// An unread, flagged message in a category, as the export writes one.
const PROPERTIES: [UnicodePstProperty<'static>; 4] = [
    UnicodePstProperty { key: Key::Tag(0x0E07), value: Value::Integer32(0) },
    UnicodePstProperty { key: Key::Tag(0x1090), value: Value::Integer32(2) },
    UnicodePstProperty { key: Key::Named(COMMON, UnicodePstName::Id(0x8530)), value: Value::Unicode("Follow up") },
    UnicodePstProperty { key: Key::Named(PS_PUBLIC_STRINGS, UnicodePstName::String("Keywords")), value: Value::MultipleUnicode(&["Red"]) },
];

fn scratch(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("mailcraft-fuzz-{}-{name}", std::process::id()))
}

fn message(subject: &str) -> UnicodePstMessage<'_> {
    UnicodePstMessage {
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

fn notes() -> UnicodePstAttachment<'static> {
    UnicodePstAttachment { filename: "notes.txt", mime_type: "text/plain", content_id: None, data: b"remember", ..Default::default() }
}

/// A file of 20,000 bytes, read as it is written.
struct Source;

impl UnicodePstAttachmentSource for Source {
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
    let _ = UnicodePstFile::create_in_folder_with_attachments(&path, &["Inbox"], &message("Plan"), &[notes()]);
    let _ = UnicodePstFile::append_in_folder(&path, &["Projects"], &message("Nested"));
    let data = std::fs::read(&path).unwrap_or_default();
    let _ = std::fs::remove_file(&path);
    assert!(data.starts_with(b"!BDN"), "the writer wrote no data file");
    data
}

static GOOD: OnceLock<Vec<u8>> = OnceLock::new();

/// Every message the file's folders list that reads, by node, with its properties as text; read
/// within the limit the writer reads a file it adds to with.
fn messages(path: &Path) -> BTreeMap<u32, String> {
    let mut out = BTreeMap::new();
    let Ok(pst) = UnicodePstFile::open(path) else { return out };
    pst.set_max_value_size(ADD_MAX_VALUE_SIZE);
    let Ok(store) = UnicodeStore::read(Rc::new(pst)) else { return out };
    let Ok(top) = store.properties().ipm_sub_tree_entry_id() else { return out };
    let mut stack = vec![(top, 0)];
    let mut seen = HashSet::new();
    while let Some((id, depth)) = stack.pop() {
        if depth > 32 || seen.len() > 1_000 || !seen.insert(u32::from(id.node_id())) {
            continue;
        }
        let Ok(folder) = store.open_folder(&id) else { continue };
        for row in folder.contents_table().iter().flat_map(|table| table.rows_matrix()) {
            let node = u32::from(row.id());
            let Ok(entry) = store.properties().make_entry_id(NodeId::from(node)) else { continue };
            let Ok(message) = store.open_message(&entry, None) else { continue };
            let mut properties: Vec<_> = message.properties().iter().collect();
            properties.sort_by_key(|(id, _)| **id);
            out.insert(node, format!("{properties:?}"));
        }
        for row in folder.hierarchy_table().iter().flat_map(|table| table.rows_matrix()) {
            if let Ok(sub) = store.properties().make_entry_id(NodeId::from(u32::from(row.id()))) {
                stack.push((sub, depth + 1));
            }
        }
    }
    out
}

fuzz_target!(|data: &[u8]| {
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
    let attached = UnicodePstAttachment {
        filename: "Forwarded",
        content: UnicodePstAttachmentContent::Message(Box::new(UnicodePstEmbeddedMessage { message: message("Forwarded"), attachments: vec![notes()] })),
        ..Default::default()
    };
    let read = UnicodePstAttachment { filename: "big.bin", content: UnicodePstAttachmentContent::Source(&Source), ..Default::default() };
    let attachments = [notes(), attached, read];
    let batch = [UnicodePstBatchMessage { message: message("Added"), attachments: &attachments }];
    for role in [Special::DeletedItems, Special::SentItems, Special::Outbox, Special::Drafts, Special::JunkEmail] {
        let _ = UnicodePstFile::special_folder(&path, role);
    }
    let before = messages(&path);
    let added = UnicodePstFile::append_many_in_folder_with_attachments(&path, &["Inbox"], &batch).is_ok();
    let after = if added { messages(&path) } else { BTreeMap::new() };
    let _ = std::fs::remove_file(&path);
    if added {
        // An append that succeeds keeps what the file held and adds what it says it wrote.
        let lost = before.iter().filter(|(node, was)| after.get(*node) != Some(*was)).count();
        assert_eq!(lost, 0, "{lost} of {} messages no longer read as they did", before.len());
        assert!(after.keys().any(|node| !before.contains_key(node)), "the added message doesn't read");
    }
});
