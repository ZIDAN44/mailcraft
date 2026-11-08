//! What is written: messages, recipients, attachments and the properties given with them.

use std::collections::HashSet;
use std::io::{self, Read};

use crate::ltp::{Bytes, GuidValue, PropertyValue, Utf16};

/// A recipient's kind (`PidTagRecipientType`).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum RecipientType {
    #[default]
    To = 1,
    Cc = 2,
    Bcc = 3,
}

/// A message's recipient.
#[derive(Clone, Copy, Default)]
pub struct NewRecipient<'a> {
    pub name: &'a str,
    pub email: &'a str,
    pub recipient_type: RecipientType,
    /// `PidTagRecipientFlags` (0x5FFD: `recipSendable`, `recipOrganizer`…), in its row when given.
    pub flags: Option<i32>,
    /// `PidTagRecipientTrackStatus` (0x5FFF: how an attendee answered), in its row when given.
    pub track_status: Option<i32>,
}

/// A message to write.
#[derive(Clone, Copy)]
pub struct NewMessage<'a> {
    pub subject: &'a str,
    pub sender_name: &'a str,
    pub sender_email: &'a str,
    pub recipients: &'a [NewRecipient<'a>],
    pub body: &'a str,
    /// UTF-8 HTML for `PR_HTML`; `body` stays the plain text.
    pub html_body: Option<&'a str>,
    pub message_id: &'a str,
    /// 100-nanosecond intervals since 1601-01-01 UTC.
    pub delivery_time: i64,
    /// `PR_MESSAGE_CLASS`, such as `IPM.Note`.
    pub message_class: &'a str,
    /// Properties written beside the writer's own, in place of the same ones, in the message and
    /// in its row of the folder's contents table.
    pub properties: &'a [NewProperty<'a>],
}

/// A file read as it is written.
pub trait AttachmentSource {
    /// Its size: exactly what [`Self::open`] reads.
    fn size(&self) -> u64;
    /// Its bytes from the start, read once.
    fn open(&self) -> io::Result<Box<dyn Read + '_>>;
}

/// What an attachment holds.
#[derive(Default)]
pub enum AttachmentContent<'a> {
    /// The file's bytes: [`NewAttachment::data`].
    #[default]
    Data,
    /// A file read from a source as it is written; `data` is empty.
    Source(&'a dyn AttachmentSource),
    /// An attached message; `data` is empty and `filename` is its display name.
    Message(Box<EmbeddedMessage<'a>>),
}

/// A message attached to another.
pub struct EmbeddedMessage<'a> {
    pub message: NewMessage<'a>,
    pub attachments: Vec<NewAttachment<'a>>,
}

/// An attachment.
#[derive(Default)]
pub struct NewAttachment<'a> {
    pub filename: &'a str,
    pub mime_type: &'a str,
    /// MIME Content-ID for an inline attachment, with or without its `<` and `>`.
    pub content_id: Option<&'a str>,
    pub data: &'a [u8],
    pub content: AttachmentContent<'a>,
    /// Tagged properties of the attachment's own (not its size, value or method).
    pub properties: &'a [NewProperty<'a>],
}

/// One message and its attachments.
#[derive(Clone, Copy)]
pub struct BatchMessage<'a> {
    pub message: NewMessage<'a>,
    pub attachments: &'a [NewAttachment<'a>],
}

/// A named property's name in its set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Name<'a> {
    Id(u32),
    String(&'a str),
}

/// A property's value, as given to be written.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NewValue<'a> {
    Integer16(i16),
    Integer32(i32),
    Boolean(bool),
    Floating64(f64),
    Time(i64),
    Unicode(&'a str),
    Binary(&'a [u8]),
    MultipleUnicode(&'a [&'a str]),
    MultipleBinary(&'a [&'a [u8]]),
}

impl NewValue<'_> {
    pub(crate) fn property_value(&self) -> PropertyValue {
        use PropertyValue as V;
        let utf16 = |s: &str| Utf16(s.encode_utf16().collect());
        match *self {
            Self::Integer16(v) => V::Integer16(v),
            Self::Integer32(v) => V::Integer32(v),
            Self::Boolean(v) => V::Boolean(v),
            Self::Floating64(v) => V::Floating64(v),
            Self::Time(v) => V::Time(v),
            Self::Unicode(v) => V::Unicode(utf16(v)),
            Self::Binary(v) => V::Binary(Bytes(v.to_vec())),
            // Each string ends in a NUL, as Outlook writes them; readers drop it.
            Self::MultipleUnicode(v) => V::MultipleUnicode(v.iter().map(|s| Utf16(s.encode_utf16().chain(Some(0)).collect())).collect()),
            Self::MultipleBinary(v) => V::MultipleBinary(v.iter().map(|b| Bytes(b.to_vec())).collect()),
        }
    }
}

/// Which property a value is for.
#[derive(Clone, Copy, Debug)]
pub enum PropertyKey<'a> {
    /// A tagged property, by its ID (below 0x8000).
    Tag(u16),
    /// A named property: its set and its name in it.
    Named(GuidValue, Name<'a>),
}

/// A property given with a message or attachment.
#[derive(Clone, Copy, Debug)]
pub struct NewProperty<'a> {
    pub key: PropertyKey<'a>,
    pub value: NewValue<'a>,
}

impl<'a> NewProperty<'a> {
    pub fn tagged(id: u16, value: NewValue<'a>) -> Self {
        Self { key: PropertyKey::Tag(id), value }
    }

    pub fn named(set: GuidValue, name: Name<'a>, value: NewValue<'a>) -> Self {
        Self { key: PropertyKey::Named(set, name), value }
    }
}

/// A special folder a data file records (MS-OXOSFLD 2.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpecialFolder {
    DeletedItems,
    SentItems,
    Outbox,
    Drafts,
    JunkEmail,
    Calendar,
    Contacts,
    Journal,
    Notes,
    Tasks,
}

impl SpecialFolder {
    /// The folder's name in a new data file.
    pub fn default_name(self) -> &'static str {
        match self {
            Self::DeletedItems => "Deleted Items",
            Self::SentItems => "Sent Items",
            Self::Outbox => "Outbox",
            Self::Drafts => "Drafts",
            Self::JunkEmail => "Junk Email",
            Self::Calendar => "Calendar",
            Self::Contacts => "Contacts",
            Self::Journal => "Journal",
            Self::Notes => "Notes",
            Self::Tasks => "Tasks",
        }
    }

    /// What its folder holds (`PR_CONTAINER_CLASS`, MS-OXOSFLD 2.2.2).
    pub fn container_class(self) -> &'static str {
        match self {
            Self::Calendar => "IPF.Appointment",
            Self::Contacts => "IPF.Contact",
            Self::Journal => "IPF.Journal",
            Self::Notes => "IPF.StickyNote",
            Self::Tasks => "IPF.Task",
            _ => "IPF.Note",
        }
    }
}

/// Attached messages nest at most this deep.
pub const MAX_EMBEDDED_DEPTH: usize = 32;
/// The most a file read from a source can hold: `PidTagAttachSize` is a signed 32-bit integer.
pub(crate) const MAX_SOURCE_SIZE: u64 = i32::MAX as u64 - (1 << 20);
/// The longest string name, in UTF-16 units.
pub(crate) const MAX_NAME: usize = 1024;

/// Properties the writer works out itself.
const MESSAGE_OWNED: [u16; 5] = [0x001A, 0x0E08, 0x0E1B, 0x67F2, 0x67F3];
const ATTACHMENT_OWNED: [u16; 3] = [0x0E20, 0x3701, 0x3705];

fn refused<T>(what: &str) -> io::Result<T> {
    Err(io::Error::new(io::ErrorKind::InvalidInput, what))
}

/// A message's properties are an error unless they can be written.
pub(crate) fn check_message(m: &NewMessage<'_>) -> io::Result<()> {
    if m.message_class.is_empty() {
        return refused("message class must not be empty");
    }
    let mut tags = HashSet::new();
    let mut names = HashSet::new();
    for p in m.properties {
        match p.key {
            PropertyKey::Tag(id) => {
                if id == 0 || id >= 0x8000 {
                    return refused("a tagged property's ID must be from 0x0001 to 0x7FFF");
                }
                if MESSAGE_OWNED.contains(&id) {
                    return refused("the writer sets this property itself");
                }
                if !tags.insert(id) {
                    return refused("a property is given twice");
                }
            }
            PropertyKey::Named(set, name) => {
                if let Name::String(n) = name {
                    let units = n.encode_utf16().count();
                    if units == 0 || units > MAX_NAME {
                        return refused("a named property's name must have 1 to 1,024 characters");
                    }
                }
                if !names.insert((set.to_bytes(), name)) {
                    return refused("a property is given twice");
                }
            }
        }
    }
    Ok(())
}

/// Attachments are an error unless they can be written; `depth`: how deep their message is.
pub(crate) fn check_attachments(list: &[NewAttachment<'_>], depth: usize) -> io::Result<()> {
    for a in list {
        match &a.content {
            AttachmentContent::Data => {}
            _ if !a.data.is_empty() => return refused("an attachment's bytes are given twice"),
            AttachmentContent::Source(s) => {
                if s.size() > MAX_SOURCE_SIZE {
                    return refused("a file too big to attach");
                }
            }
            AttachmentContent::Message(m) => {
                if depth + 1 > MAX_EMBEDDED_DEPTH {
                    return refused("attached messages nested too deep");
                }
                check_message(&m.message)?;
                check_attachments(&m.attachments, depth + 1)?;
            }
        }
        if a.data.len() as u64 > MAX_SOURCE_SIZE {
            return refused("a file too big to attach");
        }
        let mut tags = HashSet::new();
        for p in a.properties {
            match p.key {
                PropertyKey::Tag(id) if id != 0 && id < 0x8000 && !ATTACHMENT_OWNED.contains(&id) => {
                    if !tags.insert(id) {
                        return refused("a property is given twice");
                    }
                }
                _ => return refused("an attachment's added property must be tagged and not the writer's own"),
            }
        }
    }
    Ok(())
}
