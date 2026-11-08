//! Folders: finding and making them, adding to them, and the special folders a file records.

use std::collections::{BTreeMap, HashSet};
use std::io;
use std::path::Path;

use super::edit::Editor;
use super::message::{NID_TYPE_NORMAL_MESSAGE, contents_cell, is_unread, text_of};
use super::table::{Cell, NewRow, ROW_ID, ROW_VERSION};
use super::text::{Text, same_ansi_name};
use super::types::*;
use crate::ltp::{Bytes, PropertyType as P, PropertyValue, Table};
use crate::messaging::{NID_MESSAGE_STORE, PstFolder, node_of_entry_id};
use crate::ndb::{NID_TYPE_ASSOC_CONTENTS_TABLE, NID_TYPE_CONTENTS_TABLE, NID_TYPE_HIERARCHY_TABLE, NID_TYPE_MASK, SubNodes};
use crate::util::{invalid, short};

pub(crate) const NID_ROOT_FOLDER: u32 = 0x122;
const NID_TYPE_NORMAL_FOLDER: u32 = 0x02;
const NID_HIERARCHY_TEMPLATE: u32 = 0x60D;
const NID_CONTENTS_TEMPLATE: u32 = 0x60E;
const NID_ASSOC_TEMPLATE: u32 = 0x60F;
const NID_SEARCH_QUEUE: u32 = 0x1E1;

const DISPLAY_NAME: u16 = 0x3001;
const CONTENT_COUNT: u16 = 0x3602;
const CONTENT_UNREAD: u16 = 0x3603;
const SUBFOLDERS: u16 = 0x360A;
const CONTAINER_CLASS: u16 = 0x3613;

/// Search update types (`SUDT`, MS-PST 2.4.8.2).
pub(crate) const SUD_MSG_ADD: u16 = 1;
pub(crate) const SUD_MSG_DEL: u16 = 3;
const SUD_FLD_ADD: u16 = 5;
const SUD_FLD_MOD: u16 = 6;

/// How deep a special folder is looked for.
const MAX_DEPTH: usize = 64;
/// `PidTagAdditionalRenEntryIds`' value for Junk Email.
const JUNK_INDEX: usize = 4;
const INBOX_RECORDS: [u16; 7] = [0x36D0, 0x36D1, 0x36D2, 0x36D3, 0x36D4, 0x36D7, 0x36D8];

fn folder_prop(p: &mut Vec<(u16, PropertyValue)>, id: u16, v: PropertyValue) {
    p.retain(|(existing, _)| *existing != id);
    p.push((id, v));
}

pub(crate) fn writable(f: &PstFolder) -> Vec<(u16, PropertyValue)> {
    f.properties().iter().map(|(k, v)| (*k, v.clone())).collect()
}

/// A display name read as text.
fn name_matches(v: Option<&PropertyValue>, wanted: &str) -> bool {
    match v {
        Some(PropertyValue::Unicode(s)) => String::from_utf16_lossy(s.buffer()).trim_end_matches('\0').eq_ignore_ascii_case(wanted),
        Some(PropertyValue::String8(s)) => same_ansi_name(s.buffer(), wanted),
        _ => false,
    }
}

fn name_of(v: Option<&PropertyValue>) -> String {
    match v {
        Some(PropertyValue::Unicode(s)) => String::from_utf16_lossy(s.buffer()).trim_end_matches('\0').to_string(),
        Some(PropertyValue::String8(s)) => s.buffer().iter().map(|b| char::from(*b)).collect::<String>().trim_end_matches('\0').to_string(),
        _ => String::new(),
    }
}

fn check_path(path: &[&str]) -> io::Result<()> {
    if path.is_empty() || path.iter().any(|n| n.is_empty()) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "folder path must contain only non-empty names"));
    }
    Ok(())
}

impl Editor {
    /// The child of `parent` named `name`.
    fn find_child(&self, parent: u32, name: &str) -> io::Result<Option<u32>> {
        let folder = self.store.open_folder(parent)?;
        let Some(table) = folder.hierarchy_table() else { return Ok(None) };
        let column = table.columns().iter().position(|c| c.prop_id() == DISPLAY_NAME);
        for row in table.rows_matrix() {
            if column.is_some_and(|c| name_matches(table.value(row, c).as_ref(), name)) {
                return Ok(Some(row.id()));
            }
        }
        Ok(None)
    }

    /// The cell a new folder's row has for a hierarchy column.
    fn folder_cell(&self, id: u16, kind: u16, folder: u32, name: &str, class: &str, version: u32) -> Option<Cell> {
        let text = Text::plain(self.format());
        match (id, kind) {
            #[allow(clippy::cast_possible_wrap)]
            (ROW_ID, P::INT32) => Some(Cell::Fixed((folder as i32).to_le_bytes().to_vec())),
            #[allow(clippy::cast_possible_wrap)]
            (ROW_VERSION, P::INT32) => Some(Cell::Fixed((version as i32).to_le_bytes().to_vec())),
            (DISPLAY_NAME, _) => text.cell(kind, name).map(Cell::Bytes),
            (CONTENT_COUNT | CONTENT_UNREAD, P::INT32) => Some(Cell::Fixed(0i32.to_le_bytes().to_vec())),
            (SUBFOLDERS, P::BOOLEAN) => Some(Cell::Fixed(vec![0])),
            (CONTAINER_CLASS, _) => text.cell(kind, class).map(Cell::Bytes),
            (0x0FFF, P::BINARY) => Some(Cell::Bytes(self.entry_id(folder).ok()?)),
            _ => None,
        }
    }

    /// The entry ID of node `nid` in this file.
    pub(crate) fn entry_id(&self, nid: u32) -> io::Result<Vec<u8>> {
        let key = self.record_key()?;
        let mut e = vec![0u8; 4];
        e.extend_from_slice(&key);
        e.extend(nid.to_le_bytes());
        Ok(e)
    }

    pub(crate) fn record_key(&self) -> io::Result<[u8; 16]> {
        match self.store.properties().get(0x0FF9) {
            Some(PropertyValue::Binary(b)) => b.buffer().try_into().map_err(|_| invalid("the store's record key is of the wrong size")),
            _ => Err(invalid("the store has no record key")),
        }
    }

    /// Adds entries to the file's search update queue.
    pub(crate) fn queue_search_updates(&mut self, updates: &[(u16, u32, u32)]) -> io::Result<()> {
        let Ok(entry) = self.node(NID_SEARCH_QUEUE) else { return Ok(()) };
        let mut data = if entry.data == 0 { Vec::new() } else { self.pst.read_value(entry.data)? };
        for (kind, a, b) in updates {
            data.extend(0u16.to_le_bytes());
            data.extend(kind.to_le_bytes());
            data.extend(a.to_le_bytes());
            data.extend(b.to_le_bytes());
            data.extend([0u8; 8]);
        }
        let bid = self.write_tree(&data)?;
        self.release_tree(entry.data)?;
        self.set_node(NID_SEARCH_QUEUE, bid, entry.sub, entry.parent);
        Ok(())
    }

    /// Makes folder `name` under `parent`, and says which node it is.
    fn make_folder(&mut self, parent: u32, name: &str, class: &str) -> io::Result<u32> {
        let format = self.format();
        let text = Text::plain(format);
        let folder = self.next_node_id(NID_TYPE_NORMAL_FOLDER)?;
        let version = self.header.unique;
        // The parent's hierarchy table gets a row.
        let hierarchy = (parent & !NID_TYPE_MASK) | NID_TYPE_HIERARCHY_TABLE;
        let cell_rows: Vec<Option<Cell>> = {
            let entry = self.node(hierarchy)?;
            let table = Table::read(self.pst.clone(), entry.data, entry.sub)?;
            table.columns().iter().map(|c| self.folder_cell(c.prop_id(), c.prop_type(), folder, name, class, version)).collect()
        };
        self.rewrite_table(hierarchy, move |_, _, rows| {
            rows.push(NewRow { id: folder, cells: cell_rows });
            Ok(())
        })?;
        // The parent says it has subfolders, in its own properties and in its row.
        let parent_folder = self.store.open_folder(parent)?;
        if !matches!(parent_folder.properties().get(SUBFOLDERS), Some(PropertyValue::Boolean(true))) {
            let mut props = writable(&parent_folder);
            folder_prop(&mut props, SUBFOLDERS, PropertyValue::Boolean(true));
            self.rewrite_props(parent, props)?;
            let parent_parent = self.node(parent)?.parent;
            if parent_parent != 0 {
                let above = (parent_parent & !NID_TYPE_MASK) | NID_TYPE_HIERARCHY_TABLE;
                if self.node(above).is_ok() {
                    self.patch_rows(above, &BTreeMap::from([(parent, vec![(SUBFOLDERS, Cell::Fixed(vec![1]))])]))?;
                }
            }
        }
        // The folder's own nodes: its tables are the templates' blocks until something is added.
        let props = vec![
            (DISPLAY_NAME, text.value(name)),
            (CONTENT_COUNT, PropertyValue::Integer32(0)),
            (CONTENT_UNREAD, PropertyValue::Integer32(0)),
            (SUBFOLDERS, PropertyValue::Boolean(false)),
            (CONTAINER_CLASS, text.value(class)),
        ];
        let built = super::heap::build_props(format, props, 0x80)?;
        // A name too long for the heap goes in a subnode.
        let mut subs = SubNodes::new();
        for (node, bytes) in built.external {
            let bid = self.write_tree(&bytes)?;
            subs.insert(node, (bid, 0));
        }
        let data = self.write_leaves(&built.blocks)?;
        let sub = self.write_subnodes(&subs)?;
        self.set_node(folder, data, sub, parent);
        let index = folder & !NID_TYPE_MASK;
        for (template, kind) in [
            (NID_HIERARCHY_TEMPLATE, NID_TYPE_HIERARCHY_TABLE),
            (NID_CONTENTS_TEMPLATE, NID_TYPE_CONTENTS_TABLE),
            (NID_ASSOC_TEMPLATE, NID_TYPE_ASSOC_CONTENTS_TABLE),
        ] {
            let t = self.node(template)?;
            self.add_ref(t.data);
            if t.sub != 0 {
                self.add_ref(t.sub);
            }
            self.set_node(index | kind, t.data, t.sub, 0);
        }
        self.queue_search_updates(&[(SUD_FLD_ADD, parent, folder), (SUD_FLD_MOD, parent, 0)])?;
        Ok(folder)
    }

    /// Sets cells of rows in the table at `nid`, by row ID.
    pub(crate) fn patch_rows(&mut self, nid: u32, patches: &BTreeMap<u32, Vec<(u16, Cell)>>) -> io::Result<()> {
        let version = self.header.unique;
        self.rewrite_table(nid, |_, schema, rows| {
            for row in rows.iter_mut() {
                let Some(cells) = patches.get(&row.id) else { continue };
                for (id, cell) in cells {
                    if let Some(i) = schema.index_of(*id)
                        && let Some(slot) = row.cells.get_mut(i)
                    {
                        *slot = Some(cell.clone());
                    }
                }
                if let Some(i) = schema.index_of(ROW_VERSION)
                    && let Some(slot) = row.cells.get_mut(i)
                {
                    *slot = Some(Cell::Fixed(version.to_le_bytes().to_vec()));
                }
            }
            Ok(())
        })?;
        Ok(())
    }

    /// Writes `batch` into folder `folder`.
    fn append_to(&mut self, folder: u32, batch: &[BatchMessage<'_>]) -> io::Result<()> {
        let format = self.format();
        let named = self.resolve_named(batch).map_err(|e| io::Error::new(e.kind(), format!("name-to-ID map: {e}")))?;
        let version = self.header.unique.wrapping_add(1);
        let key = self.record_key()?;
        struct Written {
            nid: u32,
            size: i32,
        }
        let mut written = Vec::with_capacity(batch.len());
        let mut updates = Vec::with_capacity(batch.len() + 1);
        for input in batch {
            let text = text_of(format, &input.message, input.attachments);
            let nid = self.next_node_id(NID_TYPE_NORMAL_MESSAGE)?;
            let (data, sub, total) = self.write_message(&input.message, input.attachments, &named, nid, version, text)?;
            self.set_node(nid, data, sub, folder);
            updates.push((SUD_MSG_ADD, folder, nid));
            written.push(Written { nid, size: i32::try_from(total.saturating_sub(6)).map_err(|_| short())? });
        }
        let contents = (folder & !NID_TYPE_MASK) | NID_TYPE_CONTENTS_TABLE;
        let count = self.rewrite_table(contents, |_, schema, rows| {
            for (input, w) in batch.iter().zip(&written) {
                let text = text_of(format, &input.message, input.attachments);
                let cells = schema
                    .columns
                    .iter()
                    .map(|c| {
                        contents_cell(
                            c.prop_id(),
                            c.prop_type(),
                            &input.message,
                            !input.attachments.is_empty(),
                            &named,
                            &key,
                            folder,
                            w.nid,
                            version,
                            w.size,
                            text,
                        )
                    })
                    .collect();
                rows.push(NewRow { id: w.nid, cells });
            }
            Ok(())
        })?;
        let folder_obj = self.store.open_folder(folder)?;
        let before = match folder_obj.properties().get(CONTENT_UNREAD) {
            Some(PropertyValue::Integer32(n)) => (*n).max(0),
            _ => 0,
        };
        let added = i32::try_from(batch.iter().filter(|b| is_unread(&b.message)).count()).map_err(|_| short())?;
        let (count, unread) = (i32::try_from(count).map_err(|_| short())?, before.saturating_add(added));
        let mut props = writable(&folder_obj);
        folder_prop(&mut props, CONTENT_COUNT, PropertyValue::Integer32(count));
        folder_prop(&mut props, CONTENT_UNREAD, PropertyValue::Integer32(unread));
        self.rewrite_props(folder, props)?;
        self.update_row_counts(folder, count, unread)?;
        updates.push((SUD_FLD_MOD, folder, 0));
        self.queue_search_updates(&updates)
    }

    /// The folder's counts in its row of its parent's hierarchy table.
    pub(crate) fn update_row_counts(&mut self, folder: u32, count: i32, unread: i32) -> io::Result<()> {
        let parent = self.node(folder)?.parent;
        if parent == 0 || parent == folder {
            return Ok(());
        }
        let hierarchy = (parent & !NID_TYPE_MASK) | NID_TYPE_HIERARCHY_TABLE;
        if self.node(hierarchy).is_err() {
            return Ok(());
        }
        let cells = vec![(CONTENT_COUNT, Cell::Fixed(count.to_le_bytes().to_vec())), (CONTENT_UNREAD, Cell::Fixed(unread.to_le_bytes().to_vec()))];
        self.patch_rows(hierarchy, &BTreeMap::from([(folder, cells)]))
    }
}

/// The folder at the path of names from the IPM subtree, made when missing: the last with
/// `class`, any on the way `IPF.Note`. Each one made is its own change.
pub(crate) fn ensure_path(path: &Path, names: &[&str], class: &str) -> io::Result<u32> {
    check_path(names)?;
    let mut parent = Editor::open(path)?.store.ipm_subtree()?;
    for (i, name) in names.iter().enumerate() {
        let mut ed = Editor::open(path)?;
        match ed.find_child(parent, name)? {
            Some(child) => parent = child,
            None => {
                let this_class = if i + 1 == names.len() { class } else { "IPF.Note" };
                let made = ed.make_folder(parent, name, this_class)?;
                ed.commit()?;
                parent = made;
            }
        }
    }
    Ok(parent)
}

/// Adds `batch` to the folder at the path `names`, made when missing.
pub(crate) fn append(path: &Path, names: &[&str], batch: &[BatchMessage<'_>]) -> io::Result<()> {
    if batch.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "bulk append requires at least one message"));
    }
    for input in batch {
        check_message(&input.message)?;
        check_attachments(input.attachments, 0)?;
    }
    let folder = ensure_path(path, names, "IPF.Note")?;
    let mut ed = Editor::open(path)?;
    ed.append_to(folder, batch)?;
    ed.commit()
}

fn role_property(role: SpecialFolder) -> (u16, bool) {
    match role {
        SpecialFolder::DeletedItems => (0x35E3, true),
        SpecialFolder::SentItems => (0x35E4, true),
        SpecialFolder::Outbox => (0x35E2, true),
        SpecialFolder::Drafts => (0x36D7, false),
        SpecialFolder::JunkEmail => (0x36D8, false),
        SpecialFolder::Calendar => (0x36D0, false),
        SpecialFolder::Contacts => (0x36D1, false),
        SpecialFolder::Journal => (0x36D2, false),
        SpecialFolder::Notes => (0x36D3, false),
        SpecialFolder::Tasks => (0x36D4, false),
    }
}

/// The root folder, the IPM subtree and, when there is one, the Inbox: found by what it records
/// (MS-OXOSFLD 2.2), else by its name.
fn recording_folders(ed: &Editor) -> io::Result<Vec<u32>> {
    let top = ed.store.ipm_subtree()?;
    let mut nodes = vec![NID_ROOT_FOLDER, top];
    let parent = ed.store.open_folder(top)?;
    let mut named = None;
    let mut inbox = None;
    if let Some(table) = parent.hierarchy_table() {
        for row in table.rows_matrix() {
            let Ok(f) = ed.store.open_folder(row.id()) else { continue };
            if INBOX_RECORDS.iter().any(|id| f.properties().get(*id).is_some()) {
                inbox = Some(row.id());
                break;
            }
            if named.is_none() && name_matches(f.properties().get(DISPLAY_NAME), "Inbox") {
                named = Some(row.id());
            }
        }
    }
    nodes.extend(inbox.or(named));
    nodes.dedup();
    Ok(nodes)
}

/// The names from the IPM subtree down to `node`, each folder looked through once.
fn folder_names(ed: &Editor, node: u32) -> io::Result<Option<Vec<String>>> {
    let top = ed.store.ipm_subtree()?;
    if node == top {
        return Ok(None);
    }
    let mut seen = HashSet::new();
    let mut todo: Vec<(u32, Vec<String>)> = vec![(top, Vec::new())];
    while let Some((at, path)) = todo.pop() {
        if !seen.insert(at) || path.len() >= MAX_DEPTH {
            continue;
        }
        let Ok(folder) = ed.store.open_folder(at) else { continue };
        let Some(table) = folder.hierarchy_table() else { continue };
        for row in table.rows_matrix() {
            let Ok(child) = ed.store.open_folder(row.id()) else { continue };
            let mut names = path.clone();
            names.push(name_of(child.properties().get(DISPLAY_NAME)));
            if row.id() == node {
                return Ok(Some(names));
            }
            todo.push((row.id(), names));
        }
    }
    Ok(None)
}

/// The folder the file records as `role`, as names from the IPM subtree.
pub(crate) fn special_folder(path: &Path, role: SpecialFolder) -> io::Result<Option<Vec<String>>> {
    let ed = Editor::open(path)?;
    let (id, on_store) = role_property(role);
    let values: Vec<Option<PropertyValue>> = if on_store {
        vec![ed.store.properties().get(id).cloned()]
    } else {
        recording_folders(&ed)?.into_iter().map(|n| Ok(ed.store.open_folder(n)?.properties().get(id).cloned())).collect::<io::Result<_>>()?
    };
    for value in values.into_iter().flatten() {
        let entry = match (&value, role) {
            (PropertyValue::MultipleBinary(l), SpecialFolder::JunkEmail) => l.get(JUNK_INDEX).map(|b| b.buffer().to_vec()),
            (PropertyValue::Binary(b), _) => Some(b.buffer().to_vec()),
            _ => None,
        };
        let Some(node) = entry.and_then(|b| node_of_entry_id(&b).ok()) else { continue };
        if let Some(names) = folder_names(&ed, node)? {
            return Ok(Some(names));
        }
    }
    Ok(None)
}

/// Records the folder at `names`, made when missing, as `role`.
pub(crate) fn set_special_folder(path: &Path, role: SpecialFolder, names: &[&str]) -> io::Result<()> {
    let folder = ensure_path(path, names, role.container_class())?;
    let mut ed = Editor::open(path)?;
    let entry = ed.entry_id(folder)?;
    let (id, on_store) = role_property(role);
    let set = |props: &mut Vec<(u16, PropertyValue)>| {
        let value = if role == SpecialFolder::JunkEmail {
            let mut list = match props.iter().find(|(p, _)| *p == id) {
                Some((_, PropertyValue::MultipleBinary(l))) => l.clone(),
                _ => Vec::new(),
            };
            if list.len() <= JUNK_INDEX {
                list.resize(JUNK_INDEX + 1, Bytes::default());
            }
            if let Some(slot) = list.get_mut(JUNK_INDEX) {
                *slot = Bytes(entry.clone());
            }
            PropertyValue::MultipleBinary(list)
        } else {
            PropertyValue::Binary(Bytes(entry.clone()))
        };
        folder_prop(props, id, value);
    };
    if on_store {
        let mut props: Vec<_> = ed.store.properties().iter().map(|(k, v)| (*k, v.clone())).collect();
        set(&mut props);
        ed.rewrite_props(NID_MESSAGE_STORE, props)?;
    } else {
        for node in recording_folders(&ed)? {
            let mut props = writable(&*ed.store.open_folder(node)?);
            set(&mut props);
            ed.rewrite_props(node, props)?;
        }
    }
    ed.commit()
}

/// The file's display name set.
pub(crate) fn set_store_display_name(path: &Path, name: &str) -> io::Result<()> {
    let mut ed = Editor::open(path)?;
    let text = Text::plain(ed.format());
    let mut props: Vec<_> = ed.store.properties().iter().map(|(k, v)| (*k, v.clone())).collect();
    folder_prop(&mut props, DISPLAY_NAME, text.value(name));
    ed.rewrite_props(NID_MESSAGE_STORE, props)?;
    ed.commit()
}
