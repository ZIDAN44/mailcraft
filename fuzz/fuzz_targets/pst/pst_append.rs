//! Mail added to an Outlook data file (`.pst`) that's already there, as an export adds to one:
//! the file damaged any way, then one message with a file appended by the writer (MailCraft's
//! fork of outlook-pst-rw), and what it then holds read back.
//!
//! Random bytes rarely get past a `.pst`'s header, so most inputs damage a good file instead:
//! one the writer made (an Inbox with a message and its attachment, and a folder beside it),
//! with the input read as five-byte patches (a little-endian offset, then the byte to put
//! there). An input that starts like a data file (`!BDN`) is used whole: the seeds have one
//! encoded as Outlook encodes its files (permute), so the writer's encoded paths are fuzzed too.
//!
//! Every panic counts: the writer reads the file it adds to as its reader does, and a damaged
//! file is an error (its fork's PATCHES.md), so there is no fence here, unlike `pst`'s.
#![no_main]

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::OnceLock;

use libfuzzer_sys::fuzz_target;
use pst_rw::messaging::store::{Store as _, UnicodeStore};
use pst_rw::{UnicodePstAttachment, UnicodePstBatchMessage, UnicodePstFile, UnicodePstMessage};

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
    }
}

const FILE: UnicodePstAttachment<'static> =
    UnicodePstAttachment { filename: "notes.txt", mime_type: "text/plain", content_id: None, data: b"remember" };

/// A data file the writer made: Inbox with one message and its file, and Projects beside it.
fn good_file() -> Vec<u8> {
    let path = scratch("good.pst");
    let _ = std::fs::remove_file(&path);
    let _ = UnicodePstFile::create_in_folder_with_attachments(&path, &["Inbox"], &message("Plan"), &[FILE]);
    let _ = UnicodePstFile::append_in_folder(&path, &["Projects"], &message("Nested"));
    let data = std::fs::read(&path).unwrap_or_default();
    let _ = std::fs::remove_file(&path);
    assert!(data.starts_with(b"!BDN"), "the writer wrote no data file");
    data
}

static GOOD: OnceLock<Vec<u8>> = OnceLock::new();

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
    let attachments = [FILE];
    let batch = [UnicodePstBatchMessage { message: message("Added"), attachments: &attachments }];
    if UnicodePstFile::append_many_in_folder_with_attachments(&path, &["Inbox"], &batch).is_ok() {
        // What the writer says it wrote reads back.
        let read = UnicodePstFile::open(&path).and_then(|pst| UnicodeStore::read(Rc::new(pst)));
        if let Ok(store) = read {
            let _ = store.properties().ipm_sub_tree_entry_id().and_then(|id| store.open_folder(&id));
        }
    }
    let _ = std::fs::remove_file(&path);
});
