//! Kinds of item a data file holds beyond plain mail: meeting requests and their answers, posts
//! and documents, contact groups and journal entries.

use mailcraft_model::EmailAddress;
use mailcraft_model::job::SkipReason;
use mailcraft_model::pim::{PIM_DOC, Pim};
use mailcraft_store::Store;

use super::super::ImportOptions;
use super::super::item::{File, Item};
use super::super::out::Out;
use super::super::rebuild::message;
use crate::pst_pim::Extra;

/// The names of a message's files, read back from its source.
fn files_of(raw: &[u8]) -> Vec<String> {
    mailcraft_mime::parse(raw).map(|p| p.attachments.into_iter().map(|a| a.name).collect()).unwrap_or_default()
}

/// A meeting item as read: the meeting's time and its organizer's id.
fn meeting(class: &str) -> Item {
    Item {
        class: class.into(),
        subject: "Review".into(),
        from: EmailAddress::new("Riley", "riley@studio.example"),
        to: vec![EmailAddress::new("Ana", "ana@posteo.example")],
        date: 1_700_000_000,
        body: "Room 4".into(),
        extra: Some(Extra {
            start: Some(1_700_086_400),
            end: Some(1_700_090_000),
            uid: "040000008200E000".into(),
            sequence: 2,
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// The calendar a message carries.
fn calendar_of(raw: &[u8]) -> String {
    let p = mailcraft_mime::parse(raw).unwrap();
    let ics = p.attachments.iter().find(|a| a.mime.starts_with("text/calendar")).expect("a calendar");
    String::from_utf8(mailcraft_mime::attachment_data(raw, ics.index).unwrap()).unwrap()
}

/// A meeting request, cancellation and answer carry their calendar again, with the method that
/// lets them be answered or applied; another meeting notice, or one with no time, carries none.
#[test]
fn meeting_items_carry_their_calendar() {
    let (raw, _) = message(meeting("IPM.Schedule.Meeting.Request"), "id@x.invalid").unwrap();
    let ics = calendar_of(&raw);
    for line in ["METHOD:REQUEST", "UID:040000008200E000", "SEQUENCE:2", "ORGANIZER", "riley@studio.example", "ana@posteo.example", "SUMMARY:Review"]
    {
        assert!(ics.contains(line), "{line}: {ics}");
    }
    let (raw, _) = message(meeting("IPM.Schedule.Meeting.Canceled"), "id@x.invalid").unwrap();
    let ics = calendar_of(&raw);
    assert!(ics.contains("METHOD:CANCEL") && ics.contains("STATUS:CANCELLED"), "{ics}");
    let (raw, _) = message(meeting("IPM.Schedule.Meeting.Resp.Tent"), "id@x.invalid").unwrap();
    let ics = mailcraft_model::ical::parse(&calendar_of(&raw), 0);
    let e = &ics.events[0];
    assert_eq!(e.organizer.as_ref().map(|o| o.email.as_str()), Some("ana@posteo.example"));
    assert_eq!(e.attendees.iter().map(|a| (a.email.as_str(), a.partstat.as_str())).collect::<Vec<_>>(), [("riley@studio.example", "TENTATIVE")]);
    assert!(calendar_of(&message(meeting("IPM.Schedule.Meeting.Resp.Neg"), "id@x.invalid").unwrap().0).contains("PARTSTAT=DECLINED"));
    let (raw, _) = message(meeting("IPM.Schedule.Meeting.Notification.Forward"), "id@x.invalid").unwrap();
    assert!(files_of(&raw).is_empty());
    let mut timeless = meeting("IPM.Schedule.Meeting.Request");
    timeless.extra = Some(Extra::default());
    assert!(files_of(&message(timeless, "id@x.invalid").unwrap().0).is_empty());
    // One that carries its calendar already keeps that one alone.
    let mut carried = meeting("IPM.Schedule.Meeting.Request");
    carried.files.push(File::Data {
        name: "meeting.ics".into(),
        mime: "text/calendar".into(),
        cid: String::new(),
        data: b"BEGIN:VCALENDAR".to_vec(),
    });
    assert_eq!(files_of(&message(carried, "id@x.invalid").unwrap().0), ["meeting.ics"]);
}

/// A scratch store for `Out`, removed when dropped.
struct Scratch(std::path::PathBuf, Store);

impl Scratch {
    fn new(name: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!("mailcraft-pst-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store::open(&dir).unwrap();
        Scratch(dir, store)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A post and a document in a mail folder come in as mail; a recall, a sharing invitation and a
/// contact there are counted as left out, not dropped unsaid.
#[test]
fn posts_and_documents_are_mail_and_other_kinds_are_counted() {
    let s = Scratch::new("kinds");
    let mut progress = |_| {};
    let mut out = Out::new(&s.1, "d1", &ImportOptions::default(), &mut progress);
    let path = out.add_folder(None, "Inbox");
    for class in ["IPM.Post", "IPM.Document.Word.Document.12", "IPM.Note", "IPM.Outlook.Recall", "IPM.Sharing", "IPM.Contact"] {
        out.add_message(&path, Item { class: class.into(), subject: class.into(), date: 1_700_000_000, ..Default::default() });
    }
    out.save_folder(&path);
    let done = out.finish();
    assert_eq!((done.messages, done.skipped), (3, 3));
    assert_eq!(done.skips.into_iter().collect::<Vec<_>>(), [(SkipReason::ItemNotUsable, 3)]);
}

/// A contact group comes in as a group card with its members; a journal entry as a note with
/// its subject, its time and its text, and an empty one is counted.
#[test]
fn contact_groups_and_journal_entries_come_in() {
    let s = Scratch::new("groups");
    let mut progress = |_| {};
    let mut out = Out::new(&s.1, "d1", &ImportOptions::default(), &mut progress);
    let members = vec![("Ana".to_string(), "ana@posteo.example".to_string()), (String::new(), "ben@studio.example".to_string())];
    out.add_pim(Item {
        class: "IPM.DistList".into(),
        subject: "Crew".into(),
        extra: Some(Extra { members, ..Default::default() }),
        ..Default::default()
    });
    let entry = |subject: &str, body: &str, start: Option<i64>| Item {
        class: "IPM.Activity".into(),
        subject: subject.into(),
        body: body.into(),
        date: 1_700_000_000,
        extra: Some(Extra { log_start: start, log_end: start.map(|s| s + 45 * 60), ..Default::default() }),
        ..Default::default()
    };
    out.add_pim(entry("Call with the printer", "Proofs on Monday.\r\nPaper: matte.", Some(1_700_000_000)));
    out.add_pim(entry(" ", "", None));
    let done = out.finish();
    assert_eq!((done.contacts, done.notes, done.skipped), (1, 1, 1));
    let pim: Pim = s.1.load_account_doc("d1", PIM_DOC).unwrap();
    let card = &pim.cards[0].card;
    assert_eq!(card.name, "Crew");
    assert_eq!(card.extra, ["KIND:group", "MEMBER:mailto:ana@posteo.example", "MEMBER:mailto:ben@studio.example"]);
    assert_eq!(card.note, "Ana <ana@posteo.example>\nben@studio.example");
    let text = mailcraft_model::vcard::write(card);
    assert!(text.contains("KIND:group\r\n") && text.contains("MEMBER:mailto:ben@studio.example\r\n"), "{text}");
    assert_eq!(pim.notes[0].note.text, "Call with the printer\n2023-11-14 22:13–22:58 UTC\nProofs on Monday.\nPaper: matte.");
}
