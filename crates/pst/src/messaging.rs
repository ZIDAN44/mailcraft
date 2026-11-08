//! The top layer (MS-PST 2.4): the store, its folders, messages and attachments.

pub(crate) mod named;
mod object;

use std::cell::OnceCell;
use std::io;
use std::rc::Rc;

pub(crate) use named::{BUCKET_COUNT, STREAM_ENTRY, STREAM_GUID, STREAM_STRING};
pub use named::{NamedEntry, NamedMap, NamedPropertyGuid, NamedPropertyId, PS_MAPI, PS_PUBLIC_STRINGS};
pub use object::{Attachment, AttachmentData, MAX_EMBEDDED_DEPTH, Message};

use crate::ltp::{Heap, Properties, PropertyValue, Table};
use crate::ndb::*;
use crate::util::{invalid, short, u32_at};

/// The message store's node (MS-PST 2.4.1).
pub(crate) const NID_MESSAGE_STORE: u32 = 0x21;
/// The name-to-ID map's node (MS-PST 2.4.7).
pub(crate) const NID_NAME_TO_ID_MAP: u32 = 0x61;

pub(crate) const NID_TYPE_NORMAL_FOLDER: u32 = 0x02;
const NID_TYPE_SEARCH_FOLDER: u32 = 0x03;

/// `PidTagIpmSubtreeEntryId`.
const IPM_SUBTREE: u16 = 0x35E0;

/// The set of properties of a node, read from its data and subnode trees.
pub(crate) fn node_properties(pst: &PstFile, data: u64, sub: u64) -> io::Result<(Properties, SubNodes)> {
    let heap = Heap::read(pst, data)?;
    let subs = pst.subnodes(sub)?;
    let props = crate::ltp::read_properties(pst, &heap, &subs)?;
    Ok((props, subs))
}

/// A data file's message store: where its folders are.
pub struct PstStore {
    pst: Rc<PstFile>,
    props: Properties,
}

impl PstStore {
    /// A store with no properties yet, for a file being made.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn bare(pst: Rc<PstFile>) -> Rc<PstStore> {
        Rc::new(PstStore { pst, props: Properties::default() })
    }

    /// The store of the file `pst`.
    pub fn read(pst: Rc<PstFile>) -> io::Result<Rc<PstStore>> {
        let node = pst.node(NID_MESSAGE_STORE)?;
        let (props, _) = node_properties(&pst, node.data, node.sub)?;
        Ok(Rc::new(PstStore { pst, props }))
    }

    pub fn pst(&self) -> &Rc<PstFile> {
        &self.pst
    }

    pub fn properties(&self) -> &Properties {
        &self.props
    }

    /// The node of the folder that holds the mailbox's folders (the IPM subtree).
    pub fn ipm_subtree(&self) -> io::Result<u32> {
        match self.props.get(IPM_SUBTREE) {
            Some(PropertyValue::Binary(b)) => node_of_entry_id(b.buffer()),
            _ => Err(invalid("the store doesn't say where its folders are")),
        }
    }

    /// The folder at node `nid`.
    pub fn open_folder(&self, nid: u32) -> io::Result<Rc<PstFolder>> {
        if !matches!(nid & NID_TYPE_MASK, NID_TYPE_NORMAL_FOLDER | NID_TYPE_SEARCH_FOLDER) {
            return Err(invalid(format!("node {nid:#x} isn't a folder")));
        }
        let node = self.pst.node(nid)?;
        let (props, _) = node_properties(&self.pst, node.data, node.sub)?;
        Ok(Rc::new(PstFolder {
            pst: self.pst.clone(),
            nid,
            props,
            hierarchy: OnceCell::new(),
            contents: OnceCell::new(),
            associated: OnceCell::new(),
        }))
    }

    /// Those of `ids` among the properties of the message at node `nid`, and nothing else of it.
    pub fn message_properties(&self, nid: u32, ids: &[u16]) -> io::Result<Properties> {
        let node = self.pst.node(nid)?;
        let heap = Heap::read(&self.pst, node.data)?;
        let subs = self.pst.subnodes(node.sub)?;
        crate::ltp::read_some(&self.pst, &heap, &subs, &|id| ids.contains(&id))
    }

    /// The file's name-to-ID map.
    pub fn named_property_map(&self) -> io::Result<NamedMap> {
        let node = self.pst.node(NID_NAME_TO_ID_MAP)?;
        let (props, _) = node_properties(&self.pst, node.data, node.sub)?;
        Ok(NamedMap::new(props))
    }
}

/// The node an entry ID names: its last four bytes (MS-PST 2.4.3.2).
pub(crate) fn node_of_entry_id(b: &[u8]) -> io::Result<u32> {
    if b.len() != 24 {
        return Err(invalid("an entry ID of the wrong size"));
    }
    u32_at(b, 20).map_err(|_| short())
}

type Cached = OnceCell<Option<Rc<Table>>>;

/// A folder.
pub struct PstFolder {
    pst: Rc<PstFile>,
    nid: u32,
    props: Properties,
    hierarchy: Cached,
    contents: Cached,
    associated: Cached,
}

impl PstFolder {
    pub fn node(&self) -> u32 {
        self.nid
    }

    pub fn properties(&self) -> &Properties {
        &self.props
    }

    /// The table of the node beside this folder's, of the given type: `None` when there is none
    /// or it can't be read.
    fn table(&self, kind: u32) -> Option<Rc<Table>> {
        let pst = &self.pst;
        let node = pst.node((self.nid & !NID_TYPE_MASK) | kind).ok()?;
        match Table::read(pst.clone(), node.data, node.sub) {
            Ok(t) => Some(Rc::new(t)),
            Err(e) => {
                log::warn!("a folder's table could not be read: {e}");
                None
            }
        }
    }

    /// The folders in this one.
    pub fn hierarchy_table(&self) -> Option<&Rc<Table>> {
        self.hierarchy.get_or_init(|| self.table(NID_TYPE_HIERARCHY_TABLE)).as_ref()
    }

    /// The messages in this one.
    pub fn contents_table(&self) -> Option<&Rc<Table>> {
        self.contents.get_or_init(|| self.table(NID_TYPE_CONTENTS_TABLE)).as_ref()
    }

    /// The hidden messages in this one.
    pub fn associated_table(&self) -> Option<&Rc<Table>> {
        self.associated.get_or_init(|| self.table(NID_TYPE_ASSOC_CONTENTS_TABLE)).as_ref()
    }
}
