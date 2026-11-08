//! A new, empty data file, made as MS-PST 2.7 lays out the least a file must hold.

use std::fs::{self, OpenOptions};
use std::hash::{BuildHasher, Hasher};
use std::io::{self, Write};
use std::path::Path;

use super::btree::{finish_page, page_bytes};
use super::edit::Editor;
use super::folders::NID_ROOT_FOLDER;
use super::heap::build_props;
use super::space::{self, AMAP_BYTES, FIRST_AMAP};
use super::table::{Cell, NewRow, Schema, build_table};
use super::text::Text;
use crate::crypt::Crypt;
use crate::ltp::{Bytes, PropertyType as P, PropertyValue};
use crate::messaging::{BUCKET_COUNT, NID_MESSAGE_STORE, NID_NAME_TO_ID_MAP, STREAM_ENTRY, STREAM_GUID, STREAM_STRING};
use crate::ndb::Bref;
use crate::ndb::btree::{block_entry_size, node_entry_size};
use crate::ndb::{Format, Header, PAGE, PT_BBT, PT_NBT, Root, SubNodes};

const NID_IPM_SUBTREE: u32 = 0x8022;
const NID_SEARCH_ROOT: u32 = 0x8042;
const NID_DELETED_ITEMS: u32 = 0x8062;
const NID_SPAM_FOLDER: u32 = 0x2223;
const NID_SEARCH_QUEUE: u32 = 0x1E1;
const NID_SEARCH_ACTIVITY: u32 = 0x201;

/// The store says these of its special folders are valid (`PidTagValidFolderMask`): the
/// IPM subtree, Deleted Items and the search root's.
const VALID_FOLDER_MASK: i32 = 0xC9;

const AMAP_VALID: u8 = 2;
const PT_AMAP: u8 = 0x84;
const PT_PMAP: u8 = 0x83;
const PT_DLIST: u8 = 0x86;
const DLIST_AT: u64 = 0x4200;

/// Columns of a hierarchy table (MS-PST 2.4.4.4.1).
fn hierarchy_columns(st: u16) -> Vec<(u16, u16)> {
    vec![
        (0x0E30, P::BINARY),
        (0x0E33, P::INT64),
        (0x0E34, P::BINARY),
        (0x0E38, P::INT32),
        (0x3001, st),
        (0x3602, P::INT32),
        (0x3603, P::INT32),
        (0x360A, P::BOOLEAN),
        (0x3613, st),
        (0x6635, P::INT32),
        (0x6636, P::INT32),
    ]
}

/// Columns of a contents table (MS-PST 2.4.4.5.1).
fn contents_columns(st: u16) -> Vec<(u16, u16)> {
    vec![
        (0x0017, P::INT32),
        (0x001A, st),
        (0x0036, P::INT32),
        (0x0037, st),
        (0x0039, P::TIME),
        (0x0042, st),
        (0x0057, P::BOOLEAN),
        (0x0058, P::BOOLEAN),
        (0x0070, st),
        (0x0071, P::BINARY),
        (0x0E03, st),
        (0x0E04, st),
        (0x0E06, P::TIME),
        (0x0E07, P::INT32),
        (0x0E08, P::INT32),
        (0x0E17, P::INT32),
        (0x0E30, P::BINARY),
        (0x0E33, P::INT64),
        (0x0E34, P::BINARY),
        (0x0E38, P::INT32),
        (0x0E3C, P::BINARY),
        (0x0E3D, P::BINARY),
        (0x1097, P::INT32),
        (0x3008, P::TIME),
        (0x3013, P::BINARY),
        (0x65C6, P::INT32),
    ]
}

/// Columns of a table of hidden items (MS-PST 2.4.4.6.1).
fn associated_columns(st: u16) -> Vec<(u16, u16)> {
    vec![
        (0x001A, st),
        (0x0E07, P::INT32),
        (0x0E17, P::INT32),
        (0x3001, st),
        (0x6800, st),
        (0x6803, P::BOOLEAN),
        (0x6805, P::MULTI | P::INT32),
        (0x682F, st),
        (0x7003, P::INT32),
        (0x7004, P::BINARY),
        (0x7005, P::BINARY),
        (0x7006, st),
        (0x7007, P::INT32),
    ]
}

/// Columns of a search folder's contents table (MS-PST 2.4.8.6.2.1).
fn search_columns(st: u16) -> Vec<(u16, u16)> {
    vec![
        (0x0017, P::INT32),
        (0x001A, st),
        (0x0036, P::INT32),
        (0x0037, st),
        (0x0042, st),
        (0x0057, P::BOOLEAN),
        (0x0058, P::BOOLEAN),
        (0x0E03, st),
        (0x0E04, st),
        (0x0E05, st),
        (0x0E06, P::TIME),
        (0x0E07, P::INT32),
        (0x0E08, P::INT32),
        (0x0E17, P::INT32),
        (0x0E2A, P::BOOLEAN),
        (0x3008, P::TIME),
        (0x67F1, P::INT32),
    ]
}

/// The receive folder table: which folder mail of a class comes to.
fn receive_columns(st: u16) -> Vec<(u16, u16)> {
    vec![(0x001A, st), (0x6605, P::INT32)]
}

/// The outgoing queue.
fn outgoing_columns() -> Vec<(u16, u16)> {
    vec![(0x0039, P::TIME), (0x0E10, P::INT32), (0x0E14, P::INT32)]
}

/// The tables Outlook keeps for its change tracking, empty in a new file: their node and columns.
fn tracking_tables(st: u16) -> [(u32, Vec<(u16, u16)>); 3] {
    [
        (0x6B6, vec![(0x0E33, P::INT64), (0x0E37, P::BINARY), (0x0E38, P::INT32)]),
        (
            0x6D7,
            vec![
                (0x001A, st),
                (0x0E30, P::BINARY),
                (0x0E31, P::BINARY),
                (0x0E33, P::INT64),
                (0x0E34, P::BINARY),
                (0x0E38, P::INT32),
                (0x0E3E, P::BINARY),
            ],
        ),
        (0x6F8, vec![(0x0E33, P::INT64), (0x3007, P::TIME)]),
    ]
}

fn recipient_columns(st: u16) -> Vec<(u16, u16)> {
    vec![
        (0x0C15, P::INT32),
        (0x0E0F, P::BOOLEAN),
        (0x0FF9, P::BINARY),
        (0x0FFE, P::INT32),
        (0x0FFF, P::BINARY),
        (0x3001, st),
        (0x3002, st),
        (0x3003, st),
        (0x300B, P::BINARY),
        (0x3900, P::INT32),
        (0x39FF, st),
        (0x3A40, P::BOOLEAN),
    ]
}

fn attachment_columns(st: u16) -> Vec<(u16, u16)> {
    vec![(0x0E20, P::INT32), (0x3704, st), (0x3705, P::INT32), (0x370B, P::INT32)]
}

/// A header for an empty file: both B-trees one empty page, the first run's map written.
fn new_header(format: Format) -> Vec<u8> {
    let mut b = vec![0u8; if format.wide() { 564 } else { 512 }];
    let put = |b: &mut Vec<u8>, at: usize, v: &[u8]| {
        if let Some(s) = b.get_mut(at..at + v.len()) {
            s.copy_from_slice(v);
        }
    };
    put(&mut b, 0, b"!BDN");
    put(&mut b, 8, b"SM");
    put(&mut b, 10, &(if format.wide() { 23u16 } else { 15u16 }).to_le_bytes());
    put(&mut b, 12, &19u16.to_le_bytes());
    b[14] = 1;
    b[15] = 1;
    let (fm, fp, sentinel) = if format.wide() { (256, 384, 512) } else { (204, 332, 460) };
    b[fm..fm + 128].fill(0xFF);
    b[fp..fp + 128].fill(0xFF);
    b[sentinel] = 0x80;
    b
}

/// 16 random bytes, for the new store's key: the system's randomness, by way of what seeds the
/// standard hash maps.
fn random_key() -> [u8; 16] {
    let state = std::collections::hash_map::RandomState::new();
    let mut key = [0u8; 16];
    for (i, part) in key.as_chunks_mut::<8>().0.iter_mut().enumerate() {
        let mut h = state.build_hasher();
        h.write_usize(i);
        part.copy_from_slice(&h.finish().to_le_bytes());
    }
    key
}

/// The bare file: a header, a density list, the first map and two empty B-trees.
fn write_bare(path: &Path, format: Format) -> io::Result<()> {
    let mut file = OpenOptions::new().read(true).write(true).create_new(true).open(path)?;
    let run = 0u64;
    let pages = space::map_pages(run).len() as u64;
    let nbt_at = FIRST_AMAP + pages * PAGE as u64;
    let bbt_at = nbt_at + PAGE as u64;
    let eof = FIRST_AMAP + AMAP_BYTES;
    let mut bits = space::new_bits(run);
    let base = FIRST_AMAP;
    for at in [nbt_at, bbt_at] {
        space::mark(&mut bits, run, at, at + PAGE as u64, true)?;
    }
    let free = AMAP_BYTES - bits.iter().map(|b| u64::from(b.count_ones())).sum::<u64>() * 64;
    let raw = new_header(format);
    let nids = {
        // Where each type's numbers start (MS-PST 2.2.2.6); the normal folders below use the first.
        let mut n = [0x400u32; 32];
        n[2] = 0x404;
        n[3] = 0x4000;
        n[4] = 0x10000;
        n[6] = 0x4000;
        n[7] = 0x4000;
        n[8] = 0x8000;
        n[13..16].fill(0x404);
        n
    };
    let header = Header {
        format,
        crypt: Crypt::None,
        root: Root {
            eof,
            amap_last: base,
            amap_free: free,
            pmap_free: 0,
            nbt: Bref { bid: 2, ib: nbt_at },
            bbt: Bref { bid: 1, ib: bbt_at },
            amap_valid: AMAP_VALID,
        },
        next_page: 3,
        next_block: 4,
        unique: 1,
        nids,
        raw,
    };
    let bytes = header.to_bytes()?;
    let mut image = vec![0u8; usize::try_from(eof).map_err(|_| crate::util::short())?];
    image.get_mut(..bytes.len()).ok_or_else(crate::util::short)?.copy_from_slice(&bytes);
    let mut put_page = |at: u64, page: &[u8]| -> io::Result<()> {
        let at = usize::try_from(at).map_err(|_| crate::util::short())?;
        image.get_mut(at..at + PAGE).ok_or_else(crate::util::short)?.copy_from_slice(page);
        Ok(())
    };
    // The density list (MS-PST 2.2.2.7.4), saying nothing yet.
    let mut dlist = vec![0u8; PAGE];
    finish_page(&mut dlist, format, PT_DLIST, DLIST_AT, header.next_page, true)?;
    put_page(DLIST_AT, &dlist)?;
    let bits_at = if format.wide() { 0 } else { 4 };
    let mut amap = vec![0u8; PAGE];
    amap.get_mut(bits_at..bits_at + 496).ok_or_else(crate::util::short)?.copy_from_slice(&bits);
    finish_page(&mut amap, format, PT_AMAP, base, base, false)?;
    put_page(base, &amap)?;
    let mut pmap = vec![0u8; PAGE];
    pmap.get_mut(bits_at..bits_at + 496).ok_or_else(crate::util::short)?.fill(0xFF);
    finish_page(&mut pmap, format, PT_PMAP, base + PAGE as u64, base + PAGE as u64, false)?;
    put_page(base + PAGE as u64, &pmap)?;
    put_page(nbt_at, &page_bytes(format, PT_NBT, nbt_at, 2, node_entry_size(format), 0, &[])?)?;
    put_page(bbt_at, &page_bytes(format, PT_BBT, bbt_at, 1, block_entry_size(format), 0, &[])?)?;
    file.write_all(&image)?;
    file.sync_all()
}

impl Editor {
    fn empty_table(&mut self, nid: u32, schema: &Schema, rows: &[NewRow], parent: u32) -> io::Result<()> {
        let built = build_table(self.format(), schema, rows, 0)?;
        let (data, sub) = self.write_built(built, SubNodes::new())?;
        self.set_node(nid, data, sub, parent);
        Ok(())
    }

    fn props_node(&mut self, nid: u32, props: Vec<(u16, PropertyValue)>, parent: u32) -> io::Result<()> {
        let built = build_props(self.format(), props, 0x80)?;
        let data = self.write_leaves(&built.blocks)?;
        self.set_node(nid, data, 0, parent);
        Ok(())
    }

    /// A folder's row in its parent's hierarchy table.
    fn hierarchy_row(&self, schema: &Schema, nid: u32, name: &str, subs: bool, class: &str, text: Text) -> NewRow {
        let st = text.string_type();
        let cells = schema
            .columns
            .iter()
            .map(|c| match c.prop_id() {
                #[allow(clippy::cast_possible_wrap)]
                0x67F2 => Some(Cell::Fixed((nid as i32).to_le_bytes().to_vec())),
                0x67F3 => Some(Cell::Fixed(1i32.to_le_bytes().to_vec())),
                0x3001 => text.cell(st, name).map(Cell::Bytes),
                0x3602 | 0x3603 => Some(Cell::Fixed(0i32.to_le_bytes().to_vec())),
                0x360A => Some(Cell::Fixed(vec![u8::from(subs)])),
                0x3613 => text.cell(st, class).map(Cell::Bytes),
                _ => None,
            })
            .collect();
        NewRow { id: nid, cells }
    }

    /// Fills a bare file with the nodes every file must have (MS-PST 2.7.1).
    fn fill_new_file(&mut self) -> io::Result<()> {
        let text = Text::plain(self.format());
        let st = text.string_type();
        let key = random_key();
        let entry = |nid: u32| {
            let mut e = vec![0u8; 4];
            e.extend_from_slice(&key);
            e.extend(nid.to_le_bytes());
            PropertyValue::Binary(Bytes(e))
        };
        self.props_node(
            NID_MESSAGE_STORE,
            vec![
                (0x0E38, PropertyValue::Integer32(0)),
                (0x0FF9, PropertyValue::Binary(Bytes(key.to_vec()))),
                (0x3001, text.value("Outlook Data File")),
                (0x35DF, PropertyValue::Integer32(VALID_FOLDER_MASK)),
                (0x35E0, entry(NID_IPM_SUBTREE)),
                (0x35E3, entry(NID_DELETED_ITEMS)),
                (0x35E7, entry(NID_SEARCH_ROOT)),
                (0x67FF, PropertyValue::Integer32(0)),
            ],
            0,
        )?;
        let empty = || PropertyValue::Binary(Bytes::default());
        self.props_node(
            NID_NAME_TO_ID_MAP,
            vec![(BUCKET_COUNT, PropertyValue::Integer32(251)), (STREAM_GUID, empty()), (STREAM_ENTRY, empty()), (STREAM_STRING, empty())],
            0,
        )?;
        let hierarchy = Schema::new(&hierarchy_columns(st))?;
        let contents = Schema::new(&contents_columns(st))?;
        let associated = Schema::new(&associated_columns(st))?;
        for (nid, schema) in [(0x60D, &hierarchy), (0x60E, &contents), (0x60F, &associated)] {
            self.empty_table(nid, schema, &[], 0)?;
        }
        self.empty_table(0x610, &Schema::new(&search_columns(st))?, &[], 0)?;
        self.empty_table(0x692, &Schema::new(&recipient_columns(st))?, &[], 0)?;
        self.empty_table(0x671, &Schema::new(&attachment_columns(st))?, &[], 0)?;
        let receive = Schema::new(&receive_columns(st))?;
        let default_row = NewRow {
            id: 1,
            cells: receive
                .columns
                .iter()
                .map(|c| match c.prop_id() {
                    0x001A => Some(Cell::Bytes(Vec::new())),
                    #[allow(clippy::cast_possible_wrap)]
                    0x6605 => Some(Cell::Fixed((NID_ROOT_FOLDER as i32).to_le_bytes().to_vec())),
                    0x67F2 => Some(Cell::Fixed(1i32.to_le_bytes().to_vec())),
                    0x67F3 => Some(Cell::Fixed(2i32.to_le_bytes().to_vec())),
                    _ => None,
                })
                .collect(),
        };
        self.empty_table(0x62B, &receive, &[default_row], 0)?;
        self.empty_table(0x64C, &Schema::new(&outgoing_columns())?, &[], 0)?;
        for (nid, columns) in tracking_tables(st) {
            self.empty_table(nid, &Schema::new(&columns)?, &[], 0)?;
        }
        // Nodes that hold nothing yet.
        for nid in [NID_SEARCH_QUEUE, 0x261, 0xE41, 0xEC1, 0xF21] {
            self.set_node(nid, 0, 0, 0);
        }
        let activity = self.write_tree(&NID_SPAM_FOLDER.to_le_bytes())?;
        self.set_node(NID_SEARCH_ACTIVITY, activity, 0, 0);

        let folder = |name: &str, subs: bool| {
            let mut props =
                vec![(0x3602u16, PropertyValue::Integer32(0)), (0x3603, PropertyValue::Integer32(0)), (0x360A, PropertyValue::Boolean(subs))];
            if !name.is_empty() {
                props.push((0x3001, text.value(name)));
            }
            props
        };
        // Root: the IPM subtree, the search root and the spam search folder.
        self.props_node(NID_ROOT_FOLDER, folder("", true), NID_ROOT_FOLDER)?;
        let rows = [
            self.hierarchy_row(&hierarchy, NID_IPM_SUBTREE, "Top of Personal Folders", true, "", text),
            self.hierarchy_row(&hierarchy, NID_SEARCH_ROOT, "Search Root", false, "", text),
            self.hierarchy_row(&hierarchy, NID_SPAM_FOLDER, "SPAM Search Folder 2", false, "", text),
        ];
        self.empty_table(NID_ROOT_FOLDER | 0x0F, &associated, &[], 0)?;
        self.empty_table((NID_ROOT_FOLDER & !0x1F) | 0x0D, &hierarchy, &rows, 0)?;
        self.empty_table((NID_ROOT_FOLDER & !0x1F) | 0x0E, &contents, &[], 0)?;
        // The rest, each with its own three tables.
        let rest = [
            (NID_IPM_SUBTREE, "Top of Personal Folders", true, NID_ROOT_FOLDER),
            (NID_SEARCH_ROOT, "Search Root", false, NID_ROOT_FOLDER),
            (NID_DELETED_ITEMS, "Deleted Items", false, NID_IPM_SUBTREE),
        ];
        for (nid, name, subs, parent) in rest {
            self.props_node(nid, folder(name, subs), parent)?;
            let index = nid & !0x1F;
            let row;
            let rows: &[NewRow] = if nid == NID_IPM_SUBTREE {
                row = [self.hierarchy_row(&hierarchy, NID_DELETED_ITEMS, "Deleted Items", false, "IPF.Note", text)];
                &row
            } else {
                &[]
            };
            self.empty_table(index | 0x0D, &hierarchy, rows, 0)?;
            self.empty_table(index | 0x0E, &contents, &[], 0)?;
            self.empty_table(index | 0x0F, &associated, &[], 0)?;
        }
        // The search folder for spam: a queue, its criteria and its contents, not a folder's tables
        // (MS-PST 2.4.8).
        self.props_node(NID_SPAM_FOLDER, folder("SPAM Search Folder 2", false), NID_ROOT_FOLDER)?;
        let index = NID_SPAM_FOLDER & !0x1F;
        self.set_node(index | 0x06, 0, 0, 0);
        self.props_node(index | 0x07, vec![(0x660B, PropertyValue::Integer32(0))], 0)?;
        self.empty_table(index | 0x10, &Schema::new(&search_columns(st))?, &[], 0)?;
        Ok(())
    }
}

/// Makes a new, empty file at `path`, with an Inbox.
pub(crate) fn create_empty(path: &Path, format: Format) -> io::Result<()> {
    // A file already there is refused here, and left alone: only what this made goes on failure.
    write_bare(path, format)?;
    let made = (|| {
        let mut ed = Editor::open_bare(path)?;
        ed.fill_new_file()?;
        ed.commit()?;
        super::folders::ensure_path(path, &["Inbox"], "IPF.Note")?;
        Ok(())
    })();
    if made.is_err() {
        let _ = fs::remove_file(path);
    }
    made
}
