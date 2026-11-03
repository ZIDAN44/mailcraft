//! The files of an item's attachments: one the reader fails on is read from its row, the ones
//! left out are counted, and the paperclip is the one the rebuilt source gives.

use super::super::item::{File, Files, Got, from_row, linked, local_path, read_linked};
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
/// as one; anything else is counted as unreadable; a hidden one comes in, except a contact's
/// (its picture) and a recurring series' changed occurrence. A row that doesn't say how it's
/// attached is a file only by the error a file alone gives.
#[test]
fn an_unreadable_attachment_comes_from_its_row() {
    let get = |row: &HashMap<u16, Val<'static>>, e: &io::Error| from_row(e, &|id| row.get(&id).cloned(), "IPM.Note", true);
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
    assert!(matches!(get(&hidden, &no_value()), Got::File(File::Data { ref name, .. }) if name == "NOTES~1.TXT"), "a message's hidden file");
    for class in ["IPM.Contact", "ipm.contact.Custom"] {
        assert!(matches!(from_row(&no_value(), &|id| hidden.get(&id).cloned(), class, true), Got::Nothing), "{class}");
    }
    let mut shown = hidden.clone();
    shown.insert(0x3712, Val::Text("me@studio.example".into()));
    assert!(matches!(from_row(&no_value(), &|id| shown.get(&id).cloned(), "IPM.Contact", true), Got::File(_)), "one with a Content-ID");
    // A recurring series' changed occurrence: a hidden attached message, marked afException or
    // in a meeting or an appointment, is left out; a message's hidden attached message isn't, nor
    // the item's own when it is an appointment (it is that occurrence, read as a message, and the
    // appointment goes to the calendar). An appointment attached to a message keeps them hidden.
    let mut occurrence = hidden.clone();
    occurrence.insert(0x3705, Val::Int(5));
    let in_class = |row: &HashMap<u16, Val<'static>>, class: &str, top: bool| from_row(&no_value(), &|id| row.get(&id).cloned(), class, top);
    let read = |got: Got| matches!(got, Got::Left(SkipReason::AttachedMessageUnreadable));
    for top in [true, false] {
        assert!(matches!(in_class(&occurrence, "IPM.Schedule.Meeting.Request", top), Got::Nothing));
        assert!(read(in_class(&occurrence, "IPM.Note", top)));
        assert_eq!(read(in_class(&occurrence, "IPM.Appointment", top)), top, "an appointment's, top {top}");
    }
    occurrence.insert(0x7FFD, Val::Int(0x2));
    for top in [true, false] {
        assert!(matches!(in_class(&occurrence, "IPM.Note", top), Got::Nothing), "afException");
        assert_eq!(read(in_class(&occurrence, "IPM.Appointment", top)), top, "an appointment's, top {top}");
    }
    let mut flagged = hidden.clone();
    flagged.insert(0x7FFD, Val::Int(0x1));
    assert!(matches!(in_class(&flagged, "IPM.Schedule.Meeting.Request", true), Got::File(_)), "a meeting's hidden file");
    for method in [2, 3, 4, 7] {
        let mut link = short.clone();
        link.insert(0x3705, Val::Int(method));
        assert!(matches!(get(&link, &other), Got::Left(SkipReason::AttachmentLink)), "method {method}: no path, nothing to name");
    }
}

/// A link names a file elsewhere (by reference, resolved, only, or on the web): one on this
/// computer's drive comes in as the file, with the attachment's name; one that isn't there, a
/// network path, a web address (never fetched) or a link too big comes in as a short text file
/// naming the path, counted as a link.
#[test]
fn a_link_brings_its_file_when_it_is_here() {
    let dir = std::env::temp_dir().join(format!("mailcraft-pst-link-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // The real path: macOS's temp folder is under /var, a link to /private/var, and a link is refused.
    #[cfg(not(windows))]
    let dir = std::fs::canonicalize(&dir).unwrap();
    let here = dir.join("plan.pdf");
    std::fs::write(&here, b"%PDF-1.7 plan").unwrap();
    let here = here.to_string_lossy().into_owned();
    let row = |method: i64, path: &str| -> HashMap<u16, Val<'static>> {
        HashMap::from([(0x3705, Val::Int(method)), (0x370D, Val::Text(path.to_string().into())), (0x3707, Val::Text("Plan.pdf".into()))])
    };
    let got = |r: &HashMap<u16, Val<'static>>| linked(&|id| r.get(&id).cloned(), &read_linked);
    for method in [2, 3, 4] {
        let Got::File(File::Data { name, mime, data, .. }) = got(&row(method, &here)) else { panic!("method {method}: the file is read") };
        assert_eq!((name.as_str(), mime.as_str(), data.as_slice()), ("Plan.pdf", "application/pdf", &b"%PDF-1.7 plan"[..]));
    }
    let missing = dir.join("gone.docx").to_string_lossy().into_owned();
    let text = |g: Got| match g {
        Got::Link(File::Data { name, mime, data, .. }) => (name, mime, String::from_utf8(data).unwrap()),
        _ => panic!("a link kept as text"),
    };
    assert_eq!(text(got(&row(2, &missing))), ("Plan.pdf (link).txt".to_string(), "text/plain".to_string(), format!("{missing}\r\n")));
    // A web reference isn't read, even when its path is a file here.
    assert_eq!(text(got(&row(7, &here))).2, format!("{here}\r\n"));
    assert_eq!(text(got(&row(7, "https://share.example/plan.pdf"))).2, "https://share.example/plan.pdf\r\n");
    // A network path is never read: its name and the path are kept, its line breaks spaces.
    let unc = r"\\files.example\share\plan.pdf";
    assert!(!local_path(unc) && !local_path("//files.example/share") && !local_path(r"\/x") && !local_path("plan.pdf"));
    assert_eq!(text(got(&row(4, &format!("{unc}\r\n")))).2, format!("{unc}\r\n"));
    // No name of its own: the path's last part.
    let bare = HashMap::from([(0x3705, Val::Int(2)), (0x3708, Val::Text(missing.clone().into()))]);
    assert_eq!(text(got(&bare)).0, "gone.docx (link).txt");
    // A folder isn't a file.
    assert!(matches!(got(&row(2, &dir.to_string_lossy())), Got::Link(_)));
    assert!(local_path(&here));

    // Through the import: the file and the text come in, the text counted as a link.
    let store = Store::open(dir.join("store")).unwrap();
    let mut progress = |_| {};
    let mut out = Out::new(&store, "d1", &ImportOptions::default(), &mut progress);
    let path = out.add_folder(None, "Inbox");
    let mut files = Files::default();
    files.add(got(&row(2, &here)));
    files.add(got(&row(2, &missing)));
    out.add_message(
        &path,
        Item { class: "IPM.Note".into(), subject: "Plans".into(), files: files.files, left_out: files.left_out, ..Default::default() },
    );
    let done = out.finish();
    assert_eq!(done.skips.into_iter().collect::<Vec<_>>(), [(SkipReason::AttachmentLink, 1)]);
    let m = store.load_folder("d1", &path).unwrap().messages.remove(0);
    let raw = store.load_body(&m.key).unwrap().unwrap();
    let names: Vec<String> = mailcraft_mime::parse(&raw).unwrap().attachments.into_iter().map(|a| a.name).collect();
    assert_eq!(names, ["Plan.pdf", "Plan.pdf (link).txt"]);
    drop(store);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A link's file is read only when it fits what's asked (a bigger one isn't opened), by a plain
/// path: none with `..` in it, none through a link or junction to a folder (which could lead to
/// another computer), and on Unix none of the system's own (a process's environment).
#[test]
fn a_link_s_file_is_read_only_by_a_plain_path() {
    let dir = std::env::temp_dir().join(format!("mailcraft-pst-link-paths-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("in")).unwrap();
    // The real path: macOS's temp folder is under /var, a link to /private/var, and a link is refused.
    #[cfg(not(windows))]
    let dir = std::fs::canonicalize(&dir).unwrap();
    let here = dir.join("in").join("plan.pdf");
    std::fs::write(&here, b"%PDF-1.7 plan").unwrap();
    let path = here.to_string_lossy().into_owned();
    assert_eq!(read_linked(&path, 100).as_deref(), Some(&b"%PDF-1.7 plan"[..]));
    assert_eq!(read_linked(&path, 13).as_deref(), Some(&b"%PDF-1.7 plan"[..]));
    assert!(read_linked(&path, 12).is_none(), "bigger than asked");
    let round = dir.join("in").join("..").join("in").join("plan.pdf");
    assert!(read_linked(&round.to_string_lossy(), 100).is_none(), "a path with .. in it");
    // A junction (Windows) or link (Unix) to the folder: refused, where the system lets a test make one.
    let link = dir.join("through");
    #[cfg(windows)]
    let made = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J", &link.to_string_lossy(), &dir.join("in").to_string_lossy()])
        .output()
        .is_ok_and(|o| o.status.success());
    #[cfg(unix)]
    let made = std::os::unix::fs::symlink(dir.join("in"), &link).is_ok();
    if made {
        assert!(std::fs::read(link.join("plan.pdf")).is_ok());
        assert!(read_linked(&link.join("plan.pdf").to_string_lossy(), 100).is_none(), "through a junction");
    }
    #[cfg(unix)]
    assert!(read_linked("/proc/self/environ", 1 << 20).is_none() && read_linked("/dev/zero", 1 << 20).is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Once a few reads of links' files were slow (a drive that is gone answers only when it times
/// out), the import reads no more of them.
#[test]
fn slow_link_reads_stop_reading_links() {
    use std::time::Duration;
    let reads = super::super::item::LinkReads::default();
    let asked = std::cell::Cell::new(0);
    let read = |_: &str, _: usize| {
        asked.set(asked.get() + 1);
        None
    };
    for _ in 0..10 {
        reads.read_with("C:\\gone\\plan.pdf", 100, &read, Duration::ZERO);
    }
    assert_eq!(asked.get(), 4);
    // Quick reads don't count.
    let quick = super::super::item::LinkReads::default();
    asked.set(0);
    for _ in 0..10 {
        quick.read_with("C:\\gone\\plan.pdf", 100, &read, Duration::from_secs(60));
    }
    assert_eq!(asked.get(), 10);
}

/// The files left out of the messages that came in are counted by reason, without counting
/// those messages as skipped; an item that didn't come in (of a kind with no place, counted as
/// itself) counts none.
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
    out.add_message(&path, Item { class: "IPM.Configuration.Junk".into(), ..mail(vec![link()]) });
    let done = out.finish();
    assert_eq!((done.messages, done.skipped), (2, 1));
    assert_eq!(
        done.skips.into_iter().collect::<Vec<_>>(),
        [(SkipReason::AttachmentUnreadable, 1), (SkipReason::AttachmentLink, 3), (SkipReason::ItemNotUsable, 1)]
    );
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

/// A picture the HTML shows comes in under its own name, still inline, and gives no paperclip:
/// the import and the source read back agree. It went out unnamed, and showed as `attachment-1`.
#[test]
fn a_shown_picture_keeps_its_name() {
    let it = Item {
        subject: "Logo".into(),
        from: EmailAddress::new("Ana", "ana@example.com"),
        date: 1_704_067_200,
        body: "Hi".into(),
        html: Some("<p>Hi</p><img src=\"cid:logo@studio.example\">".into()),
        files: vec![data_file("image001.png", "image/png", "logo@studio.example")],
        ..Default::default()
    };
    let (raw, glance) = message(it, "logo@x").unwrap();
    let p = mime::parse(&raw).unwrap();
    let parts: Vec<_> = p.attachments.iter().map(|a| (a.name.as_str(), a.inline)).collect();
    assert_eq!(parts, [("image001.png", true)]);
    let key = MessageKey { account: "d1".into(), folder: "INBOX".into(), validity: 1, uid: 1 };
    let full = mime::summary_from_full(&raw, key.clone(), Flags::default(), 0, 0);
    assert_eq!((summary_of(&raw, glance, key, 0).has_attachments, full.has_attachments), (false, false));
}

/// A file with a Content-ID the HTML doesn't show (or with no HTML at all) stays a file and keeps
/// it: listed, not inline, with a paperclip, the import and the source read back agreeing. It went
/// out without it.
#[test]
fn a_file_the_html_doesnt_show_keeps_its_content_id() {
    for html in [Some("<p>Hi</p><img src=\"cid:other@studio.example\">"), None] {
        let it = Item {
            subject: "Chart".into(),
            from: EmailAddress::new("Ana", "ana@example.com"),
            date: 1_704_067_200,
            body: "Hi".into(),
            html: html.map(String::from),
            files: vec![data_file("chart.png", "image/png", "<chart@studio.example>")],
            ..Default::default()
        };
        let (raw, glance) = message(it, "chart@x").unwrap();
        let p = mime::parse(&raw).unwrap();
        let parts: Vec<_> = p.attachments.iter().map(|a| (a.name.as_str(), a.inline, a.content_id.as_deref())).collect();
        assert_eq!(parts, [("chart.png", false, Some("chart@studio.example"))], "{html:?}");
        let key = MessageKey { account: "d1".into(), folder: "INBOX".into(), validity: 1, uid: 1 };
        let full = mime::summary_from_full(&raw, key.clone(), Flags::default(), 0, 0);
        assert_eq!((summary_of(&raw, glance, key, 0).has_attachments, full.has_attachments), (true, true), "{html:?}");
    }
}

/// A draft saved before files kept a Content-ID still reads, and one without it is saved as
/// before.
#[test]
fn a_draft_file_reads_with_or_without_a_content_id() {
    let old: mime::OutAttachment = serde_json::from_str(r#"{"name":"a.txt","mime":"text/plain","data":[104]}"#).unwrap();
    assert_eq!((old.name.as_str(), old.content_id.as_str()), ("a.txt", ""));
    assert!(!serde_json::to_string(&old).unwrap().contains("content_id"));
    let kept = mime::OutAttachment { content_id: "a@studio.example".into(), ..old };
    assert_eq!(serde_json::from_str::<mime::OutAttachment>(&serde_json::to_string(&kept).unwrap()).unwrap(), kept);
}
