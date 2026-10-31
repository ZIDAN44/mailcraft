//! The `.pst` reader's fixes (its fork's PATCHES.md) on files the writer makes and these
//! tests then change byte by byte where the writer can't go: a data tree that says less than it
//! holds or points back at itself. Each file is read on a thread of its own with a time limit.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

use outlook_pst::UnicodePstFile;
use outlook_pst::messaging::attachment::{Attachment, AttachmentData, UnicodeAttachment};
use outlook_pst::messaging::message::UnicodeMessage;
use outlook_pst::messaging::store::{Store as PstStore, UnicodeStore};
use outlook_pst::ndb::node_id::NodeId;
use pst_rw::{UnicodePstAttachment, UnicodePstFile as Writer, UnicodePstMessage};

/// The writer's node for an attachment's bytes (`NID_TYPE_INTERNAL`, index 0x200 + n) and for
/// the attachment itself (`NID_TYPE_ATTACHMENT`, index 0x100 + n).
const DATA_NODE: u64 = (0x200 << 5) | 0x01;
const ATTACHMENT_NODE: u64 = (0x100 << 5) | 0x05;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mailcraft-pst-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn message(subject: &str) -> UnicodePstMessage<'_> {
    UnicodePstMessage {
        subject,
        sender_name: "Riley Moreno",
        sender_email: "riley@example.com",
        recipients: &[],
        body: "See attached.",
        html_body: None,
        message_id: "<outer@example.com>",
        delivery_time: 133_485_408_000_000_000,
    }
}

fn file<'a>(filename: &'a str, data: &'a [u8]) -> UnicodePstAttachment<'a> {
    UnicodePstAttachment { filename, mime_type: "application/octet-stream", content_id: None, data }
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(b[at..at + 2].try_into().unwrap())
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}

fn put32(b: &mut [u8], at: usize, v: u32) {
    b[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

fn put64(b: &mut [u8], at: usize, v: u64) {
    b[at..at + 8].copy_from_slice(&v.to_le_bytes());
}

/// MS-PST's CRC: CRC-32's table, started at 0 and not inverted at the end.
fn crc(data: &[u8]) -> u32 {
    let mut c = 0u32;
    for &b in data {
        c ^= u32::from(b);
        for _ in 0..8 {
            c = if c & 1 == 1 { (c >> 1) ^ 0xEDB8_8320 } else { c >> 1 };
        }
    }
    c
}

/// A Unicode file's blocks by id: where each starts and how many bytes it holds, found from the
/// 16-byte trailers that end them on 64-byte boundaries (a trailer whose CRC matches the bytes
/// before it).
fn blocks(f: &[u8]) -> HashMap<u64, (usize, usize)> {
    let mut out = HashMap::new();
    for end in (64..=f.len()).step_by(64) {
        let size = usize::from(u16_at(f, end - 16));
        if size == 0 || size > 8176 {
            continue;
        }
        let Some(start) = end.checked_sub((size + 16).div_ceil(64) * 64) else { continue };
        if crc(&f[start..start + size]) == u32_at(f, end - 12) {
            out.insert(u64_at(f, end - 8), (start, size));
        }
    }
    out
}

/// Write a changed block's CRC into its trailer.
fn seal(f: &mut [u8], (start, size): (usize, usize)) {
    let end = start + (size + 16).div_ceil(64) * 64;
    let c = crc(&f[start..start + size]);
    put32(f, end - 12, c);
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

/// The first message in the file's folders (the writer's Inbox).
fn first_message(store: &Rc<UnicodeStore>) -> Rc<UnicodeMessage> {
    let any: Rc<dyn PstStore> = store.clone();
    let mut folders = vec![any.open_folder(&any.properties().ipm_sub_tree_entry_id().unwrap()).unwrap()];
    while let Some(folder) = folders.pop() {
        if let Some(row) = folder.contents_table().and_then(|t| t.rows_matrix().next()) {
            let entry = any.properties().make_entry_id(NodeId::from(u32::from(row.id()))).unwrap();
            return UnicodeMessage::read(store.clone(), &entry, None).unwrap();
        }
        for row in folder.hierarchy_table().into_iter().flat_map(|t| t.rows_matrix()) {
            let entry = any.properties().make_entry_id(NodeId::from(u32::from(row.id()))).unwrap();
            folders.push(any.open_folder(&entry).unwrap());
        }
    }
    panic!("no message in the file");
}

/// Read the file on a thread of its own: what `read` makes of it, or a failure when it hasn't
/// finished in 20 s (the reader once hung on an attached message, waiting on its own lock).
fn read_in_time<T: Send + 'static>(path: &Path, read: fn(UnicodePstFile) -> T) -> T {
    let bytes = std::fs::read(path).unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let pst = UnicodePstFile::read_from(Box::new(std::io::Cursor::new(bytes))).unwrap();
        let _ = tx.send(read(pst));
    });
    rx.recv_timeout(Duration::from_secs(20)).expect("the file was read in time")
}

/// A file of 20,000 bytes: three data blocks under an XBLOCK (btype 1, level 1, count, total).
fn with_a_big_file(dir: &Path) -> (PathBuf, Vec<u8>) {
    let path = dir.join("big.pst");
    let data: Vec<u8> = (0..20_000u32).map(|i| (i % 251) as u8).collect();
    drop(Writer::create_with_attachments(&path, &message("Big"), &[file("big.bin", &data)]).unwrap());
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

fn first_attachment(pst: UnicodePstFile) -> Result<Vec<u8>, String> {
    let store = UnicodeStore::read(Rc::new(pst)).unwrap();
    let m = first_message(&store);
    let a = UnicodeAttachment::read(m, NodeId::from(ATTACHMENT_NODE as u32), None).map_err(|e| e.to_string())?;
    match a.data() {
        Some(AttachmentData::Binary(b)) => Ok(b.buffer().to_vec()),
        _ => Err("not a file".into()),
    }
}

/// A data tree is read up to the size it declares, and not at all past a limit the caller sets.
#[test]
fn a_data_tree_reads_no_more_than_it_declares_or_is_allowed() {
    let dir = scratch("big");
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
    let _ = std::fs::remove_dir_all(dir);
}

/// An XBLOCK whose entry names the XBLOCK itself is an error, not a stack overflow (which ends
/// the whole program, with no unwinding to catch).
#[test]
fn a_data_tree_that_points_at_itself_is_an_error() {
    let dir = scratch("cycle");
    let (path, _) = with_a_big_file(&dir);
    let mut f = std::fs::read(&path).unwrap();
    let x = xblock(&f);
    let id = blocks(&f).iter().find(|(_, b)| **b == x).map(|(id, _)| *id).unwrap();
    put64(&mut f, x.0 + 8, id);
    seal(&mut f, x);
    std::fs::write(&path, &f).unwrap();
    let read = read_in_time(&path, first_attachment);
    assert!(read.as_ref().is_err_and(|e| e.contains("cLevel")), "{read:?}");
    let _ = std::fs::remove_dir_all(dir);
}
