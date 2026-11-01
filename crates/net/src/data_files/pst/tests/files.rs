//! The files of an item's attachments: one the reader fails on is read from its row, the ones
//! left out are counted, and the paperclip is the one the rebuilt source gives.

use super::super::item::{File, Files, Got, from_row};
use super::super::rebuild::{message, summary_of};
use super::*;

/// The reader's error for an attachment whose data has no value (`PtypNull`), as a reader that
/// doesn't know Outlook's zero-byte files gives it: for any attachment's data, a file's or an
/// attached message's.
fn no_value() -> io::Error {
    use outlook_pst::ltp::prop_type::PropertyType;
    io::Error::from(outlook_pst::messaging::MessagingError::InvalidMessageObjectData(PropertyType::Null))
}

/// The reader's error only a file's data having no value gives.
fn no_file_value() -> io::Error {
    use outlook_pst::ltp::prop_type::PropertyType;
    io::Error::from(outlook_pst::messaging::MessagingError::InvalidAttachmentFileBinaryData(PropertyType::Null))
}

/// An attachment the reader couldn't read comes from its row: a zero-byte file comes in empty,
/// named from its short name with its type from the name when the row has only that (the row's
/// size counts its properties, so it isn't 0); a link is counted; an attached message is counted
/// as one; anything else is counted as unreadable; a hidden one isn't part of the message. A row
/// that doesn't say how it's attached is a file only by the error a file alone gives.
#[test]
fn an_unreadable_attachment_comes_from_its_row() {
    let get = |row: &HashMap<u16, Val<'static>>, e: &io::Error| from_row(e, &|id| row.get(&id).cloned());
    let short = HashMap::from([(0x0E20, Val::Int(165)), (0x3704, Val::Text("NOTES~1.TXT".into())), (0x3705, Val::Int(1)), (0x370B, Val::Int(-1))]);
    let Got::File(File::Data { name, mime, cid, data }) = get(&short, &no_value()) else { panic!("the empty file is kept") };
    assert_eq!((name.as_str(), mime.as_str(), cid.as_str(), data.len()), ("NOTES~1.TXT", "text/plain", "", 0));

    let mut full = short.clone();
    full.extend([
        (0x3707, Val::Text("Meeting notes.docx".into())),
        (0x370E, Val::Text("application/octet-stream".into())),
        (0x3712, Val::Text("notes@studio.example".into())),
    ]);
    let Got::File(File::Data { name, mime, cid, data }) = get(&full, &no_value()) else { panic!("the empty file is kept") };
    assert_eq!(
        (name.as_str(), mime.as_str(), cid.as_str(), data.len()),
        ("Meeting notes.docx", "application/octet-stream", "notes@studio.example", 0)
    );
    assert!(matches!(get(&short, &no_file_value()), Got::File(File::Data { ref name, .. }) if name == "NOTES~1.TXT"));
    let empty = HashMap::new();
    assert!(matches!(get(&empty, &no_file_value()), Got::File(File::Data { ref name, .. }) if name == "attachment"), "a row with nothing in it");
    // Without the method, the error any attachment's data gives may be an attached message's.
    assert!(matches!(get(&empty, &no_value()), Got::Left(SkipReason::AttachmentUnreadable)));

    // Another error: it couldn't be read; an attached message is counted as one.
    let other = io::Error::from(outlook_pst::messaging::MessagingError::AttachmentMethodNotFound);
    assert!(matches!(get(&short, &other), Got::Left(SkipReason::AttachmentUnreadable)));
    assert!(matches!(get(&short, &io::Error::other("damaged")), Got::Left(SkipReason::AttachmentUnreadable)));
    let mut embedded = short.clone();
    embedded.insert(0x3705, Val::Int(5));
    for e in [no_value(), no_file_value(), other] {
        assert!(matches!(get(&embedded, &e), Got::Left(SkipReason::AttachedMessageUnreadable)), "{e}");
    }
    let mut storage = short.clone();
    storage.insert(0x3705, Val::Int(6));
    assert!(matches!(get(&storage, &no_value()), Got::Left(SkipReason::AttachmentUnreadable)));
    let other = io::Error::from(outlook_pst::messaging::MessagingError::AttachmentMethodNotFound);
    let mut hidden = short.clone();
    hidden.insert(0x7FFE, Val::Bool(true));
    assert!(matches!(get(&hidden, &no_value()), Got::Nothing));
    for method in [2, 4, 7] {
        let mut link = short.clone();
        link.insert(0x3705, Val::Int(method));
        assert!(matches!(get(&link, &other), Got::Left(SkipReason::AttachmentLink)), "method {method}");
    }
}

/// The files left out of the messages that came in are counted by reason, without counting
/// those messages as skipped; a message that didn't come in counts none.
#[test]
fn files_left_out_are_counted_by_reason() {
    let dir = std::env::temp_dir().join(format!("mailcraft-pst-left-out-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    let mut progress = |_| {};
    let mut out = Out::new(&store, "d1", &ImportOptions::default(), &mut progress);
    let path = out.add_folder(None, "Inbox");
    let mail = |got: Vec<Got>| {
        let mut files = Files::default();
        for g in got {
            files.add(g);
        }
        Item {
            class: "IPM.Note".into(),
            subject: "Plans".into(),
            body: "See the share.".into(),
            files: files.files,
            left_out: files.left_out,
            ..Default::default()
        }
    };
    let link = || Got::Left(SkipReason::AttachmentLink);
    out.add_message(&path, mail(vec![link(), link(), Got::Nothing]));
    out.add_message(&path, mail(vec![link(), Got::Left(SkipReason::AttachmentUnreadable), Got::File(data_file("plan.pdf", "application/pdf", ""))]));
    out.add_message(&path, Item { class: "IPM.Contact".into(), ..mail(vec![link()]) });
    let done = out.finish();
    assert_eq!((done.messages, done.skipped), (2, 0));
    assert_eq!(done.skips.into_iter().collect::<Vec<_>>(), [(SkipReason::AttachmentUnreadable, 1), (SkipReason::AttachmentLink, 3)]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A file as the import holds it.
fn data_file(name: &str, mime: &str, cid: &str) -> File {
    File::Data { name: name.into(), mime: mime.into(), cid: cid.into(), data: b"\x89PNG\r\n\x1a\nfake".to_vec() }
}

/// The paperclip an imported message gets is the one its source gives (`has:attachment` and
/// "Has files" read it): a picture the HTML shows by its `cid:` and S/MIME's own part aren't
/// files; a picture it doesn't show (`cid:image10` doesn't show `image1`), any other file and an
/// attached message are. Counting the item's files gave every one of these a paperclip.
#[test]
fn the_paperclip_is_the_one_the_source_gives() {
    let html = "<p>Hi</p><img src=\"cid:LOGO@studio.example\"><img src='cid:image10'>";
    let cases = [
        (vec![data_file("logo.png", "image/png", "<logo@studio.example>")], false),
        (vec![data_file("logo.png", "image/png", "logo@studio.example"), data_file("smime.p7s", "application/pkcs7-signature", "")], false),
        (vec![data_file("image1.png", "image/png", "image1")], true),
        (vec![data_file("logo.png", "image/png", "logo@studio.example"), data_file("plan.pdf", "application/pdf", "")], true),
        (vec![File::Message(Box::new(Item { subject: "Old".into(), body: "Hi".into(), ..Default::default() }))], true),
        (Vec::new(), false),
    ];
    for (n, (files, has)) in cases.into_iter().enumerate() {
        let it = Item {
            subject: "Logo".into(),
            from: EmailAddress::new("Ana", "ana@example.com"),
            date: 1_704_067_200,
            body: "Hi".into(),
            html: Some(html.into()),
            files,
            ..Default::default()
        };
        let (raw, glance) = message(it, "logo@x").unwrap();
        let key = MessageKey { account: "d1".into(), folder: "INBOX".into(), validity: 1, uid: 1 };
        let s = summary_of(&raw, glance, key.clone(), 0);
        let full = mime::summary_from_full(&raw, key, Flags::default(), 0, 0);
        assert_eq!((s.has_attachments, full.has_attachments), (has, has), "case {n}");
    }
}

/// A file whose Content-ID is the name of a file the HTML only links to (`plan.pdf`, and a link
/// to `https://files.example/share/plan.pdf`) isn't shown by it: it stays a named file, listed,
/// with a paperclip. Matching it as the reading pane matches an address moved it into the body,
/// unnamed and hidden.
#[test]
fn a_file_named_by_a_link_stays_a_file() {
    let html = "<p>Hi</p><a href=\"https://files.example/share/plan.pdf\">the plan</a><img src=\"cid:LOGO@studio.example\">";
    let pdf = File::Data { name: "plan.pdf".into(), mime: "application/pdf".into(), cid: "plan.pdf".into(), data: b"%PDF-1.4".to_vec() };
    let it = Item {
        subject: "Plan".into(),
        from: EmailAddress::new("Ana", "ana@example.com"),
        date: 1_704_067_200,
        body: "Hi".into(),
        html: Some(html.into()),
        files: vec![pdf, data_file("logo.png", "image/png", "logo@studio.example")],
        ..Default::default()
    };
    let (raw, glance) = message(it, "plan@x").unwrap();
    let p = mime::parse(&raw).unwrap();
    let parts: Vec<_> = p.attachments.iter().map(|a| (a.name.as_str(), a.inline)).collect();
    assert!(parts.contains(&("plan.pdf", false)), "{parts:?}");
    assert!(p.attachments.iter().any(|a| a.inline && a.content_id.as_deref() == Some("logo@studio.example")), "{parts:?}");
    let key = MessageKey { account: "d1".into(), folder: "INBOX".into(), validity: 1, uid: 1 };
    let full = mime::summary_from_full(&raw, key.clone(), Flags::default(), 0, 0);
    assert_eq!((summary_of(&raw, glance, key, 0).has_attachments, full.has_attachments), (true, true));
}
