//! Handing a batch of messages to the writer: each with its recipients, properties and files,
//! its attached messages with theirs in turn, and its big files read from the files they wait in
//! as the writer comes to them ([`super::files`]).

use outlook_pst::{UnicodePstAttachmentContent, UnicodePstEmbeddedMessage};

use super::files::{File, StoredFile};
use super::*;

/// What a message's attachments borrow while its batch is written: the recipients and
/// properties of each message attached to it and the sources of its big files, in the order its
/// files list them.
struct Held<'a> {
    recipients: Vec<UnicodePstRecipient<'a>>,
    properties: Vec<UnicodePstProperty<'a>>,
    sources: Vec<StoredFile<'a>>,
    inner: Vec<Held<'a>>,
}

/// What `m`'s attachments borrow. `top`: it is the message written, whose properties are the
/// batch's own.
fn held(m: &Owned, top: bool) -> Held<'_> {
    Held {
        recipients: recipients(m),
        // An attached message has no state in the store: no categories, nor a meeting's.
        properties: if top { Vec::new() } else { item::properties(m, &[]) },
        sources: m
            .files
            .iter()
            .filter_map(|f| match f {
                File::Stored { size, file, .. } => Some(StoredFile { file, size: *size }),
                _ => None,
            })
            .collect(),
        inner: m
            .files
            .iter()
            .filter_map(|f| match f {
                File::Message { message, .. } => Some(held(message, false)),
                _ => None,
            })
            .collect(),
    }
}

fn recipients(m: &Owned) -> Vec<UnicodePstRecipient<'_>> {
    m.recipients.iter().map(|(name, email, kind)| UnicodePstRecipient { name, email, recipient_type: *kind, ..Default::default() }).collect()
}

fn message<'a>(m: &'a Owned, recipients: &'a [UnicodePstRecipient<'a>], properties: &'a [UnicodePstProperty<'a>]) -> UnicodePstMessage<'a> {
    UnicodePstMessage {
        subject: &m.subject,
        sender_name: &m.from.0,
        sender_email: &m.from.1,
        recipients,
        body: &m.body,
        html_body: m.html.as_deref(),
        message_id: &m.message_id,
        delivery_time: m.received,
        message_class: m.class(),
        properties,
    }
}

/// `m`'s attachments, as the writer takes them.
fn attachments<'a>(m: &'a Owned, h: &'a Held<'a>) -> Vec<UnicodePstAttachment<'a>> {
    let mut sources = h.sources.iter();
    let mut inner = h.inner.iter();
    let mut out = Vec::new();
    for f in &m.files {
        out.push(match f {
            File::Data { name, mime, cid, data } => {
                UnicodePstAttachment { filename: name, mime_type: mime, content_id: cid.as_deref(), data, ..Default::default() }
            }
            File::Stored { name, mime, cid, .. } => {
                let Some(source) = sources.next() else { continue };
                UnicodePstAttachment {
                    filename: name,
                    mime_type: mime,
                    content_id: cid.as_deref(),
                    content: UnicodePstAttachmentContent::Source(source),
                    ..Default::default()
                }
            }
            File::Message { name, message: attached } => {
                let Some(h) = inner.next() else { continue };
                let embedded =
                    UnicodePstEmbeddedMessage { message: message(attached, &h.recipients, &h.properties), attachments: attachments(attached, h) };
                UnicodePstAttachment {
                    filename: name,
                    mime_type: "message/rfc822",
                    content: UnicodePstAttachmentContent::Message(Box::new(embedded)),
                    ..Default::default()
                }
            }
        });
    }
    out
}

/// Write `batch` into `folder` of the file at `path`, made with the first message unless it's
/// `created`.
pub(super) fn write(path: &Path, folder: &[&str], batch: &[Owned], created: &mut bool) -> Result<(), String> {
    let held: Vec<Held<'_>> = batch.iter().map(|m| held(m, true)).collect();
    let files: Vec<Vec<UnicodePstAttachment<'_>>> = batch.iter().zip(&held).map(|(m, h)| attachments(m, h)).collect();
    let categories: Vec<Vec<&str>> = batch.iter().map(|m| m.state.categories.iter().map(String::as_str).collect()).collect();
    // A meeting's own properties beside the message's.
    let none = Vec::new();
    let meetings: Vec<&Vec<(pim::Key, pim::Value)>> = batch.iter().map(|m| m.meeting.as_ref().map_or(&none, |x| &x.props)).collect();
    let lists: Vec<pim::Lists<'_>> = meetings.iter().map(|p| pim::lists(p)).collect();
    let properties: Vec<Vec<UnicodePstProperty<'_>>> = batch
        .iter()
        .zip(&categories)
        .zip(meetings.iter().zip(&lists))
        .map(|((m, c), (p, l))| [item::properties(m, c), pim::properties(p, l)].concat())
        .collect();
    let messages: Vec<UnicodePstBatchMessage<'_>> = batch
        .iter()
        .zip(&held)
        .zip(&files)
        .zip(&properties)
        .map(|(((m, h), f), p)| UnicodePstBatchMessage { message: message(m, &h.recipients, p), attachments: f })
        .collect();
    let (first, rest) = match (*created, messages.split_first()) {
        (_, None) => return Ok(()),
        (true, _) => (None, messages.as_slice()),
        (false, Some((first, rest))) => (Some(first), rest),
    };
    if let Some(first) = first {
        UnicodePstFile::create_in_folder_with_attachments(path, folder, &first.message, first.attachments).map_err(|e| e.to_string())?;
        *created = true;
    }
    if !rest.is_empty() {
        append(path, folder, rest)?;
    }
    Ok(())
}

/// Add `batch` to the file at `path`. A file whose name-to-ID map can't take names (damaged, or
/// made by a program that keeps it some other way) gets the batch without its named properties:
/// the mail goes in, without its follow-up text, due days and categories, and the log says why.
pub(super) fn append(path: &Path, folder: &[&str], batch: &[UnicodePstBatchMessage<'_>]) -> Result<(), String> {
    let Err(e) = UnicodePstFile::append_many_in_folder_with_attachments(path, folder, batch) else { return Ok(()) };
    let named = |p: &UnicodePstProperty<'_>| matches!(p.key, UnicodePstPropertyKey::Named(..));
    // The fork's writer says so before it writes any of the batch's messages (their folder may be
    // made by then, and stays).
    if !e.to_string().starts_with("name-to-ID map") || !batch.iter().any(|m| m.message.properties.iter().any(named)) {
        return Err(e.to_string());
    }
    log::info!("the data file's name-to-ID map can't be added to ({e}): mail written without its flags' text, due days and categories");
    let tagged: Vec<Vec<UnicodePstProperty<'_>>> =
        batch.iter().map(|m| m.message.properties.iter().filter(|p| !named(p)).copied().collect()).collect();
    let batch: Vec<UnicodePstBatchMessage<'_>> = batch
        .iter()
        .zip(&tagged)
        .map(|(m, p)| UnicodePstBatchMessage { message: UnicodePstMessage { properties: p, ..m.message }, attachments: m.attachments })
        .collect();
    UnicodePstFile::append_many_in_folder_with_attachments(path, folder, &batch).map(drop).map_err(|e| e.to_string())
}
