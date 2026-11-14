//! B-tree pages whose level byte says what no tree has: refused, with no overflow on the way.

use std::path::{Path, PathBuf};

use super::*;
use crate::ndb::PT_NBT;
use crate::ndb::btree::node_entry_size;
use crate::write::{NewMessage, append_message, create_empty};

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

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mailcraft-pst-levels-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let _ = std::fs::remove_file(&path);
    path
}

/// A Unicode file with one message, the last page under its node B-tree's root saying it is at
/// `level` (its CRC made right).
fn with_a_child_at(level: u8, path: &Path) {
    create_empty(path, Format::Unicode).unwrap();
    append_message(path, &["Inbox"], &message("Mail"), &[]).unwrap();
    let editor = Editor::open(path).unwrap();
    let format = Format::Unicode;
    let root = BtPage::parse(editor.pst.page(editor.header.root.nbt.ib, PT_NBT, 0).unwrap(), format, node_entry_size(format)).unwrap();
    assert!(root.level == 1 && root.count >= 2);
    let at = usize::try_from(root.child(root.count - 1, format).unwrap().ib).unwrap();
    drop(root);
    drop(editor);
    let mut file = std::fs::read(path).unwrap();
    // `cLevel`, after the entries (488 bytes) and `cEnt`, `cEntMax`, `cbEnt`.
    file[at + 491] = level;
    let crc = crate::crc::crc(&file[at..at + 496]);
    file[at + 500..at + 504].copy_from_slice(&crc.to_le_bytes());
    std::fs::write(path, &file).unwrap();
}

/// A child page at level 255 under a root at level 1: `255 + 1` would overflow, so the level is
/// checked first, when the tree is walked and when it is changed.
#[test]
fn a_page_at_level_255_is_refused() {
    let path = scratch("level.pst");
    with_a_child_at(0xFF, &path);
    if let Some(to) = std::env::var_os("MC_PST_LEVEL_SEED") {
        std::fs::copy(&path, to).unwrap();
    }
    let mut editor = Editor::open(&path).unwrap();
    let (root, size) = (editor.header.root.nbt, node_entry_size(Format::Unicode));
    // A change that goes down to that page.
    let changes: Changes = [(u64::from(u32::MAX), None)].into_iter().collect();
    let updated = editor.update_tree(root, PT_NBT, size, &changes);
    assert!(updated.is_err_and(|e| e.to_string().contains("level by level")));
    drop(editor);
    let added = append_message(&path, &["Inbox"], &message("More"), &[]);
    assert!(added.is_err_and(|e| e.to_string().contains("level by level")));
    let _ = std::fs::remove_file(path);
}

/// Every level from 8 up is past what a tree has, and a page one level too high is refused too.
#[test]
fn every_level_but_the_one_below_is_refused() {
    for level in [2, 8, 9, 0x80, 0xFE] {
        let path = scratch(&format!("level-{level}.pst"));
        with_a_child_at(level, &path);
        let added = append_message(&path, &["Inbox"], &message("More"), &[]);
        assert!(added.is_err_and(|e| e.to_string().contains("level by level")), "level {level}");
        let _ = std::fs::remove_file(path);
    }
}
