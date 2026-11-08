//! Damaged files: whatever a file holds, reading it or adding to it gives data or an error, and
//! ends. (The fuzz targets go further, with the CRCs out of the way; `fuzz/README.md`.)

use std::io::Cursor;
use std::rc::Rc;

use mailcraft_pst::write::{self, *};
use mailcraft_pst::*;

fn temp(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("mailcraft-pst-hostile-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let _ = std::fs::remove_file(&path);
    path
}

fn message(subject: &str) -> NewMessage<'_> {
    NewMessage {
        subject,
        sender_name: "Ann",
        sender_email: "ann@example.org",
        recipients: &[],
        body: "The body.",
        html_body: None,
        message_id: "<one@example.org>",
        delivery_time: 133_000_000_000_000_000,
        message_class: "IPM.Note",
        properties: &[],
    }
}

/// A small file of the format, mail in two folders.
fn good(format: Format, name: &str) -> Vec<u8> {
    let path = temp(name);
    write::create_empty(&path, format).unwrap();
    let file = NewAttachment { filename: "a.txt", data: b"attached", ..Default::default() };
    let attachments = [file];
    for (folder, subject) in [("Inbox", "One"), ("Inbox", "Two"), ("Later", "Three")] {
        write::append_message(&path, &[folder], &message(subject), &attachments).unwrap();
    }
    let bytes = std::fs::read(&path).unwrap();
    let _ = std::fs::remove_file(&path);
    bytes
}

/// Reads all it can of `bytes`, and how many messages that was.
fn read_all(bytes: &[u8]) -> usize {
    let Ok(pst) = PstFile::read_from(Box::new(Cursor::new(bytes.to_vec()))) else { return 0 };
    pst.set_max_value_size(1 << 20);
    let Ok(store) = PstStore::read(Rc::new(pst)) else { return 0 };
    let _ = store.named_property_map().map(|m| (m.stream_guid().len(), m.stream_entry().len(), m.stream_string().len()));
    let Ok(top) = store.ipm_subtree() else { return 0 };
    let (mut stack, mut seen, mut read) = (vec![(top, 0usize)], Vec::new(), 0);
    while let Some((node, depth)) = stack.pop() {
        if depth > 32 || seen.contains(&node) {
            continue;
        }
        seen.push(node);
        let Ok(folder) = store.open_folder(node) else { continue };
        for row in folder.contents_table().iter().flat_map(|t| t.rows_matrix()) {
            let Ok(m) = Message::read(store.clone(), row.id()) else { continue };
            read += 1;
            if let Some(t) = m.recipient_table() {
                for r in t.rows_matrix() {
                    for i in 0..t.columns().len() {
                        let _ = t.value(r, i);
                    }
                }
            }
            for r in m.attachment_table().iter().flat_map(|t| t.rows_matrix()) {
                if let Ok(a) = Attachment::read(m.clone(), r.id()) {
                    let _ = a.data().is_some();
                }
            }
        }
        for row in folder.hierarchy_table().iter().flat_map(|t| t.rows_matrix()) {
            stack.push((row.id(), depth + 1));
        }
    }
    read
}

/// A small generator, the same run each time.
struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

#[test]
fn a_good_file_reads_whole() {
    for (format, name) in [(Format::Unicode, "whole-u.pst"), (Format::Ansi, "whole-a.pst")] {
        assert_eq!(read_all(&good(format, name)), 3, "{format:?}");
    }
}

#[test]
fn a_file_cut_short_anywhere_is_read_as_far_as_it_goes() {
    for (format, name) in [(Format::Unicode, "cut-u.pst"), (Format::Ansi, "cut-a.pst")] {
        let file = good(format, name);
        for len in (0..file.len()).step_by(397) {
            assert!(read_all(&file[..len]) <= 3);
        }
    }
}

#[test]
fn damage_anywhere_reads_as_far_as_it_goes() {
    let mut random = Random(0x9E37_79B9_7F4A_7C15);
    for (format, name) in [(Format::Unicode, "bytes-u.pst"), (Format::Ansi, "bytes-a.pst")] {
        let file = good(format, name);
        for _ in 0..1500 {
            let mut damaged = file.clone();
            for _ in 0..=random.below(16) {
                let at = random.below(damaged.len());
                damaged[at] = random.next() as u8;
            }
            assert!(read_all(&damaged) <= 3);
        }
    }
}

/// Adding to a damaged file fails or leaves a file that still reads; neither panics.
#[test]
fn adding_to_a_damaged_file_ends() {
    let mut random = Random(0xD1B5_4A32_D192_ED03);
    let file = good(Format::Unicode, "add-source.pst");
    let path = temp("add.pst");
    for _ in 0..200 {
        let mut damaged = file.clone();
        for _ in 0..=random.below(8) {
            let at = random.below(damaged.len());
            damaged[at] = random.next() as u8;
        }
        std::fs::write(&path, &damaged).unwrap();
        if write::append_message(&path, &["Inbox"], &message("Four"), &[]).is_ok() {
            assert!(read_all(&std::fs::read(&path).unwrap()) >= 1);
        }
    }
}
