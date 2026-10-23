//! An Outlook data file (`.pst`) as the user picks it to import: read into an empty cache.
//!
//! Random bytes rarely get past a `.pst`'s header, so most inputs damage a good file instead:
//! one MailCraft exported (two folders, mail with an attachment, an event and a contact), with
//! the input read as five-byte patches (a little-endian offset, then the byte to put there).
//! An input that starts like a data file (`!BDN`) is read as one, whole.
//!
//! `pst::import` fences the reader with `catch_unwind` and calls a file that panics it damaged,
//! so a panic inside outlook-pst (an overflow on a damaged block count, say) is the expected
//! answer here, not a crash. A panic in MailCraft's own code still aborts the run.
#![no_main]

use std::path::PathBuf;
use std::sync::OnceLock;

use libfuzzer_sys::fuzz_target;
use mailcraft_mime as mime;
use mailcraft_model::pim::{PIM_DOC, Pim, StoredCard, StoredEvent};
use mailcraft_model::{EmailAddress, Flags, Folder, FolderRole, MessageKey, ical, vcard};
use mailcraft_net::{pst, pst_export};
use mailcraft_store::{FolderState, Store};

fn scratch(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("mailcraft-fuzz-{}-{name}", std::process::id()))
}

/// A data file exported from a small account.
fn good_file() -> Vec<u8> {
    let store = Store::memory();
    let mk = |folder: &str, uid: u32, subject: &str, with_file: bool| {
        let d = mime::Draft {
            to: vec![EmailAddress::new("Ana", "ana@example.com")],
            subject: subject.into(),
            body: "Hello there.".into(),
            html: Some("<p>Hello <b>there</b>.</p>".into()),
            attachments: if with_file {
                vec![mime::OutAttachment { name: "notes.txt".into(), mime: "text/plain".into(), data: b"remember".to_vec() }]
            } else {
                Vec::new()
            },
            ..Default::default()
        };
        let raw =
            mime::build(&d, &EmailAddress::new("Riley", "riley@example.com"), 1_704_067_200, &format!("m{uid}@example.com")).unwrap_or_default();
        let key = MessageKey { account: "a1".into(), folder: folder.into(), validity: 1, uid };
        let _ = store.write_body(&key, &raw);
        mime::summary_from_full(&raw, key, Flags::default(), 10, 0)
    };
    let folder =
        |path: &str, role| Folder { account: "a1".into(), path: path.into(), delimiter: Some('/'), role, selectable: true, ..Default::default() };
    let inbox = FolderState {
        validity: 1,
        uid_next: 3,
        messages: vec![mk("INBOX", 1, "Plan — café", true), mk("INBOX", 2, "Second", false)],
        ..Default::default()
    };
    let projects = FolderState { validity: 1, uid_next: 2, messages: vec![mk("INBOX/Projects", 1, "Nested", false)], ..Default::default() };
    let _ = store.save_folder("a1", "INBOX", &inbox);
    let _ = store.save_folder("a1", "INBOX/Projects", &projects);
    let _ = store.save_folders("a1", &[folder("INBOX", FolderRole::Inbox), folder("INBOX/Projects", FolderRole::Other)]);
    let mut pim = Pim::default();
    pim.events.push(StoredEvent {
        event: ical::CalEvent { uid: "e1".into(), summary: "Screening".into(), start: 1_792_000_000, end: 1_792_003_600, ..Default::default() },
        ..Default::default()
    });
    pim.cards.push(StoredCard { card: vcard::Card { uid: "c1".into(), name: "Ana Lima".into(), ..Default::default() }, ..Default::default() });
    let _ = store.save_account_doc("a1", PIM_DOC, &pim);
    let path = scratch("good.pst");
    let _ = pst_export::export(&store, "a1", "Riley's mail", &path, &mut |_| {});
    let data = std::fs::read(&path).unwrap_or_default();
    for f in ["good.pst", "good Calendar.ics", "good Contacts.vcf"] {
        let _ = std::fs::remove_file(scratch(f));
    }
    assert!(data.starts_with(b"!BDN"), "the exporter wrote no data file");
    data
}

static GOOD: OnceLock<Vec<u8>> = OnceLock::new();

/// Abort on a panic in MailCraft's code; let one in a dependency reach `import`'s fence. (One
/// that escapes the fence is still caught by libfuzzer-sys, which aborts.)
fn only_our_panics_abort() {
    std::panic::set_hook(Box::new(|info| {
        if info.location().is_none_or(|l| !l.file().contains(".cargo/registry")) {
            eprintln!("{info}");
            std::process::abort();
        }
    }));
}

fuzz_target!(init: only_our_panics_abort(), |data: &[u8]| {
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
    let path = scratch("in.pst");
    if std::fs::write(&path, &file).is_err() {
        return;
    }
    let store = Store::memory();
    let _ = pst::import(&path, &store, "d1", &mut |_| {});
});
