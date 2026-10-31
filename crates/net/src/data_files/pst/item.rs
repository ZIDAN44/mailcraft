//! An item as read from the file: its properties, recipients and attachments.

use super::props::{Val, decode, val};
use super::*;

/// A mail item, as read.
#[derive(Default)]
pub(super) struct Item {
    pub(super) class: String,
    pub(super) subject: String,
    pub(super) from: EmailAddress,
    pub(super) to: Vec<EmailAddress>,
    pub(super) cc: Vec<EmailAddress>,
    pub(super) bcc: Vec<EmailAddress>,
    pub(super) date: i64,
    pub(super) body: String,
    pub(super) html: Option<String>,
    pub(super) message_id: String,
    pub(super) in_reply_to: String,
    pub(super) references: Vec<String>,
    pub(super) headers: String,
    pub(super) flags: Flags,
    pub(super) files: Vec<File>,
    /// A contact's or appointment's own properties.
    pub(super) extra: Option<crate::pst_pim::Extra>,
}

pub(super) enum File {
    Data { name: String, mime: String, cid: String, data: Vec<u8> },
    Message(Box<Item>),
}

pub(super) fn item(m: &dyn Message, files: Vec<File>, named: &crate::pst_pim::Named) -> Item {
    let props = m.properties();
    let p = |id: u16| props.get(id).map(|v| val(v, 0));
    let table = m.recipient_table().map(|t| rows(&**t)).unwrap_or_default();
    let recipients: Vec<_> = table.iter().map(|row| move |id: u16| row.get(&id).map(|v| val(v, 0))).collect();
    item_of(&p, &recipients, files, named)
}

/// An item from its properties (`p`) and its recipients' (one getter per recipient).
pub(super) fn item_of<'a>(
    p: &dyn Fn(u16) -> Option<Val<'a>>,
    recipients: &[impl Fn(u16) -> Option<Val<'a>>],
    files: Vec<File>,
    named: &crate::pst_pim::Named,
) -> Item {
    let s = |id: u16| p(id).and_then(Val::into_text).unwrap_or_default();
    let mut it = Item { class: s(0x001A), files, ..Default::default() };
    let class = it.class.to_ascii_lowercase();
    if class.starts_with("ipm.contact")
        || class.starts_with("ipm.appointment")
        || class.starts_with("ipm.task")
        || class.starts_with("ipm.stickynote")
    {
        it.extra = Some(crate::pst_pim::extract(p, named));
    }
    // A subject may start with a prefix marker (0x01, length).
    it.subject = {
        let raw = s(0x0037);
        if raw.starts_with('\u{1}') { raw.chars().skip(2).collect() } else { raw }
    };
    let email = |ids: &[u16]| ids.iter().map(|id| s(*id)).find(|e| e.contains('@')).unwrap_or_default();
    let name = |ids: &[u16]| ids.iter().map(|id| s(*id)).find(|n| !n.is_empty()).unwrap_or_default();
    it.from = EmailAddress::new(name(&[0x0C1A, 0x0042]), email(&[0x5D01, 0x0C1F, 0x5D02, 0x0065]));
    it.date = [0x0E06, 0x0039, 0x3007].iter().find_map(|id| p(*id).and_then(|v| v.time())).unwrap_or(0);
    it.body = s(0x1000);
    it.html = p(0x1013).and_then(|v| match v {
        Val::Bin(b) => {
            let page = match p(0x3FDE) {
                Some(Val::Int(n)) => u32::try_from(n).unwrap_or(0),
                _ => 0,
            };
            Some(decode(b, page))
        }
        other => other.into_text(),
    });
    if it.html.as_deref().is_none_or(|h| h.trim().is_empty())
        && let Some(Val::Bin(rtf)) = p(0x1009)
    {
        rtf_into(&mut it, rtf);
    }
    it.message_id = s(0x1035);
    it.in_reply_to = s(0x1042);
    it.references = s(0x1039).split_whitespace().map(str::to_string).collect();
    it.headers = s(0x007D);
    let flags = match p(0x0E07) {
        Some(Val::Int(n)) => n,
        _ => 0,
    };
    let follow_up = matches!(p(0x1090), Some(Val::Int(2)));
    let replied = matches!(p(0x1081), Some(Val::Int(n)) if n == 0x0066 || n == 0x0067);
    // MSGFLAG_READ, MSGFLAG_UNSENT.
    it.flags = Flags { seen: flags & 0x1 != 0, draft: flags & 0x8 != 0, flagged: follow_up, answered: replied, ..Default::default() };
    for row in recipients {
        let get = |id: u16| row(id).and_then(Val::into_text).unwrap_or_default();
        let address = [get(0x39FE), get(0x3003)].into_iter().find(|e| e.contains('@'));
        let Some(address) = address else { continue };
        let a = EmailAddress::new(get(0x3001), address);
        match row(0x0C15) {
            Some(Val::Int(2)) => it.cc.push(a),
            Some(Val::Int(3)) => it.bcc.push(a),
            _ => it.to.push(a),
        }
    }
    it
}

/// A body kept as compressed RTF (`PidTagRtfCompressed`) and no HTML: the HTML it wraps, or,
/// when the item has no text, its own formatting or its text. Its pictures are then inline, as
/// the HTML names their `cid:`. A plain body is kept over RTF of its own, whose simple HTML
/// leaves out tables, pictures and objects the text still has.
fn rtf_into(it: &mut Item, compressed: &[u8]) {
    let plain = !it.body.trim().is_empty();
    let rtf = mime::decompress_rtf(compressed);
    if plain && !rtf.as_deref().is_some_and(mime::rtf_wraps_html) {
        return;
    }
    match rtf.map(|rtf| mime::rtf_body(&rtf)) {
        Some(mime::TnefBody::Html(h)) => it.html = Some(h),
        Some(mime::TnefBody::Text(t)) if !plain && !t.trim().is_empty() => it.body = t,
        _ if plain => {}
        _ => log::info!("a Rich Text body could not be read"),
    }
}

/// An attachment's file, or the message it holds.
pub(super) fn attachment(a: &dyn Attachment, nesting: usize, named: &crate::pst_pim::Named) -> Option<File> {
    let props = a.properties();
    let p = |id: u16| props.get(id).map(|v| val(v, 0));
    let s = |id: u16| p(id).and_then(Val::into_text).unwrap_or_default();
    // Hidden attachments (e.g. a contact picture) aren't part of the message.
    if matches!(p(0x7FFE), Some(Val::Bool(true))) && s(0x3712).is_empty() {
        return None;
    }
    match a.data()? {
        AttachmentData::Binary(b) => {
            if b.buffer().len() > MAX_ATTACHMENT {
                log::info!("an attachment of {} bytes was left out", b.buffer().len());
                return None;
            }
            let name = [s(0x3707), s(0x3704), s(0x3001)].into_iter().find(|n| !n.is_empty()).unwrap_or_else(|| "attachment".into());
            Some(File::Data { name, mime: s(0x370E), cid: s(0x3712), data: b.buffer().to_vec() })
        }
        AttachmentData::Message(m) if nesting < MAX_NESTING => {
            // The embedded message's own attachments need its concrete type: left out.
            Some(File::Message(Box::new(item(&**m, Vec::new(), named))))
        }
        AttachmentData::Message(_) => None,
    }
}

/// A table's rows as property maps.
fn rows(table: &dyn TableContext) -> Vec<HashMap<u16, PropertyValue>> {
    let context = table.context();
    let mut out = Vec::new();
    for row in table.rows_matrix() {
        let Ok(values) = row.columns(context) else { continue };
        let mut map = HashMap::new();
        for (column, value) in context.columns().iter().zip(values) {
            if let Some(v) = value
                && let Ok(pv) = table.read_column(&v, column.prop_type())
            {
                map.insert(column.prop_id(), pv);
            }
        }
        out.push(map);
    }
    out
}
