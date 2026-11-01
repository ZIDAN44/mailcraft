//! An item as read from the file: its properties, recipients and attachments.

use super::props::{Val, clean_name, code_page, is_address, one_off_address, read, val};
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
    /// When it was delivered (`PidTagMessageDeliveryTime`), 0 when the item doesn't say.
    pub(super) received: i64,
    pub(super) body: String,
    pub(super) html: Option<String>,
    pub(super) message_id: String,
    pub(super) in_reply_to: String,
    pub(super) references: Vec<String>,
    pub(super) headers: String,
    /// High (1) or low (-1), for an item with no headers of its own.
    pub(super) importance: i8,
    /// Outlook's conversation index and topic, and the sensitivity (1 personal, 2 private, 3
    /// confidential): headers for an item whose own header lacks them.
    pub(super) thread_index: Vec<u8>,
    pub(super) thread_topic: String,
    pub(super) sensitivity: u8,
    pub(super) flags: Flags,
    pub(super) files: Vec<File>,
    /// A contact's or appointment's own properties.
    pub(super) extra: Option<crate::pst_pim::Extra>,
    /// Text of the item's, or of a message attached to it, is in a code page MailCraft can't read
    /// and lost all but its ASCII.
    pub(super) charset_unsupported: bool,
    /// Recipients of a header-less item left out for having no address anywhere.
    pub(super) recipients_without_address: usize,
    /// The item's only body is Rich Text that couldn't be read: it came in with none.
    pub(super) body_not_converted: bool,
    /// The item's attachments that didn't come in, by reason (a link has no file to bring).
    pub(super) left_out: BTreeMap<SkipReason, usize>,
}

pub(super) enum File {
    Data { name: String, mime: String, cid: String, data: Vec<u8> },
    Message(Box<Item>),
}

/// What one of an item's attachments gives.
pub(super) enum Got {
    File(File),
    /// Not part of the message (a contact's picture), or nothing in it: not counted.
    Nothing,
    /// Left out, for this reason.
    Left(SkipReason),
}

/// An item's files as its attachments are read, and those left out.
#[derive(Default)]
pub(super) struct Files {
    pub(super) files: Vec<File>,
    pub(super) left_out: BTreeMap<SkipReason, usize>,
}

impl Files {
    pub(super) fn add(&mut self, got: Got) {
        match got {
            Got::File(f) => self.files.push(f),
            Got::Nothing => {}
            Got::Left(reason) => {
                let n = self.left_out.entry(reason).or_default();
                *n = n.saturating_add(1);
            }
        }
    }
}

pub(super) fn item(m: &dyn Message, files: Vec<File>, named: &crate::pst_pim::Named) -> Item {
    let props = m.properties();
    // 8-bit strings are in the message's code page.
    let cp = message_code_page(m);
    let p = |id: u16| props.get(id).map(|v| val(v, cp));
    let table = m.recipient_table().map(|t| rows(&**t)).unwrap_or_default();
    let recipients: Vec<_> = table.iter().map(|row| move |id: u16| row.get(&id).map(|v| val(v, cp))).collect();
    let mut it = item_of(&p, &recipients, files, named);
    if mime::unsupported_code_page(cp) {
        let lost = |v: &PropertyValue| matches!(v, PropertyValue::String8(s) if read(s.buffer(), cp, false).1);
        it.charset_unsupported |= props.iter().any(|(_, v)| lost(v)) || table.iter().any(|row| row.values().any(lost));
    }
    it
}

/// The code page a message's 8-bit strings are in (PidTagMessageCodepage; 0 when not said).
pub(super) fn message_code_page(m: &dyn Message) -> u32 {
    code_page(m.properties().get(0x3FFD))
}

/// A message's class (PidTagMessageClass; empty when not said), its 8-bit text in `code_page`.
pub(super) fn message_class(m: &dyn Message, code_page: u32) -> String {
    m.properties().get(0x001A).map(|v| val(v, code_page)).and_then(Val::into_text).unwrap_or_default()
}

/// An item from its properties (`p`) and its recipients' (one getter per recipient).
pub(super) fn item_of<'a>(
    p: &dyn Fn(u16) -> Option<Val<'a>>,
    recipients: &[impl Fn(u16) -> Option<Val<'a>>],
    files: Vec<File>,
    named: &crate::pst_pim::Named,
) -> Item {
    let s = |id: u16| p(id).and_then(Val::into_text).unwrap_or_default();
    let charset_unsupported = files.iter().any(|f| matches!(f, File::Message(m) if m.charset_unsupported));
    let mut it = Item { class: s(0x001A), files, charset_unsupported, ..Default::default() };
    let class = it.class.to_ascii_lowercase();
    if class.starts_with("ipm.contact")
        || class.starts_with("ipm.appointment")
        || class.starts_with("ipm.task")
        || class.starts_with("ipm.stickynote")
    {
        it.extra = Some(crate::pst_pim::extract(p, named));
    }
    // A subject may start with a prefix marker (0x01, length); control characters in the rest
    // are spaces.
    it.subject = {
        let raw = s(0x0037);
        let skip = if raw.starts_with('\u{1}') { 2 } else { 0 };
        raw.chars().skip(skip).map(|c| if c.is_control() { ' ' } else { c }).collect()
    };
    // Names and addresses go into header lines: an address only when it is one, a name with
    // no line breaks.
    let email = |ids: &[u16]| ids.iter().map(|id| s(*id).trim().to_string()).find(|e| is_address(e)).unwrap_or_default();
    let name = |ids: &[u16]| ids.iter().map(|id| clean_name(&s(*id))).find(|n| !n.is_empty()).unwrap_or_default();
    it.from = EmailAddress::new(name(&[0x0C1A, 0x0042]), email(&[0x5D01, 0x0C1F, 0x5D02, 0x0065]));
    it.date = [0x0E06, 0x0039, 0x3007].iter().find_map(|id| p(*id).and_then(|v| v.time())).unwrap_or(0);
    it.received = p(0x0E06).and_then(|v| v.time()).unwrap_or(0);
    it.body = s(0x1000);
    it.html = p(0x1013).and_then(|v| match v {
        Val::Bin(b) => {
            // PidTagInternetCodepage.
            let page = match p(0x3FDE) {
                Some(Val::Int(n)) => u32::try_from(n).unwrap_or(0),
                _ => 0,
            };
            let (html, lost) = read(b, page, true);
            it.charset_unsupported |= lost;
            Some(html)
        }
        other => other.into_text(),
    });
    if it.html.as_deref().is_none_or(|h| h.trim().is_empty())
        && let Some(Val::Bin(rtf)) = p(0x1009)
    {
        // PidTagRtfInSync: false when the plain text was changed after the RTF.
        let in_sync = !matches!(p(0x0E1F), Some(Val::Bool(false)));
        rtf_into(&mut it, rtf, in_sync);
    }
    it.message_id = message_ids(&s(0x1035)).into_iter().next().unwrap_or_default();
    it.in_reply_to = message_ids(&s(0x1042)).into_iter().next().unwrap_or_default();
    it.references = message_ids(&s(0x1039));
    it.headers = s(0x007D);
    // PidTagImportance: 0 low, 1 normal, 2 high.
    it.importance = match p(0x0017) {
        Some(Val::Int(0)) => -1,
        Some(Val::Int(2)) => 1,
        _ => 0,
    };
    // A conversation index is a 22-byte header and 5-byte blocks (MS-OXOMSG 2.2.1.3): kept
    // whole blocks only, at most MAX_THREAD_BLOCKS of them.
    if let Some(Val::Bin(b)) = p(0x0071)
        && b.len() >= 22
    {
        let blocks = ((b.len() - 22) / 5).min(MAX_THREAD_BLOCKS);
        it.thread_index = b.get(..22 + blocks * 5).unwrap_or_default().to_vec();
    }
    it.thread_topic = s(0x0070);
    it.sensitivity = match p(0x0036) {
        Some(Val::Int(n @ 1..=3)) => u8::try_from(n).unwrap_or(0),
        _ => 0,
    };
    let flags = match p(0x0E07) {
        Some(Val::Int(n)) => n,
        _ => 0,
    };
    let follow_up = matches!(p(0x1090), Some(Val::Int(2)));
    // The last thing done to it (PidTagLastVerbExecuted): reply, reply to all, forward; or,
    // without one, its forwarded icon (PidTagIconIndex).
    let verb = match p(0x1081) {
        Some(Val::Int(n)) => n,
        _ => 0,
    };
    let replied = verb == 0x0066 || verb == 0x0067;
    let forwarded = verb == 0x0068 || (verb == 0 && matches!(p(0x1080), Some(Val::Int(0x0106))));
    // MSGFLAG_READ, MSGFLAG_UNSENT.
    it.flags = Flags { seen: flags & 0x1 != 0, draft: flags & 0x8 != 0, flagged: follow_up, answered: replied, forwarded, ..Default::default() };
    // Its categories, as the keywords MailCraft keeps them as (a name that would make one of
    // the system's, `$Junk` say, is left out).
    if let Some(Val::Texts(names)) = named.keywords().and_then(p) {
        let keywords = names.iter().filter(|n| !n.trim().is_empty()).map(|n| mailcraft_model::keyword_for(n));
        for k in keywords.filter(|k| !mailcraft_model::organize::keywords::is_system_keyword(k)) {
            if it.flags.keywords.len() < MAX_KEYWORDS && !it.flags.keywords.contains(&k) {
                it.flags.keywords.push(k);
            }
        }
    }
    let headers = super::rebuild::usable_headers(&it.headers);
    for row in recipients {
        let get = |id: u16| row(id).and_then(Val::into_text).unwrap_or_default();
        // The SMTP address, the address, or the one in the recipient's one-off entry id.
        let one_off = || match row(0x0FFF) {
            Some(Val::Bin(b)) => one_off_address(b),
            _ => None,
        };
        let found =
            [get(0x39FE), get(0x3003)].into_iter().map(|e| e.trim().to_string()).find(|e| is_address(e)).map(|e| (String::new(), e)).or_else(one_off);
        // Left out without an address: counted for a header-less item only, as a header item's
        // own To and Cc still carry the address.
        let Some((one_off_name, address)) = found else {
            if !headers {
                it.recipients_without_address = it.recipients_without_address.saturating_add(1);
            }
            continue;
        };
        let name = clean_name(&get(0x3001));
        let a = EmailAddress::new(if name.is_empty() { one_off_name } else { name }, address);
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
/// leaves out tables, pictures and objects the text still has, and over RTF that isn't `in_sync`
/// with it (the text is newer).
fn rtf_into(it: &mut Item, compressed: &[u8], in_sync: bool) {
    let plain = !it.body.trim().is_empty();
    if plain && !in_sync {
        return;
    }
    let rtf = mime::decompress_rtf(compressed);
    if plain && !rtf.as_deref().is_some_and(mime::rtf_wraps_html) {
        return;
    }
    match rtf.map(|rtf| mime::rtf_body(&rtf)) {
        Some(mime::TnefBody::Html(h)) => it.html = Some(h),
        Some(mime::TnefBody::Text(t)) if !plain && !t.trim().is_empty() => it.body = t,
        _ if plain => {}
        _ => {
            log::info!("a Rich Text body could not be read");
            it.body_not_converted = true;
        }
    }
}

/// The message ids in a References or In-Reply-To value as the file keeps it: every `<…>` when
/// there is one (`<a><b>`, `<a>,<b>`, `<a> , <b>`), else the words between spaces and commas;
/// without their brackets, each at most [`MAX_ID`] bytes; the last [`MAX_IDS`], as threading
/// keeps them.
pub(super) fn message_ids(s: &str) -> Vec<String> {
    let words: Vec<&str> = if s.contains('<') {
        s.split('<').skip(1).filter_map(|w| w.split_once('>').map(|(id, _)| id)).collect()
    } else {
        s.split(|c: char| c.is_whitespace() || c == ',').collect()
    };
    let cap = |mut id: String| {
        let end = (0..=MAX_ID.min(id.len())).rev().find(|i| id.is_char_boundary(*i)).unwrap_or(0);
        id.truncate(end);
        id
    };
    let mut ids: Vec<String> = words.into_iter().map(|w| cap(mailcraft_model::thread::norm_id(w))).filter(|id| !id.is_empty()).collect();
    ids.drain(..ids.len().saturating_sub(MAX_IDS));
    ids
}

/// An attachment's file, or the message it holds; its 8-bit strings are in `code_page` (the
/// message's), and `class` is the message's class.
pub(super) fn attachment(a: &dyn Attachment, nesting: usize, code_page: u32, class: &str, named: &crate::pst_pim::Named) -> Got {
    let props = a.properties();
    let p = |id: u16| props.get(id).map(|v| val(v, code_page));
    let s = |id: u16| p(id).and_then(Val::into_text).unwrap_or_default();
    if hidden(&p, class) {
        return Got::Nothing;
    }
    match a.data() {
        Some(AttachmentData::Binary(b)) => {
            if b.buffer().len() > MAX_ATTACHMENT {
                log::info!("an attachment of {} bytes was left out", b.buffer().len());
                return Got::Left(SkipReason::AttachmentTooBig);
            }
            Got::File(File::Data { name: name_of(&s), mime: s(0x370E), cid: s(0x3712), data: b.buffer().to_vec() })
        }
        Some(AttachmentData::Message(m)) if nesting < MAX_NESTING => {
            // The embedded message's own attachments need its concrete type: left out.
            Got::File(File::Message(Box::new(item(&**m, Vec::new(), named))))
        }
        Some(AttachmentData::Message(_)) => Got::Left(SkipReason::EmbeddedTooDeep),
        None if link(&p) => Got::Left(SkipReason::AttachmentLink),
        None => Got::Nothing,
    }
}

/// An attachment the reader couldn't read (`e`), from its row in the item's attachment table,
/// read only then: a backstop for what the reader fails on. Outlook keeps a zero-byte file with
/// no value at all, which a reader may take for an error (MailCraft's fork reads it as a file
/// with no bytes, so it doesn't come here): that file comes in, empty. It is told from the error,
/// not from the row's size (`0x0E20`), which counts the attachment's other properties too. A row
/// may have no more than the file's short name (`0x3704`), and no type, which then comes from the
/// name. An attached message the reader fails on is counted as one.
pub(super) fn unread_attachment(e: &io::Error, table: &dyn TableContext, row: &TableRowData, code_page: u32, class: &str) -> Got {
    let values = row_values(table, row);
    from_row(e, &|id| values.get(&id).map(|v| val(v, code_page)), class)
}

/// [`unread_attachment`] from the row's values (`p`), in a message of class `class`.
pub(super) fn from_row<'a>(e: &io::Error, p: &dyn Fn(u16) -> Option<Val<'a>>, class: &str) -> Got {
    let s = |id: u16| p(id).and_then(Val::into_text).unwrap_or_default();
    // PidTagAttachMethod: 1 by value, 5 an attached message; a row may not say.
    let method = match p(0x3705) {
        Some(Val::Int(m)) => Some(m),
        _ => None,
    };
    if hidden(p, class) {
        Got::Nothing
    } else if link(p) {
        Got::Left(SkipReason::AttachmentLink)
    } else if kept_empty(e, method) {
        let name = name_of(&s);
        let mime = Some(s(0x370E)).filter(|m| !m.is_empty()).unwrap_or_else(|| mime::mime_from_name(&name).to_string());
        Got::File(File::Data { name, mime, cid: s(0x3712), data: Vec::new() })
    } else if method == Some(5) {
        log::info!("an attached message could not be read: {e}");
        Got::Left(SkipReason::AttachedMessageUnreadable)
    } else {
        log::info!("an attachment could not be read: {e}");
        Got::Left(SkipReason::AttachmentUnreadable)
    }
}

/// Whether the reader's error is a by-value attachment's data having no value (`PtypNull`). The
/// error a reader gives for any attachment's data (an attached message's too) counts only when
/// the row says by value (`method` 1); the one only a file's data gives, also when it doesn't say.
fn kept_empty(e: &io::Error, method: Option<i64>) -> bool {
    use outlook_pst::ltp::prop_type::PropertyType;
    use outlook_pst::messaging::MessagingError;
    match e.get_ref().and_then(|e| e.downcast_ref::<MessagingError>()) {
        Some(MessagingError::InvalidMessageObjectData(PropertyType::Null)) => method == Some(1),
        Some(MessagingError::InvalidAttachmentFileBinaryData(PropertyType::Null)) => matches!(method, None | Some(1)),
        _ => false,
    }
}

/// A contact's hidden attachment (its picture) isn't part of what comes in, unless it has a
/// `Content-ID`; nor is a recurring series' changed occurrence, a hidden attached message
/// (PidTagAttachmentFlags afException, or any hidden attached message of a meeting or an
/// appointment): calendar bookkeeping, not a file. A message's other hidden files are: Outlook
/// hides some files a message carries, and leaving them out lost them.
fn hidden<'a>(p: &dyn Fn(u16) -> Option<Val<'a>>, class: &str) -> bool {
    let is = |prefix: &str| class.get(..prefix.len()).is_some_and(|c| c.eq_ignore_ascii_case(prefix));
    let exception = matches!(p(0x7FFD), Some(Val::Int(f)) if f & 0x2 != 0)
        || ((is("ipm.schedule.meeting") || is("ipm.appointment")) && matches!(p(0x3705), Some(Val::Int(5))));
    matches!(p(0x7FFE), Some(Val::Bool(true))) && p(0x3712).and_then(Val::into_text).is_none_or(|c| c.is_empty()) && (is("ipm.contact") || exception)
}

/// By reference, by reference only, or by web reference (PidTagAttachMethod 2, 4, 7): a path
/// or an address, not the file.
fn link<'a>(p: &dyn Fn(u16) -> Option<Val<'a>>) -> bool {
    matches!(p(0x3705), Some(Val::Int(2 | 4 | 7)))
}

/// A file's name: its long name, its short one, then its display name.
fn name_of(s: &dyn Fn(u16) -> String) -> String {
    [s(0x3707), s(0x3704), s(0x3001)].into_iter().find(|n| !n.is_empty()).unwrap_or_else(|| "attachment".into())
}

/// A table's rows as property maps.
fn rows(table: &dyn TableContext) -> Vec<HashMap<u16, PropertyValue>> {
    table.rows_matrix().map(|row| row_values(table, row)).collect()
}

/// One row's values by property id (none when the row can't be read).
fn row_values(table: &dyn TableContext, row: &TableRowData) -> HashMap<u16, PropertyValue> {
    let context = table.context();
    let mut map = HashMap::new();
    let Ok(values) = row.columns(context) else { return map };
    for (column, value) in context.columns().iter().zip(values) {
        if let Some(v) = value
            && let Ok(pv) = table.read_column(&v, column.prop_type())
        {
            map.insert(column.prop_id(), pv);
        }
    }
    map
}
