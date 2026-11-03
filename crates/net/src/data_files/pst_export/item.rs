//! A message as it goes into the data file: its text, addresses and files from its source, and
//! what the store keeps of it (read, flagged, answered, categories, when it's due) as the
//! properties Outlook keeps them in.

use super::*;
use outlook_pst::ltp::prop_context::GuidValue;
use outlook_pst::messaging::named_prop::PS_PUBLIC_STRINGS;
use outlook_pst::{UnicodePstName, UnicodePstProperty, UnicodePstValue};

/// PSETID_Common and PSETID_Task (MS-OXPROPS 1.3.2).
pub(super) const PSETID_COMMON: GuidValue = GuidValue::new(0x0006_2008, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
pub(super) const PSETID_TASK: GuidValue = GuidValue::new(0x0006_2003, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);

/// `PidTagMessageFlags`' mfRead and mfUnsent (MS-OXCMSG 2.2.1.6).
const READ: i32 = 0x1;
const UNSENT: i32 = 0x8;
/// `PidTagLastVerbExecuted` for a reply and a forward, and the matching `PidTagIconIndex`
/// (MS-OXOMSG 2.2.1.10, 2.2.1.37).
const VERB_REPLY: i32 = 102;
const VERB_FORWARD: i32 = 104;
const ICON_REPLIED: i32 = 0x105;
const ICON_FORWARDED: i32 = 0x106;
/// `PidTagFlagStatus` flagged, `PidTagFollowupIcon` red flag, `PidTagToDoItemFlags` marked
/// (MS-OXOFLAG 2.2.1).
const FLAGGED: i32 = 2;
const RED_FLAG: i32 = 6;
const TO_DO_MARKED: i32 = 1;
/// Seconds from 1601-01-01 to 1970-01-01.
const EPOCH_1601: i64 = 11_644_473_600;
/// The longest header block written (`PidTagTransportMessageHeaders`): a longer one is left out,
/// as the import leaves it out.
const MAX_HEADERS: usize = 256 * 1024;

/// A message, ready to write.
pub(super) struct Owned {
    pub(super) subject: String,
    pub(super) from: (String, String),
    pub(super) recipients: Vec<(String, String, UnicodePstRecipientType)>,
    pub(super) body: String,
    pub(super) html: Option<String>,
    pub(super) message_id: String,
    /// When it was received (delivery), and its `Date:` (submitted): 100 ns since 1601.
    pub(super) received: i64,
    pub(super) date: i64,
    pub(super) files: Vec<(String, String, Option<String>, Vec<u8>)>,
    /// Its header block, as it came (`PidTagTransportMessageHeaders`).
    pub(super) headers: String,
    pub(super) in_reply_to: String,
    pub(super) references: String,
    /// `PidTagImportance` (0 low, 1 normal, 2 high) and `PidTagSensitivity` (1 personal,
    /// 2 private, 3 confidential).
    pub(super) importance: i32,
    pub(super) sensitivity: Option<i32>,
    pub(super) state: State,
}

/// What the store keeps of a message beyond its source.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct State {
    pub(super) read: bool,
    /// A draft: in the Drafts folder, or flagged `\Draft`.
    pub(super) draft: bool,
    pub(super) flagged: bool,
    pub(super) answered: bool,
    pub(super) forwarded: bool,
    /// When a flagged message is due: 100 ns since 1601, at midnight of its day.
    pub(super) due: Option<i64>,
    /// Its categories, by name.
    pub(super) categories: Vec<String>,
}

/// Unix seconds as 100 ns since 1601 (0 before then).
pub(super) fn filetime(unix: i64) -> i64 {
    unix.saturating_add(EPOCH_1601).saturating_mul(10_000_000).max(0)
}

/// What the store keeps of `s`, a message in a folder of role `role`; `due` is when the flagged
/// ones are due (local days, by [`Summary::follow_up_id`]).
pub(super) fn state(s: &mailcraft_model::Summary, role: FolderRole, due: &BTreeMap<String, i64>) -> State {
    let f = &s.flags;
    let categories = f
        .keywords
        .iter()
        .filter(|k| !mailcraft_model::organize::keywords::is_system_keyword(k))
        .map(|k| mailcraft_model::organize::keywords::readable(k))
        .filter(|name| !name.trim().is_empty())
        .collect();
    // A day as its midnight: Outlook keeps a due date as the day's, not a moment in a time zone.
    let due = f.flagged.then(|| due.get(&s.follow_up_id()).map(|day| filetime(day.saturating_mul(86_400)))).flatten();
    State {
        read: f.seen,
        draft: f.draft || role == FolderRole::Drafts,
        flagged: f.flagged,
        answered: f.answered,
        forwarded: f.forwarded,
        due,
        categories,
    }
}

/// The message in `raw`, with `state`; `received` is when it was received (Unix seconds).
pub(super) fn owned(raw: &[u8], received: i64, state: State) -> Option<Owned> {
    let p = mime::parse(raw)?;
    let mut recipients = Vec::new();
    for (list, kind) in [(&p.to, UnicodePstRecipientType::To), (&p.cc, UnicodePstRecipientType::Cc), (&p.bcc, UnicodePstRecipientType::Bcc)] {
        recipients.extend(list.iter().filter(|a| !a.email.is_empty()).map(|a| (a.name.clone(), a.email.clone(), kind)));
    }
    let files = p
        .attachments
        .iter()
        .filter(|a| a.size <= MAX_ATTACHMENT)
        .filter_map(|a| Some((a.name.clone(), a.mime.clone(), a.content_id.clone(), mime::attachment_data(raw, a.index)?)))
        .collect();
    let headers = header_block(raw);
    let field = |name: &str| header_value(&headers, name).unwrap_or_default();
    let importance = match field("Importance").to_ascii_lowercase().as_str() {
        "high" => 2,
        "low" => 0,
        _ => match field("X-Priority").trim().chars().next() {
            Some('1' | '2') => 2,
            Some('4' | '5') => 0,
            _ => 1,
        },
    };
    let sensitivity = match field("Sensitivity").to_ascii_lowercase().as_str() {
        "personal" => Some(1),
        "private" => Some(2),
        "company-confidential" => Some(3),
        _ => None,
    };
    let bracket = |id: &str| if id.is_empty() { String::new() } else { format!("<{id}>") };
    Some(Owned {
        subject: p.subject,
        from: (p.from.name, p.from.email),
        recipients,
        body: p.text,
        html: mime::html_body(raw),
        message_id: bracket(&p.message_id),
        received: filetime(if received > 0 { received } else { p.date }),
        // No `Date:`: no submit time, not 1970.
        date: if p.date > 0 { filetime(p.date) } else { 0 },
        files,
        headers: if headers.len() <= MAX_HEADERS { headers } else { String::new() },
        in_reply_to: bracket(&p.in_reply_to),
        references: p.references.iter().map(|id| bracket(id)).collect::<Vec<_>>().join(" "),
        importance,
        sensitivity,
        state,
    })
}

/// A message's header block (up to its first blank line, whichever its line ends), as text.
fn header_block(raw: &[u8]) -> String {
    let crlf = raw.windows(4).position(|w| w == b"\r\n\r\n");
    let lf = raw.windows(2).position(|w| w == b"\n\n");
    let end = crlf.into_iter().chain(lf).min().unwrap_or(raw.len());
    let head = String::from_utf8_lossy(raw.get(..end).unwrap_or_default()).into_owned();
    if head.is_empty() { head } else { format!("{head}\r\n") }
}

/// A header field's value in a header block, its folded lines unfolded.
fn header_value(head: &str, name: &str) -> Option<String> {
    let mut lines = head.lines();
    while let Some(line) = lines.next() {
        let Some((n, value)) = line.split_once(':') else { continue };
        if line.starts_with([' ', '\t']) || !n.trim_end().eq_ignore_ascii_case(name) {
            continue;
        }
        let mut value = value.trim().to_string();
        for more in lines.by_ref().take_while(|l| l.starts_with([' ', '\t'])) {
            value.push(' ');
            value.push_str(more.trim());
        }
        return Some(value);
    }
    None
}

/// What the message is beyond the writer's own list: its read, draft and follow-up state, its
/// categories (`categories`, the names in `m.state`), how important and private it is, its
/// header block, what it answers, when it was sent and that its addresses are SMTP ones.
pub(super) fn properties<'a>(m: &'a Owned, categories: &'a [&'a str]) -> Vec<UnicodePstProperty<'a>> {
    let int = |id, n| UnicodePstProperty::tagged(id, UnicodePstValue::Integer32(n));
    let text = |id, s: &'a str| UnicodePstProperty::tagged(id, UnicodePstValue::Unicode(s));
    let st = &m.state;
    let flags = if st.read { READ } else { 0 } | if st.draft { UNSENT } else { 0 };
    let mut out = vec![int(0x0E07, flags), int(0x0017, m.importance)];
    if let Some(sensitivity) = m.sensitivity {
        out.push(int(0x0036, sensitivity));
    }
    if m.date > 0 {
        // PidTagClientSubmitTime: its Date:, beside when it was received (the writer's delivery time).
        out.push(UnicodePstProperty::tagged(0x0039, UnicodePstValue::Time(m.date)));
    }
    for (id, value) in [(0x007D, &m.headers), (0x1042, &m.in_reply_to), (0x1039, &m.references)] {
        if !value.is_empty() {
            out.push(text(id, value));
        }
    }
    if !m.from.1.is_empty() {
        // The sender's and the sent-representing address types and addresses.
        out.extend([text(0x0C1E, "SMTP"), text(0x0064, "SMTP"), text(0x0065, &m.from.1), text(0x5D01, &m.from.1), text(0x5D02, &m.from.1)]);
    }
    if st.flagged {
        out.extend([
            int(0x1090, FLAGGED),
            int(0x1095, RED_FLAG),
            int(0x0E2B, TO_DO_MARKED),
            UnicodePstProperty::named(PSETID_COMMON, UnicodePstName::Id(0x8530), UnicodePstValue::Unicode("Follow up")),
        ]);
        if let Some(due) = st.due {
            // PidLidTaskStartDate and PidLidTaskDueDate: MailCraft keeps the day it's due only.
            for lid in [0x8104, 0x8105] {
                out.push(UnicodePstProperty::named(PSETID_TASK, UnicodePstName::Id(lid), UnicodePstValue::Time(due)));
            }
        }
    }
    if st.answered {
        out.extend([int(0x1081, VERB_REPLY), int(0x1080, ICON_REPLIED)]);
    } else if st.forwarded {
        out.extend([int(0x1081, VERB_FORWARD), int(0x1080, ICON_FORWARDED)]);
    }
    if !categories.is_empty() {
        out.push(UnicodePstProperty::named(PS_PUBLIC_STRINGS, UnicodePstName::String("Keywords"), UnicodePstValue::MultipleUnicode(categories)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_fields() {
        let head = header_block(b"Subject: x\r\nImportance: High\r\nX-Long: a\r\n b\r\n\r\nbody");
        assert_eq!(head, "Subject: x\r\nImportance: High\r\nX-Long: a\r\n b\r\n");
        assert_eq!(header_value(&head, "importance").as_deref(), Some("High"));
        assert_eq!(header_value(&head, "X-Long").as_deref(), Some("a b"));
        assert_eq!(header_value(&head, "Missing"), None);
        assert_eq!(header_block(b"no header end"), "no header end\r\n");
        // A header with LF line ends, and a CRLF blank line further on, in the body: the body's
        // lines aren't header fields.
        let raw = b"Subject: x\nFrom: a@b\n\nImportance: high\r\nSensitivity: Private\r\n\r\nmore";
        assert_eq!(header_block(raw), "Subject: x\nFrom: a@b\r\n");
        let m = owned(raw, 0, State::default()).unwrap();
        assert_eq!((m.importance, m.sensitivity), (1, None));
    }

    /// A message without a `Date:` gets no submit time, not 1970-01-01.
    #[test]
    fn no_date_no_submit_time() {
        let submit = |m: &Owned| properties(m, &[]).iter().any(|p| matches!(p.key, outlook_pst::UnicodePstPropertyKey::Tag(0x0039)));
        let m = owned(b"Subject: x\r\nFrom: a@b\r\n\r\nbody", 0, State::default()).unwrap();
        assert_eq!(m.date, 0);
        assert!(!submit(&m));
        let m = owned(b"Date: Mon, 1 Jan 2024 00:00:00 +0000\r\nSubject: x\r\n\r\nbody", 0, State::default()).unwrap();
        assert_eq!(m.date, (1_704_067_200 + EPOCH_1601) * 10_000_000);
        assert!(submit(&m));
    }

    #[test]
    fn state_from_a_summary() {
        let mut s = mailcraft_model::Summary { message_id: "m1@example.com".into(), ..Default::default() };
        s.flags.flagged = true;
        s.flags.keywords = vec!["to=20do".into(), "$Junk".into(), "$label1".into()];
        let due = BTreeMap::from([("m1@example.com".to_string(), 1)]);
        let st = state(&s, FolderRole::Drafts, &due);
        assert!(st.draft && st.flagged && !st.read);
        assert_eq!(st.categories, ["to do", "$label1"]);
        // Midnight of 1970-01-02.
        assert_eq!(st.due, Some((86_400 + EPOCH_1601) * 10_000_000));
        s.flags.flagged = false;
        assert_eq!(state(&s, FolderRole::Inbox, &due).due, None);
    }
}
