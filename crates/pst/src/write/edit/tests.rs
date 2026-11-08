//! The change to a file on files damaged with their checksums made right: what a CRC would
//! otherwise stop at the door.

use std::path::PathBuf;

use super::super::table::{Schema, build_table};
use super::super::types::{NewAttachment, NewProperty, NewValue};
use super::*;
use crate::ltp::PropertyValue;
use crate::write::{NewMessage, append_message, create_empty, delete_messages};

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

/// The messages in the folders at the top of the file.
fn mail_in(path: &Path) -> Vec<u32> {
    let store = PstStore::read(Rc::new(PstFile::open(path).unwrap())).unwrap();
    let top = store.open_folder(store.ipm_subtree().unwrap()).unwrap();
    let folders: Vec<u32> = top.hierarchy_table().unwrap().rows_matrix().map(|r| r.id()).collect();
    let mut ids = Vec::new();
    for f in folders {
        let folder = store.open_folder(f).unwrap();
        ids.extend(folder.contents_table().iter().flat_map(|t| t.rows_matrix()).map(|r| r.id()));
    }
    ids
}

/// A page of the node B-tree whose keys are out of order (its CRC good) is refused, not
/// rewritten: the rewrite would put entries where the rest of the tree doesn't look for them.
#[test]
fn a_page_with_its_keys_out_of_order_is_refused() {
    let dir = std::env::temp_dir().join(format!("mailcraft-pst-order-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("order.pst");
    let _ = std::fs::remove_file(&path);
    create_empty(&path, Format::Unicode).unwrap();
    append_message(&path, &["Inbox"], &message("Mail"), &[]).unwrap();
    let editor = Editor::open(&path).unwrap();
    let root = editor.header.root.nbt;
    let mut pages = Vec::new();
    editor.walk_tree(root, PT_NBT, node_entry_size(Format::Unicode), &mut |_, _| Ok(()), &mut |ib| pages.push(ib)).unwrap();
    let mut leaf = None;
    for ib in pages {
        let p = crate::ndb::btree::BtPage::parse(editor.pst.page(ib, PT_NBT, 0).unwrap(), Format::Unicode, node_entry_size(Format::Unicode)).unwrap();
        if p.level == 0 && p.count >= 2 {
            leaf = Some(ib);
        }
    }
    let at = usize::try_from(leaf.unwrap()).unwrap();
    drop(editor);
    let mut file = std::fs::read(&path).unwrap();
    let size = node_entry_size(Format::Unicode);
    let (first, second) = file[at..at + 2 * size].split_at_mut(size);
    first.swap_with_slice(second);
    let crc = crate::crc::crc(&file[at..at + 496]);
    file[at + 500..at + 504].copy_from_slice(&crc.to_le_bytes());
    std::fs::write(&path, &file).unwrap();
    let tried = append_message(&path, &["Inbox"], &message("More"), &[]);
    assert!(tried.is_err_and(|e| e.to_string().contains("out of order")));
    let _ = std::fs::remove_dir_all(dir);
}

/// A leaf holding a key its parent would send elsewhere is refused too: rewritten, the leaf's
/// new first key would take its parent's lookups from the page before.
#[test]
fn a_page_with_keys_its_parent_does_not_send_to_it_is_refused() {
    let dir = std::env::temp_dir().join(format!("mailcraft-pst-range-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("range.pst");
    let _ = std::fs::remove_file(&path);
    create_empty(&path, Format::Unicode).unwrap();
    append_message(&path, &["Inbox"], &message("Mail"), &[]).unwrap();
    let editor = Editor::open(&path).unwrap();
    let size = node_entry_size(Format::Unicode);
    let root = crate::ndb::btree::BtPage::parse(editor.pst.page(editor.header.root.nbt.ib, PT_NBT, 0).unwrap(), Format::Unicode, size).unwrap();
    assert!(root.level > 0 && root.count >= 2);
    let last = root.count - 1;
    let (key, at) = (root.key(last, Format::Unicode).unwrap(), root.child(last, Format::Unicode).unwrap().ib);
    drop(root);
    drop(editor);
    let at = usize::try_from(at).unwrap();
    let mut file = std::fs::read(&path).unwrap();
    file[at..at + 8].copy_from_slice(&(key - 1).to_le_bytes());
    let crc = crate::crc::crc(&file[at..at + 496]);
    file[at + 500..at + 504].copy_from_slice(&crc.to_le_bytes());
    std::fs::write(&path, &file).unwrap();
    let tried = append_message(&path, &["Inbox"], &message("More"), &[]);
    assert!(tried.is_err_and(|e| e.to_string().contains("doesn't send")));
    let _ = std::fs::remove_dir_all(dir);
}

/// What the maps mark free after mail is taken out is offered again, and none of it holds
/// anything the file still uses; in both formats.
#[test]
fn the_maps_match_what_the_file_holds() {
    let dir = std::env::temp_dir().join(format!("mailcraft-pst-maps-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (format, name) in [(Format::Unicode, "u.pst"), (Format::Ansi, "a.pst")] {
        let path = dir.join(name);
        let _ = std::fs::remove_file(&path);
        create_empty(&path, format).unwrap();
        for _ in 0..20 {
            append_message(&path, &["Inbox"], &message("Mail"), &[]).unwrap();
        }
        let ids = mail_in(&path);
        assert_eq!(ids.len(), 20, "{format:?}");
        delete_messages(&path, &ids[..10]).unwrap();
        let editor = Editor::open(&path).unwrap();
        assert!(editor.maps_valid, "{format:?}");
        assert!(!editor.space.free_ranges().is_empty(), "{format:?}: what was taken out is free");
        assert!(editor.free_space_is_free().unwrap(), "{format:?}");
    }
    let _ = std::fs::remove_dir_all(dir);
}

// --- Files damaged with their checksums made right ---------------------------------------

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mailcraft-pst-forged-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let _ = std::fs::remove_file(&path);
    path
}

/// A file with one message.
fn one_message(name: &str, properties: &[NewProperty<'_>], attachments: &[NewAttachment<'_>]) -> PathBuf {
    let path = scratch(name);
    create_empty(&path, Format::Unicode).unwrap();
    let m = NewMessage { properties, ..message("Mail") };
    append_message(&path, &["Inbox"], &m, attachments).unwrap();
    path
}

fn store_of(path: &Path, limit: u64) -> Rc<PstStore> {
    let pst = PstFile::open(path).unwrap();
    pst.set_max_value_size(limit);
    PstStore::read(Rc::new(pst)).unwrap()
}

/// Makes a page's CRC right.
fn seal_page(file: &mut [u8], at: usize) {
    let crc = crate::crc::crc(&file[at..at + 496]);
    file[at + 500..at + 504].copy_from_slice(&crc.to_le_bytes());
}

/// Where block `bid` is, and its size.
fn block_span(pst: &PstFile, bid: u64) -> (usize, usize) {
    let e = pst.block_entry(bid).unwrap();
    (usize::try_from(e.ib).unwrap(), usize::from(e.size))
}

/// Makes a block's CRC right.
fn seal_block(file: &mut [u8], (at, len): (usize, usize)) {
    let crc = crate::crc::crc(&file[at..at + len]);
    let trailer = at + align64(len + 16) - 16;
    file[trailer + 4..trailer + 8].copy_from_slice(&crc.to_le_bytes());
}

/// The offset in the block at `span` of the record of property `id` of type `kind`.
fn record_at(file: &[u8], (at, len): (usize, usize), id: u16, kind: u16) -> usize {
    let pattern = [id.to_le_bytes(), kind.to_le_bytes()].concat();
    at + file[at..at + len].windows(4).position(|w| w == pattern).unwrap()
}

/// The message of a one-message file.
fn the_message(store: &Rc<PstStore>) -> u32 {
    let top = store.open_folder(store.ipm_subtree().unwrap()).unwrap();
    for row in top.hierarchy_table().unwrap().rows_matrix() {
        let folder = store.open_folder(row.id()).unwrap();
        if let Some(found) = folder.contents_table().and_then(|t| t.rows_matrix().next().map(|r| r.id())) {
            return found;
        }
    }
    panic!("no message");
}

fn big_properties(value: &[u8]) -> Vec<NewProperty<'_>> {
    (0..4).map(|i| NewProperty::tagged(0x6000 + i, NewValue::Binary(value))).collect()
}

/// Values kept in subnodes add up: a few may each be under the file's limit and still be
/// more than one object may read, as a small file naming a huge value many times would be.
#[test]
fn an_objects_values_share_one_limit() {
    let big = vec![7u8; 20_000];
    let path = one_message("budget.pst", &big_properties(&big), &[]);
    let store = store_of(&path, 1 << 30);
    let nid = the_message(&store);
    assert!(crate::messaging::Message::read(store, nid).is_ok());
    let tight = store_of(&path, 30_000);
    let e = crate::messaging::Message::read(tight, nid).err().unwrap();
    assert!(e.to_string().contains("over the limit"), "{e}");
}

/// One value that can't be read doesn't stop those asked for by name.
#[test]
fn a_damaged_value_costs_only_itself_when_others_are_asked_for() {
    let big = vec![7u8; 20_000];
    let path = one_message("one-bad.pst", &big_properties(&big), &[]);
    let store = store_of(&path, 1 << 30);
    let nid = the_message(&store);
    let span = block_span(store.pst(), store.pst().node(nid).unwrap().data);
    drop(store);
    let mut file = std::fs::read(&path).unwrap();
    let at = record_at(&file, span, 0x6000, 0x0102);
    file[at + 4..at + 8].copy_from_slice(&0x7FFF_FFE1u32.to_le_bytes());
    seal_block(&mut file, span);
    std::fs::write(&path, &file).unwrap();
    let store = store_of(&path, 1 << 30);
    assert!(crate::messaging::Message::read(store.clone(), nid).is_err());
    let wanted = store.message_properties(nid, &[0x1035]).unwrap();
    assert!(wanted.get(0x1035).is_some());
    assert!(wanted.get(0x6000).is_none());
}

/// `PidTagAttachDataBinary` typed Null: a file of no bytes.
#[test]
fn an_attachments_data_typed_null_is_an_empty_file() {
    let file_part = NewAttachment { filename: "empty.txt", mime_type: "text/plain", ..Default::default() };
    let path = one_message("null-data.pst", &[], std::slice::from_ref(&file_part));
    let store = store_of(&path, 1 << 30);
    let nid = the_message(&store);
    let subs = store.pst().subnodes(store.pst().node(nid).unwrap().sub).unwrap();
    let (_, (data, _)) = subs.iter().find(|(n, _)| **n & NID_TYPE_MASK == NID_TYPE_ATTACHMENT).unwrap();
    let span = block_span(store.pst(), *data);
    drop(store);
    let mut file = std::fs::read(&path).unwrap();
    let at = record_at(&file, span, 0x3701, 0x0102);
    file[at + 2..at + 4].copy_from_slice(&0x0001u16.to_le_bytes());
    seal_block(&mut file, span);
    std::fs::write(&path, &file).unwrap();
    let store = store_of(&path, 1 << 30);
    let m = crate::messaging::Message::read(store, nid).unwrap();
    let row = m.attachment_table().unwrap().rows_matrix().next().unwrap().id();
    let a = crate::messaging::Attachment::read(m.clone(), row).unwrap();
    assert!(matches!(a.data(), Some(crate::messaging::AttachmentData::Binary(b)) if b.buffer().is_empty()));
}

/// `cbEnt` may be more than an entry needs: such a page reads, and a change writes it back
/// without losing a thing.
#[test]
fn a_page_of_wider_entries_reads_and_is_rewritten() {
    let path = one_message("wide.pst", &[], &[]);
    let size = node_entry_size(Format::Unicode);
    // Mail goes in until a leaf has room for its entries to be wider.
    let leaf = loop {
        let editor = Editor::open(&path).unwrap();
        let mut pages = Vec::new();
        editor.walk_tree(editor.header.root.nbt, PT_NBT, size, &mut |_, _| Ok(()), &mut |ib| pages.push(ib)).unwrap();
        let found = pages.iter().copied().find(|ib| {
            let p = crate::ndb::btree::BtPage::parse(editor.pst.page(*ib, PT_NBT, 0).unwrap(), Format::Unicode, size).unwrap();
            p.level == 0 && (2..=12).contains(&p.count)
        });
        drop(editor);
        match found {
            Some(ib) => break ib,
            None => append_message(&path, &["Inbox"], &message("Mail"), &[]).unwrap(),
        }
    };
    let held = mail_in(&path).len();
    let at = usize::try_from(leaf).unwrap();
    let mut file = std::fs::read(&path).unwrap();
    let count = usize::from(file[at + 488]);
    let old = file[at..at + count * size].to_vec();
    file[at..at + 488].fill(0);
    for (i, e) in old.chunks_exact(size).enumerate() {
        file[at + i * 40..at + i * 40 + size].copy_from_slice(e);
    }
    file[at + 490] = 40;
    seal_page(&mut file, at);
    std::fs::write(&path, &file).unwrap();
    let store = store_of(&path, 1 << 30);
    crate::messaging::Message::read(store.clone(), the_message(&store)).unwrap();
    drop(store);
    append_message(&path, &["Inbox"], &message("More"), &[]).unwrap();
    assert_eq!(mail_in(&path).len(), held + 1);
}

fn with_version(path: &Path, version: u16) {
    let mut file = std::fs::read(path).unwrap();
    let mut header = Header::parse(&file[..564]).unwrap();
    header.raw[10..12].copy_from_slice(&version.to_le_bytes());
    let bytes = header.to_bytes().unwrap();
    file[..bytes.len()].copy_from_slice(&bytes);
    std::fs::write(path, &file).unwrap();
}

/// Later Unicode versions read, but aren't added to; those with 4 KiB pages don't even read.
#[test]
fn a_later_version_reads_and_is_not_added_to() {
    let path = one_message("later.pst", &[], &[]);
    with_version(&path, 24);
    assert_eq!(mail_in(&path).len(), 1);
    let e = append_message(&path, &["Inbox"], &message("More"), &[]).unwrap_err();
    assert!(e.to_string().contains("version 24"), "{e}");
    with_version(&path, 36);
    assert!(PstFile::open(&path).is_err());
}

/// A subnode tree in two levels, the leaves of which are still another's: they stay.
#[test]
fn a_shared_subnode_tree_keeps_the_blocks_it_lists() {
    let path = one_message("shared.pst", &[], &[]);
    let mut ed = Editor::open(&path).unwrap();
    let data = ed.write_tree(b"x").unwrap();
    let subs: SubNodes = (1..=400u32).map(|i| ((i << 5) | 1, (data, 0))).collect();
    let top = ed.write_subnodes(&subs).unwrap();
    ed.set_node(0x7E1, data, top, 0);
    ed.commit().unwrap();
    let mut ed = Editor::open(&path).unwrap();
    assert_eq!(ed.pst.raw_block(top).unwrap()[1], 1, "two levels");
    ed.add_ref(top);
    ed.release_node_data(0, top).unwrap();
    assert!(ed.refs.iter().all(|(bid, delta)| *bid == top || *delta >= 0), "{:?}", ed.refs);
}

/// A tree that names itself is let go of once, not until the stack runs out.
#[test]
fn trees_that_name_themselves_end() {
    let path = one_message("cycle.pst", &[], &[]);
    let mut ed = Editor::open(&path).unwrap();
    let data = ed.write_tree(b"x").unwrap();
    let list_id = ed.header.next_block | BID_INTERNAL;
    let mut list = vec![1u8, 2, 1, 0];
    list.extend(1u32.to_le_bytes());
    list.extend(list_id.to_le_bytes());
    assert_eq!(ed.write_block(&list, true).unwrap(), list_id);
    let sub_id = ed.header.next_block | BID_INTERNAL;
    let mut sub = vec![2u8, 0, 1, 0, 0, 0, 0, 0];
    sub.extend(0x41u64.to_le_bytes());
    sub.extend(data.to_le_bytes());
    sub.extend(sub_id.to_le_bytes());
    assert_eq!(ed.write_block(&sub, true).unwrap(), sub_id);
    ed.set_node(0x7E1, list_id, sub_id, 0);
    ed.commit().unwrap();
    let mut ed = Editor::open(&path).unwrap();
    let _ = ed.release_tree(list_id);
    ed.release_node_data(0, sub_id).unwrap();
}

/// A table whose columns overlap isn't written again.
#[test]
fn overlapping_columns_are_refused_on_rewrite() {
    let path = one_message("overlap.pst", &[], &[]);
    let mut ed = Editor::open(&path).unwrap();
    let mut schema = Schema::new(&[(0x3001, 0x0003), (0x3002, 0x0003)]).unwrap();
    let first = schema.columns[0].offset;
    schema.columns[1].offset = first;
    let built = build_table(Format::Unicode, &schema, &[], 0).unwrap();
    let (data, sub) = ed.write_built(built, SubNodes::new()).unwrap();
    ed.set_node(0x7EE, data, sub, 0);
    ed.commit().unwrap();
    let mut ed = Editor::open(&path).unwrap();
    let e = ed.rewrite_table(0x7EE, |_, _, _| Ok(())).unwrap_err();
    assert!(e.to_string().contains("overlap"), "{e}");
}

fn with_page_marked_free(path: &Path, ib: u64) {
    let mut file = std::fs::read(path).unwrap();
    let map = usize::try_from(run_at(0)).unwrap();
    let mut bits = [0u8; 496];
    bits.copy_from_slice(&file[map..map + 496]);
    space::mark(&mut bits, 0, ib, ib + PAGE as u64, false).unwrap();
    file[map..map + 496].copy_from_slice(&bits);
    seal_page(&mut file, map);
    std::fs::write(path, &file).unwrap();
}

/// A map that marks something in use as free isn't believed, and isn't said to be right: the
/// file is added to at its end, its maps left alone, and marked for Outlook to rebuild them.
#[test]
fn a_map_that_is_wrong_is_marked_invalid() {
    let path = one_message("wrong-map.pst", &[], &[]);
    let root = Editor::open(&path).unwrap().header.root.nbt.ib;
    with_page_marked_free(&path, root);
    let map = usize::try_from(run_at(0)).unwrap();
    let before = std::fs::read(&path).unwrap()[map..map + PAGE].to_vec();
    let ed = Editor::open(&path).unwrap();
    assert!(!ed.maps_valid);
    assert!(ed.space.free_ranges().is_empty());
    drop(ed);
    append_message(&path, &["Inbox"], &message("More"), &[]).unwrap();
    let file = std::fs::read(&path).unwrap();
    assert_eq!(file[248], AMAP_INVALID, "still said not to be valid");
    assert_eq!(&file[map..map + PAGE], &before[..], "and left as it was");
    assert_eq!(mail_in(&path).len(), 2);
}

/// A density list that is damaged isn't made to look sound.
#[test]
fn a_damaged_density_list_is_left_alone() {
    let path = one_message("dlist.pst", &[], &[]);
    let mut file = std::fs::read(&path).unwrap();
    let at = usize::try_from(DLIST_AT).unwrap();
    file[at + 8] ^= 0xFF;
    std::fs::write(&path, &file).unwrap();
    let before = file[at..at + PAGE].to_vec();
    append_message(&path, &["Inbox"], &message("More"), &[]).unwrap();
    assert_eq!(&std::fs::read(&path).unwrap()[at..at + PAGE], &before[..]);
}

/// Asked to make a file that is already there, it says so and leaves that file alone.
#[test]
fn making_a_file_over_one_that_is_there_keeps_it() {
    let path = scratch("there.pst");
    std::fs::write(&path, b"somebody's file").unwrap();
    assert!(create_empty(&path, Format::Unicode).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"somebody's file");
}

/// Records of types read only to be written back survive a rewrite of their object, and so
/// does an empty value of a type with more to it.
#[test]
fn records_of_other_types_survive_a_rewrite() {
    let path = one_message("other.pst", &[], &[]);
    let mut ed = Editor::open(&path).unwrap();
    let store = store_of(&path, 1 << 30);
    let top = store.open_folder(store.ipm_subtree().unwrap()).unwrap();
    let inbox = top
        .hierarchy_table()
        .unwrap()
        .rows_matrix()
        .map(|r| r.id())
        .find(|n| store.open_folder(*n).unwrap().contents_table().is_some_and(|t| t.rows_matrix().next().is_some()))
        .unwrap();
    let mut props = super::super::folders::writable(&store.open_folder(inbox).unwrap());
    props.push((0x7FF0, PropertyValue::Other(0x00FB, crate::ltp::Bytes(vec![1, 2, 3]))));
    props.push((0x7FF1, PropertyValue::Other(0x101F, crate::ltp::Bytes::default())));
    ed.rewrite_props(inbox, props).unwrap();
    ed.commit().unwrap();
    crate::write::create_folder(&path, &["Inbox", "Below"], "IPF.Note").unwrap();
    let store = store_of(&path, 1 << 30);
    let folder = store.open_folder(inbox).unwrap();
    assert_eq!(folder.properties().get(0x7FF0), Some(&PropertyValue::Other(0x00FB, crate::ltp::Bytes(vec![1, 2, 3]))));
    assert_eq!(folder.properties().get(0x7FF1), Some(&PropertyValue::Other(0x101F, crate::ltp::Bytes::default())));
}
