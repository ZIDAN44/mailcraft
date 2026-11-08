//! Taking messages out of a file (MS-PST 2.6.1): each one's node goes, the blocks only it used
//! go with it, and its row leaves its folder's contents table and any search folder's.

use std::collections::{BTreeMap, HashSet};
use std::io;
use std::path::Path;

use super::edit::Editor;
use super::folders::{SUD_MSG_DEL, writable};
use super::table::Cell;
use crate::ltp::{PropertyType as P, PropertyValue, Table};
use crate::ndb::btree::{node_entry, node_entry_size};
use crate::ndb::{NID_TYPE_CONTENTS_TABLE, NID_TYPE_MASK, NodeEntry, PT_NBT};
use crate::util::short;

const NID_TYPE_NORMAL_MESSAGE: u32 = 0x04;
const NID_TYPE_NORMAL_FOLDER: u32 = 0x02;
const NID_TYPE_SEARCH_FOLDER: u32 = 0x03;
const NID_TYPE_SEARCH_CONTENTS_TABLE: u32 = 0x10;

const MESSAGE_FLAGS: u16 = 0x0E07;
const MF_READ: i32 = 0x0001;
const CONTENT_COUNT: u16 = 0x3602;
const CONTENT_UNREAD: u16 = 0x3603;

fn refused(what: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, what)
}

/// Whether a row of a contents table is unread: its flags, read when it has none.
fn row_unread(table: &Table, row: &crate::ltp::Row) -> bool {
    let at = table.columns().iter().position(|c| c.prop_id() == MESSAGE_FLAGS && c.prop_type() == P::INT32);
    matches!(at.and_then(|i| table.value(row, i)), Some(PropertyValue::Integer32(f)) if f & MF_READ == 0)
}

/// Takes `messages` (each a message's node ID) out of the file at `path`, in one change.
pub(crate) fn delete_messages(path: &Path, messages: &[u32]) -> io::Result<()> {
    if messages.is_empty() {
        return Err(refused("deleting requires at least one message".into()));
    }
    let mut ed = Editor::open(path)?;
    // Each message, in a folder of the file: the folders and what each loses.
    let mut removed = HashSet::new();
    let mut folders: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for m in messages {
        let entry = ed
            .node(*m)
            .ok()
            .filter(|_| m & NID_TYPE_MASK == NID_TYPE_NORMAL_MESSAGE)
            .ok_or_else(|| refused(format!("node {m:X} is not a message of the file")))?;
        if entry.parent & NID_TYPE_MASK != NID_TYPE_NORMAL_FOLDER || ed.node(entry.parent).is_err() {
            return Err(refused(format!("message {m:X} is in no folder of the file")));
        }
        if !removed.insert(*m) {
            return Err(refused(format!("message {m:X} is given twice")));
        }
        folders.entry(entry.parent).or_default().push(*m);
    }
    for m in messages {
        let entry = ed.node(*m)?;
        ed.release_node_data(entry.data, entry.sub)?;
        ed.drop_node(*m);
    }
    let mut counted: BTreeMap<u32, Vec<(u32, i32, i32)>> = BTreeMap::new();
    for folder in folders.keys() {
        let node = (folder & !NID_TYPE_MASK) | NID_TYPE_CONTENTS_TABLE;
        let entry = ed.node(node)?;
        let table = Table::read(ed.pst.clone(), entry.data, entry.sub)?;
        let unread_gone =
            i32::try_from(table.rows_matrix().filter(|r| removed.contains(&r.id()) && row_unread(&table, r)).count()).map_err(|_| short())?;
        let left = ed.rewrite_table(node, |_, _, rows| {
            rows.retain(|r| !removed.contains(&r.id));
            Ok(())
        })?;
        let folder_obj = ed.store.open_folder(*folder)?;
        let before = match folder_obj.properties().get(CONTENT_UNREAD) {
            Some(PropertyValue::Integer32(n)) => (*n).max(0),
            _ => 0,
        };
        let count = i32::try_from(left).map_err(|_| short())?;
        let unread = before.saturating_sub(unread_gone).clamp(0, count);
        let mut props = writable(&folder_obj);
        props.retain(|(id, _)| *id != CONTENT_COUNT && *id != CONTENT_UNREAD);
        props.push((CONTENT_COUNT, PropertyValue::Integer32(count)));
        props.push((CONTENT_UNREAD, PropertyValue::Integer32(unread)));
        ed.rewrite_props(*folder, props)?;
        let parent = ed.node(*folder)?.parent;
        if parent != 0 && parent != *folder {
            counted.entry(parent).or_default().push((*folder, count, unread));
        }
    }
    for (parent, rows) in counted {
        let hierarchy = (parent & !NID_TYPE_MASK) | crate::ndb::NID_TYPE_HIERARCHY_TABLE;
        if ed.node(hierarchy).is_err() {
            continue;
        }
        let patches = rows
            .into_iter()
            .map(|(folder, count, unread)| {
                (
                    folder,
                    vec![(CONTENT_COUNT, Cell::Fixed(count.to_le_bytes().to_vec())), (CONTENT_UNREAD, Cell::Fixed(unread.to_le_bytes().to_vec()))],
                )
            })
            .collect();
        ed.patch_rows(hierarchy, &patches)?;
    }
    // Search folders listing any of them.
    let format = ed.format();
    let root = ed.header.root.nbt;
    let mut search_tables: Vec<NodeEntry> = Vec::new();
    ed.walk_tree(
        root,
        PT_NBT,
        node_entry_size(format),
        &mut |p, i| {
            let e = node_entry(p, i, format)?;
            if e.nid & NID_TYPE_MASK == NID_TYPE_SEARCH_CONTENTS_TABLE {
                search_tables.push(e);
            }
            Ok(())
        },
        &mut |_| {},
    )?;
    for entry in search_tables {
        let Ok(table) = Table::read(ed.pst.clone(), entry.data, entry.sub) else { continue };
        let folder = (entry.nid & !NID_TYPE_MASK) | NID_TYPE_SEARCH_FOLDER;
        if ed.node(folder).is_ok() && table.rows_matrix().any(|r| removed.contains(&r.id())) {
            ed.rewrite_table(entry.nid, |_, _, rows| {
                rows.retain(|r| !removed.contains(&r.id));
                Ok(())
            })?;
        }
    }
    let updates: Vec<(u16, u32, u32)> = folders.iter().flat_map(|(f, ms)| ms.iter().map(move |m| (SUD_MSG_DEL, *f, *m))).collect();
    ed.queue_search_updates(&updates)?;
    ed.commit()
}
