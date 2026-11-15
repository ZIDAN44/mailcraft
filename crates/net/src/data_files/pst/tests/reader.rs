//! The `.pst` reader on hostile files: ones the writer makes and these
//! tests then change byte by byte where the writer can't go: an attachment that holds a message,
//! and a data tree that says less than it holds or points back at itself. Each file is read on a
//! thread of its own with a time limit, as an attached message once hung the reader.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

use mailcraft_model::job::SkipReason;
use mailcraft_pst::{
    Attachment, AttachmentData, Message, PstFile, PstStore,
    write::{self, NewAttachment, NewMessage},
};

use super::super::props::text_of;
use super::pipeline::everything;
use crate::data_files::pst_bytes::{blocks, crc, put32, put64, seal, u16_at, u32_at, u64_at};
use crate::data_files::scratch::scratch;
use crate::data_files::test_data::plain_message;

/// The writer's node for an attachment's bytes (`NID_TYPE_INTERNAL`, index 0x200 + n) and for
/// the attachment itself (`NID_TYPE_ATTACHMENT`, index 0x100 + n).
const DATA_NODE: u64 = (0x200 << 5) | 0x01;
const ATTACHMENT_NODE: u64 = (0x100 << 5) | 0x05;

fn message(subject: &str) -> NewMessage<'_> {
    plain_message(subject, "<outer@example.com>")
}

fn file<'a>(filename: &'a str, data: &'a [u8]) -> NewAttachment<'a> {
    NewAttachment { filename, mime_type: "application/octet-stream", content_id: None, data, ..Default::default() }
}

/// The entry for `nid` in a subnode block (an SLBLOCK: btype 2, level 0, then 24-byte entries
/// of node, data block, subnode block): which block, and where the entry starts.
fn sub_node_entry(f: &[u8], blocks: &HashMap<u64, (usize, usize)>, nid: u64) -> ((usize, usize), usize) {
    for &(start, size) in blocks.values() {
        if f[start] != 0x02 || f[start + 1] != 0 {
            continue;
        }
        let count = usize::from(u16_at(f, start + 2));
        for i in 0..count {
            let at = start + 8 + i * 24;
            if at + 24 <= start + size && u64_at(f, at) == nid {
                return ((start, size), at);
            }
        }
    }
    panic!("no subnode entry for {nid:#x}");
}

/// Where a record for `prop` starts in a property context's heap block (a BTH leaf of 8-byte
/// records: id, type, value).
fn record(f: &[u8], (start, size): (usize, usize), prop: u16, prop_type: u16) -> usize {
    let pattern = [prop.to_le_bytes(), prop_type.to_le_bytes()].concat();
    start + f[start..start + size].windows(4).position(|w| w == pattern).unwrap()
}

/// Where a heap item starts in a one-block heap, from its HID (MS-PST 2.3.1).
fn heap_item(f: &[u8], (start, _): (usize, usize), hid: u32) -> usize {
    let page_map = start + usize::from(u16_at(f, start));
    let index = (hid >> 5) as usize & 0x7FF;
    start + usize::from(u16_at(f, page_map + 4 + (index - 1) * 2))
}

/// The attachment whose property context's block is `attachment` made one holding a message,
/// the one in its subnode `node`: its method 1 becomes 5, and its 0x3701 a `PtypObject` on an
/// 8-byte heap item (node, size), written over the MIME type's text. Its block sealed.
fn holds_a_message(f: &mut [u8], attachment: (usize, usize), node: u32) {
    let method = record(f, attachment, 0x3705, 0x0003);
    put32(f, method + 4, 5);
    let mime = u32_at(f, record(f, attachment, 0x370E, 0x001F) + 4);
    let item = heap_item(f, attachment, mime);
    put32(f, item, node);
    put32(f, item + 4, 0);
    let value = record(f, attachment, 0x3701, 0x0102);
    f[value + 2..value + 4].copy_from_slice(&0x000D_u16.to_le_bytes());
    put32(f, value + 4, mime);
    seal(f, attachment);
}

/// Where each entry of the node B-tree's leaf pages (ptype 0x81, cLevel 0) starts: 32 bytes of
/// node, data block, subnode block and parent. The writer may leave older copies of a page
/// behind: their entries are here too.
fn leaf_entries(f: &[u8]) -> impl Iterator<Item = usize> + '_ {
    (0..f.len() - 32).step_by(32).filter(|&o| f.get(o / 512 * 512 + 496) == Some(&0x81) && f[o / 512 * 512 + 491] == 0)
}

/// A data file with one message, "Outer", whose first attachment holds a message and whose
/// second is a zero-byte file. The writer only writes files, so the first one is turned into an
/// attached message after: its method becomes 5, its 0x3701 a `PtypObject` naming its node, and
/// that node, in the attachment's own subnode tree, points at the outer message's own blocks
/// (the attached message is a copy of the message it is attached to).
fn with_an_attached_message(dir: &Path) -> PathBuf {
    attached_in(dir, "Outer")
}

/// [`with_an_attached_message`], its message's subject `subject`.
fn attached_in(dir: &Path, subject: &str) -> PathBuf {
    let path = dir.join("attached.pst");
    let files = [file("letter.bin", b"twelve bytes"), file("empty.txt", b"")];
    write::create_with(&path, &["Inbox"], &message(subject), &files).unwrap();
    let mut f = std::fs::read(&path).unwrap();
    let all = blocks(&f);
    // The outer message's subnode block holds the attachment: its PC, its subnode block.
    let (outer_sub_nodes, at) = sub_node_entry(&f, &all, ATTACHMENT_NODE);
    let outer_sub_nodes_id = all.iter().find(|(_, b)| **b == outer_sub_nodes).map(|(id, _)| *id).unwrap();
    let attachment = all[&u64_at(&f, at + 8)];
    // The outer message's PC: the node B-tree entry whose subnode block is that one.
    let outer =
        leaf_entries(&f).find(|&o| u64_at(&f, o + 16) == outer_sub_nodes_id && (u64_at(&f, o) & 0x1F) == 0x04).map(|o| u64_at(&f, o + 8)).unwrap();
    // The attachment's own subnode block: its bytes' node now names the outer message's blocks.
    let (own, at) = sub_node_entry(&f, &all, DATA_NODE);
    put64(&mut f, at + 8, outer);
    put64(&mut f, at + 16, outer_sub_nodes_id);
    seal(&mut f, own);
    holds_a_message(&mut f, attachment, DATA_NODE as u32);
    std::fs::write(&path, f).unwrap();
    path
}

/// The first message in the file's folders (the writer's Inbox).
fn first_message(store: &Rc<PstStore>) -> Rc<Message> {
    let any: Rc<PstStore> = store.clone();
    let mut folders = vec![any.open_folder(any.ipm_subtree().unwrap()).unwrap()];
    while let Some(folder) = folders.pop() {
        if let Some(row) = folder.contents_table().and_then(|t| t.rows_matrix().next()) {
            let entry = row.id();
            return Message::read(store.clone(), entry).unwrap();
        }
        for row in folder.hierarchy_table().into_iter().flat_map(|t| t.rows_matrix()) {
            let entry = row.id();
            folders.push(any.open_folder(entry).unwrap());
        }
    }
    panic!("no message in the file");
}

fn subject(m: &Message) -> String {
    m.properties().get(0x0037).and_then(text_of).unwrap_or_default()
}

/// Each attachment of `m`, as "file <bytes>", "message <subject>" or the error reading it.
fn attachments(m: &Rc<Message>) -> Vec<String> {
    let rows: Vec<u32> = m.attachment_table().map(|t| t.rows_matrix().map(|r| r.id()).collect()).unwrap_or_default();
    rows.into_iter()
        .map(|id| match Attachment::read(m.clone(), id) {
            Ok(a) => match a.data() {
                Some(AttachmentData::Binary(b)) => format!("file {}", b.buffer().len()),
                Some(AttachmentData::Message(inner)) => format!("message {}", subject(inner)),
                None => "nothing".into(),
            },
            Err(e) => format!("error {e}"),
        })
        .collect()
}

/// Read the file on a thread of its own: what `read` makes of it, or a failure when it hasn't
/// finished in 20 s (the reader once hung on an attached message, waiting on its own lock).
fn read_in_time<T: Send + 'static>(path: &Path, read: impl FnOnce(PstFile) -> T + Send + 'static) -> T {
    let bytes = std::fs::read(path).unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let pst = PstFile::read_from(Box::new(std::io::Cursor::new(bytes))).unwrap();
        let _ = tx.send(read(pst));
    });
    rx.recv_timeout(Duration::from_secs(20)).expect("the file was read in time")
}

/// An attached message reads: looked up in the attachment's own subnode tree, read once the
/// file's lock is let go, and handed back as the file's own type, whose attachments read in
/// turn. A zero-byte file reads as one with no bytes.
#[test]
fn an_attached_message_and_an_empty_file_read() {
    let dir = scratch("pst-attached");
    let path = with_an_attached_message(&dir);
    let (outer, inner) = read_in_time(&path, |pst| {
        let store = PstStore::read(Rc::new(pst)).unwrap();
        let m = first_message(&store);
        let a = Attachment::read(m.clone(), ATTACHMENT_NODE as u32).unwrap();
        let attached = a.embedded_message().expect("the attachment's message, as Message");
        (attachments(&m), (subject(&attached), attachments(&attached)))
    });
    assert_eq!(outer, ["message Outer", "file 0"]);
    // The copy has the same attachments: its own are read with the file's own types.
    assert_eq!(inner, ("Outer".to_string(), vec!["message Outer".to_string(), "file 0".to_string()]));
}

/// What the import makes of a message attached to itself: it finishes in time, the zero-byte
/// file comes in, and the attached message, which holds the message it is attached to (its
/// search key is that message's), is counted and left out rather than read again and again.
#[test]
fn the_import_of_a_message_attached_to_itself_ends() {
    let dir = scratch("pst-attached-import");
    let path = with_an_attached_message(&dir);
    let (done, sources) = import_in_time(&path, &dir);
    assert_eq!((done.messages, done.skipped), (1, 0), "{done:?}");
    assert_eq!(done.skips.into_iter().collect::<Vec<_>>(), [(SkipReason::AttachedMessageUnreadable, 1)]);
    assert_eq!(sources.iter().map(|s| tree(s)).collect::<Vec<_>>(), ["Outer[empty.txt]"]);
}

/// A message of a data file whose subject is `subject`: its property context's block and its
/// subnode block, from the node B-tree's leaf pages (ptype 0x81, cLevel 0) and the subject's
/// UTF-16 in the block.
fn message_blocks(f: &[u8], all: &HashMap<u64, (usize, usize)>, subject: &str) -> (u64, u64) {
    let wide: Vec<u8> = subject.encode_utf16().flat_map(u16::to_le_bytes).collect();
    leaf_entries(f)
        .filter(|&o| (u64_at(f, o) & 0x1F) == 0x04)
        .map(|o| (u64_at(f, o + 8), u64_at(f, o + 16)))
        .find(|(pc, _)| all.get(pc).is_some_and(|&(start, size)| f[start..start + size].windows(wide.len()).any(|w| w == wide)))
        .unwrap()
}

/// The entries of a subnode block (an SLBLOCK) whose node is of `kind`: node, data block,
/// subnode block, and where the entry starts.
fn entries_of(f: &[u8], (start, size): (usize, usize), kind: u64) -> Vec<(u64, u64, u64, usize)> {
    assert_eq!((f[start], f[start + 1]), (0x02, 0), "an SLBLOCK");
    let count = usize::from(u16_at(f, start + 2));
    (0..count)
        .map(|i| start + 8 + i * 24)
        .filter(|&at| at + 24 <= start + size && (u64_at(f, at) & 0x1F) == kind)
        .map(|at| (u64_at(f, at), u64_at(f, at + 8), u64_at(f, at + 16), at))
        .collect()
}

/// The first attachment of the message whose subnode block is `holder` becomes one holding the
/// message whose blocks are `target` (its property context's, its subnode block): its method
/// becomes 5, its 0x3701 a `PtypObject` naming its own first subnode, which now points at the
/// target's blocks.
fn attach(f: &mut [u8], holder: u64, target: (u64, u64)) {
    let all = blocks(f);
    let (_, pc, own, _) = *entries_of(f, all[&holder], 0x05).iter().min_by_key(|e| e.0).unwrap();
    let attachment = all[&pc];
    let own = all[&own];
    let (object, _, _, at) = entries_of(f, own, 0x01)[0];
    put64(f, at + 8, target.0);
    put64(f, at + 16, target.1);
    seal(f, own);
    holds_a_message(f, attachment, object as u32);
}

/// A data file of three messages, each with an id of its own: "Outer" holds "Middle", which
/// holds "Deep", each beside a file of its own (attachments the writer made as files and these
/// turn into attached messages).
fn with_nested_messages(dir: &Path) -> PathBuf {
    let path = dir.join("nested.pst");
    let outer = [file("a.bin", b"one"), file("empty.txt", b"")];
    write::create_with(&path, &["Inbox"], &plain_message("Outer", "<outer@example.com>"), &outer).unwrap();
    let middle = [file("b.bin", b"two"), file("middle.txt", b"in the middle")];
    write::append_message(&path, &["Inbox"], &plain_message("Middle", "<middle@example.com>"), &middle).unwrap();
    let deep = [file("deep.txt", b"at the bottom")];
    write::append_message(&path, &["Inbox"], &plain_message("Deep", "<deep@example.com>"), &deep).unwrap();
    let mut f = std::fs::read(&path).unwrap();
    let all = blocks(&f);
    let outer = message_blocks(&f, &all, "Outer");
    let middle = message_blocks(&f, &all, "Middle");
    let deep = message_blocks(&f, &all, "Deep");
    attach(&mut f, outer.1, middle);
    attach(&mut f, middle.1, deep);
    std::fs::write(&path, f).unwrap();
    path
}

/// Import the file into a cache of its own on a thread with a time limit: what the import did,
/// and each message's source. The limit is there to fail an import that never ends (a message
/// read again and again), so it's long: these imports take a fifth of a second alone, but 12 to
/// 16 s with 6 runs of the net tests at once, and failed the old 20 s in most such runs.
fn import_in_time(path: &Path, dir: &Path) -> (super::super::Imported, Vec<Vec<u8>>) {
    use mailcraft_store::Store;
    let (path, cache) = (path.to_path_buf(), dir.join("cache"));
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let store = Store::open(cache).unwrap();
        let done = super::import(&path, &store, "a1", &mut |_| {}).unwrap();
        let _ = tx.send((done, everything(&store, "a1").2));
    });
    rx.recv_timeout(Duration::from_secs(120)).expect("the file was imported in time")
}

/// MEASURE: a small data file's import, as the app makes one (a cache on disk, its own lanes),
/// twenty times: the slowest and the median, and where the median one's time went. Run in
/// release, alone and beside other work:
/// `cargo test --release -p mailcraft-net --lib measure_a_small_import -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement"]
fn measure_a_small_import() {
    let dir = scratch("measure-small-import");
    let path = with_nested_messages(&dir);
    let mut runs: Vec<(Duration, super::super::ImportStats)> = (0..20)
        .map(|n| {
            let store = mailcraft_store::Store::open(dir.join(format!("cache{n}"))).unwrap();
            let began = std::time::Instant::now();
            let done = super::import(&path, &store, "a1", &mut |_| {}).unwrap();
            (began.elapsed(), done.stats)
        })
        .collect();
    runs.sort_by_key(|(took, _)| *took);
    let (median, s) = &runs[runs.len() / 2];
    eprintln!(
        "MEASURE small import: median {median:?}, slowest {:?}; the median's ms: total {}, plan {}, read {}, build {}, write {}, commit {}, writer idle {}, budget wait {}, index {}; lanes {}",
        runs[runs.len() - 1].0,
        s.total_ms,
        s.plan_ms,
        s.read_ms,
        s.build_ms,
        s.write_ms,
        s.commit_ms,
        s.writer_idle_ms,
        s.budget_wait_ms,
        s.index_ms,
        s.lanes
    );
    let _ = std::fs::remove_dir_all(dir);
}

/// A message's subject and its attachments' names, an attached message's as its own tree:
/// `Outer[Middle[…], empty.txt]`.
fn tree(raw: &[u8]) -> String {
    let p = mailcraft_mime::parse(raw).unwrap();
    let parts: Vec<String> = p
        .attachments
        .iter()
        .map(|a| match a.name.strip_suffix(".eml") {
            Some(_) => tree(&mailcraft_mime::attachment_data(raw, a.index).unwrap()),
            None => a.name.clone(),
        })
        .collect();
    format!("{}[{}]", p.subject, parts.join(", "))
}

/// What the import makes of messages attached inside messages: each comes in as an attached
/// .eml with its own files, the one attached inside it too, and the zero-byte file comes in.
/// An attached message's own files were left out before.
#[test]
fn the_import_keeps_attached_messages_with_their_own_files() {
    let dir = scratch("pst-nested-import");
    let path = with_nested_messages(&dir);
    let (done, sources) = import_in_time(&path, &dir);
    assert_eq!((done.messages, done.skipped), (3, 0), "{done:?}");
    assert!(done.skips.is_empty(), "{:?}", done.skips);
    let mut trees: Vec<String> = sources.iter().map(|s| tree(s)).collect();
    trees.sort();
    assert_eq!(trees, ["Deep[deep.txt]", "Middle[Deep[deep.txt], middle.txt]", "Outer[Middle[Deep[deep.txt], middle.txt], empty.txt]"]);
}

/// A file of 20,000 bytes: three data blocks under an XBLOCK (btype 1, level 1, count, total).
fn with_a_big_file(dir: &Path) -> (PathBuf, Vec<u8>) {
    let path = dir.join("big.pst");
    let data: Vec<u8> = (0..20_000u32).map(|i| (i % 251) as u8).collect();
    write::create_with(&path, &["Inbox"], &message("Big"), &[file("big.bin", &data)]).unwrap();
    (path, data)
}

/// The XBLOCK that holds the attachment's bytes.
fn xblock(f: &[u8]) -> (usize, usize) {
    let all = blocks(f);
    let (_, at) = sub_node_entry(f, &all, DATA_NODE);
    let x = all[&u64_at(f, at + 8)];
    assert_eq!((f[x.0], f[x.0 + 1]), (0x01, 0x01), "an XBLOCK");
    x
}

fn first_attachment(pst: PstFile) -> Result<Vec<u8>, String> {
    let store = PstStore::read(Rc::new(pst)).unwrap();
    let m = first_message(&store);
    let a = Attachment::read(m, ATTACHMENT_NODE as u32).map_err(|e| e.to_string())?;
    match a.data() {
        Some(AttachmentData::Binary(b)) => Ok(b.buffer().to_vec()),
        _ => Err("not a file".into()),
    }
}

/// A data tree is read up to the size it declares, and not at all past a limit the caller sets.
#[test]
fn a_data_tree_reads_no_more_than_it_declares_or_is_allowed() {
    let dir = scratch("pst-big");
    let (path, data) = with_a_big_file(&dir);
    assert_eq!(read_in_time(&path, first_attachment), Ok(data.clone()));
    // A limit under the value's size: an error before it is read.
    let limited = read_in_time(&path, |pst| {
        pst.set_max_value_size(10_000);
        first_attachment(pst)
    });
    assert!(limited.is_err_and(|e| e.contains("over the limit")));
    // The XBLOCK says it holds 100 bytes: that is what is read.
    let mut f = std::fs::read(&path).unwrap();
    let x = xblock(&f);
    put32(&mut f, x.0 + 4, 100);
    seal(&mut f, x);
    std::fs::write(&path, &f).unwrap();
    assert_eq!(read_in_time(&path, first_attachment), Ok(data[..100].to_vec()));
}

/// An XBLOCK whose entry names the XBLOCK itself is an error, not a stack overflow (which ends
/// the whole program, with no unwinding to catch).
#[test]
fn a_data_tree_that_points_at_itself_is_an_error() {
    let dir = scratch("pst-cycle");
    let (path, _) = with_a_big_file(&dir);
    let mut f = std::fs::read(&path).unwrap();
    let x = xblock(&f);
    let id = blocks(&f).iter().find(|(_, b)| **b == x).map(|(id, _)| *id).unwrap();
    put64(&mut f, x.0 + 8, id);
    seal(&mut f, x);
    std::fs::write(&path, &f).unwrap();
    let read = read_in_time(&path, first_attachment);
    assert!(read.as_ref().is_err_and(|e| e.contains("a list where a block should be")), "{read:?}");
}

/// An XBLOCK whose entries all name the same data block reads no more than it declares, and
/// stops naming blocks there: blocks named over and over (a thousand times a thousand under an
/// XXBLOCK) can't make the reader hold gigabytes for one value.
#[test]
fn a_data_tree_that_names_one_block_over_and_over_reads_no_more_than_it_declares() {
    let dir = scratch("pst-repeat");
    let (path, data) = with_a_big_file(&dir);
    let mut f = std::fs::read(&path).unwrap();
    let x = xblock(&f);
    let first = u64_at(&f, x.0 + 8);
    put64(&mut f, x.0 + 16, first);
    put64(&mut f, x.0 + 24, first);
    seal(&mut f, x);
    std::fs::write(&path, &f).unwrap();
    let read = read_in_time(&path, first_attachment).unwrap();
    assert_eq!(read.len(), data.len());
    assert_eq!(read[..8176], data[..8176]);
    assert_eq!(read[8176..2 * 8176], data[..8176]);
    // The same through the file's own block reader, which read to the end.
    let id = blocks(&f).iter().find(|(_, b)| **b == x).map(|(id, _)| *id).unwrap();
    let block = read_in_time(&path, move |pst| pst.read_block(id).map(|b| b.len()).map_err(|e| e.to_string()));
    assert_eq!(block, Ok(data.len()));
    // It says it holds one block's worth: one block is read.
    put32(&mut f, x.0 + 4, 8176);
    seal(&mut f, x);
    std::fs::write(&path, &f).unwrap();
    assert_eq!(read_in_time(&path, first_attachment), Ok(data[..8176].to_vec()));
}

/// The big file, with the binary property `prop` of node `nid` (a store, a folder, the
/// name-to-id map) kept in the attachment's 20,000-byte data tree: the node's B-tree entry gets
/// the attachment's subnode block, and the property's record names the data node.
fn with_a_big_value(dir: &Path, nid: u64, prop: u16) -> PathBuf {
    let (path, _) = with_a_big_file(dir);
    let mut f = std::fs::read(&path).unwrap();
    let all = blocks(&f);
    let (own, _) = sub_node_entry(&f, &all, DATA_NODE);
    let own_id = all.iter().find(|(_, b)| **b == own).map(|(id, _)| *id).unwrap();
    // The node's entry in a leaf page of the node B-tree (ptype 0x81, cLevel 0): nid, data,
    // subnodes. The writer may leave older copies of a page behind: each gets the change.
    let entries: Vec<usize> = leaf_entries(&f).filter(|&o| u64_at(&f, o) == nid).collect();
    let pc = all[&u64_at(&f, entries[0] + 8)];
    for entry in entries {
        put64(&mut f, entry + 16, own_id);
        let page = entry / 512 * 512;
        let c = crc(&f[page..page + 496]);
        put32(&mut f, page + 500, c);
    }
    let at = record(&f, pc, prop, 0x0102);
    put32(&mut f, at + 4, DATA_NODE as u32);
    seal(&mut f, pc);
    std::fs::write(&path, &f).unwrap();
    path
}

fn binary_len(value: Option<&mailcraft_pst::PropertyValue>) -> Option<usize> {
    match value {
        Some(mailcraft_pst::PropertyValue::Binary(b)) => Some(b.buffer().len()),
        _ => None,
    }
}

/// The file's limit holds for the values every import reads first, the store's and the
/// name-to-id map's, and not only for messages and attachments: those once read whatever size
/// their data tree declared (up to 4 GiB).
#[test]
fn a_store_or_name_map_value_is_held_to_the_file_limit() {
    let dir = scratch("pst-store-limit");
    // The store's record key (0x0FF9).
    let path = with_a_big_value(&dir, 0x21, 0x0FF9);
    let store = |limit: Option<u64>| {
        move |pst: PstFile| {
            if let Some(limit) = limit {
                pst.set_max_value_size(limit);
            }
            PstStore::read(Rc::new(pst)).map(|s| binary_len(s.properties().get(0x0FF9))).map_err(|e| e.to_string())
        }
    };
    assert_eq!(read_in_time(&path, store(None)), Ok(Some(20_000)));
    let limited = read_in_time(&path, store(Some(10_000)));
    assert!(limited.as_ref().is_err_and(|e| e.contains("over the limit")), "{limited:?}");
    // The name-to-id map's GUID stream (0x0002).
    std::fs::remove_file(&path).unwrap();
    let path = with_a_big_value(&dir, 0x61, 0x0002);
    let map = |limit: Option<u64>| {
        move |pst: PstFile| {
            if let Some(limit) = limit {
                pst.set_max_value_size(limit);
            }
            let store = PstStore::read(Rc::new(pst)).unwrap();
            store.named_property_map().map(|m| binary_len(m.properties().get(0x0002))).map_err(|e| e.to_string())
        }
    };
    assert_eq!(read_in_time(&path, map(None)), Ok(Some(20_000)));
    let limited = read_in_time(&path, map(Some(10_000)));
    assert!(limited.as_ref().is_err_and(|e| e.contains("over the limit")), "{limited:?}");
}

/// A node B-tree page whose entry names the page itself is an error, not a stack overflow:
/// every read goes through that tree.
#[test]
fn a_btree_page_that_points_at_itself_is_an_error() {
    let dir = scratch("pst-page-cycle");
    let (path, _) = with_a_big_file(&dir);
    let mut f = std::fs::read(&path).unwrap();
    // The header's root (offset 180): the node B-tree's BREF (bid, ib) at 216.
    let (bid, ib) = (u64_at(&f, 216), u64_at(&f, 224) as usize);
    assert_eq!((f[ib + 496], u64_at(&f, ib + 504)), (0x81, bid), "the node B-tree's root page");
    // One BTENTRY (key 0, then this page's BREF); cEnt 1, cEntMax 20, cbEnt 24, cLevel 1.
    f[ib..ib + 488].fill(0);
    put64(&mut f, ib + 8, bid);
    put64(&mut f, ib + 16, ib as u64);
    f[ib + 488..ib + 496].copy_from_slice(&[1, 20, 24, 1, 0, 0, 0, 0]);
    let c = crc(&f[ib..ib + 496]);
    put32(&mut f, ib + 500, c);
    std::fs::write(&path, &f).unwrap();
    let read = read_in_time(&path, |pst| PstStore::read(Rc::new(pst)).map(|_| ()).map_err(|e| e.to_string()));
    assert!(read.as_ref().is_err_and(|e| e.contains("level by level")), "{read:?}");
}

/// An attachment whose own subnode tree can't be read finds its object in the message's
/// subnode tree, the one place it was looked for before.
#[test]
fn an_attached_message_in_the_message_tree_reads_when_its_own_tree_is_damaged() {
    let dir = scratch("pst-own-damaged");
    let path = with_an_attached_message(&dir);
    let mut f = std::fs::read(&path).unwrap();
    let all = blocks(&f);
    // The object's entry, moved to the message's tree in place of the second attachment's.
    let (own, at) = sub_node_entry(&f, &all, DATA_NODE);
    let (outer, sub) = (u64_at(&f, at + 8), u64_at(&f, at + 16));
    let (message, second) = sub_node_entry(&f, &all, ATTACHMENT_NODE + (1 << 5));
    put64(&mut f, second, DATA_NODE);
    put64(&mut f, second + 8, outer);
    put64(&mut f, second + 16, sub);
    seal(&mut f, message);
    // The attachment's own tree: level 5, which no subnode tree has.
    f[own.0 + 1] = 5;
    seal(&mut f, own);
    std::fs::write(&path, &f).unwrap();
    let read = read_in_time(&path, |pst| {
        let store = PstStore::read(Rc::new(pst)).unwrap();
        let m = first_message(&store);
        attachments(&m).into_iter().next()
    });
    assert_eq!(read.as_deref(), Some("message Outer"));
}

/// An attachment the reader fails on, and one that only links to a file elsewhere, are counted
/// by reason; the message comes in without them, and isn't counted as skipped.
#[test]
fn an_unreadable_attachment_and_a_link_are_counted() {
    use mailcraft_model::job::SkipReason;
    use mailcraft_store::Store;
    let dir = scratch("pst-unread-and-link");
    let path = dir.join("left-out.pst");
    let files = [file("letter.bin", b"twelve bytes"), file("share.txt", b"a path")];
    write::create_with(&path, &["Inbox"], &message("Outer"), &files).unwrap();
    let mut f = std::fs::read(&path).unwrap();
    let all = blocks(&f);
    // The first one's method (0x3705) becomes 0x3706, so the reader finds none; its row in the
    // attachment table still has it.
    let (_, at) = sub_node_entry(&f, &all, ATTACHMENT_NODE);
    let first = all[&u64_at(&f, at + 8)];
    let method = record(&f, first, 0x3705, 0x0003);
    f[method..method + 2].copy_from_slice(&0x3706_u16.to_le_bytes());
    seal(&mut f, first);
    // The second one's method: by reference (2).
    let (_, at) = sub_node_entry(&f, &all, ATTACHMENT_NODE + (1 << 5));
    let second = all[&u64_at(&f, at + 8)];
    let method = record(&f, second, 0x3705, 0x0003);
    put32(&mut f, method + 4, 2);
    seal(&mut f, second);
    std::fs::write(&path, &f).unwrap();

    let store = Store::open(dir.join("cache")).unwrap();
    let done = super::import(&path, &store, "a1", &mut |_| {}).unwrap();
    assert_eq!((done.messages, done.skipped), (1, 0), "{done:?}");
    assert_eq!(done.skips.into_iter().collect::<Vec<_>>(), [(SkipReason::AttachmentUnreadable, 1), (SkipReason::AttachmentLink, 1)]);
    let st = done.folders.iter().find_map(|f| store.load_folder("a1", &f.path).ok().filter(|s| !s.messages.is_empty())).unwrap();
    assert!(!st.messages[0].has_attachments);
    let p = mailcraft_mime::parse(&store.read_body(&st.messages[0].key).unwrap()).unwrap();
    assert!(p.attachments.is_empty(), "{:?}", p.attachments.iter().map(|a| &a.name).collect::<Vec<_>>());
}

/// Mark the attachment `nid` hidden (PidTagAttachmentHidden): the record of its file's type
/// (0x370E), the last of them, becomes a `PtypBoolean` 0x7FFE that is true, so they stay in order.
fn hide(f: &mut [u8], nid: u64) {
    let all = blocks(f);
    let (_, at) = sub_node_entry(f, &all, nid);
    let attachment = all[&u64_at(f, at + 8)];
    let position = record(f, attachment, 0x370E, 0x001F);
    f[position..position + 4].copy_from_slice(&[0xFE, 0x7F, 0x0B, 0x00]);
    put32(f, position + 4, 1);
    seal(f, attachment);
}

/// Give the message whose subject is `subject` the class that subject names: its class's record
/// (0x001A) points at the subject's text.
fn class_from_subject(f: &mut [u8], subject: &str) {
    let all = blocks(f);
    let pc = all[&message_blocks(f, &all, subject).0];
    let subject = u32_at(f, record(f, pc, 0x0037, 0x001F) + 4);
    let class = record(f, pc, 0x001A, 0x001F);
    put32(f, class + 4, subject);
    seal(f, pc);
}

/// The import reads each item's class and leaves hidden files out by it: a message's hidden
/// attached message comes in; a meeting request's (a recurring series' changed occurrence)
/// doesn't, and its other files still do.
#[test]
fn hidden_files_are_left_out_by_the_items_class() {
    use mailcraft_store::Store;
    let names = |path: &Path, cache: &Path| {
        let store = Store::open(cache).unwrap();
        let done = super::import(path, &store, "a1", &mut |_| {}).unwrap();
        assert_eq!((done.messages, done.skipped), (2, 0), "{done:?}");
        let sources = everything(&store, "a1").2;
        sources.iter().flat_map(|raw| mailcraft_mime::parse(raw).unwrap().attachments).map(|a| a.name).collect::<Vec<_>>()
    };
    let dir = scratch("pst-hidden-by-class");
    for (subject, kept) in [("IPM.Note", true), ("IPM.Schedule.Meeting.Request", false)] {
        let at = dir.join(subject);
        std::fs::create_dir_all(&at).unwrap();
        let path = at.join("hidden.pst");
        let files = [file("letter.bin", b"twelve bytes"), file("empty.txt", b"")];
        write::create_with(&path, &["Inbox"], &plain_message(subject, "<outer@example.com>"), &files).unwrap();
        write::append_message(&path, &["Inbox"], &plain_message("Other", "<other@example.com>"), &[]).unwrap();
        let mut f = std::fs::read(&path).unwrap();
        let all = blocks(&f);
        // The first file becomes the other message, which isn't one holding the message.
        let (holder, other) = (message_blocks(&f, &all, subject).1, message_blocks(&f, &all, "Other"));
        attach(&mut f, holder, other);
        hide(&mut f, ATTACHMENT_NODE);
        class_from_subject(&mut f, subject);
        std::fs::write(&path, &f).unwrap();
        let names = names(&path, &at.join("cache"));
        assert_eq!(names.iter().any(|n| n.ends_with(".eml")), kept, "{subject}: {names:?}");
        assert!(names.iter().any(|n| n == "empty.txt"), "{subject}: {names:?}");
    }
}
