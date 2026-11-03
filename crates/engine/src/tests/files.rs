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
    s.apply(Event::MessageFileRead { path: at.display().to_string(), raw: Err("the file is damaged".into()) });
    assert!(s.opened_file.take().unwrap().is_err());
}

fn mailcraft_engine_is_message_file(path: &str) -> bool {
    crate::cmd::is_message_file(path)
}
