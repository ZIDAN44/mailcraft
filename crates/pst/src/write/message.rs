//! A message written: its properties, its recipients, its attachments and those attached to it.

use std::collections::BTreeMap;
use std::io;

use super::edit::Editor;
use super::heap::{BuiltProps, build_props, value_type};
use super::named::NamedIds;
use super::table::{Built, Cell, NewRow, ROW_ID, ROW_VERSION, Schema, build_table, kept_elsewhere};
use super::text::Text;
use super::types::*;
use crate::ltp::{Bytes, ObjectRef, PropertyType as P, PropertyValue};
use crate::ndb::{Format, SubNodes};
use crate::util::short;

const ATTACH_BY_VALUE: i32 = 1;
const ATTACH_EMBEDDED_MESSAGE: i32 = 5;
const MESSAGE_FLAG_READ: i32 = 0x01;
const MESSAGE_FLAG_HAS_ATTACH: i32 = 0x10;

const NID_RECIPIENT_TABLE: u32 = 0x0692;
const NID_ATTACHMENT_TABLE: u32 = 0x0671;
const NID_TYPE_ATTACHMENT: u32 = 0x05;
const NID_TYPE_INTERNAL: u32 = 0x01;
pub(crate) const NID_TYPE_NORMAL_MESSAGE: u32 = 0x04;

/// A message's values kept in subnodes of the message get indexes from here.
const FIRST_VALUE_NODE: u32 = 0x80;
const FIRST_ATTACHMENT: u32 = 0x100;
const FIRST_ATTACHMENT_DATA: u32 = 0x200;

/// How each message's strings are written: in Windows-1252 when all of its fit, else UTF-8.
pub(crate) fn text_of(format: Format, m: &NewMessage<'_>, attachments: &[NewAttachment<'_>]) -> Text {
    let mut strings = Vec::new();
    message_strings(m, attachments, &mut strings);
    Text::for_message(format, strings.into_iter())
}

/// Every string of the message, its attachments and the messages attached to it.
fn message_strings<'a>(m: &NewMessage<'a>, attachments: &[NewAttachment<'a>], out: &mut Vec<&'a str>) {
    out.extend([m.subject, m.sender_name, m.sender_email, m.body, m.message_id, m.message_class]);
    out.extend(m.recipients.iter().flat_map(|r| [r.name, r.email]));
    for p in m.properties {
        property_strings(p, out);
    }
    for a in attachments {
        out.extend([a.filename, a.mime_type]);
        out.extend(a.content_id);
        for p in a.properties {
            property_strings(p, out);
        }
        if let AttachmentContent::Message(inner) = &a.content {
            message_strings(&inner.message, &inner.attachments, out);
        }
    }
}

fn property_strings<'a>(p: &NewProperty<'a>, out: &mut Vec<&'a str>) {
    match p.value {
        NewValue::Unicode(s) => out.push(s),
        NewValue::MultipleUnicode(l) => out.extend(l.iter().copied()),
        _ => {}
    }
}

/// `PidTagAttachSize`: what the attachment holds and the room its names take, as Outlook counts
/// them, in UTF-16 with their NULs whatever the file's format.
fn attachment_size(a: &NewAttachment<'_>, content: usize) -> io::Result<i32> {
    let bytes = |s: &str| s.encode_utf16().count().saturating_add(1).saturating_mul(2);
    let cid = a.content_id.map(|c| c.trim_start_matches('<').trim_end_matches('>')).filter(|c| !c.is_empty());
    let size = content
        .saturating_add(12)
        .saturating_add(bytes(a.filename).saturating_mul(2))
        .saturating_add(bytes(a.mime_type))
        .saturating_add(cid.map_or(0, |c| bytes(c).saturating_add(4)));
    i32::try_from(size).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "an attachment too big to keep"))
}

/// `PidTagMessageFlags`: as given (read when not), with `mfHasAttach` as the message is.
pub(crate) fn message_flags(m: &NewMessage<'_>, has_attachments: bool) -> i32 {
    let given = m.properties.iter().find_map(|p| match (p.key, p.value) {
        (PropertyKey::Tag(0x0E07), NewValue::Integer32(f)) => Some(f),
        _ => None,
    });
    let attach = if has_attachments { MESSAGE_FLAG_HAS_ATTACH } else { 0 };
    (given.unwrap_or(MESSAGE_FLAG_READ) & !MESSAGE_FLAG_HAS_ATTACH) | attach
}

/// Whether a message goes in unread.
pub(crate) fn is_unread(m: &NewMessage<'_>) -> bool {
    message_flags(m, false) & MESSAGE_FLAG_READ == 0
}

fn display(m: &NewMessage<'_>, kind: RecipientType) -> String {
    m.recipients.iter().filter(|r| r.recipient_type == kind).map(|r| if r.name.is_empty() { r.email } else { r.name }).collect::<Vec<_>>().join("; ")
}

/// The writer's properties of a message, with those given in place of the same IDs'.
fn message_properties(
    m: &NewMessage<'_>,
    has_attachments: bool,
    named: &NamedIds,
    nid: u32,
    version: u32,
    size: i32,
    text: Text,
) -> io::Result<Vec<(u16, PropertyValue)>> {
    use PropertyValue as V;
    let s = |v: &str| text.value(v);
    let mut props = vec![
        (0x001A, s(m.message_class)),
        (0x0037, s(m.subject)),
        (0x0039, V::Time(m.delivery_time)),
        (0x0042, s(m.sender_name)),
        (0x0070, s(m.subject)),
        (0x0C1A, s(m.sender_name)),
        (0x0C1F, s(m.sender_email)),
        (0x0E06, V::Time(m.delivery_time)),
        (0x0E07, V::Integer32(message_flags(m, has_attachments))),
        (0x0E08, V::Integer32(size)),
        (0x0E17, V::Integer32(0)),
        (0x0E1B, V::Boolean(has_attachments)),
        (0x1000, s(m.body)),
        (0x1035, s(m.message_id)),
        (0x3007, V::Time(m.delivery_time)),
        (0x3008, V::Time(m.delivery_time)),
        (0x300B, V::Binary(Bytes(m.message_id.as_bytes().to_vec()))),
        (0x3FDE, V::Integer32(65001)),
        #[allow(clippy::cast_possible_wrap)] // a node ID, kept as these are
        (ROW_ID, V::Integer32(nid as i32)),
        #[allow(clippy::cast_possible_wrap)]
        (ROW_VERSION, V::Integer32(version as i32)),
    ];
    for (id, kind) in [(0x0E02, RecipientType::Bcc), (0x0E03, RecipientType::Cc), (0x0E04, RecipientType::To)] {
        let list = display(m, kind);
        if !list.is_empty() {
            props.push((id, s(&list)));
        }
    }
    if let Some(html) = m.html_body {
        props.push((0x1013, V::Binary(Bytes(html.as_bytes().to_vec()))));
    }
    for p in m.properties {
        let id = named.property_id(&p.key)?;
        if id == 0x0E07 {
            continue;
        }
        props.retain(|(existing, _)| *existing != id);
        props.push((id, text.narrow(p.value.property_value())));
    }
    if let Some(page) = text.code_page() {
        props.retain(|(id, _)| *id != 0x3FFD);
        props.push((0x3FFD, V::Integer32(page)));
    }
    Ok(props)
}

fn one_off_entry_id(r: &NewRecipient<'_>) -> Vec<u8> {
    let utf16 = |s: &str| s.encode_utf16().chain(Some(0)).flat_map(u16::to_le_bytes).collect::<Vec<u8>>();
    let mut id = vec![0; 4];
    id.extend_from_slice(&[0x81, 0x2B, 0x1F, 0xA4, 0xBE, 0xA3, 0x10, 0x19, 0x9D, 0x6E, 0x00, 0xDD, 0x01, 0x0F, 0x54, 0x02]);
    id.extend_from_slice(&0u16.to_le_bytes());
    id.extend_from_slice(&0x8000u16.to_le_bytes());
    id.extend(utf16(if r.name.is_empty() { r.email } else { r.name }));
    id.extend(utf16("SMTP"));
    id.extend(utf16(r.email));
    id
}

fn int(n: i32) -> Option<Cell> {
    Some(Cell::Fixed(n.to_le_bytes().to_vec()))
}

fn boolean(b: bool) -> Option<Cell> {
    Some(Cell::Fixed(vec![u8::from(b)]))
}

fn string(kind: u16, text: Text, s: &str) -> Option<Cell> {
    text.cell(kind, s).map(Cell::Bytes)
}

fn recipient_table(format: Format, list: &[NewRecipient<'_>], version: u32, text: Text) -> io::Result<Built> {
    let st = text.string_type();
    let columns = [
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
        (0x39FE, st),
        (0x39FF, st),
        (0x3A40, P::BOOLEAN),
        (0x5FFD, P::INT32),
        (0x5FFF, P::INT32),
    ];
    let schema = Schema::new(&columns)?;
    let mut rows = Vec::with_capacity(list.len());
    for (i, r) in list.iter().enumerate() {
        let id = u32::try_from(i + 1).map_err(|_| short())?;
        let name = if r.name.is_empty() { r.email } else { r.name };
        let mut key = format!("SMTP:{}", r.email).to_ascii_uppercase().into_bytes();
        key.push(0);
        let cells = schema
            .columns
            .iter()
            .map(|c| match c.prop_id() {
                ROW_ID => int(0),
                #[allow(clippy::cast_possible_wrap)]
                ROW_VERSION => int(version as i32),
                0x0C15 => int(r.recipient_type as i32),
                0x0E0F => boolean(true),
                0x0FFE => int(6),
                0x0FFF => Some(Cell::Bytes(one_off_entry_id(r))),
                0x3001 | 0x39FF => string(st, text, name),
                0x3002 => string(st, text, "SMTP"),
                0x3003 | 0x39FE => string(st, text, r.email),
                0x300B => Some(Cell::Bytes(key.clone())),
                0x3900 => int(0),
                0x3A40 => boolean(false),
                0x5FFD => r.flags.and_then(int),
                0x5FFF => r.track_status.and_then(int),
                _ => None,
            })
            .collect();
        rows.push(NewRow { id, cells });
    }
    build_table(format, &schema, &rows, 0)
}

/// The bytes a built table takes.
fn table_bytes(b: &Built) -> usize {
    b.blocks.iter().map(Vec::len).sum::<usize>()
        + b.rows.as_ref().map_or(0, |(_, r)| r.iter().map(Vec::len).sum())
        + b.external.iter().map(|(_, d)| d.len()).sum::<usize>()
}

fn props_bytes(b: &BuiltProps) -> usize {
    b.blocks.iter().map(Vec::len).sum::<usize>() + b.external.iter().map(|(_, d)| d.len()).sum::<usize>()
}

impl Editor {
    fn write_props_into(&mut self, built: BuiltProps, subs: &mut SubNodes) -> io::Result<u64> {
        for (nid, bytes) in built.external {
            let bid = self.write_tree(&bytes)?;
            subs.insert(nid, (bid, 0));
        }
        self.write_leaves(&built.blocks)
    }

    fn write_table_into(&mut self, nid: u32, built: Built, subs: &mut SubNodes) -> io::Result<()> {
        let inner = BTreeMap::new();
        let (data, sub) = self.write_built(built, inner)?;
        subs.insert(nid, (data, sub));
        Ok(())
    }

    /// Writes the attachments, into the message's subnodes `subs`: the attachment table's bytes
    /// and how many bytes they all take.
    fn write_attachments(
        &mut self,
        list: &[NewAttachment<'_>],
        named: &NamedIds,
        version: u32,
        text: Text,
        subs: &mut SubNodes,
    ) -> io::Result<(Built, usize)> {
        let format = self.format();
        let st = text.string_type();
        let schema = Schema::new(&[(0x0E20, P::INT32), (0x3704, st), (0x3705, P::INT32), (0x370B, P::INT32)])?;
        let mut rows = Vec::with_capacity(list.len());
        let mut total = 0usize;
        for (i, a) in list.iter().enumerate() {
            let i = u32::try_from(i).map_err(|_| short())?;
            let node = ((FIRST_ATTACHMENT + i) << 5) | NID_TYPE_ATTACHMENT;
            let mut own = SubNodes::new();
            let (size, method, built) = match &a.content {
                AttachmentContent::Message(em) => {
                    let inner = self.next_node_id(NID_TYPE_NORMAL_MESSAGE)?;
                    let (data, sub, content) = self.write_message(&em.message, &em.attachments, named, inner, version, text)?;
                    own.insert(inner, (data, sub));
                    let content32 = u32::try_from(content).map_err(|_| short())?;
                    let mut props = vec![
                        (0x3001u16, text.value(a.filename)),
                        (0x3701, PropertyValue::Object(ObjectRef { node: inner, size: content32 })),
                        (0x3705, PropertyValue::Integer32(ATTACH_EMBEDDED_MESSAGE)),
                        (0x370B, PropertyValue::Integer32(-1)),
                    ];
                    add_attachment_properties(a, &mut props, text);
                    (content, ATTACH_EMBEDDED_MESSAGE, finish_attachment(format, props, None, attachment_size(a, content)?)?)
                }
                content => {
                    let (bytes, node_id) = self.write_attachment_bytes(a, content, i, &mut own)?;
                    let mut props = vec![
                        (0x3704u16, text.value(a.filename)),
                        (0x3705, PropertyValue::Integer32(ATTACH_BY_VALUE)),
                        (0x3707, text.value(a.filename)),
                        (0x370B, PropertyValue::Integer32(-1)),
                        (0x370E, text.value(a.mime_type)),
                    ];
                    if let Some(cid) = a.content_id {
                        let cid = cid.trim_start_matches('<').trim_end_matches('>');
                        if !cid.is_empty() {
                            props.push((0x3712, text.value(cid)));
                            props.push((0x3714, PropertyValue::Integer32(4)));
                        }
                    }
                    add_attachment_properties(a, &mut props, text);
                    (bytes, ATTACH_BY_VALUE, finish_attachment(format, props, node_id, attachment_size(a, bytes)?)?)
                }
            };
            total = total.checked_add(size).and_then(|t| t.checked_add(props_bytes(&built))).ok_or_else(short)?;
            let size_cell = attachment_size(a, size)?;
            let data = self.write_props_into(built, &mut own)?;
            let sub = self.write_subnodes(&own)?;
            subs.insert(node, (data, sub));
            let cells = schema
                .columns
                .iter()
                .map(|c| match c.prop_id() {
                    #[allow(clippy::cast_possible_wrap)]
                    ROW_ID => int(node as i32),
                    #[allow(clippy::cast_possible_wrap)]
                    ROW_VERSION => int(version as i32),
                    0x0E20 => int(size_cell),
                    0x3704 if method == ATTACH_BY_VALUE => string(st, text, a.filename),
                    0x3705 => int(method),
                    0x370B => int(-1),
                    _ => None,
                })
                .collect();
            rows.push(NewRow { id: node, cells });
        }
        Ok((build_table(format, &schema, &rows, 0)?, total))
    }

    /// Writes an attachment's file and says how many bytes it is and in which subnode.
    fn write_attachment_bytes(
        &mut self,
        a: &NewAttachment<'_>,
        content: &AttachmentContent<'_>,
        i: u32,
        own: &mut SubNodes,
    ) -> io::Result<(usize, Option<u32>)> {
        let node = ((FIRST_ATTACHMENT_DATA + i) << 5) | NID_TYPE_INTERNAL;
        let (bid, size) = match content {
            AttachmentContent::Source(source) => {
                let size = source.size();
                if size == 0 {
                    return Ok((0, None));
                }
                let mut r = source.open()?;
                (self.write_stream(&mut r, size)?, crate::util::size(size)?)
            }
            _ => {
                if a.data.is_empty() {
                    return Ok((0, None));
                }
                (self.write_tree(a.data)?, a.data.len())
            }
        };
        own.insert(node, (bid, 0));
        Ok((size, Some(node)))
    }

    /// Writes a message's nodes: its data, its subnode tree, and the bytes it takes.
    pub(crate) fn write_message(
        &mut self,
        m: &NewMessage<'_>,
        attachments: &[NewAttachment<'_>],
        named: &NamedIds,
        nid: u32,
        version: u32,
        text: Text,
    ) -> io::Result<(u64, u64, usize)> {
        let format = self.format();
        let mut subs = SubNodes::new();
        let recipients = recipient_table(format, m.recipients, version, text)?;
        let mut total = table_bytes(&recipients);
        self.write_table_into(NID_RECIPIENT_TABLE, recipients, &mut subs)?;
        if !attachments.is_empty() {
            let (table, bytes) = self.write_attachments(attachments, named, version, text, &mut subs)?;
            total = total.checked_add(table_bytes(&table)).and_then(|t| t.checked_add(bytes)).ok_or_else(short)?;
            self.write_table_into(NID_ATTACHMENT_TABLE, table, &mut subs)?;
        }
        // Built twice: its size is one of its own properties, of a fixed width.
        let sized = build_props(format, message_properties(m, !attachments.is_empty(), named, nid, version, 0, text)?, FIRST_VALUE_NODE)?;
        total = total.checked_add(props_bytes(&sized)).ok_or_else(short)?;
        // Outlook leaves the heap's six-byte header out of a message's size.
        let size = i32::try_from(total.saturating_sub(6)).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "a message too big to keep"))?;
        let built = build_props(format, message_properties(m, !attachments.is_empty(), named, nid, version, size, text)?, FIRST_VALUE_NODE)?;
        let data = self.write_props_into(built, &mut subs)?;
        let sub = self.write_subnodes(&subs)?;
        Ok((data, sub, total))
    }
}

fn add_attachment_properties(a: &NewAttachment<'_>, props: &mut Vec<(u16, PropertyValue)>, text: Text) {
    for p in a.properties {
        if let PropertyKey::Tag(id) = p.key {
            props.retain(|(existing, _)| *existing != id);
            props.push((id, text.narrow(p.value.property_value())));
        }
    }
}

/// An attachment's property set, `0x3701` naming the subnode that holds its bytes.
fn finish_attachment(format: Format, mut props: Vec<(u16, PropertyValue)>, data_node: Option<u32>, size: i32) -> io::Result<BuiltProps> {
    props.retain(|(id, _)| *id != 0x0E20);
    props.push((0x0E20, PropertyValue::Integer32(size)));
    if !props.iter().any(|(id, _)| *id == 0x3701) {
        props.push((0x3701, PropertyValue::Binary(Bytes::default())));
    }
    super::heap::build_props_linked(format, props, FIRST_VALUE_NODE, data_node.map(|n| (0x3701, P::BINARY, n)))
}

/// The cell of a message's row in its folder's contents table for a column.
#[allow(clippy::too_many_arguments)]
pub(crate) fn contents_cell(
    id: u16,
    kind: u16,
    m: &NewMessage<'_>,
    has_attachments: bool,
    named: &NamedIds,
    record_key: &[u8; 16],
    folder: u32,
    nid: u32,
    version: u32,
    size: i32,
    text: Text,
) -> Option<Cell> {
    // An added property's value, as the message has it.
    if id != 0x0E07 {
        for p in m.properties {
            if named.property_id(&p.key).ok() == Some(id) {
                if let NewValue::Unicode(s) = p.value {
                    return string(kind, text, s);
                }
                let v = text.narrow(p.value.property_value());
                if value_type(&v) != kind {
                    return None;
                }
                let bytes = super::heap::value_bytes(&v);
                return Some(if kept_elsewhere(kind) { Cell::Bytes(bytes) } else { Cell::Fixed(bytes) });
            }
        }
    }
    let entry = |node: u32| {
        let mut e = vec![0u8; 4];
        e.extend_from_slice(record_key);
        e.extend(node.to_le_bytes());
        e
    };
    let list = |t: RecipientType| {
        let d = display(m, t);
        if d.is_empty() { None } else { string(kind, text, &d) }
    };
    match (id, kind) {
        #[allow(clippy::cast_possible_wrap)]
        (ROW_ID, P::INT32) => int(nid as i32),
        #[allow(clippy::cast_possible_wrap)]
        (ROW_VERSION, P::INT32) => int(version as i32),
        (0x001A, _) => string(kind, text, m.message_class),
        (0x0037 | 0x0E1D | 0x0070 | 0x3001, _) => string(kind, text, m.subject),
        (0x0042 | 0x0C1A, _) => string(kind, text, m.sender_name),
        (0x0C1F, _) => string(kind, text, m.sender_email),
        (0x0E02, _) => list(RecipientType::Bcc),
        (0x0E03, _) => list(RecipientType::Cc),
        (0x0E04, _) => list(RecipientType::To),
        (0x1035, _) => string(kind, text, m.message_id),
        (0x0039 | 0x0E06 | 0x3008, P::TIME) => Some(Cell::Fixed(m.delivery_time.to_le_bytes().to_vec())),
        (0x0E17, P::INT32) => int(0),
        (0x0E07, P::INT32) => int(message_flags(m, has_attachments)),
        (0x0E08, P::INT32) => int(size),
        (0x0E1B, P::BOOLEAN) => boolean(has_attachments),
        (0x3FDE, P::INT32) => int(65001),
        (0x0E09, P::BINARY) => Some(Cell::Bytes(entry(folder))),
        (0x0FFF, P::BINARY) => Some(Cell::Bytes(entry(nid))),
        (0x300B, P::BINARY) => Some(Cell::Bytes(m.message_id.as_bytes().to_vec())),
        _ => None,
    }
}
