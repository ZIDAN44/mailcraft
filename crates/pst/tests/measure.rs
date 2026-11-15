//! Timings of the writer, run by hand in release:
//! `cargo test --release -p mailcraft-pst --test measure -- --ignored --nocapture`.

#[path = "support/scratch.rs"]
mod scratch;

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Instant;

use mailcraft_pst::write::{self, AttachmentContent, BatchMessage, Name, NewAttachment, NewMessage, NewProperty, NewValue, SpecialFolder};
use mailcraft_pst::{Format, Message, PS_PUBLIC_STRINGS, PropertyValue, PstFile, PstStore};
use scratch::scratch;

/// A file `name` in a scratch folder of its own.
fn temp(name: &str) -> PathBuf {
    scratch(&format!("pst-measure-{name}")).join(name)
}

/// Times each timing is taken; the best is the one told, the others being slowed by the machine.
const ROUNDS: usize = 3;

/// Where a timing's file starts from: with `MAILCRAFT_PST_MEASURE_BASE` set, a copy of the file
/// `name` in that folder (made there when missing), so two builds start from the same bytes and
/// what they write can be compared; else a new one.
fn start_from(name: &str, make: impl Fn(&Path)) -> PathBuf {
    let work = temp(name);
    match std::env::var_os("MAILCRAFT_PST_MEASURE_BASE") {
        Some(dir) => {
            let base = Path::new(&dir).join(name);
            if !base.exists() {
                make(&base);
            }
            std::fs::copy(&base, &work).unwrap();
        }
        None => make(&work),
    }
    work
}

/// The file's bytes, as one number (FNV-1a).
fn digest(path: &Path) -> u64 {
    std::fs::read(path).unwrap().iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3))
}

/// `n` small messages, numbered from `from`, added to the folder at `folder`.
fn add(path: &Path, folder: &[&str], from: usize, n: usize) {
    add_with(path, folder, from, n, &[]);
}

/// [`add`], each message carrying `properties`.
fn add_with(path: &Path, folder: &[&str], from: usize, n: usize, properties: &[NewProperty<'_>]) {
    let subjects: Vec<String> = (from..from + n).map(|i| format!("Message {i}")).collect();
    let ids: Vec<String> = (from..from + n).map(|i| format!("<m{i}@example.org>")).collect();
    let batch: Vec<BatchMessage<'_>> = subjects
        .iter()
        .zip(&ids)
        .enumerate()
        .map(|(i, (subject, id))| BatchMessage {
            message: NewMessage {
                subject,
                sender_name: "Ann",
                sender_email: "ann@example.org",
                recipients: &[],
                body: "A short body.",
                html_body: None,
                message_id: id,
                delivery_time: 133_000_000_000_000_000 + i64::try_from(from + i).unwrap() * 10_000_000,
                message_class: "IPM.Note",
                properties,
            },
            attachments: &[],
        })
        .collect();
    write::append(path, folder, &batch).unwrap();
}

/// Small batches added to a file that already holds much: each addition opens the file to change
/// it, and each opening reads its maps and both of its B-trees through.
#[test]
#[ignore = "a measurement"]
fn measure_small_batches_into_a_full_file() {
    let make = |p: &Path| {
        write::create_empty(p, Format::Unicode).unwrap();
        for b in 0..10 {
            add(p, &["Archive"], b * 2000, 2000);
        }
    };
    let mut best = u128::MAX;
    for _ in 0..ROUNDS {
        let path = start_from("full.pst", make);
        let t = Instant::now();
        for b in 0..40 {
            add(&path, &["Inbox"], 100_000 + b * 25, 25);
            add(&path, &["Mail", "2026", "Kept"], 200_000 + b * 25, 25);
        }
        best = best.min(t.elapsed().as_millis());
        println!("file {:016x}", digest(&path));
    }
    println!("MEASURE 80 batches of 25 into a file of 20000: {best} ms (best of {ROUNDS})");
}

/// Naming the special folders of a file with many folders.
#[test]
#[ignore = "a measurement"]
fn measure_special_folders_among_many_folders() {
    let path = temp("many-folders.pst");
    write::create_empty(&path, Format::Unicode).unwrap();
    for i in 0..200 {
        let top = format!("Folder {i}");
        for j in 0..5 {
            write::create_folder(&path, &[top.as_str(), &format!("Sub {j}")], "IPF.Note").unwrap();
        }
    }
    write::set_special_folder(&path, SpecialFolder::SentItems, &["Folder 100", "Sub 2"]).unwrap();
    write::set_special_folder(&path, SpecialFolder::Tasks, &["Folder 0", "Sub 4"]).unwrap();
    let t = Instant::now();
    for _ in 0..20 {
        assert_eq!(write::special_folder(&path, SpecialFolder::SentItems).unwrap(), Some(vec!["Folder 100".to_string(), "Sub 2".to_string()]));
        assert_eq!(write::special_folder(&path, SpecialFolder::Tasks).unwrap(), Some(vec!["Folder 0".to_string(), "Sub 4".to_string()]));
    }
    println!("MEASURE 40 special folder lookups among 1200 folders: {} ms", t.elapsed().as_millis());
}

/// Batches added to, and messages taken out of, a folder that holds many: its contents table is
/// written again each time.
#[test]
#[ignore = "a measurement"]
fn measure_a_big_folder_added_to_and_taken_from() {
    let make = |p: &Path| {
        write::create_empty(p, Format::Unicode).unwrap();
        for b in 0..30 {
            add(p, &["Inbox"], b * 2000, 2000);
        }
    };
    let (mut added, mut taken) = (u128::MAX, u128::MAX);
    for _ in 0..ROUNDS {
        let path = start_from("big-folder.pst", make);
        let t = Instant::now();
        for b in 0..5 {
            add(&path, &["Inbox"], 60_000 + b * 2000, 2000);
        }
        added = added.min(t.elapsed().as_millis());
        let store = PstStore::read(Rc::new(PstFile::open(&path).unwrap())).unwrap();
        let top = store.open_folder(store.ipm_subtree().unwrap()).unwrap();
        let named_inbox = |id: u32| matches!(store.open_folder(id).unwrap().properties().get(0x3001), Some(PropertyValue::Unicode(s)) if String::from_utf16_lossy(s.buffer()).trim_end_matches('\0') == "Inbox");
        let inbox = top.hierarchy_table().unwrap().rows_matrix().map(|r| r.id()).find(|id| named_inbox(*id)).unwrap();
        let rows: Vec<u32> = store.open_folder(inbox).unwrap().contents_table().unwrap().rows_matrix().map(|r| r.id()).collect();
        drop((top, store));
        let t = Instant::now();
        for chunk in rows.chunks(20).take(5) {
            write::delete_messages(&path, chunk).unwrap();
        }
        taken = taken.min(t.elapsed().as_millis());
        println!("file {:016x}", digest(&path));
    }
    println!("MEASURE 5 batches of 2000 into a folder of 60000: {added} ms; 5 deletes of 20: {taken} ms (best of {ROUNDS})");
}

/// Messages that carry named properties: each one's row in its folder's contents table is made
/// from them.
#[test]
#[ignore = "a measurement"]
fn measure_messages_with_named_properties() {
    let names: Vec<String> = (0..30).map(|i| format!("Label {i}")).collect();
    let values: Vec<String> = (0..30).map(|i| format!("value {i}")).collect();
    let properties: Vec<NewProperty<'_>> =
        names.iter().zip(&values).map(|(n, v)| NewProperty::named(PS_PUBLIC_STRINGS, Name::String(n), NewValue::Unicode(v))).collect();
    let mut best = u128::MAX;
    for _ in 0..ROUNDS {
        let path = start_from("named.pst", |p| write::create_empty(p, Format::Unicode).unwrap());
        let t = Instant::now();
        for b in 0..5 {
            add_with(&path, &["Inbox"], b * 1000, 1000, &properties);
        }
        best = best.min(t.elapsed().as_millis());
        println!("file {:016x}", digest(&path));
    }
    println!("MEASURE 5 batches of 1000 messages of 30 named properties: {best} ms (best of {ROUNDS})");
}

/// A large attachment written into a file full of small holes: every block of it is looked for in
/// the free space the maps mark first.
#[test]
#[ignore = "a measurement"]
fn measure_a_large_attachment_into_a_fragmented_file() {
    let make = |p: &Path| {
        write::create_empty(p, Format::Unicode).unwrap();
        for b in 0..10 {
            add(p, &["Inbox"], b * 2000, 2000);
        }
        // Every other message taken out: what they held is free, in holes between the rest.
        let store = PstStore::read(Rc::new(PstFile::open(p).unwrap())).unwrap();
        let top = store.open_folder(store.ipm_subtree().unwrap()).unwrap();
        let named_inbox = |id: u32| matches!(store.open_folder(id).unwrap().properties().get(0x3001), Some(PropertyValue::Unicode(s)) if String::from_utf16_lossy(s.buffer()).trim_end_matches('\0') == "Inbox");
        let inbox = top.hierarchy_table().unwrap().rows_matrix().map(|r| r.id()).find(|id| named_inbox(*id)).unwrap();
        let rows: Vec<u32> = store.open_folder(inbox).unwrap().contents_table().unwrap().rows_matrix().map(|r| r.id()).step_by(2).collect();
        drop((top, store));
        write::delete_messages(p, &rows).unwrap();
    };
    let data: Vec<u8> = (0..40_000_000u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8).collect();
    let attachment = NewAttachment {
        filename: "big.bin",
        mime_type: "application/octet-stream",
        content_id: None,
        data: &data,
        content: AttachmentContent::Data,
        properties: &[],
    };
    let mut best = u128::MAX;
    for _ in 0..ROUNDS {
        let path = start_from("fragmented.pst", make);
        let message = NewMessage {
            subject: "Big",
            sender_name: "Ann",
            sender_email: "ann@example.org",
            recipients: &[],
            body: "A big attachment.",
            html_body: None,
            message_id: "<big@example.org>",
            delivery_time: 133_000_000_000_000_000,
            message_class: "IPM.Note",
            properties: &[],
        };
        let t = Instant::now();
        write::append_message(&path, &["Inbox"], &message, std::slice::from_ref(&attachment)).unwrap();
        best = best.min(t.elapsed().as_millis());
        println!("file {:016x}", digest(&path));
    }
    println!("MEASURE a 40 MB attachment into a file of 20000 messages, every other one taken out: {best} ms (best of {ROUNDS})");
}

/// Every message of a big folder read: each one's node and blocks are looked up in the B-trees,
/// whose pages are kept.
#[test]
#[ignore = "a measurement"]
fn measure_reading_every_message_of_a_big_folder() {
    let make = |p: &Path| {
        write::create_empty(p, Format::Unicode).unwrap();
        for b in 0..30 {
            add(p, &["Inbox"], b * 2000, 2000);
        }
    };
    let path = start_from("big-folder.pst", make);
    let store = PstStore::read(Rc::new(PstFile::open(&path).unwrap())).unwrap();
    let top = store.open_folder(store.ipm_subtree().unwrap()).unwrap();
    let named_inbox = |id: u32| matches!(store.open_folder(id).unwrap().properties().get(0x3001), Some(PropertyValue::Unicode(s)) if String::from_utf16_lossy(s.buffer()).trim_end_matches('\0') == "Inbox");
    let inbox = top.hierarchy_table().unwrap().rows_matrix().map(|r| r.id()).find(|id| named_inbox(*id)).unwrap();
    let rows: Vec<u32> = store.open_folder(inbox).unwrap().contents_table().unwrap().rows_matrix().map(|r| r.id()).collect();
    let mut best = u128::MAX;
    for _ in 0..ROUNDS {
        let t = Instant::now();
        let mut subjects = 0;
        for nid in &rows {
            let message = Message::read(store.clone(), *nid).unwrap();
            subjects += usize::from(message.properties().get(0x0037).is_some());
        }
        assert_eq!(subjects, rows.len());
        best = best.min(t.elapsed().as_millis());
    }
    println!("MEASURE {} messages read: {best} ms (best of {ROUNDS})", rows.len());
}

/// Messages with many named properties added to a file whose name-to-ID map holds many: each
/// addition looks every name it gives up in the map.
#[test]
#[ignore = "a measurement"]
fn measure_named_properties_looked_up_in_a_big_name_map() {
    let names: Vec<String> = (0..5000).map(|i| format!("Label {i}")).collect();
    let named = |range: std::ops::Range<usize>| -> Vec<NewProperty<'_>> {
        names[range].iter().map(|n| NewProperty::named(PS_PUBLIC_STRINGS, Name::String(n), NewValue::Integer32(1))).collect()
    };
    let make = |p: &Path| {
        write::create_empty(p, Format::Unicode).unwrap();
        for k in 0..50 {
            add_with(p, &["Inbox"], k, 1, &named(k * 100..k * 100 + 100));
        }
    };
    let given = named(0..300);
    let mut best = u128::MAX;
    for _ in 0..ROUNDS {
        let path = start_from("name-map.pst", make);
        let t = Instant::now();
        for b in 0..20 {
            add_with(&path, &["Inbox"], 1000 + b * 5, 5, &given);
        }
        best = best.min(t.elapsed().as_millis());
        println!("file {:016x}", digest(&path));
    }
    println!("MEASURE 20 batches of 5 messages of 300 names into a map of 5000: {best} ms (best of {ROUNDS})");
}
