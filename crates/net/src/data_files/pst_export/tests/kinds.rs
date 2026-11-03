//! Mail that is more than a note through a data file: attached messages, signed and encrypted
//! mail, meeting mail and reports, exported as Outlook keeps them and read back by the import.

use super::*;
use mailcraft_mime::smime::{self, Protection};
use outlook_pst::messaging::attachment::{Attachment as _, AttachmentData, UnicodeAttachment};

/// The S/MIME fixtures were made on 2026-10-24; their certificates run for two years.
const NOW: i64 = 1_792_900_000;

fn riley() -> mailcraft_smime::Identity {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../smime/testdata/riley-aes.p12");
    mailcraft_smime::read_p12(&std::fs::read(path).unwrap(), "secret").unwrap()
}

fn scratch(label: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("mailcraft-pst-export-kinds-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn draft(subject: &str, body: &str, attachments: Vec<mime::OutAttachment>) -> mime::Draft {
    mime::Draft {
        to: vec![EmailAddress::new("Ana", "ana@studio.example")],
        subject: subject.into(),
        body: body.into(),
        attachments,
        ..Default::default()
    }
}

fn build(d: &mime::Draft, id: &str) -> Vec<u8> {
    mime::build(d, &EmailAddress::new("Riley", "riley@studio.example"), 1_790_000_000, id).unwrap()
}

fn file(name: &str, mime: &str, data: &[u8]) -> mime::OutAttachment {
    mime::OutAttachment { name: name.into(), mime: mime.into(), data: data.to_vec(), ..Default::default() }
}

/// `raws` in an account's Inbox, exported to `dir/Mail.pst` and imported back into `dir/back`:
/// the file, and the cache it was read into.
fn round_trip(dir: &Path, raws: &[Vec<u8>]) -> (std::path::PathBuf, Store) {
    let store = Store::open(dir.join("cache")).unwrap();
    let mut messages = Vec::new();
    for (uid, raw) in (1..).zip(raws) {
        let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid };
        store.write_body(&key, raw).unwrap();
        messages.push(mime::summary_from_full(raw, key, Flags::default(), 10, 0).into());
    }
    let n = u32::try_from(messages.len()).unwrap();
    store.save_folder("a1", "INBOX", &FolderState { validity: 1, uid_next: n + 1, messages, ..Default::default() }).unwrap();
    let inbox =
        mailcraft_model::Folder { account: "a1".into(), path: "INBOX".into(), role: FolderRole::Inbox, selectable: true, ..Default::default() };
    store.save_folders("a1", &[inbox]).unwrap();
    let path = dir.join("Mail.pst");
    let done = export(&store, "a1", "Mail", &path, NONE, &mut |_| {}).unwrap();
    assert_eq!(done.messages, raws.len(), "{done:?}");
    let back = Store::open(dir.join("back")).unwrap();
    crate::pst::import(&path, &back, "b1", &mut |_| {}).unwrap();
    (path, back)
}

/// The source of the message with `subject` in the imported Inbox.
fn imported(back: &Store, subject: &str) -> Vec<u8> {
    let inbox = back.load_folder("b1", "INBOX").unwrap();
    let m = inbox.messages.iter().find(|m| m.subject == subject).unwrap_or_else(|| panic!("no {subject}"));
    back.read_body(&m.key).unwrap()
}

/// The message with `subject` in the file's Inbox, as the writer's reader reads it.
fn in_file(path: &Path, subject: &str) -> Rc<UnicodeMessage> {
    let store = UnicodeStore::read(Rc::new(outlook_pst::UnicodePstFile::open(path).unwrap())).unwrap();
    let any: Rc<dyn PstStore> = store.clone();
    let top = any.open_folder(&any.properties().ipm_sub_tree_entry_id().unwrap()).unwrap();
    let table = top.hierarchy_table().unwrap().clone();
    let inbox = table
        .rows_matrix()
        .filter_map(|row| any.open_folder(&any.properties().make_entry_id(NodeId::from(u32::from(row.id()))).ok()?).ok())
        .find(|f| f.properties().display_name().is_ok_and(|n| n == "Inbox"))
        .unwrap();
    for row in inbox.contents_table().unwrap().rows_matrix() {
        let entry = any.properties().make_entry_id(NodeId::from(u32::from(row.id()))).unwrap();
        let m = UnicodeMessage::read(store.clone(), &entry, None).unwrap();
        if m.properties().get(0x0037).and_then(crate::pst::text_of).as_deref() == Some(subject) {
            return m;
        }
    }
    panic!("no {subject} in the file")
}

fn attachments_of(m: &Rc<UnicodeMessage>) -> Vec<Rc<UnicodeAttachment>> {
    let Some(table) = m.attachment_table() else { return Vec::new() };
    table.rows_matrix().map(|row| UnicodeAttachment::read(m.clone(), NodeId::from(u32::from(row.id())), None).unwrap()).collect()
}

fn text(value: Option<&outlook_pst::ltp::prop_context::PropertyValue>) -> Option<String> {
    value.and_then(crate::pst::text_of)
}

/// A message attached three deep, a file at the bottom, goes in as attached messages (method 5,
/// no `.eml`), each with its own recipients and files, and comes back through the import as the
/// messages it was, down to the file.
#[test]
fn attached_messages_go_in_as_messages() {
    use outlook_pst::ltp::prop_context::PropertyValue as V;
    let dir = scratch("attached");
    let deep = build(&draft("Level 3", "The bottom.", vec![file("deep.txt", "text/plain", b"found it")]), "l3@studio.example");
    let two = build(&draft("Level 2", "Two.", vec![file("Level 3.eml", "message/rfc822", &deep)]), "l2@studio.example");
    let one = build(&draft("Level 1", "One.", vec![file("Level 2.eml", "message/rfc822", &two)]), "l1@studio.example");
    let long = build(&draft(&"Long ".repeat(1000), "Long.", Vec::new()), "long@studio.example");
    let top = build(
        &draft(
            "Forwarded",
            "See attached.",
            vec![file("Level 1.eml", "message/rfc822", &one), file("a.txt", "text/plain", b"beside"), file("long.eml", "message/rfc822", &long)],
        ),
        "top@studio.example",
    );
    let (path, back) = round_trip(&dir, &[top]);

    let mut m = in_file(&path, "Forwarded");
    let files = attachments_of(&m);
    assert_eq!(files.len(), 3);
    assert!(matches!(files[1].properties().get(0x3705), Some(V::Integer32(1))));
    // A subject too long for an attachment's name is cut short, not the export refused.
    assert!(matches!(files[2].properties().get(0x3705), Some(V::Integer32(5))));
    assert_eq!(text(files[2].properties().get(0x3001)).map(|n| n.chars().count()), Some(255));
    for level in ["Level 1", "Level 2", "Level 3"] {
        let attached = attachments_of(&m).into_iter().find(|a| matches!(a.properties().get(0x3705), Some(V::Integer32(5)))).unwrap();
        assert_eq!(text(attached.properties().get(0x3001)).as_deref(), Some(level));
        m = attached.embedded_message().unwrap();
        assert_eq!(text(m.properties().get(0x0037)).as_deref(), Some(level));
        assert_eq!(m.recipient_table().map(|t| t.rows_matrix().count()), Some(1));
    }
    let bottom = attachments_of(&m);
    assert_eq!(text(bottom[0].properties().get(0x3707)).as_deref(), Some("deep.txt"));
    assert!(matches!(bottom[0].data(), Some(AttachmentData::Binary(b)) if b.buffer() == b"found it"));

    // Back through the import: an attached message at each level, the file at the bottom.
    let mut raw = imported(&back, "Forwarded");
    for level in ["Level 1", "Level 2", "Level 3"] {
        let p = mime::parse(&raw).unwrap();
        let a = p.attachments.iter().find(|a| a.mime == "message/rfc822").unwrap_or_else(|| panic!("{level}: {:?}", p.attachments));
        raw = mime::attachment_data(&raw, a.index).unwrap();
        assert_eq!(mime::parse(&raw).unwrap().subject, level);
    }
    let p = mime::parse(&raw).unwrap();
    assert_eq!(p.attachments.len(), 1);
    assert_eq!(mime::attachment_data(&raw, p.attachments[0].index).as_deref(), Some(&b"found it"[..]));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A clear-signed message goes in as `IPM.Note.SMIME.MultipartSigned` and comes back with its
/// signature verifying over the same bytes; an encrypted one as `IPM.Note.SMIME`, its body empty,
/// and it still decrypts; an opaque signature's CMS comes back as it was.
#[test]
fn signed_and_encrypted_mail_goes_in_as_outlook_keeps_it() {
    let dir = scratch("smime");
    let id = riley();
    let cert = id.certificate().to_vec();
    let mut sign = |entity: &[u8]| mailcraft_smime::sign(entity, &id, NOW, &mut rand_core::OsRng).map_err(|e| format!("{e:?}"));
    let plain = build(&draft("Signed", "Approved, with a file.", vec![file("terms.txt", "text/plain", b"the terms")]), "s1@studio.example");
    let signed = smime::protect(&plain, Some(&mut sign), None, false, false, "=_sig").unwrap();
    let Protection::Signed { content, .. } = smime::protection(&signed) else { panic!("not signed") };
    let mut encrypt = |entity: &[u8]| {
        mailcraft_smime::encrypt(entity, std::slice::from_ref(&cert), mailcraft_smime::Cipher::Aes256Cbc, &mut rand_core::OsRng)
            .map_err(|e| format!("{e:?}"))
    };
    let secret =
        smime::protect(&build(&draft("Sealed", "Secret plans.", Vec::new()), "s2@studio.example"), None, Some(&mut encrypt), false, false, "=_e")
            .unwrap();
    // A made-up opaque signature: its bytes come back, whatever they hold.
    let mut cms = vec![0x30, 0x80, 0x06, 0x09, 0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x07, 0x02, 0xA0, 0x80];
    cms.extend([0x5A; 40]);
    let outer = b"From: Riley <riley@studio.example>\r\nTo: ana@studio.example\r\nSubject: Opaque\r\nDate: Mon, 2 Nov 2026 10:00:00 +0000\r\nMessage-ID: <s3@studio.example>\r\n";
    let opaque = smime::pkcs7_message(outer, &cms, "signed-data");
    let (path, back) = round_trip(&dir, &[signed, secret, opaque]);

    for (subject, class, mime_type) in [
        ("Signed", "IPM.Note.SMIME.MultipartSigned", "multipart/signed"),
        ("Sealed", "IPM.Note.SMIME", "application/pkcs7-mime"),
        ("Opaque", "IPM.Note.SMIME", "application/pkcs7-mime"),
    ] {
        let m = in_file(&path, subject);
        assert_eq!(text(m.properties().get(0x001A)).as_deref(), Some(class));
        let files = attachments_of(&m);
        assert_eq!(files.len(), 1, "{subject}: the signed message's file is inside it");
        assert_eq!(text(files[0].properties().get(0x3707)).as_deref(), Some("smime.p7m"));
        assert_eq!(text(files[0].properties().get(0x370E)).as_deref(), Some(mime_type));
    }
    assert_eq!(text(in_file(&path, "Signed").properties().get(0x1000)).as_deref().map(str::trim), Some("Approved, with a file."));
    // An encrypted message's body is nothing: what it says is inside.
    assert_eq!(text(in_file(&path, "Sealed").properties().get(0x1000)).unwrap_or_default(), "");

    let raw = imported(&back, "Signed");
    let Protection::Signed { content: back_content, signature } = smime::protection(&raw) else { panic!("not signed after the round trip") };
    assert_eq!(back_content, content);
    assert!(mailcraft_smime::verify(&signature, Some(&back_content)).is_ok(), "the signature verifies");
    let raw = imported(&back, "Sealed");
    let Protection::Encrypted(cms_back) = smime::protection(&raw) else { panic!("not encrypted after the round trip") };
    let (entity, _) = mailcraft_smime::decrypt(&cms_back, &id, &mut rand_core::OsRng).unwrap();
    assert!(String::from_utf8_lossy(&entity).contains("Secret plans."));
    assert_eq!(smime::protection(&imported(&back, "Opaque")), Protection::Opaque(cms));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A report: its header, a text, its status and the message it returns.
fn report(id: &str, kind: &str, status: &str, returned: (&str, &[u8])) -> Vec<u8> {
    let mut raw = format!(
        "From: Mail Delivery <postmaster@studio.example>\r\nTo: riley@studio.example\r\nSubject: Report {id}\r\n\
         Date: Mon, 2 Nov 2026 10:00:00 +0000\r\nMessage-ID: <{id}@studio.example>\r\nMIME-Version: 1.0\r\n\
         Content-Type: multipart/report; report-type={kind}; boundary=\"r1\"\r\n\r\n\
         --r1\r\nContent-Type: text/plain\r\n\r\nAbout your message.\r\n\
         --r1\r\nContent-Type: message/{kind}\r\n\r\n{status}\r\n\
         --r1\r\nContent-Type: {}\r\n\r\n",
        returned.0
    )
    .into_bytes();
    raw.extend_from_slice(returned.1);
    raw.extend_from_slice(b"\r\n--r1--\r\n");
    raw
}

/// A meeting request is meeting mail with its calendar; a delivery report and a read receipt keep
/// a report's class, and come back as reports, the returned message attached as a message.
#[test]
fn meeting_mail_and_reports_keep_their_classes() {
    let dir = scratch("reports");
    let ics = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nUID:review@studio.example\r\nDTSTAMP:20261101T100000Z\r\n\
               DTSTART:20261110T100000Z\r\nDTEND:20261110T110000Z\r\nSUMMARY:Review\r\nORGANIZER:mailto:riley@studio.example\r\n\
               ATTENDEE;PARTSTAT=NEEDS-ACTION:mailto:ana@studio.example\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
    let invite = build(&draft("Review", "Join us.", vec![file("invite.ics", "text/calendar; method=REQUEST", ics.as_bytes())]), "m1@studio.example");
    let sent = build(&draft("Proposal", "Here it is.", Vec::new()), "p1@studio.example");
    let dsn = "Reporting-MTA: dns; mx.studio.example\r\n\r\nFinal-Recipient: rfc822; nobody@studio.example\r\nAction: failed\r\nStatus: 5.1.1";
    let mdn =
        "Reporting-UA: mail.studio.example\r\nFinal-Recipient: rfc822; ana@studio.example\r\nDisposition: manual-action/MDN-sent-manually; displayed";
    let bounce = report("ndr", "delivery-status", dsn, ("message/rfc822", &sent));
    let head = sent.get(..sent.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 2).unwrap();
    let receipt = report("read", "disposition-notification", mdn, ("text/rfc822-headers", head));
    let (path, back) = round_trip(&dir, &[invite, bounce, receipt]);

    let class = |subject: &str| text(in_file(&path, subject).properties().get(0x001A));
    assert_eq!(class("Review").as_deref(), Some("IPM.Schedule.Meeting.Request"));
    assert_eq!(class("Report ndr").as_deref(), Some("REPORT.IPM.Note.NDR"));
    assert_eq!(class("Report read").as_deref(), Some("REPORT.IPM.Note.IPNRN"));
    // The bounce's returned message is an attached message in the file.
    let files = attachments_of(&in_file(&path, "Report ndr"));
    let attached = files.iter().find_map(|a| a.embedded_message()).unwrap();
    assert_eq!(text(attached.properties().get(0x0037)).as_deref(), Some("Proposal"));

    let raw = imported(&back, "Review");
    let p = mime::parse(&raw).unwrap();
    let cal = p.attachments.iter().find(|a| a.mime.starts_with("text/calendar")).unwrap();
    assert!(mime::attachment_text(&raw, cal.index).unwrap().contains("METHOD:REQUEST"));
    for (subject, kind, returned) in
        [("Report ndr", "delivery-status", "message/rfc822"), ("Report read", "disposition-notification", "text/rfc822-headers")]
    {
        let raw = imported(&back, subject);
        let text = String::from_utf8_lossy(&raw);
        assert!(text.contains(&format!("multipart/report; report-type={kind}")), "{text}");
        let p = mime::parse(&raw).unwrap();
        assert!(p.attachments.iter().any(|a| a.mime == returned), "{subject}: {:?}", p.attachments);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// A big file's bytes, the `n`th file's: not one repeated block.
fn big_file(n: u8, len: usize) -> Vec<u8> {
    (0..len).map(|at| ((at as u64).wrapping_add(u64::from(n)).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 56) as u8).collect()
}

/// A big file waits in a file of its own, not in the cache: a message whose source leaves the
/// cache between its batch's collecting and its writing (a sync's expunge, a move) is still
/// written whole, and the export leaves nothing beside its file.
#[test]
fn a_big_file_doesnt_need_the_cache_once_collected() {
    let dir = scratch("waits");
    let data = big_file(1, 5 << 20);
    let raw = build(&draft("Footage", "The take.", vec![file("take.bin", "application/octet-stream", &data)]), "take@studio.example");
    let store = Store::open(dir.join("cache")).unwrap();
    let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid: 1 };
    store.write_body(&key, &raw).unwrap();
    let spill = files::Spill::new(dir.join("Mail.pst.part-files"));
    let m = item::owned(&store.read_body(&key).unwrap(), 0, item::State::default(), &Default::default(), Some(&spill)).unwrap();
    assert!(matches!(m.files.as_slice(), [files::File::Stored { .. }]));
    assert!(spill.bytes() >= 5 << 20);
    store.remove_body(&key);
    let path = dir.join("Mail.pst");
    let mut created = false;
    write(&path, &["Inbox"], &[m], &mut created).unwrap();
    spill.clear();
    assert!(!dir.join("Mail.pst.part-files").exists());
    let got = attachments_of(&in_file(&path, "Footage"));
    assert!(matches!(got[0].data(), Some(AttachmentData::Binary(b)) if b.buffer() == data.as_slice()));

    // Through a whole export: the file byte for byte, and nothing left beside it.
    let again = scratch("waits-export");
    let (path, _) = round_trip(&again, &[raw]);
    let got = attachments_of(&in_file(&path, "Footage"));
    assert!(matches!(got[0].data(), Some(AttachmentData::Binary(b)) if b.buffer() == data.as_slice()));
    assert!(!again.join("Mail.pst.part-files").exists() && !again.join("Mail.pst.part").exists());
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&again);
}

/// Attached messages hold what they decode while those below them are read: past their room
/// ([`files::MAX_ATTACHED`]) the next one down stays a `.eml` file, rather than each level holding
/// a copy of all below it.
#[test]
fn attached_messages_hold_no_more_than_their_room() {
    let level =
        |subject: &str, inner: (&str, &str, &[u8])| build(&draft(subject, "Level.", vec![file(inner.0, inner.1, inner.2)]), "l@studio.example");
    let three = level("Level 3", ("big.bin", "application/octet-stream", &big_file(3, 1 << 20)));
    let two = level("Level 2", ("Level 3.eml", "message/rfc822", &three));
    let one = level("Level 1", ("Level 2.eml", "message/rfc822", &two));
    let top = level("Top", ("Level 1.eml", "message/rfc822", &one));
    // Room for Level 1 and Level 2, not Level 3 as well.
    let room = one.len() + two.len() + three.len() / 2;
    let m = item::owned_in(&top, 0, item::State::default(), &Default::default(), &[], None, room).unwrap();
    fn inner(m: &item::Owned) -> Option<&item::Owned> {
        match m.files.as_slice() {
            [files::File::Message { message, .. }] => Some(message.as_ref()),
            _ => None,
        }
    }
    let two_in = inner(inner(&m).unwrap()).unwrap();
    assert_eq!(two_in.subject, "Level 2");
    assert!(matches!(two_in.files.as_slice(), [files::File::Data { name, .. }] if name == "Level 3.eml"), "Level 3 stays a file");
    // With the room it has, all three are attached messages.
    let m = item::owned(&top, 0, item::State::default(), &Default::default(), None).unwrap();
    let three_in = inner(inner(inner(&m).unwrap()).unwrap()).unwrap();
    assert_eq!(three_in.subject, "Level 3");
}

/// A name as long as a message's names get keeps its extension, and a `Content-ID` too long for
/// an attachment's block is cut with the HTML's reference to it, so the picture still shows;
/// both go into the data file.
#[test]
fn long_names_keep_their_extension_and_long_ids_their_pictures() {
    let dir = scratch("long-names");
    let name = format!("{}.pdf", "report ".repeat(50));
    let cid = format!("{}@studio.example", "c".repeat(1200));
    // By hand: MailCraft's own mail never carries such a Content-ID.
    let raw = format!(
        "From: Riley <riley@studio.example>\r\nTo: Ana <ana@studio.example>\r\nSubject: Long names\r\nMessage-ID: <long-names@studio.example>\r\n\
         MIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"b\"\r\n\r\n\
         --b\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<p>See <img src=\"cid:{cid}\"></p>\r\n\
         --b\r\nContent-Type: image/png\r\nContent-ID: <{cid}>\r\nContent-Disposition: attachment; filename=\"p.png\"\r\n\
         Content-Transfer-Encoding: base64\r\n\r\niVBORw==\r\n\
         --b\r\nContent-Type: application/pdf\r\nContent-Disposition: attachment; filename=\"{name}\"\r\n\r\n%PDF-1.4\r\n--b--\r\n"
    )
    .into_bytes();
    let m = item::owned(&raw, 0, item::State::default(), &Default::default(), None).unwrap();
    let [files::File::Data { cid: Some(short_cid), .. }, files::File::Data { name: short, .. }] = m.files.as_slice() else {
        panic!("two files: {:?}", mime::parse(&raw).map(|p| p.attachments))
    };
    assert!(short.ends_with(".pdf") && short.chars().count() <= 255, "{short}");
    assert_eq!(short_cid.chars().count(), 1000);
    assert!(cid.starts_with(short_cid.as_str()));
    assert!(m.html.as_deref().is_some_and(|h| h.contains(&format!("cid:{short_cid}\""))), "{:?}", m.html);

    let (path, _) = round_trip(&dir, &[raw]);
    let got = attachments_of(&in_file(&path, "Long names"));
    assert_eq!(text(got[0].properties().get(0x3712)).as_deref(), Some(short_cid.as_str()));
    assert_eq!(text(got[1].properties().get(0x3707)).as_deref(), Some(short.as_str()));
    let _ = std::fs::remove_dir_all(&dir);
}
