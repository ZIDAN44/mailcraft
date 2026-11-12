//! Engine tests: what the system hands MailCraft as its mail app, `mailto:` links and the
//! `.eml`, `.ics` and `.vcf` files a user opens with it.

use super::*;

/// A scratch folder for one test, emptied when the test is over (pass or fail).
struct TempFolder(std::path::PathBuf);

impl TempFolder {
    fn new(label: &str) -> TempFolder {
        let folder = std::env::temp_dir().join(format!("mailcraft-open-{}-{label}", std::process::id()));
        if folder.exists() {
            std::fs::remove_dir_all(&folder).unwrap();
        }
        std::fs::create_dir_all(&folder).unwrap();
        TempFolder(folder)
    }

    /// Writes `contents` to `file_name` in the folder, and hands back where it went.
    fn put(&self, file_name: &str, contents: &[u8]) -> std::path::PathBuf {
        let at = self.0.join(file_name);
        std::fs::write(&at, contents).unwrap();
        at
    }
}

impl Drop for TempFolder {
    fn drop(&mut self) {
        // Leftovers in the temporary folder are harmless; a failed clean-up isn't a failed test.
        std::fs::remove_dir_all(&self.0).ok();
    }
}

/// Runs `file.open` on `path`.
fn open_path(s: &mut Session, path: &std::path::Path) -> Result<Value, CmdError> {
    s.run("file.open", &json!({ "path": path.display().to_string() }))
}

#[test]
fn a_mailto_link_starts_a_message_and_attaches_nothing() {
    let mut s = sample_session("mailto");
    let link = "mailto:lee@greenhouse.example?subject=Seed%20order&cc=mo@farm.example&body=Dear%20Lee%2C%0D%0Athanks&bcc=ned@farm.example&attach=%2Fetc%2Fpasswd";
    let r = s.run("mail.mailto", &json!({ "url": link })).unwrap();
    let draft = s.compose.as_ref().unwrap();
    assert_eq!(r["compose"], draft.id);
    assert_eq!(draft.to, "lee@greenhouse.example");
    assert_eq!(draft.cc, "mo@farm.example");
    assert_eq!(draft.bcc, "ned@farm.example");
    assert_eq!(draft.subject, "Seed order");
    assert!(draft.body.starts_with("Dear Lee,\nthanks"), "{:?}", draft.body);
    assert!(draft.show_cc, "the copy lines are shown when the link fills them");
    assert!(draft.attachments.is_empty(), "`attach=` in a link is ignored");

    for bad in [json!({ "url": "ftp://farm.example/seeds" }), json!({})] {
        assert!(s.run("mail.mailto", &bad).is_err(), "{bad}");
    }
    let sending = s.take_jobs().into_iter().filter(|j| matches!(j, Job::Send { .. })).count();
    assert_eq!(sending, 0, "the draft waits for the user");
    done(s);
}

#[test]
fn a_message_file_is_shown() {
    let mut s = sample_session("open-eml");
    let scratch = TempFolder::new("eml");
    let source = concat!(
        "Date: Sat, 7 Nov 2026 14:15:00 +0100\r\n",
        "Subject: Greenhouse keys\r\n",
        "From: Lee Park <lee@greenhouse.example>\r\n",
        "To: pat@studio.example\r\n",
        "\r\n",
        "Under the blue pot.\r\n",
    );
    let path = scratch.put("Greenhouse keys.eml", source.as_bytes());
    let shown = open_path(&mut s, &path).unwrap();
    assert_eq!(shown["kind"], "message");
    assert_eq!(shown["name"], "Greenhouse keys.eml");
    assert_eq!(shown["subject"], "Greenhouse keys");
    assert!(shown["from"].as_str().unwrap().contains("lee@greenhouse.example"), "{shown}");
    assert!(shown["text"].as_str().unwrap().contains("blue pot"), "{shown}");

    // Not there, a folder, or nothing at all: each is an error, not a crash.
    assert!(open_path(&mut s, &path.with_file_name("gone.eml")).is_err());
    assert!(open_path(&mut s, &scratch.0).is_err());
    assert!(s.run("file.open", &json!({ "path": "   " })).is_err());
    done(s);
}

#[test]
fn calendar_and_contacts_files_are_added() {
    let mut s = sample_session("open-pim");
    let scratch = TempFolder::new("pim");
    let calendar = [
        "BEGIN:VCALENDAR",
        "VERSION:2.0",
        "BEGIN:VEVENT",
        "UID:harvest@farm.example",
        "SUMMARY:Harvest day",
        "DTSTART:20261112T120000Z",
        "DTEND:20261112T150000Z",
        "END:VEVENT",
        // An event with nothing but a start is passed over and counted.
        "BEGIN:VEVENT",
        "UID:bare@farm.example",
        "DTSTART:20261113T120000Z",
        "END:VEVENT",
        "END:VCALENDAR",
        "",
    ]
    .join("\r\n");
    // The extension is matched whatever its case.
    let added = open_path(&mut s, &scratch.put("harvest.Ics", calendar.as_bytes())).unwrap();
    assert_eq!(added["kind"], "calendar", "{added}");
    assert_eq!((added["added"].as_u64(), added["skipped"].as_u64()), (Some(1), Some(1)), "{added}");
    assert!(s.find_event("harvest@farm.example").is_some());
    let on_screen = s.run("calendar.goto", &json!({})).unwrap();
    assert_eq!(on_screen["date"], "2026-11-12", "the calendar moves to the first added event");

    let cards = [
        "BEGIN:VCARD",
        "VERSION:3.0",
        "FN:Mo Ortiz",
        "EMAIL:mo@farm.example",
        "END:VCARD",
        "BEGIN:VCARD",
        "VERSION:3.0",
        "FN:Nobody",
        "EMAIL:no at sign here",
        "END:VCARD",
        "",
    ]
    .join("\r\n");
    // A name that says nothing: the first line tells what the file is.
    let added = open_path(&mut s, &scratch.put("export.dat", cards.as_bytes())).unwrap();
    assert_eq!(added["kind"], "contacts", "{added}");
    assert_eq!((added["added"].as_u64(), added["skipped"].as_u64()), (Some(1), Some(1)), "{added}");
    assert!(s.people("Ortiz").iter().any(|p| p.card.display() == "Mo Ortiz"));
    done(s);
}

/// With no account there's nowhere to put them: said, not crashed.
#[test]
fn without_an_account_nothing_is_added() {
    let mut s = Session::open(None, mailcraft_model::dates::Clock::new(NOW, 0));
    let scratch = TempFolder::new("no-account");
    let card = scratch.put("one.vcf", b"BEGIN:VCARD\r\nFN:Solo\r\nEND:VCARD\r\n");
    assert!(matches!(open_path(&mut s, &card), Err(CmdError::Disabled(_))));
    let link = s.run("mail.mailto", &json!({ "url": "mailto:solo@farm.example" }));
    assert!(matches!(link, Err(CmdError::Disabled(_))));
}

/// Issue #38: message files dropped on the window go into the folder open, filed by the network
/// layer, which says how many went in; anything but a folder open (All Inboxes), or no files,
/// is refused. An Outlook `.msg` opened with MailCraft is read by the network layer and shown
/// when it comes.
#[test]
fn message_files_go_into_the_folder_open_and_a_msg_is_shown_when_read() {
    let mut s = sample_session("message-files");
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    let r = s.run("file.addToFolder", &json!({"paths": ["C:/mail/Quarterly figures.msg", "  ", "C:/mail/hello.eml"]})).unwrap();
    assert_eq!((r["folder"].as_str(), r["files"].as_u64()), (Some("INBOX"), Some(2)));
    let jobs = s.take_jobs();
    assert!(
        jobs.iter()
            .any(|j| matches!(j, Job::FileMessages { account, folder, paths } if account == "sample" && folder == "INBOX" && paths.len() == 2)),
        "{jobs:?}"
    );
    assert!(s.run("file.addToFolder", &json!({"paths": []})).is_err());
    assert!(s.run("file.addToFolder", &json!({"paths": ["a.msg"], "folder": "Nowhere"})).is_err());
    s.run("folder.unified", &json!({"role": "inbox"})).unwrap();
    assert!(s.run("file.addToFolder", &json!({"paths": ["a.msg"]})).is_err(), "no one folder open");
    s.apply(Event::MessagesFiled {
        account: "sample".into(),
        folder: "INBOX".into(),
        added: 1,
        failed: vec![("hello.eml".into(), "not a message".into())],
    });
    assert_eq!(s.take_filed().iter().map(|f| (f.added, f.failed.len())).collect::<Vec<_>>(), [(1, 1)]);
    assert!(s.take_filed().is_empty());
    assert!(mailcraft_engine_is_message_file("x.MSG") && mailcraft_engine_is_message_file("y.eml") && !mailcraft_engine_is_message_file("z.pdf"));

    // A .msg opened: read elsewhere, then shown.
    let t = TempFolder::new("msg");
    let at = t.put("Quarterly figures.msg", &[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1, 0, 0]);
    let r = open_path(&mut s, &at).unwrap();
    assert_eq!(r["kind"], "reading");
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::ReadMessageFile { path } if path.ends_with("Quarterly figures.msg"))));
    let raw = b"From: Sam <sam@studio.test>\r\nTo: me@studio.test\r\nSubject: Quarterly figures\r\n\r\nHello.\r\n".to_vec();
    s.apply(Event::MessageFileRead { path: at.display().to_string(), raw: Ok(raw) });
    let shown = s.opened_file.take().unwrap().unwrap();
    assert_eq!(
        (shown["kind"].as_str(), shown["subject"].as_str(), shown["name"].as_str()),
        (Some("message"), Some("Quarterly figures"), Some("Quarterly figures.msg"))
    );
    // In a message window of its own, as one opened from a folder is (issue #56).
    let key = shown["key"].as_str().and_then(mailcraft_model::MessageKey::from_id).unwrap();
    assert_eq!(s.opened_file_name(&key), Some("Quarterly figures.msg"));
    assert!(s.windows.iter().any(|w| w.key == key && w.parsed.as_ref().is_some_and(|p| p.text.contains("Hello."))));
    s.apply(Event::MessageFileRead { path: at.display().to_string(), raw: Err("the file is damaged".into()) });
    assert!(s.opened_file.take().unwrap().is_err());
}

/// Issue #56: a message file opens in a message window of its own, as one double-clicked in a
/// folder does, in no account and no folder. Who else it went to, what's attached and where its
/// links really go are shown; nothing in it reaches the web until its pictures are asked for,
/// even from a sender whose pictures always load; and once its window closes, nothing of it is
/// kept.
#[test]
fn a_message_file_opens_in_a_window_of_its_own_and_leaves_no_trace() {
    let mut s = sample_session("open-window");
    let scratch = TempFolder::new("window");
    let source = concat!(
        "From: Billing <billing@paypa1.example>\r\n",
        "To: Ana <ana@x.example>\r\n",
        "Cc: Ben <ben@x.example>\r\n",
        "Subject: Invoice\r\n",
        "MIME-Version: 1.0\r\n",
        "Content-Type: multipart/mixed; boundary=b\r\n\r\n",
        "--b\r\nContent-Type: text/html\r\n\r\n",
        "<p>Dear <b>customer</b>, <a href=\"https://evil.example/login\">https://bank.example/<b>login</b></a>",
        " or <a href=\"https://bank.example/help/\">bank.example/help</a></p><img src=\"https://t.example/o.gif\" width=300 alt=\"logo\">\r\n",
        "--b\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=update.exe\r\n\r\nMZ\r\n--b--\r\n",
    );
    let path = scratch.put("inspect me.eml", source.as_bytes());
    let messages = s.list.len();
    let reading = s.reading.as_ref().map(|r| r.key.clone());
    s.prefs.trusted_senders.insert("billing@paypa1.example".into());
    s.take_jobs();

    let shown = open_path(&mut s, &path).unwrap();
    let key = shown["key"].as_str().and_then(mailcraft_model::MessageKey::from_id).unwrap();
    assert_eq!(s.opened_file_name(&key), Some("inspect me.eml"));
    let w = s.windows.iter().find(|w| w.key == key).cloned().unwrap();
    let p = w.parsed.clone().unwrap();
    assert_eq!((p.subject.as_str(), p.cc.len()), ("Invoice", 1));
    assert!(p.attachments.iter().any(|a| a.name == "update.exe"));
    // A link whose words name another place says where it goes, once, after its last word; one
    // whose words name it doesn't.
    let all: String = p.layout.iter().flat_map(texts).chain(p.rich.iter().flatten().map(|sp| sp.text.clone())).collect();
    assert!(all.contains("https://bank.example/login <https://evil.example/login>"), "{all}");
    assert!(!all.contains("<https://bank.example/help/>"), "{all}");
    assert!(p.text.contains("https://evil.example/login"), "shown as plain text, it says so too: {}", p.text);
    assert_eq!(s.list.len(), messages, "nothing added to a folder");
    assert_eq!(s.reading.as_ref().map(|r| r.key.clone()), reading, "the reading pane is as it was");

    // As the open message (what its window's buttons act on): its source, its files, its PDF.
    s.reading = Some(w);
    // Its pictures from the web: only when asked for this one. Its sender's pictures always
    // load and the setting loads them by themselves, and still nothing is fetched: who it says
    // it's from may be anyone.
    s.prefs.auto_pictures = true;
    assert_eq!(s.remote_pictures(), ["https://t.example/o.gif"]);
    assert!(!s.pictures_allowed(), "not on its sender's standing, nor by the setting");
    assert!(!s.take_jobs().iter().any(|j| matches!(j, Job::FetchUrl { .. })));
    assert_eq!(s.attachment_bytes(p.attachments.iter().find(|a| a.name == "update.exe").unwrap().index).unwrap(), b"MZ");
    assert!(s.message_source().unwrap().starts_with(b"From: Billing"));
    assert!(s.message_pdf().is_ok());
    s.reading = None;
    let shown_source = s.run("message.source", &json!({"key": key.to_id()})).unwrap();
    assert!(shown_source["text"].as_str().unwrap().contains("Subject: Invoice"), "{shown_source}");
    let asked = s.take_jobs();
    assert!(!asked.iter().any(|j| matches!(j, Job::FetchUrl { .. } | Job::FetchBody { .. })), "nothing asked of a server: {asked:?}");
    // Asked for (Show pictures): this message's are fetched, and its sender gains nothing by it,
    // even asked to always show theirs.
    s.prefs.trusted_senders.clear();
    s.reading = s.windows.iter().find(|w| w.key == key).cloned();
    let shown_pictures = s.run("message.showPictures", &json!({"always": true})).unwrap();
    assert_eq!((shown_pictures["pictures"].as_u64(), shown_pictures["trusted"].as_str()), (Some(1), None), "{shown_pictures}");
    assert!(s.pictures_allowed() && s.prefs.trusted_senders.is_empty());
    let asked = s.take_jobs();
    assert!(asked.iter().any(|j| matches!(j, Job::FetchUrl { url } if url == "https://t.example/o.gif")), "{asked:?}");
    s.reading = None;
    // It's in no list: the list's commands don't reach it.
    assert!(s.run("list.open", &json!({"key": key.to_id()})).is_err());

    // Closed: nothing of it is kept, and it can't be opened again by its key.
    s.run("message.closeWindow", &json!({"key": key.to_id()})).unwrap();
    assert!(s.opened_files.is_empty() && s.windows.is_empty());
    assert!(s.run("message.popOut", &json!({"key": key.to_id()})).is_err());

    // Past the windows open at once, the oldest goes, and its file with it.
    use crate::session::MAX_WINDOWS;
    for n in 0..=MAX_WINDOWS {
        open_path(&mut s, &scratch.put(&format!("{n}.eml"), source.as_bytes())).unwrap();
    }
    assert_eq!((s.windows.len(), s.opened_files.len()), (MAX_WINDOWS, MAX_WINDOWS));
    assert!(s.windows.iter().all(|w| s.opened_file_name(&w.key).is_some()));
    done(s);
}

/// The words of laid-out blocks, table cells included.
fn texts(b: &mailcraft_mime::layout::Block) -> Vec<String> {
    match b {
        mailcraft_mime::layout::Block::Text(lines) => lines.iter().flatten().map(|sp| sp.text.clone()).collect(),
        mailcraft_mime::layout::Block::Table(t) => t.rows.iter().flatten().flat_map(|c| c.blocks.iter().flat_map(texts)).collect(),
    }
}

fn mailcraft_engine_is_message_file(path: &str) -> bool {
    crate::cmd::is_message_file(path)
}

/// Print and Save as PDF lay a message out as it's read (it came out as lines of text, its
/// tables drawn with dashes and its pictures as their names): its pictures are in the PDF, the
/// one it carries always, one from the web once it has been shown, a tracking picture never;
/// its links can be clicked.
#[test]
fn a_message_prints_as_it_is_laid_out() {
    const DOT: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";
    let mut s = sample_session("print-layout");
    let scratch = TempFolder::new("print");
    let source = format!(
        concat!(
            "From: Shop <orders@shop.example>\r\nTo: Ana <ana@x.example>\r\nSubject: Shipped\r\nMIME-Version: 1.0\r\n",
            "Content-Type: multipart/related; boundary=b\r\n\r\n",
            "--b\r\nContent-Type: text/html; charset=utf-8\r\n\r\n",
            "<table width=\"600\" bgcolor=\"#f3f3f3\"><tr><td width=\"150\"><img src=\"cid:logo@shop\" width=\"40\" height=\"40\" alt=\"logo\"></td>",
            "<td><a href=\"https://shop.example/track\">Track package</a></td></tr>",
            "<tr><td colspan=\"2\"><img src=\"https://cdn.shop.example/item.png\" width=\"90\" height=\"90\" alt=\"the item\">",
            "<img src=\"https://t.shop.example/open.gif?id=1\" width=\"1\" height=\"1\"></td></tr></table>\r\n",
            "--b\r\nContent-Type: image/png\r\nContent-ID: <logo@shop>\r\nContent-Transfer-Encoding: base64\r\n",
            "Content-Disposition: inline; filename=logo.png\r\n\r\n{dot}\r\n--b--\r\n",
        ),
        dot = DOT
    );
    let path = scratch.put("shipped.eml", source.as_bytes());
    let shown = open_path(&mut s, &path).unwrap();
    let key = shown["key"].as_str().and_then(mailcraft_model::MessageKey::from_id).unwrap();
    s.reading = s.windows.iter().find(|w| w.key == key).cloned();
    let p = s.reading.as_ref().and_then(|r| r.parsed.clone()).unwrap();
    assert!(!p.layout.is_empty(), "laid out in columns");
    let pictures = |pdf: &[u8]| {
        let text = String::from_utf8_lossy(pdf).to_string();
        let dict = text.split("/XObject <<").nth(1).and_then(|r| r.split(">>").next()).unwrap_or("").to_string();
        (dict.matches("/Im").count(), text.matches("/URI (https://shop.example/track)").count())
    };
    // The picture it carries is in it, and its link; the one from the web isn't fetched for it.
    let pdf = s.message_pdf().unwrap();
    assert!(pdf.starts_with(b"%PDF-") && pdf.ends_with(b"%%EOF\n"));
    assert_eq!(pictures(&pdf), (1, 1));
    assert!(!s.take_jobs().iter().any(|j| matches!(j, Job::FetchUrl { .. })), "a PDF fetches nothing");
    // Shown (the reader asked), it's in it too; the tracking picture never is.
    s.run("message.showPictures", &json!({})).unwrap();
    let png = base64_decode(DOT);
    s.apply(Event::Fetched { url: "https://cdn.shop.example/item.png".into(), data: Some(png.clone()), error: None });
    s.apply(Event::Fetched { url: "https://t.shop.example/open.gif?id=1".into(), data: Some(png), error: None });
    assert_eq!(pictures(&s.message_pdf().unwrap()), (2, 1));
    // A picture that can't be read isn't one: the PDF is made without it.
    s.apply(Event::Fetched { url: "https://cdn.shop.example/item.png".into(), data: Some(b"not a picture".to_vec()), error: None });
    assert_eq!(pictures(&s.message_pdf().unwrap()), (1, 1));
    s.reading = None;
}

fn base64_decode(text: &str) -> Vec<u8> {
    let value = |c: u8| match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    };
    let bits: Vec<u8> = text.bytes().filter_map(value).collect();
    bits.chunks(4)
        .flat_map(|c| {
            let n = c.iter().fold(0u32, |n, b| (n << 6) | u32::from(*b)) << (6 * (4 - c.len()));
            [(n >> 16) as u8, (n >> 8) as u8, n as u8].into_iter().take(c.len().saturating_sub(1))
        })
        .collect()
}
