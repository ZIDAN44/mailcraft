use super::item::{File, item_of};
use super::props::{decode, one_off_address, val};
use super::rebuild::to_raw;
use super::*;
use crate::data_files::scratch::scratch;
use crate::pst_pim::Named;
use mailcraft_mime::tnef_fixtures::{SPEC_LZFU, STORYBOARD_RTF, stored_rtf};
use mailcraft_pst::write;

/// A value of `props` by its id, as an item's reading asks for one.
fn get<'a>(props: &HashMap<u16, Val<'a>>, id: u16) -> Option<Val<'a>> {
    props.get(&id).cloned()
}

/// The names of a rebuilt message's files, read back from its source (which must parse: a
/// message that doesn't carries no file either, and would pass for one that has none).
fn files_of(raw: &[u8]) -> Vec<String> {
    mailcraft_mime::parse(raw).expect("the rebuilt message parses").attachments.into_iter().map(|a| a.name).collect()
}

/// An item from its properties alone, with no recipients.
fn item_with<'a>(props: &HashMap<u16, Val<'a>>, files: Vec<File>, named: &Named) -> Item {
    let none: [fn(u16) -> Option<Val<'a>>; 0] = [];
    item_of(&|id| get(props, id), &none, files, named)
}

/// A one-off entry id holding `name`, `kind` and `address`, in UTF-16 or 8-bit.
fn one_off(name: &str, kind: &str, address: &str, unicode: bool) -> Vec<u8> {
    let mut b = vec![0, 0, 0, 0, 0x81, 0x2B, 0x1F, 0xA4, 0xBE, 0xA3, 0x10, 0x19, 0x9D, 0x6E, 0x00, 0xDD, 0x01, 0x0F, 0x54, 0x02, 0, 0];
    b.extend_from_slice(&(if unicode { 0x8000u16 } else { 0 }).to_le_bytes());
    for s in [name, kind, address] {
        if unicode {
            b.extend(s.encode_utf16().chain([0]).flat_map(u16::to_le_bytes));
        } else {
            b.extend(s.bytes().chain([0]));
        }
    }
    b
}

/// Outlook's notes come in as notes: their text, colour and time; an empty one doesn't.
#[test]
fn notes_come_in_as_notes() {
    let dir = scratch("pst-notes");
    let store = Store::open(&dir).unwrap();
    let mut progress = |_| {};
    let mut out = Out::new(&store, "d1", &ImportOptions::default(), &mut progress);
    let note = |body: &str, color: Option<u8>| Item {
        class: "IPM.StickyNote".into(),
        subject: body.lines().next().unwrap_or_default().into(),
        body: body.into(),
        date: 1_700_000_000,
        extra: Some(crate::pst_pim::Extra { note_color: color, modified: Some(1_700_000_500), ..Default::default() }),
        ..Default::default()
    };
    out.add_pim(note("Poster\r\nAsk about 170 gsm", Some(1)));
    out.add_pim(note("Plain", None));
    out.add_pim(note("  ", Some(2)));
    let done = out.finish();
    assert_eq!((done.notes, done.skipped), (2, 1));
    assert_eq!(done.skips.into_iter().collect::<Vec<_>>(), [(SkipReason::ItemNotUsable, 1)], "an empty note isn't an unreadable message");
    let pim: Pim = store.load_account_doc("d1", PIM_DOC).unwrap();
    let n = &pim.notes[0].note;
    assert_eq!((n.text.as_str(), n.color, n.modified), ("Poster\nAsk about 170 gsm", 1, 1_700_000_500));
    assert_eq!(pim.notes[1].note.color, NOTE_YELLOW, "no colour: Outlook's yellow");
    assert!(pim.notes.iter().all(|s| !s.dirty && s.href.is_empty()), "kept on this computer");
}

/// A message too big for the batch being gathered goes into a transaction of its own, not on top
/// of the batch (which made one far over the cap): a write of it that fails costs only it.
#[test]
fn a_big_message_goes_alone() {
    let store = Store::failing();
    store.fail(None, Some("2.eml"));
    let mut progress = |_| {};
    let mut out = Out::new(&store, "d1", &ImportOptions::default(), &mut progress);
    let inbox = out.add_folder(None, "Inbox");
    let mail = |body: String| Item { class: "IPM.Note".into(), subject: "s".into(), date: 1_704_067_200, body, ..Default::default() };
    out.add_message(&inbox, mail("small".into()));
    out.add_message(&inbox, mail("x".repeat(mailcraft_store::BACKGROUND_COMMIT)));
    let done = out.finish();
    assert_eq!((done.messages, done.skipped), (1, 1), "the small one was written in a batch of its own");
}

/// The header of a rebuilt message: its lines, each ASCII without control characters (but tab),
/// at most 998 bytes.
fn clean_header(raw: &[u8]) -> Vec<String> {
    let end = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let head = std::str::from_utf8(&raw[..end]).unwrap();
    let lines: Vec<String> = head.split("\r\n").map(str::to_string).collect();
    for l in &lines {
        assert!(l.len() <= 998 && l.chars().all(|c| c == '\t' || (c.is_ascii() && !c.is_ascii_control())), "{l:?}");
    }
    lines
}

#[test]
fn an_offline_copy_is_explained() {
    let dir = scratch("ost");
    let file = dir.join("me@example.com.ost");
    std::fs::write(&file, b"!BDN\0\0\0\0SO$\0rest").unwrap();
    let store = Store::open(dir.join("cache")).unwrap();
    let e = import(&file, &store, "a1", &mut |_| {}).unwrap_err();
    assert!(e.contains(".ost") && e.contains("add the account"), "{e}");
}

#[test]
fn junk_is_refused_and_nothing_is_written() {
    let dir = scratch("pst-junk");
    let file = dir.join("junk.pst");
    std::fs::write(&file, b"!BDN this is not really a data file").unwrap();
    let store = Store::open(dir.join("cache")).unwrap();
    assert!(import(&file, &store, "a1", &mut |_| {}).is_err());
    assert!(import(&dir.join("missing.pst"), &store, "a1", &mut |_| {}).is_err());
}

#[test]
fn imports_a_data_file() {
    use mailcraft_pst::write::{NewAttachment, NewMessage, NewRecipient, RecipientType};
    let dir = scratch("pst");
    let file = dir.join("Archive 2024.pst");
    let recipients = [
        NewRecipient { name: "Ana Ruiz", email: "ana@example.com", recipient_type: RecipientType::To, ..Default::default() },
        NewRecipient { name: "Ben", email: "ben@example.com", recipient_type: RecipientType::Cc, ..Default::default() },
    ];
    let message = NewMessage {
        subject: "Quarterly plan — café",
        sender_name: "Riley Moreno",
        sender_email: "riley@example.com",
        recipients: &recipients,
        body: "Here is the plan.",
        html_body: Some("<p>Here is <b>the plan</b>.</p><img src=\"cid:chart@example.com\">"),
        message_id: "<plan@example.com>",
        // 2024-01-01T00:00:00Z
        delivery_time: 133_485_408_000_000_000,
        message_class: "IPM.Note",
        properties: &[],
    };
    let files = [
        NewAttachment {
            filename: "chart.png",
            mime_type: "image/png",
            content_id: Some("<chart@example.com>"),
            data: b"\x89PNG\r\n\x1a\nfake",
            ..Default::default()
        },
        NewAttachment { filename: "notes.txt", mime_type: "text/plain", content_id: None, data: b"remember the milk", ..Default::default() },
    ];
    write::create_with(&file, &["Inbox"], &message, &files).unwrap();
    let nested = NewMessage { subject: "Old thread", message_id: "<old@example.com>", html_body: None, ..message };
    write::append_message(&file, &["Projects", "2024"], &nested, &[]).unwrap();

    let store = Store::open(dir.join("cache")).unwrap();
    let mut ticks = 0;
    let done = import(&file, &store, "a1", &mut |_| ticks += 1).unwrap();
    assert_eq!(done.messages, 2, "{done:?}");
    assert_eq!(done.skipped, 0);
    let inbox = done.folders.iter().find(|f| f.role == FolderRole::Inbox).unwrap();
    assert_eq!(inbox.path, "INBOX");
    let all = store.load_folders("a1").unwrap();
    assert_eq!(all, done.folders);
    let nested_folder = all.iter().find(|f| f.path == "Projects/2024").unwrap();
    assert_eq!(nested_folder.total, 1);
    assert!(all.iter().any(|f| f.path == "Projects"));

    // The message where the writer put it, with its addresses, body, picture and file.
    let (path, st) = all
        .iter()
        .filter(|f| f.path != "Projects/2024")
        .find_map(|f| store.load_folder("a1", &f.path).ok().filter(|s| !s.messages.is_empty()).map(|s| (f.path.clone(), s)))
        .unwrap();
    let s = &st.messages[0];
    assert_eq!(s.subject, "Quarterly plan — café");
    assert_eq!(s.date, 1_704_067_200);
    let key = MessageKey { account: "a1".into(), folder: path, validity: 1, uid: s.key.uid };
    let raw = store.read_body(&key).unwrap();
    let p = mime::parse(&raw).unwrap();
    assert_eq!(p.from.email, "riley@example.com");
    assert_eq!(p.to.iter().map(|a| a.email.as_str()).collect::<Vec<_>>(), ["ana@example.com"]);
    assert_eq!(p.cc.iter().map(|a| a.email.as_str()).collect::<Vec<_>>(), ["ben@example.com"]);
    assert_eq!(p.message_id, "plan@example.com");
    assert!(p.had_html && p.text.contains("the plan"), "{}", p.text);
    let names: Vec<&str> = p.attachments.iter().map(|a| a.name.as_str()).collect();
    assert!(names.contains(&"notes.txt"), "{names:?}");
    // The picture goes inline, under its own name.
    assert!(p.attachments.iter().any(|a| a.name == "chart.png" && a.inline), "{names:?}");

    // Into a data file, each source compressed before the writer takes it, and the file sealed at
    // the end: the same folders and mail.
    let packed = Store::open_packed(dir.join("app"), dir.join("Files")).unwrap();
    let again = import(&file, &packed, "a1", &mut |_| {}).unwrap();
    assert_eq!((again.messages, again.skipped), (2, 0));
    assert_eq!(packed.load_folders("a1").unwrap(), all);
    assert_eq!(packed.read_body(&key), Some(raw));
    drop(packed);
}

mod chunked;
mod indexing;
mod items;
mod losses;
mod messages;
mod options;
mod pim_gaps;
mod pim_written;
mod pipeline;
mod reader;
mod resume;
mod sent_ids;

mod real_files {
    use crate::data_files::scratch::scratch;

    /// Import a real data file named by `MAILCRAFT_PST` (counts only: no folder name, subject or
    /// address is printed). Fails when the variable is unset, the import fails or brings no mail.
    #[test]
    #[ignore = "run by hand: needs MAILCRAFT_PST"]
    fn imports_the_file_in_mailcraft_pst() {
        let path = std::env::var("MAILCRAFT_PST").expect("MAILCRAFT_PST names the data file to import");
        let store = mailcraft_store::Store::open(scratch("real-pst")).unwrap();
        let t = std::time::Instant::now();
        let done = super::import(std::path::Path::new(&path), &store, "r1", &mut |_| {}).unwrap_or_else(|e| panic!("the import failed: {e}"));
        eprintln!("messages {} contacts {} events {} skipped {} in {:?}", done.messages, done.contacts, done.events, done.skipped, t.elapsed());
        eprintln!("skips {:?}", done.skips);
        let (mut parsed, mut html, mut files, mut dated) = (0, 0, 0, 0);
        // Attached messages by level (0 = attached to an item) and the files inside them.
        let (mut attached, mut inner) = ([0usize; 4], 0);
        for (i, f) in done.folders.iter().enumerate() {
            eprintln!("  folder {i:3} {:?} total {} unread {}", f.role, f.total, f.unread);
            for m in store.load_folder("r1", &f.path).unwrap().messages {
                dated += usize::from(m.date > 0);
                let raw = store.read_body(&m.key).unwrap_or_default();
                if let Some(p) = mailcraft_mime::parse(&raw) {
                    parsed += 1;
                    html += usize::from(p.had_html);
                    files += p.attachments.len();
                    count_attached(&raw, 0, &mut attached, &mut inner);
                }
            }
        }
        eprintln!("parsed {parsed} (html {html}, attachments {files}, dated {dated})");
        eprintln!("attached messages by level {attached:?}, files inside them {inner}");
        assert!(done.messages > 0, "the import brought no mail");
    }
    /// Counts the attached messages in `raw`, `level` down, and the other files inside them.
    fn count_attached(raw: &[u8], level: usize, attached: &mut [usize; 4], inner: &mut usize) {
        let Some(p) = mailcraft_mime::parse(raw) else { return };
        for a in &p.attachments {
            if a.mime.eq_ignore_ascii_case("message/rfc822") {
                if let Some(n) = attached.get_mut(level) {
                    *n += 1;
                }
                if let Some(data) = mailcraft_mime::attachment_data(raw, a.index) {
                    count_attached(&data, level + 1, attached, inner);
                }
            } else if level > 0 {
                *inner += 1;
            }
        }
    }
}
