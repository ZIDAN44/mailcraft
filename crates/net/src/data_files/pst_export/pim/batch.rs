//! Items as the writer takes them: their properties, recipients and files, and the items
//! attached to them (an appointment's changed occurrences), written a batch at a time.

use super::*;
use outlook_pst::{UnicodePstAttachmentContent, UnicodePstEmbeddedMessage};

/// The multi-valued properties' lists of an item, in the order of its properties.
pub(in super::super) struct Lists<'a> {
    texts: Vec<Vec<&'a str>>,
    bins: Vec<Vec<&'a [u8]>>,
}

/// The lists of `props`' multi-valued properties.
pub(in super::super) fn lists(props: &[(Key, Value)]) -> Lists<'_> {
    let mut l = Lists { texts: Vec::new(), bins: Vec::new() };
    for (_, v) in props {
        match v {
            Value::Texts(t) => l.texts.push(t.iter().map(String::as_str).collect()),
            Value::Bins(b) => l.bins.push(b.iter().map(Vec::as_slice).collect()),
            _ => {}
        }
    }
    l
}

/// A single value as the writer takes it (`None` for a list).
fn value(v: &Value) -> Option<UnicodePstValue<'_>> {
    Some(match v {
        Value::Int(n) => UnicodePstValue::Integer32(*n),
        Value::Bool(b) => UnicodePstValue::Boolean(*b),
        Value::Time(t) => UnicodePstValue::Time(*t),
        Value::Float(f) => UnicodePstValue::Floating64(*f),
        Value::Text(s) => UnicodePstValue::Unicode(s),
        Value::Bin(b) => UnicodePstValue::Binary(b),
        Value::Texts(_) | Value::Bins(_) => return None,
    })
}

/// Properties (`props`, whose lists are `lists`) as the writer takes them.
pub(in super::super) fn properties<'a>(props: &'a [(Key, Value)], lists: &'a Lists<'a>) -> Vec<UnicodePstProperty<'a>> {
    let (mut texts, mut bins) = (lists.texts.iter(), lists.bins.iter());
    let mut out = Vec::new();
    for (key, v) in props {
        let value = match (v, value(v)) {
            (_, Some(single)) => single,
            (Value::Texts(_), None) => match texts.next() {
                Some(t) => UnicodePstValue::MultipleUnicode(t),
                None => continue,
            },
            (_, None) => match bins.next() {
                Some(b) => UnicodePstValue::MultipleBinary(b),
                None => continue,
            },
        };
        out.push(match key {
            Key::Tag(id) => UnicodePstProperty::tagged(*id, value),
            Key::Lid(set, id) => UnicodePstProperty::named(*set, UnicodePstName::Id(*id), value),
            Key::Keywords => UnicodePstProperty::named(PS_PUBLIC_STRINGS, UnicodePstName::String("Keywords"), value),
            Key::Exported => UnicodePstProperty::named(PS_PUBLIC_STRINGS, UnicodePstName::String(crate::pst_pim::EXPORTED_ITEM), value),
        });
    }
    out
}

/// An item's recipients as the writer takes them.
fn recipients(item: &Item) -> Vec<UnicodePstRecipient<'_>> {
    item.people
        .iter()
        .map(|p| UnicodePstRecipient { name: &p.name, email: &p.email, recipient_type: p.kind, flags: p.flags, track_status: p.track })
        .collect()
}

/// What the writer takes of an item, built before it borrows it: its lists, then its properties
/// and recipients; the same for each item attached to it, and the attachment's own properties.
struct Parts<'a> {
    lists: Lists<'a>,
    attached: Vec<(Lists<'a>, Vec<UnicodePstProperty<'a>>)>,
}

/// An item's files as attachments; `picture`: the properties of a contact's picture.
fn files<'a>(item: &'a Item, picture: &'a [UnicodePstProperty<'a>]) -> Vec<UnicodePstAttachment<'a>> {
    item.files
        .iter()
        .map(|f| UnicodePstAttachment {
            filename: &f.name,
            mime_type: &f.mime,
            data: &f.data,
            properties: if f.picture { picture } else { &[] },
            ..Default::default()
        })
        .collect()
}

/// The message the writer writes for `item`, of its properties `p` and recipients `r`.
fn message<'a>(item: &'a Item, p: &'a [UnicodePstProperty<'a>], r: &'a [UnicodePstRecipient<'a>]) -> UnicodePstMessage<'a> {
    UnicodePstMessage {
        subject: &item.subject,
        sender_name: &item.sender.0,
        sender_email: &item.sender.1,
        recipients: r,
        body: &item.body,
        html_body: None,
        message_id: "",
        delivery_time: item.modified,
        message_class: item.class,
        properties: p,
    }
}

pub(super) fn write_batch(path: &Path, format: Format, folder: &[&str], batch: &[Item]) -> Result<(), String> {
    // PidTagAttachmentContactPhoto, and hidden, as Outlook marks a contact's picture.
    let picture =
        [UnicodePstProperty::tagged(0x7FFF, UnicodePstValue::Boolean(true)), UnicodePstProperty::tagged(0x7FFE, UnicodePstValue::Boolean(true))];
    let parts: Vec<Parts<'_>> = batch
        .iter()
        .map(|item| Parts {
            lists: lists(&item.props),
            attached: item
                .attached
                .iter()
                .map(|a| (lists(&a.item.props), a.props.iter().filter_map(|(id, v)| Some(UnicodePstProperty::tagged(*id, value(v)?))).collect()))
                .collect(),
        })
        .collect();
    let own: Vec<Vec<UnicodePstProperty<'_>>> = batch.iter().zip(&parts).map(|(item, p)| properties(&item.props, &p.lists)).collect();
    let people: Vec<Vec<UnicodePstRecipient<'_>>> = batch.iter().map(recipients).collect();
    // Each attached item's properties and recipients.
    let inner: Vec<Vec<(Vec<UnicodePstProperty<'_>>, Vec<UnicodePstRecipient<'_>>)>> = batch
        .iter()
        .zip(&parts)
        .map(|(item, p)| item.attached.iter().zip(&p.attached).map(|(a, (l, _))| (properties(&a.item.props, l), recipients(&a.item))).collect())
        .collect();
    let attachments: Vec<Vec<UnicodePstAttachment<'_>>> = batch
        .iter()
        .zip(&parts)
        .zip(&inner)
        .map(|((item, p), inner)| {
            let mut out = files(item, &picture);
            for ((a, (_, own)), (props, people)) in item.attached.iter().zip(&p.attached).zip(inner) {
                let embedded = UnicodePstEmbeddedMessage { message: message(&a.item, props, people), attachments: files(&a.item, &picture) };
                out.push(UnicodePstAttachment {
                    filename: &a.item.subject,
                    content: UnicodePstAttachmentContent::Message(Box::new(embedded)),
                    properties: own,
                    ..Default::default()
                });
            }
            out
        })
        .collect();
    let messages: Vec<UnicodePstBatchMessage<'_>> = batch
        .iter()
        .zip(&own)
        .zip(&people)
        .zip(&attachments)
        .map(|(((item, p), r), f)| UnicodePstBatchMessage { message: message(item, p, r), attachments: f })
        .collect();
    append(path, format, folder, &messages)
}
