//! Engine tests: files and the messages written: attached (from a path, or handed over by the
//! window), refused past 25 MB, previewed and saved, forwarded as they came; and the open
//! message saved as `.eml` and PDF.

use super::*;

/// A TrueType font the system has, when it has one of these (as the print crate's tests find one).
fn system_font() -> Option<Vec<u8>> {
    // Windows' fonts are in its folder (`%WINDIR%`), which needn't be `C:\Windows`.
    let windows = std::env::var_os("WINDIR").map_or_else(|| std::path::PathBuf::from("C:\\Windows"), std::path::PathBuf::from).join("Fonts");
    let candidates = [
        windows.join("segoeui.ttf"),
        windows.join("arial.ttf"),
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf".into(),
        "/System/Library/Fonts/Supplemental/Arial.ttf".into(),
    ];
    candidates.iter().find_map(|p| std::fs::read(p).ok())
}

/// The open message saved as `.eml`, and as PDF: in Helvetica with no fonts given, and with the
/// fonts the app gives (`print_fonts`) embedded. Windows and macOS always have one to give; a
/// Linux without DejaVu has none, and checks the first only.
#[test]
fn messages_save_as_eml_and_pdf() {
    let mut s = sample_session("export");
    let k = key_of(&s, "Storyboard review on Thursday");
    s.run("list.open", &k).unwrap();
    let dir = s.store.as_ref().unwrap().root().join("export");
    std::fs::create_dir_all(&dir).unwrap();
    let eml = dir.join("m.eml");
    s.run("message.saveAs", &json!({"path": eml.to_string_lossy()})).unwrap();
    assert_eq!(mailcraft_mime::parse(&std::fs::read(&eml).unwrap()).unwrap().subject, "Storyboard review on Thursday");
    let pdf = dir.join("m.pdf");
    let export = |s: &mut Session| {
        s.run("message.exportPdf", &json!({"path": pdf.to_string_lossy()})).unwrap();
        let bytes = std::fs::read(&pdf).unwrap();
        assert!(bytes.starts_with(b"%PDF-") && bytes.ends_with(b"%%EOF\n"));
        String::from_utf8_lossy(&bytes).into_owned()
    };
    assert!(s.print_fonts.is_none());
    let plain = export(&mut s);
    assert!(plain.contains("/BaseFont /Helvetica") && !plain.contains("/FontFile2"), "no fonts given: Helvetica");
    let Some(font) = system_font() else {
        if cfg!(any(windows, target_os = "macos")) {
            panic!("no system font to embed");
        }
        return;
    };
    s.print_fonts = Some((font, None));
    let embedded = export(&mut s);
    assert!(embedded.contains("/FontFile2"), "the font given is embedded");
}

/// Hand-run: the open message saved as PDF, with a system font, to the file
/// `MAILCRAFT_KEEP_PDF` names, to look at (`cargo test -p mailcraft-engine --lib
/// a_pdf_to_look_at -- --ignored`).
#[test]
#[ignore = "writes a PDF to look at, where MAILCRAFT_KEEP_PDF says"]
fn a_pdf_to_look_at() {
    let keep = std::env::var_os("MAILCRAFT_KEEP_PDF").expect("MAILCRAFT_KEEP_PDF: where the PDF goes");
    let mut s = sample_session("export-kept");
    let k = key_of(&s, "Storyboard review on Thursday");
    s.run("list.open", &k).unwrap();
    s.print_fonts = system_font().map(|f| (f, None));
    s.run("message.exportPdf", &json!({"path": std::path::PathBuf::from(keep).to_string_lossy()})).unwrap();
}

#[test]
fn attachments_preview_and_save_all() {
    let mut s = sample_session("attachments");
    let k = key_of(&s, "Your proof for the festival poster is ready");
    s.run("list.open", &k).unwrap();
    let r = s.run("attachment.preview", &json!({"index": 0})).unwrap();
    assert_eq!((r["kind"].as_str(), r["name"].as_str()), (Some("text"), Some("proof-notes.txt")));
    assert!(!r["text"].as_str().unwrap().is_empty());
    let k = key_of(&s, "Character turnarounds");
    s.run("list.open", &k).unwrap();
    assert!(s.run("attachment.preview", &json!({"index": 0})).is_err(), "a picture isn't text");
    let dir = s.store.as_ref().unwrap().root().join("out");
    std::fs::create_dir_all(&dir).unwrap();
    let saved = s.run("attachment.saveAll", &json!({"dir": dir.to_string_lossy()})).unwrap();
    // Each file's own bytes (all of them come from one parse of the message).
    let r = s.reading.as_ref().unwrap();
    let raw = s.readable_raw(&r.key).unwrap();
    let files: Vec<&mailcraft_mime::AttachmentInfo> = r.parsed.as_ref().unwrap().attachments.iter().filter(|a| !a.inline).collect();
    assert_eq!(saved["saved"].as_array().map(Vec::len), Some(files.len()));
    for (path, a) in saved["saved"].as_array().unwrap().iter().zip(files) {
        assert_eq!(std::fs::read(path.as_str().unwrap()).unwrap(), mailcraft_mime::attachment_data(&raw, a.index).unwrap(), "{}", a.name);
    }
    let again = s.run("attachment.saveAll", &json!({"dir": dir.to_string_lossy()})).unwrap();
    assert!(again["saved"][0].as_str().unwrap().contains("(2)"), "names already taken get a number");
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
    // Only the ones asked for (those selected).
    let only = s.run("attachment.saveAll", &json!({"dir": dir.to_string_lossy(), "indices": [9999]})).unwrap();
    assert_eq!(only["saved"].as_array().map(Vec::len), Some(0));
    let index = s.reading.as_ref().and_then(|r| r.parsed.as_ref()).map(|p| p.attachments[0].index).unwrap();
    let only = s.run("attachment.saveAll", &json!({"dir": dir.to_string_lossy(), "indices": [index]})).unwrap();
    assert_eq!(only["saved"].as_array().map(Vec::len), Some(1));
    assert!(s.run("attachment.saveAll", &json!({"dir": dir.to_string_lossy(), "indices": "junk"})).is_ok(), "junk means all");
    assert!(s.run("attachment.saveAll", &json!({"dir": "/no/such/folder"})).is_err());
    assert_eq!(mailcraft_mime::craft_app_for("application/pdf", "a.pdf"), Some(("pdfcraft", "PdfCraft")));
    assert_eq!(mailcraft_mime::craft_app_for("application/octet-stream", "q.XLSX").map(|a| a.0), Some("gridcraft"));
    assert_eq!(mailcraft_mime::text_preview_kind("message/rfc822", "fwd.eml"), Some("message"));
}

/// Save all attachments numbers a name that's taken up to (999); with all of those taken too,
/// it's refused, and no file there is written over.
#[test]
fn saving_all_never_writes_over_a_file() {
    let mut s = sample_session("save-all-full");
    s.run("list.open", &key_of(&s, "Character turnarounds")).unwrap();
    let name = s.reading.as_ref().and_then(|r| r.parsed.as_ref()).and_then(|p| p.attachments.iter().find(|a| !a.inline)).map(|a| a.name.clone());
    let name = mailcraft_mime::sanitize_filename(&name.unwrap());
    let (stem, ext) = name.rsplit_once('.').unwrap();
    let dir = s.store.as_ref().unwrap().root().join("full");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(&name), b"mine").unwrap();
    for n in 2..=999 {
        std::fs::write(dir.join(format!("{stem} ({n}).{ext}")), b"mine").unwrap();
    }
    let r = s.run("attachment.saveAll", &json!({"dir": dir.to_string_lossy()}));
    assert!(matches!(&r, Err(CmdError::Failed(m)) if m.contains("(999)")), "{r:?}");
    assert_eq!(std::fs::read(dir.join(format!("{stem} (999).{ext}"))).unwrap(), b"mine");
    let r = s.run("attachment.save", &json!({"index": 0, "path": dir.join("tool.exe").to_string_lossy(), "open": true}));
    assert!(matches!(&r, Err(CmdError::Disabled(m)) if m.contains("could run a program")), "{r:?}");
}

/// A file picked in the browser, or a picture pasted, is handed over as it is and named by the
/// command (a JSON list of its bytes made each byte a value of 24 bytes); each is taken once,
/// and only the last few are kept.
#[test]
fn files_handed_over_are_attached_as_they_are() {
    let mut s = sample_session("handed");
    s.run("mail.new", &json!({})).unwrap();
    let file: Vec<u8> = (0..=255u8).cycle().take(300_000).collect();
    let n = s.hand(file.clone());
    s.run("compose.attach", &json!({"name": "big.bin", "handed": n})).unwrap();
    const DOT: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0x0D, 0x49, 0x48, 0x44, 0x52, 0, 0, 0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0, 0x1F, 0x15,
        0xC4, 0x89,
    ];
    let picture = s.hand(DOT.to_vec());
    s.run("compose.insertPicture", &json!({"name": "dot.png", "handed": picture})).unwrap();
    let c = s.compose.as_ref().unwrap();
    assert_eq!(c.attachments.last().map(|a| (a.name.as_str(), &a.data)), Some(("big.bin", &file)));
    assert_eq!(c.inline.last().map(|p| p.data.as_slice()), Some(DOT));
    // Taken once; one never handed over is an error, not an empty file.
    assert!(matches!(s.run("compose.attach", &json!({"name": "again.bin", "handed": n})), Err(CmdError::Params(_))));
    assert!(matches!(s.run("compose.attach", &json!({"name": "x.bin", "handed": 999})), Err(CmdError::Params(_))));
    let kept: Vec<u64> = (0..6u8).map(|i| s.hand(vec![i])).collect();
    assert_eq!(s.handed.len(), 4, "only the last few are kept");
    assert!(s.run("compose.attach", &json!({"name": "old.bin", "handed": kept[0]})).is_err());
    s.run("compose.attach", &json!({"name": "new.bin", "handed": kept[5]})).unwrap();
}

/// A file handed over for a command that's refused before it looks for it (no message is being
/// written, there's no such command) isn't kept: nothing would take it, and it stayed, up to
/// 25 MB of it, until four more were handed over. Others handed over stay for their commands.
#[test]
fn a_file_handed_to_a_refused_command_is_let_go() {
    let mut s = sample_session("handed-refused");
    let other = s.hand(vec![7; 10]);
    let n = s.hand(vec![1; 1000]);
    let r = s.run("compose.attach", &json!({"name": "a.bin", "handed": n}));
    assert!(matches!(r, Err(CmdError::Disabled(_))), "{r:?}");
    assert_eq!(s.handed.iter().map(|(k, _)| *k).collect::<Vec<_>>(), [other]);
    let n = s.hand(vec![2; 1000]);
    assert!(matches!(s.run("compose.nothing", &json!({"handed": n})), Err(CmdError::Unknown(_))));
    assert_eq!(s.handed.iter().map(|(k, _)| *k).collect::<Vec<_>>(), [other]);
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.attach", &json!({"name": "b.bin", "handed": other})).unwrap();
    assert!(s.handed.is_empty());
}

/// Attaching from a path reads a plain file only: a folder, a device that reads forever
/// (`/dev/zero`, which reports a size of 0) or a pipe that blocks are refused, at once.
#[test]
fn attaching_reads_plain_files_only() {
    let mut s = sample_session("attach-paths");
    s.run("mail.new", &json!({})).unwrap();
    let dir = s.store.as_ref().unwrap().root().to_path_buf();
    let e = s.run("compose.attach", &json!({"path": dir.to_string_lossy()})).unwrap_err().to_string();
    assert!(e.contains("isn't a file"), "{e}");
    let device = if cfg!(windows) { r"\\.\NUL" } else { "/dev/zero" };
    // Refused for what it is, before any of it is read: read, `/dev/zero` is too big and `NUL`
    // is attached empty. Windows gives no facts about `NUL` at all ("Incorrect function"): then
    // the refusal is that, which a missing file's isn't.
    let e = s.run("compose.attach", &json!({"path": device})).unwrap_err().to_string();
    match std::fs::metadata(device) {
        Ok(_) => assert!(e.contains("isn't a file"), "{e}"),
        Err(why) => {
            assert_ne!(why.kind(), std::io::ErrorKind::NotFound, "{device} is there");
            assert!(e.contains(&why.to_string()), "{e}");
        }
    }
    assert_eq!(s.compose.as_ref().map(|c| c.attachments.len()), Some(0));
    let file = dir.join("ok.txt");
    std::fs::write(&file, b"hi").unwrap();
    s.run("compose.attach", &json!({"path": file.to_string_lossy()})).unwrap();
    assert_eq!(s.compose.as_ref().map(|c| c.attachments.len()), Some(1));
}

/// A file over 25 MB is refused in the words of attaching, however it comes, and so is one that
/// takes the message past 25 MB.
#[test]
fn attachments_over_25_mb_are_refused() {
    let mut s = sample_session("attach-big");
    s.run("mail.new", &json!({})).unwrap();
    let dir = s.store.as_ref().unwrap().root().to_path_buf();
    let big = dir.join("big.bin");
    std::fs::File::create(&big).unwrap().set_len((25 << 20) + 1).unwrap();
    let path = big.to_string_lossy().into_owned();
    assert_eq!(s.run("compose.attach", &json!({ "path": path })), Err(CmdError::Failed(format!("{path} is larger than 25 MB"))));
    let n = s.hand(vec![0; (25 << 20) + 1]);
    assert_eq!(s.run("compose.attach", &json!({"name": "big.bin", "handed": n})), Err(CmdError::Failed("attachments are limited to 25 MB".into())));
    let n = s.hand(vec![0; 25 << 20]);
    s.run("compose.attach", &json!({"name": "full.bin", "handed": n})).unwrap();
    let e = s.run("compose.attach", &json!({"name": "one.txt", "data": [1]}));
    assert_eq!(e, Err(CmdError::Failed("attachments are limited to 25 MB in total".into())));
    assert_eq!(s.compose.as_ref().map(|c| c.attachments.len()), Some(1));
}

/// A forwarded text file in Windows-1252 goes as it came and keeps its charset: it went out as
/// bare `text/plain`, so the copy read as UTF-8, with replacement characters.
#[test]
fn a_forward_keeps_a_text_files_charset() {
    let mut s = sample_session("forward-charset");
    let raw = b"From: Sam Okafor <sam@studio.example>\r\nTo: riley@studio.example\r\nSubject: Menu\r\nDate: Thu, 29 Oct 2026 10:00:00 +0000\r\nMIME-Version: 1.0\r\n\
Content-Type: multipart/mixed; boundary=\"M\"\r\n\r\n--M\r\nContent-Type: text/plain\r\n\r\nThe menu.\r\n\
--M\r\nContent-Type: text/plain; charset=windows-1252\r\nContent-Disposition: attachment; filename=\"menu.txt\"\r\n\r\nCaf\xe9 au lait\r\n--M--\r\n";
    let key = mailcraft_model::MessageKey { uid: 9302, ..next_key(&s, "sample", "INBOX") };
    let key = file_whole(&mut s, key, raw);
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    s.run("list.open", &json!({"key": key.to_id()})).unwrap();
    s.run("mail.forward", &json!({})).unwrap();
    let c = s.compose.clone().unwrap();
    let a = &c.attachments[0];
    assert_eq!((c.attachments.len(), a.name.as_str(), a.mime.as_str()), (1, "menu.txt", "text/plain; charset=windows-1252"));
    assert!(a.data.starts_with(b"Caf\xe9 au lait"));
    let d = mailcraft_mime::Draft { subject: c.subject.clone(), body: c.body.clone(), attachments: c.attachments.clone(), ..Default::default() };
    let sent = mailcraft_mime::build(&d, &mailcraft_model::EmailAddress::new("Riley", "riley@studio.example"), NOW, "<fwd@x>").unwrap();
    assert!(mailcraft_mime::attachment_text(&sent, 0).is_some_and(|t| t.starts_with("Caf\u{e9} au lait")));
}

/// Messages dragged from the list onto a message being written (issue #270) are attached whole,
/// each an `.eml` named by its subject, as Forward as attachment attaches one: the selected ones,
/// or those it names. A message not on this computer yet attaches none of them.
#[test]
fn messages_are_attached_whole() {
    let mut s = sample_session("attach-messages");
    s.prefs.new_mail_window = true;
    let keys: Vec<_> = s.list.iter().map(|m| m.key.clone()).take(3).collect();
    let subjects: Vec<String> = keys.iter().map(|k| s.summary(k).unwrap().subject.clone()).collect();
    s.run("mail.new", &json!({})).unwrap();
    // Two selected (in the list, while it's written in a window of its own).
    s.run("list.select", &json!({"key": keys[0].to_id()})).unwrap();
    s.run("list.selectRange", &json!({"key": keys[1].to_id()})).unwrap();
    assert!(s.compose.is_some() && s.selection.len() == 2);
    let answer = s.run("compose.attachMessages", &json!({})).unwrap();
    assert_eq!(answer["count"], 2);
    let c = s.compose.clone().unwrap();
    for (a, (key, subject)) in c.attachments.iter().zip(keys.iter().zip(&subjects)) {
        // Its name one any system can save (`Re: …` is `Re_ …`).
        assert_eq!((a.name.clone(), a.mime.as_str()), (mailcraft_mime::sanitize_filename(&format!("{subject}.eml")), "message/rfc822"));
        assert_eq!(Some(a.data.clone()), s.store.as_ref().unwrap().read_body(key), "the message whole, as it came");
    }
    // Named by their keys.
    s.run("compose.attachMessages", &json!({"keys": [keys[2].to_id()]})).unwrap();
    assert_eq!(s.compose.as_ref().unwrap().attachments.len(), 3);
    // One not downloaded yet: none goes, and it's said which.
    let missing = mailcraft_model::MessageKey { uid: 99_999, ..keys[0].clone() };
    let err = s.run("compose.attachMessages", &json!({"keys": [keys[0].to_id(), missing.to_id()]})).unwrap_err();
    assert!(err.to_string().contains("still downloading"), "{err}");
    assert_eq!(s.compose.as_ref().unwrap().attachments.len(), 3);
    for bad in [json!({"keys": "x"}), json!({"keys": [1]}), json!({"keys": ["not a key"]})] {
        assert!(s.run("compose.attachMessages", &bad).is_err(), "{bad}");
    }
}
