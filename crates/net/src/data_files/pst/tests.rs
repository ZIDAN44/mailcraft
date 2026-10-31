use super::item::item_of;
use super::props::{decode, val};
use super::rebuild::to_raw;
use super::*;

/// Outlook's notes come in as notes: their text, colour and time; an empty one doesn't.
#[test]
fn notes_come_in_as_notes() {
    let dir = std::env::temp_dir().join(format!("mailcraft-pst-notes-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    let mut progress = |_| {};
    let mut out = Out {
        store: &store,
        account: "d1",
        folders: Vec::new(),
        states: HashMap::new(),
        imported: 0,
        skipped: 0,
        progress: &mut progress,
        pim: Pim::default(),
        pending: Vec::new(),
        pending_bytes: 0,
    };
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
    let done = out.finish().unwrap();
    assert_eq!((done.notes, done.skipped), (2, 1));
    let pim: Pim = store.load_account_doc("d1", PIM_DOC).unwrap();
    let n = &pim.notes[0].note;
    assert_eq!((n.text.as_str(), n.color, n.modified), ("Poster\nAsk about 170 gsm", 1, 1_700_000_500));
    assert_eq!(pim.notes[1].note.color, NOTE_YELLOW, "no colour: Outlook's yellow");
    assert!(pim.notes.iter().all(|s| !s.dirty && s.href.is_empty()), "kept on this computer");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn windows_1252_and_utf8() {
    assert_eq!(decode(b"caf\xe9 \x80 \x93q\x94", 1252), "café € “q”");
    assert_eq!(decode("café".as_bytes(), 0), "café");
    assert_eq!(decode(b"\xff\xfe", 65001), "\u{FFFD}\u{FFFD}");
}

#[test]
fn filetime() {
    // 2024-01-01T00:00:00Z
    assert_eq!(val(&PropertyValue::Time(133_485_408_000_000_000), 0).time(), Some(1_704_067_200));
    assert_eq!(val(&PropertyValue::Time(0), 0).time(), None);
}

/// Only a 32-bit integer reads as a number, and a value of a type the import does not read is
/// still there, as it was before items were read through `Val`.
#[test]
fn only_a_32_bit_integer_is_a_number() {
    assert_eq!(val(&PropertyValue::Integer32(2), 0), Val::Int(2));
    for v in [PropertyValue::Integer16(2), PropertyValue::Integer64(2), PropertyValue::ErrorCode(2), PropertyValue::Null] {
        assert_eq!(val(&v, 0), Val::Other, "{v:?}");
    }
}

/// An item is read from its values alone: the subject's prefix marker goes, flags, time and
/// recipients come through, and an HTML body in bytes is read in its code page.
#[test]
fn an_item_reads_from_its_values() {
    let props: HashMap<u16, Val<'static>> = HashMap::from([
        (0x001A, Val::Text("IPM.Note".into())),
        (0x0037, Val::Text("\u{1}\u{5}FW: Hello".into())),
        (0x0C1A, Val::Text("Ana".into())),
        (0x0C1F, Val::Text("ana@example.com".into())),
        (0x0E06, Val::Time(133_485_408_000_000_000)),
        (0x0E07, Val::Int(0x1)),
        (0x1090, Val::Int(2)),
        (0x1081, Val::Int(0x0066)),
        (0x1013, Val::Bin(b"caf\xe9")),
        (0x3FDE, Val::Int(65001)),
    ]);
    let ben = HashMap::from([(0x3001, Val::Text("Ben".into())), (0x39FE, Val::Text("ben@example.com".into())), (0x0C15, Val::Int(2))]);
    let nobody = HashMap::from([(0x3001, Val::Text("No address".into()))]);
    let get = |m: &HashMap<u16, Val<'static>>, id: u16| m.get(&id).cloned();
    let recipients = [&ben, &nobody].map(|r| move |id| get(r, id));
    let it = item_of(&|id| get(&props, id), &recipients, Vec::new(), &crate::pst_pim::Named::default());
    assert_eq!(it.subject, "FW: Hello");
    assert_eq!((it.from.name.as_str(), it.from.email.as_str(), it.date), ("Ana", "ana@example.com", 1_704_067_200));
    assert!(it.flags.seen && !it.flags.draft && it.flags.flagged && it.flags.answered);
    assert_eq!(it.html.as_deref(), Some("caf\u{FFFD}"), "the HTML is read in its code page");
    assert_eq!(it.cc.iter().map(|a| a.email.as_str()).collect::<Vec<_>>(), ["ben@example.com"]);
    assert!(it.to.is_empty() && it.bcc.is_empty() && it.extra.is_none());
}

#[test]
fn original_headers_are_kept_over_the_rebuilt_body() {
    let it = Item {
        subject: "Hello".into(),
        from: EmailAddress::new("Ana", "ana@example.com"),
        to: vec![EmailAddress::new("", "me@example.com")],
        date: 1_704_067_200,
        body: "Hi there".into(),
        message_id: "<orig@example.com>".into(),
        headers: "Received: from x\r\n by y\r\nFrom: Ana <ana@example.com>\r\nTo: me@example.com\r\nSubject: Hello\r\nList-Id: <l.example.com>\r\nContent-Type: multipart/mixed;\r\n boundary=zzz\r\nMIME-Version: 1.0\r\n"
            .into(),
        ..Default::default()
    };
    let raw = to_raw(&it, "fallback@x").unwrap();
    let text = String::from_utf8_lossy(&raw);
    assert!(text.contains("List-Id: <l.example.com>"));
    assert!(text.contains("Received: from x\r\n by y"));
    assert!(!text.contains("boundary=zzz"), "the original body's description goes");
    assert_eq!(text.matches("MIME-Version").count(), 1);
    let p = mime::parse(&raw).unwrap();
    assert_eq!(p.subject, "Hello");
    assert!(p.text.contains("Hi there"));
}

#[test]
fn a_sent_item_keeps_its_bcc_and_gets_an_id() {
    let it = Item {
        subject: "Plans".into(),
        from: EmailAddress::new("Me", "me@example.com"),
        bcc: vec![EmailAddress::new("", "secret@example.com")],
        date: 1_704_067_200,
        body: "Hi".into(),
        ..Default::default()
    };
    let raw = to_raw(&it, "fallback@x").unwrap();
    let text = String::from_utf8_lossy(&raw);
    assert!(text.starts_with("Bcc: secret@example.com\r\n"));
    assert!(text.contains("<fallback@x>"));
}

#[test]
fn an_offline_copy_is_explained() {
    let dir = std::env::temp_dir().join(format!("mailcraft-ost-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let file = dir.join("me@example.com.ost");
    std::fs::write(&file, b"!BDN\0\0\0\0SO$\0rest").unwrap();
    let store = Store::open(dir.join("cache")).unwrap();
    let e = import(&file, &store, "a1", &mut |_| {}).unwrap_err();
    assert!(e.contains(".ost") && e.contains("add the account"), "{e}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn junk_is_refused_and_nothing_is_written() {
    let dir = std::env::temp_dir().join(format!("mailcraft-pst-junk-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("junk.pst");
    std::fs::write(&file, b"!BDN this is not really a data file").unwrap();
    let store = Store::open(dir.join("cache")).unwrap();
    assert!(import(&file, &store, "a1", &mut |_| {}).is_err());
    assert!(import(&dir.join("missing.pst"), &store, "a1", &mut |_| {}).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn imports_a_data_file() {
    use pst_rw::{UnicodePstAttachment, UnicodePstFile as Writer, UnicodePstMessage, UnicodePstRecipient, UnicodePstRecipientType};
    let dir = std::env::temp_dir().join(format!("mailcraft-pst-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("Archive 2024.pst");
    let recipients = [
        UnicodePstRecipient { name: "Ana Ruiz", email: "ana@example.com", recipient_type: UnicodePstRecipientType::To },
        UnicodePstRecipient { name: "Ben", email: "ben@example.com", recipient_type: UnicodePstRecipientType::Cc },
    ];
    let message = UnicodePstMessage {
        subject: "Quarterly plan — café",
        sender_name: "Riley Moreno",
        sender_email: "riley@example.com",
        recipients: &recipients,
        body: "Here is the plan.",
        html_body: Some("<p>Here is <b>the plan</b>.</p><img src=\"cid:chart@example.com\">"),
        message_id: "<plan@example.com>",
        // 2024-01-01T00:00:00Z
        delivery_time: 133_485_408_000_000_000,
    };
    let files = [
        UnicodePstAttachment {
            filename: "chart.png",
            mime_type: "image/png",
            content_id: Some("<chart@example.com>"),
            data: b"\x89PNG\r\n\x1a\nfake",
        },
        UnicodePstAttachment { filename: "notes.txt", mime_type: "text/plain", content_id: None, data: b"remember the milk" },
    ];
    drop(Writer::create_with_attachments(&file, &message, &files).unwrap());
    let nested = UnicodePstMessage { subject: "Old thread", message_id: "<old@example.com>", html_body: None, ..message };
    drop(Writer::append_in_folder(&file, &["Projects", "2024"], &nested).unwrap());

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
    assert!(!names.contains(&"chart.png"), "the picture goes inline: {names:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

mod real_files {
    /// Import a real data file named by `MAILCRAFT_PST` (counts only).
    #[test]
    #[ignore]
    fn imports_the_file_in_mailcraft_pst() {
        let Ok(path) = std::env::var("MAILCRAFT_PST") else { return };
        let dir = std::env::temp_dir().join(format!("mailcraft-real-pst-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = mailcraft_store::Store::open(&dir).unwrap();
        let t = std::time::Instant::now();
        let r = super::import(std::path::Path::new(&path), &store, "r1", &mut |_| {});
        match r {
            Ok(done) => {
                eprintln!(
                    "messages {} contacts {} events {} skipped {} in {:?}",
                    done.messages,
                    done.contacts,
                    done.events,
                    done.skipped,
                    t.elapsed()
                );
                let (mut parsed, mut html, mut files, mut dated) = (0, 0, 0, 0);
                for f in &done.folders {
                    eprintln!("  {:30} {:?} total {} unread {}", f.path, f.role, f.total, f.unread);
                    for m in store.load_folder("r1", &f.path).unwrap().messages {
                        dated += usize::from(m.date > 0);
                        if let Some(p) = store.read_body(&m.key).and_then(|raw| mailcraft_mime::parse(&raw)) {
                            parsed += 1;
                            html += usize::from(p.had_html);
                            files += p.attachments.len();
                        }
                    }
                }
                eprintln!("parsed {parsed} (html {html}, attachments {files}, dated {dated})");
            }
            Err(e) => eprintln!("FAILED: {e}"),
        }
        let _ = std::fs::remove_dir_all(dir);
    }
}
