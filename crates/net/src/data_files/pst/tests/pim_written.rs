//! Calendar items and contacts in a file the writer makes, as Outlook keeps them: a repeating
//! appointment whose pattern and times are named properties found through the file's own
//! name-to-ID map, its changed occurrences hidden attached messages (afException) that say which
//! occurrence they replace, and a contact whose picture is an attachment flagged as one. What
//! `pim_gaps` checks on items made up in memory, read here from the file by the import itself.

use mailcraft_model::dates::days_from_civil;
use mailcraft_model::pim::{PIM_DOC, Pim};
use mailcraft_store::Store;
use outlook_pst::ltp::prop_context::GuidValue;
use outlook_pst::{
    UnicodePstAttachment, UnicodePstAttachmentContent, UnicodePstEmbeddedMessage, UnicodePstFile as Writer, UnicodePstMessage, UnicodePstName,
    UnicodePstProperty, UnicodePstValue,
};

use super::super::import;

/// PSETID_Appointment and PSETID_Address (MS-OXPROPS 1.3.2).
const PSETID_APPOINTMENT: GuidValue = GuidValue::new(0x0006_2002, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
const PSETID_ADDRESS: GuidValue = GuidValue::new(0x0006_2004, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
const PSETID_SHARING: GuidValue = GuidValue::new(0x0006_2040, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);

/// Unix seconds as a FILETIME.
fn filetime(t: i64) -> i64 {
    (t + 11_644_473_600) * 10_000_000
}

fn message<'a>(subject: &'a str, class: &'a str, body: &'a str, properties: &'a [UnicodePstProperty<'a>]) -> UnicodePstMessage<'a> {
    UnicodePstMessage {
        subject,
        sender_name: "Riley Moreno",
        sender_email: "riley@studio.example",
        recipients: &[],
        body,
        html_body: None,
        message_id: "",
        delivery_time: filetime(1_790_000_000),
        message_class: class,
        properties,
    }
}

fn file<'a>(filename: &'a str, mime_type: &'a str, data: &'a [u8]) -> UnicodePstAttachment<'a> {
    UnicodePstAttachment { filename, mime_type, data, ..Default::default() }
}

fn named<'a>(set: GuidValue, lid: u32, value: UnicodePstValue<'a>) -> UnicodePstProperty<'a> {
    UnicodePstProperty::named(set, UnicodePstName::Id(lid), value)
}

/// A weekly appointment with a file, one changed occurrence that has an item of its own (with its
/// own text and file) and one that hasn't; and a contact with a picture and a file. They come in
/// as the series, its two changed occurrences (RECURRENCE-ID) with the item's text and file on its
/// own, and a card with its picture (PHOTO) and file, nothing left out.
#[test]
fn a_written_file_s_calendar_and_contacts_come_in() {
    let dir = std::env::temp_dir().join(format!("mailcraft-pst-pim-written-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("Calendar.pst");
    let at = |d: u32, h: i64| days_from_civil(2026, 10, d) * 86_400 + h * 3600;
    drop(Writer::create(&path, &message("Hello", "IPM.Note", "Hi.", &[])).unwrap());

    // The series: 07:00Z each Monday and Wednesday, its pattern the one `pst_pim` reads.
    let (pattern, start) = crate::pst_pim::tests::series(0x3009);
    let series = [
        named(PSETID_APPOINTMENT, 0x820D, UnicodePstValue::Time(filetime(start))),
        named(PSETID_APPOINTMENT, 0x820E, UnicodePstValue::Time(filetime(start + 3600))),
        named(PSETID_APPOINTMENT, 0x8208, UnicodePstValue::Unicode("Room 1")),
        named(PSETID_APPOINTMENT, 0x8216, UnicodePstValue::Binary(&pattern)),
    ];
    // The occurrence of the 19th, an hour later than the pattern says (summer time ended), with
    // its own text and file.
    let own = [
        named(PSETID_APPOINTMENT, 0x820D, UnicodePstValue::Time(filetime(at(19, 8)))),
        named(PSETID_APPOINTMENT, 0x820E, UnicodePstValue::Time(filetime(at(19, 9)))),
        named(PSETID_APPOINTMENT, 0x8228, UnicodePstValue::Time(filetime(at(19, 8)))),
    ];
    // PidTagAttachmentHidden and PidTagAttachmentFlags' afException, as Outlook marks one.
    let exception =
        [UnicodePstProperty::tagged(0x7FFE, UnicodePstValue::Boolean(true)), UnicodePstProperty::tagged(0x7FFD, UnicodePstValue::Integer32(2))];
    let changed = UnicodePstAttachment {
        filename: "Review",
        content: UnicodePstAttachmentContent::Message(Box::new(UnicodePstEmbeddedMessage {
            message: message("Review", "IPM.OLE.CLASS.{00061055-0000-0000-C000-000000000046}", "Bring the proofs.", &own),
            attachments: vec![file("proofs.pdf", "application/pdf", b"%PDF-1.4 proofs")],
        })),
        properties: &exception,
        ..Default::default()
    };
    drop(Writer::create_folder(&path, &["Calendar"], "IPF.Appointment").unwrap());
    let attachments = [file("agenda.pdf", "application/pdf", b"%PDF-1.4 agenda"), changed];
    drop(
        Writer::append_in_folder_with_attachments(&path, &["Calendar"], &message("Review", "IPM.Appointment", "Weekly.", &series), &attachments)
            .unwrap(),
    );

    // The contact: PidTagAttachmentContactPhoto and PidTagAttachmentHidden on its picture.
    let card = [
        UnicodePstProperty::tagged(0x3A06, UnicodePstValue::Unicode("Ana")),
        UnicodePstProperty::tagged(0x3A11, UnicodePstValue::Unicode("Ruiz")),
        named(PSETID_ADDRESS, 0x8083, UnicodePstValue::Unicode("ana@posteo.example")),
    ];
    let picture =
        [UnicodePstProperty::tagged(0x7FFF, UnicodePstValue::Boolean(true)), UnicodePstProperty::tagged(0x7FFE, UnicodePstValue::Boolean(true))];
    let photo = UnicodePstAttachment { properties: &picture, ..file("ContactPicture.jpg", "image/jpeg", &[0xFF, 0xD8, 0xFF, 0xE0]) };
    drop(Writer::create_folder(&path, &["Contacts"], "IPF.Contact").unwrap());
    drop(
        Writer::append_in_folder_with_attachments(
            &path,
            &["Contacts"],
            &message("Ana Ruiz", "IPM.Contact", "", &card),
            &[photo, file("cv.pdf", "application/pdf", b"cv")],
        )
        .unwrap(),
    );

    let store = Store::open(dir.join("cache")).unwrap();
    let done = import(&path, &store, "a1", &mut |_| {}).unwrap();
    assert_eq!((done.messages, done.events, done.contacts), (1, 1, 1), "{done:?}");
    assert_eq!((done.skipped, done.skips.len()), (0, 0), "{done:?}");
    let pim: Pim = store.load_account_doc("a1", PIM_DOC).unwrap();
    let files_of = |extra: &[String]| -> Vec<String> {
        extra.iter().filter(|l| crate::pst_pim::is_file_line(l)).map(|l| l.split(':').next().unwrap_or_default().to_string()).collect()
    };
    let events: Vec<_> = pim.events.iter().map(|e| &e.event).collect();
    assert_eq!(events.len(), 3);
    let series = events[0];
    assert_eq!((series.start, series.location.as_str()), (start, "Room 1"));
    assert_eq!(series.rrule.as_deref(), Some("FREQ=WEEKLY;INTERVAL=1;BYDAY=MO,WE;COUNT=10"));
    assert_eq!(series.exdates, [at(7, 7)]);
    assert_eq!(files_of(&series.extra), ["ATTACH;FMTTYPE=application/pdf;ENCODING=BASE64;VALUE=BINARY;X-FILENAME=\"agenda.pdf\""]);
    let (moved, quiet) = (events[1], events[2]);
    assert_eq!((moved.uid.as_str(), moved.recurrence_id, moved.start), (series.uid.as_str(), Some(at(12, 7)), at(13, 8)));
    assert!(files_of(&moved.extra).is_empty());
    assert_eq!((quiet.uid.as_str(), quiet.recurrence_id, quiet.start, quiet.end), (series.uid.as_str(), Some(at(19, 7)), at(19, 8), at(19, 9)));
    assert_eq!(quiet.description, "Bring the proofs.");
    assert_eq!(files_of(&quiet.extra), ["ATTACH;FMTTYPE=application/pdf;ENCODING=BASE64;VALUE=BINARY;X-FILENAME=\"proofs.pdf\""]);
    let card = &pim.cards[0].card;
    assert_eq!(
        (card.display(), card.emails.iter().map(|e| e.value.as_str()).collect::<Vec<_>>()),
        ("Ana Ruiz".to_string(), vec!["ana@posteo.example"])
    );
    assert_eq!(card.extra, ["PHOTO:data:image/jpeg;base64,/9j/4A==", "X-MAILCRAFT-ATTACH;X-FILENAME=\"cv.pdf\":data:application/pdf;base64,Y3Y="]);
    drop(store);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Outlook's RSS Feeds folder and a feed's folder under it come in as mail folders of their
/// posts; a recall, a sharing invitation and a contact in a mail folder come in too, the contact
/// to the address book; a note comes in with its file. Read from a file the writer makes, by the
/// import itself.
#[test]
fn a_written_file_s_feeds_and_other_kinds_in_mail_come_in() {
    let dir = std::env::temp_dir().join(format!("mailcraft-pst-feeds-written-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("Feeds.pst");
    drop(Writer::create(&path, &message("Hello", "IPM.Note", "Hi.", &[])).unwrap());
    drop(Writer::create_folder(&path, &["RSS Feeds"], "IPF.Note.OutlookHomepage").unwrap());
    drop(Writer::create_folder(&path, &["RSS Feeds", "Studio news"], "IPF.Note").unwrap());
    for title in ["New lenses in stock", "Workshop on Friday"] {
        drop(
            Writer::append_in_folder_with_attachments(
                &path,
                &["RSS Feeds", "Studio news"],
                &message(title, "IPM.Post.Rss", "From the feed.", &[]),
                &[],
            )
            .unwrap(),
        );
    }
    drop(Writer::create_folder(&path, &["Kept"], "IPF.Note").unwrap());
    let card = [
        UnicodePstProperty::tagged(0x3A06, UnicodePstValue::Unicode("Ana")),
        named(PSETID_ADDRESS, 0x8083, UnicodePstValue::Unicode("ana@posteo.example")),
    ];
    // What the invitation shares: a calendar, by whom, at an internet address.
    let shared = [
        named(PSETID_SHARING, 0x8A05, UnicodePstValue::Unicode("Studio bookings")),
        named(PSETID_SHARING, 0x8A1D, UnicodePstValue::Unicode("IPF.Appointment")),
        named(PSETID_SHARING, 0x8A07, UnicodePstValue::Unicode("Ana")),
        named(PSETID_SHARING, 0x8A08, UnicodePstValue::Unicode("ana@posteo.example")),
        named(PSETID_SHARING, 0x8A04, UnicodePstValue::Unicode("webcal://calendar.example/studio.ics")),
    ];
    for (subject, class, props) in
        [("Recall: Plans", "IPM.Outlook.Recall", &[][..]), ("Sharing invitation", "IPM.Sharing", &shared[..]), ("Ana", "IPM.Contact", &card[..])]
    {
        drop(Writer::append_in_folder_with_attachments(&path, &["Kept"], &message(subject, class, "Text.", props), &[]).unwrap());
    }
    // A note with a file.
    drop(Writer::create_folder(&path, &["Notes"], "IPF.StickyNote").unwrap());
    let swatch = file("swatch.pdf", "application/pdf", b"%PDF-1.7 swatch");
    drop(Writer::append_in_folder_with_attachments(&path, &["Notes"], &message("Paper", "IPM.StickyNote", "Paper", &[]), &[swatch]).unwrap());
    // The fuzz target's seed (`fuzz/seeds/pst/import-rest`) is this file: kept where asked.
    if let Some(to) = std::env::var_os("MC_PST_SEED") {
        std::fs::copy(&path, to).unwrap();
    }

    let store = Store::open(dir.join("cache")).unwrap();
    let done = import(&path, &store, "a1", &mut |_| {}).unwrap();
    assert_eq!((done.messages, done.contacts, done.notes, done.skipped), (5, 1, 1, 0), "{done:?}");
    let feeds = done.folders.iter().find(|f| f.path.ends_with("Studio news")).expect("the feed's folder");
    assert!(feeds.path.starts_with("RSS Feeds"), "{}", feeds.path);
    let posts = store.load_folder("a1", &feeds.path).unwrap().messages;
    assert_eq!(posts.len(), 2);
    let kept = store.load_folder("a1", "Kept").unwrap().messages;
    let mut subjects: Vec<&str> = kept.iter().map(|m| m.subject.as_str()).collect();
    subjects.sort_unstable();
    assert_eq!(subjects, ["Recall: Plans", "Sharing invitation"]);
    let invitation = kept.iter().find(|m| m.subject == "Sharing invitation").unwrap();
    let text = mailcraft_mime::parse(&store.load_body(&invitation.key).unwrap().unwrap()).unwrap().text;
    for line in ["Shared calendar: Studio bookings", "Shared by Ana <ana@posteo.example>", "webcal://calendar.example/studio.ics"] {
        assert!(text.contains(line), "{line}: {text}");
    }
    let pim: Pim = store.load_account_doc("a1", PIM_DOC).unwrap();
    assert!(pim.cards.iter().any(|c| c.card.emails.iter().any(|e| e.value == "ana@posteo.example")));
    let note = &pim.notes[0].note;
    let kept = note.kept_files();
    let files: Vec<(&str, &str)> = kept.iter().map(|f| (f.file_name(), f.mime)).collect();
    assert_eq!((note.text.as_str(), files), ("Paper", vec![("swatch.pdf", "application/pdf")]));
    drop(store);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A message Outlook flagged for a day comes in with it (PidLidTaskDueDate, else
/// PidLidTaskStartDate, near the day's midnight): left for the engine in the account's
/// follow-up note, by its Message-ID. Not flagged, or a time that's no day (Outlook's 4501-01-01,
/// one out of range, a value that isn't a time), it comes in with none. Marked complete (status
/// 1), it comes in so, with the day it was done (PidLidTaskDateCompleted, else the UTC day of
/// PidTagFlagCompleteTime) in the done note.
#[test]
fn a_written_file_s_flagged_mail_comes_in_with_its_due_day() {
    use mailcraft_model::organize::{FOLLOW_UP_DOC, FOLLOW_UP_DONE_DOC};
    use std::collections::BTreeMap;
    const PSETID_TASK: GuidValue = GuidValue::new(0x0006_2003, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
    let dir = std::env::temp_dir().join(format!("mailcraft-pst-due-written-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("Flags.pst");
    drop(Writer::create(&path, &message("Hello", "IPM.Note", "Hi.", &[])).unwrap());
    let day = days_from_civil(2026, 11, 20);
    let midnight = UnicodePstValue::Time(filetime(day * 86_400));
    let flag = |status| UnicodePstProperty::tagged(0x1090, UnicodePstValue::Integer32(status));
    let due = |v| named(PSETID_TASK, 0x8105, v);
    let start = |v| named(PSETID_TASK, 0x8104, v);
    let done_on = |v| named(PSETID_TASK, 0x810F, v);
    let done_at = |day: i64| UnicodePstProperty::tagged(0x1091, UnicodePstValue::Time(filetime(day * 86_400 + 15 * 3600)));
    let cases: [(&str, Vec<UnicodePstProperty<'_>>); 8] = [
        ("due@studio.example", vec![flag(2), due(midnight), start(UnicodePstValue::Time(filetime((day - 3) * 86_400)))]),
        // Its start alone, a few hours off its midnight (written in another time zone).
        ("start@studio.example", vec![flag(2), start(UnicodePstValue::Time(filetime((day + 1) * 86_400 - 5 * 3600)))]),
        // Marked complete: done two days before (PidLidTaskDateCompleted, over the moment).
        ("done@studio.example", vec![flag(1), due(midnight), done_on(UnicodePstValue::Time(filetime((day - 2) * 86_400))), done_at(day - 5)]),
        // Its moment alone, in the afternoon: that day.
        ("finished@studio.example", vec![flag(1), done_at(day - 1)]),
        ("none@studio.example", vec![flag(2), due(UnicodePstValue::Time(filetime(days_from_civil(4501, 1, 1) * 86_400)))]),
        ("far@studio.example", vec![flag(2), due(UnicodePstValue::Time(i64::MAX)), start(UnicodePstValue::Time(i64::MIN))]),
        ("number@studio.example", vec![flag(2), due(UnicodePstValue::Integer32(7))]),
        ("unflagged@studio.example", vec![due(midnight)]),
    ];
    drop(Writer::create_folder(&path, &["Kept"], "IPF.Note").unwrap());
    for (id, props) in &cases {
        let m = UnicodePstMessage { message_id: id, ..message("Flagged", "IPM.Note", "Text.", props) };
        drop(Writer::append_in_folder_with_attachments(&path, &["Kept"], &m, &[]).unwrap());
    }

    let store = Store::open(dir.join("cache")).unwrap();
    let done = import(&path, &store, "a1", &mut |_| {}).unwrap();
    assert_eq!((done.messages, done.skipped), (9, 0), "{done:?}");
    let days: BTreeMap<String, i64> = store.load_account_doc("a1", FOLLOW_UP_DOC).unwrap();
    let ids = |pairs: &[(&str, i64)]| pairs.iter().map(|(id, d)| (id.to_string(), *d)).collect::<BTreeMap<_, _>>();
    assert_eq!(days, ids(&[("due@studio.example", day), ("start@studio.example", day + 1), ("done@studio.example", day)]));
    let done_days: BTreeMap<String, i64> = store.load_account_doc("a1", FOLLOW_UP_DONE_DOC).unwrap();
    assert_eq!(done_days, ids(&[("done@studio.example", day - 2), ("finished@studio.example", day - 1)]));
    let kept = store.load_folder("a1", "Kept").unwrap().messages;
    let flagged: Vec<&str> = kept.iter().filter(|m| m.flags.flagged).map(|m| m.message_id.as_str()).collect();
    assert_eq!(flagged.len(), 5, "{flagged:?}");
    // Status 1: complete, not flagged, the mark not a category.
    let mut complete: Vec<&str> = kept.iter().filter(|m| m.flags.follow_up_complete()).map(|m| m.message_id.as_str()).collect();
    complete.sort_unstable();
    assert_eq!(complete, ["done@studio.example", "finished@studio.example"]);
    drop(store);
    let _ = std::fs::remove_dir_all(&dir);
}
