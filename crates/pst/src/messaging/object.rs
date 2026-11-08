//! A message and its attachments (MS-PST 2.4.5, 2.4.6).

use std::io;
use std::rc::Rc;

use super::{PstStore, node_properties};
use crate::ltp::{Bytes, Heap, Properties, PropertyValue, Table, read_properties};
use crate::ndb::{NID_TYPE_ATTACHMENT, NID_TYPE_MASK, SubNodes};
use crate::util::invalid;

const NID_TYPE_NORMAL_MESSAGE: u32 = 0x04;
const NID_TYPE_ASSOC_MESSAGE: u32 = 0x08;
const NID_TYPE_ATTACHMENT_TABLE: u32 = 0x11;
const NID_TYPE_RECIPIENT_TABLE: u32 = 0x12;

/// `PidTagAttachDataBinary` / `PidTagAttachDataObject`, and `PidTagAttachMethod`.
const ATTACH_DATA: u16 = 0x3701;
const ATTACH_METHOD: u16 = 0x3705;

/// Attached messages nested at most, in what's written.
pub const MAX_EMBEDDED_DEPTH: usize = 32;

/// A message.
pub struct Message {
    store: Rc<PstStore>,
    props: Properties,
    subs: SubNodes,
    recipients: Option<Rc<Table>>,
    attachments: Option<Rc<Table>>,
}

impl Message {
    /// The message at node `nid`.
    pub fn read(store: Rc<PstStore>, nid: u32) -> io::Result<Rc<Message>> {
        if !matches!(nid & NID_TYPE_MASK, NID_TYPE_NORMAL_MESSAGE | NID_TYPE_ASSOC_MESSAGE | NID_TYPE_ATTACHMENT) {
            return Err(invalid(format!("node {nid:#x} isn't a message")));
        }
        let node = store.pst.node(nid)?;
        Self::read_at(store, node.data, node.sub)
    }

    fn read_at(store: Rc<PstStore>, data: u64, sub: u64) -> io::Result<Rc<Message>> {
        let (props, subs) = node_properties(&store.pst, data, sub)?;
        let table = |kind: u32| -> io::Result<Option<Rc<Table>>> {
            let mut found = subs.iter().filter(|(nid, _)| *nid & NID_TYPE_MASK == kind);
            match (found.next(), found.next()) {
                (None, _) => Ok(None),
                (Some((_, (d, s))), None) => Ok(Some(Rc::new(Table::read(store.pst.clone(), *d, *s)?))),
                _ => Err(invalid("a message with two tables of one kind")),
            }
        };
        let recipients = table(NID_TYPE_RECIPIENT_TABLE)?;
        let attachments = table(NID_TYPE_ATTACHMENT_TABLE)?;
        Ok(Rc::new(Message { store, props, subs, recipients, attachments }))
    }

    pub fn properties(&self) -> &Properties {
        &self.props
    }

    pub fn recipient_table(&self) -> Option<&Rc<Table>> {
        self.recipients.as_ref()
    }

    pub fn attachment_table(&self) -> Option<&Rc<Table>> {
        self.attachments.as_ref()
    }
}

/// What an attachment holds.
pub enum AttachmentData {
    /// A file's bytes (by value, or OLE storage).
    Binary(Bytes),
    /// A message attached to the message.
    Message(Rc<Message>),
}

/// A message's attachment.
pub struct Attachment {
    props: Properties,
    data: Option<AttachmentData>,
}

impl Attachment {
    /// The attachment at subnode `nid` of `message`.
    pub fn read(message: Rc<Message>, nid: u32) -> io::Result<Rc<Attachment>> {
        if nid & NID_TYPE_MASK != NID_TYPE_ATTACHMENT {
            return Err(invalid(format!("node {nid:#x} isn't an attachment")));
        }
        let store = &message.store;
        let (data, sub) = *message.subs.get(&nid).ok_or_else(|| invalid("an attachment isn't there"))?;
        let heap = Heap::read(&store.pst, data)?;
        // A damaged tree of its own doesn't lose it: what it holds may be in the message's.
        let own = store.pst.subnodes(sub).unwrap_or_else(|e| {
            log::warn!("an attachment's subnode tree can't be read: {e}");
            SubNodes::new()
        });
        let mut props = read_properties(&store.pst, &heap, &own)?;
        let method = match props.get(ATTACH_METHOD) {
            Some(PropertyValue::Integer32(m)) => *m,
            _ => return Err(invalid("an attachment that doesn't say what it holds")),
        };
        if !matches!(method, 0 | 1 | 2 | 4 | 5 | 6 | 7) {
            return Err(invalid(format!("an attachment of method {method}, which none has")));
        }
        let object = |value: Option<&PropertyValue>| -> io::Result<(u64, u64)> {
            let Some(PropertyValue::Object(o)) = value else { return Err(invalid("an attachment's object isn't there")) };
            // In the attachment's own subnode tree, where MS-PST keeps it; else in the message's.
            own.get(&o.node).or_else(|| message.subs.get(&o.node)).copied().ok_or_else(|| invalid("an attachment's object isn't there"))
        };
        let data = match method {
            1 => match props.0.remove(&ATTACH_DATA) {
                Some(PropertyValue::Binary(b)) => Some(AttachmentData::Binary(b)),
                Some(PropertyValue::Null) => Some(AttachmentData::Binary(Bytes::default())),
                _ => return Err(invalid("an attachment's bytes aren't there")),
            },
            5 => {
                let (d, s) = object(props.get(ATTACH_DATA))?;
                Some(AttachmentData::Message(Message::read_at(store.clone(), d, s)?))
            }
            6 => {
                let (d, _) = object(props.get(ATTACH_DATA))?;
                Some(AttachmentData::Binary(Bytes(store.pst.read_value(d)?)))
            }
            _ => None,
        };
        Ok(Rc::new(Attachment { props, data }))
    }

    pub fn properties(&self) -> &Properties {
        &self.props
    }

    pub fn data(&self) -> Option<&AttachmentData> {
        self.data.as_ref()
    }

    /// The message it holds, when it holds one.
    pub fn embedded_message(&self) -> Option<Rc<Message>> {
        match &self.data {
            Some(AttachmentData::Message(m)) => Some(m.clone()),
            _ => None,
        }
    }
}
