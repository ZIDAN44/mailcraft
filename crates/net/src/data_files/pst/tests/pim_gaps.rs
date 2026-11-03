//! Kinds of item a data file holds beyond plain mail: meeting requests and their answers, posts
//! and documents, contact groups and journal entries; and what contacts and calendar items carry:
//! changed occurrences of a repeating appointment, pictures and files.

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

/// A post (an RSS feed's too), a document, a recall and its report, a sharing invitation and a
/// task request in a mail folder come in as mail, with their subject, text and sender; a contact
/// there goes to the address book as one in a contacts folder does; another kind is counted as
/// left out, not dropped unsaid.
#[test]
fn posts_and_documents_are_mail_and_other_kinds_are_counted() {
    let s = Scratch::new("kinds");
    let mut progress = |_| {};
    let mut out = Out::new(&s.1, "d1", &ImportOptions::default(), &mut progress);
    let path = out.add_folder(None, "Inbox");
    let mail = [
        "IPM.Post",
        "IPM.Post.Rss",
        "IPM.Document.Word.Document.12",
        "IPM.Note",
        "IPM.Outlook.Recall",
        "IPM.Recall.Report.Success",
        "IPM.Sharing",
        "IPM.TaskRequest",
    ];
    for class in mail {
        let from = EmailAddress::new("Riley", "riley@studio.example");
        out.add_message(
            &path,
            Item { class: class.into(), subject: class.into(), body: format!("Text of {class}"), from, date: 1_700_000_000, ..Default::default() },
        );
    }
    let contact = Extra { display: "Ana Lima".into(), emails: vec!["ana@posteo.example".into()], ..Default::default() };
    out.add_message(&path, Item { class: "IPM.Contact".into(), extra: Some(contact), ..Default::default() });
    out.add_message(&path, Item { class: "IPM.Configuration.Junk".into(), subject: "x".into(), ..Default::default() });
    out.save_folder(&path);
    let done = out.finish();
    assert_eq!((done.messages, done.contacts, done.skipped), (mail.len(), 1, 1));
    assert_eq!(done.skips.into_iter().collect::<Vec<_>>(), [(SkipReason::ItemNotUsable, 1)]);
    let list = s.1.load_folder("d1", &path).unwrap().messages;
    for class in mail {
        let m = list.iter().find(|m| m.subject == class).expect(class);
        assert_eq!(m.from.email, "riley@studio.example");
        let raw = s.1.load_body(&m.key).unwrap().unwrap();
        assert!(mailcraft_mime::parse(&raw).unwrap().text.contains(&format!("Text of {class}")), "{class}");
    }
    let pim: Pim = s.1.load_account_doc("d1", PIM_DOC).unwrap();
    assert!(pim.cards.iter().any(|c| c.card.emails.iter().any(|e| e.value == "ana@posteo.example")));
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

/// A note's and a journal entry's files come in on the note, each as an event keeps one (its
/// line, read back by name, type and bytes), an attached message as its `.eml`; one past the
/// cap on a file is counted as too big, as a card's or an event's is.
#[test]
fn notes_and_journal_entries_keep_their_files() {
    let s = Scratch::new("note-files");
    let mut progress = |_| {};
    let mut out = Out::new(&s.1, "d1", &ImportOptions::default(), &mut progress);
    let file = |name: &str, data: Vec<u8>| File::Data { name: name.into(), mime: "application/pdf".into(), cid: String::new(), data };
    let mail = Item { class: "IPM.Note".into(), subject: "Proofs".into(), body: "Attached.".into(), date: 1_700_000_000, ..Default::default() };
    out.add_pim(Item {
        class: "IPM.StickyNote".into(),
        body: "Paper\r\n170 gsm".into(),
        date: 1_700_000_000,
        files: vec![file("swatch.pdf", b"%PDF-1.7 swatch".to_vec()), File::Message(Box::new(mail)), file("huge.pdf", vec![0; 2 << 20])],
        extra: Some(Extra::default()),
        ..Default::default()
    });
    out.add_pim(Item {
        class: "IPM.Activity".into(),
        subject: "Call".into(),
        date: 1_700_000_000,
        files: vec![file("minutes.pdf", b"%PDF-1.7 minutes".to_vec())],
        extra: Some(Extra::default()),
        ..Default::default()
    });
    let done = out.finish();
    assert_eq!((done.notes, done.skipped), (2, 0));
    assert_eq!(done.skips.into_iter().collect::<Vec<_>>(), [(SkipReason::AttachmentTooBig, 1)]);
    let pim: Pim = s.1.load_account_doc("d1", PIM_DOC).unwrap();
    let files = |i: usize| -> Vec<(String, String, Vec<u8>)> {
        pim.notes[i]
            .note
            .kept_files()
            .iter()
            .map(|f| (f.file_name().to_string(), f.mime.to_string(), mailcraft_mime::decode_base64(f.base64).unwrap()))
            .collect()
    };
    let note = files(0);
    assert_eq!(
        note.iter().map(|(n, m, _)| (n.as_str(), m.as_str())).collect::<Vec<_>>(),
        [("swatch.pdf", "application/pdf"), ("Proofs.eml", "message/rfc822")]
    );
    assert_eq!(note[0].2, b"%PDF-1.7 swatch");
    assert_eq!(mailcraft_mime::parse(&note[1].2).unwrap().subject, "Proofs");
    assert_eq!(files(1).iter().map(|(n, _, d)| (n.as_str(), d.len())).collect::<Vec<_>>(), [("minutes.pdf", 16)]);
}

/// Every task comes in as a task, due or not, done or not, with its files; none is left out, and
/// none lands in the calendar (the calendar draws a due one from the task). A task request is a
/// task too; flagged mail stays mail.
#[test]
fn every_task_comes_in() {
    let s = Scratch::new("tasks");
    let mut progress = |_| {};
    let mut out = Out::new(&s.1, "d1", &ImportOptions::default(), &mut progress);
    let day = mailcraft_model::dates::days_from_civil(2026, 11, 5) * 86_400;
    let task = |class: &str, subject: &str, x: Extra, files: Vec<File>| Item {
        class: class.into(),
        subject: subject.into(),
        body: "Notes".into(),
        date: 1_700_000_000,
        importance: 1,
        files,
        extra: Some(x),
        ..Default::default()
    };
    out.add_pim(task(
        "IPM.Task",
        "No day at all",
        Extra { task_status: Some(3), ..Default::default() },
        vec![data("brief.pdf", "application/pdf", 5)],
    ));
    out.add_pim(task("IPM.Task", "Due", Extra { due: Some(day - 7200), percent: Some(40), ..Default::default() }, Vec::new()));
    out.add_pim(task("IPM.Task", "Done", Extra { done: true, completed: Some(day), ..Default::default() }, Vec::new()));
    out.add_pim(task("IPM.TaskRequest", "Asked of me", Extra::default(), Vec::new()));
    let path = out.add_folder(None, "Inbox");
    out.add_message(&path, Item { class: "IPM.Note".into(), subject: "Flagged".into(), date: 1_700_000_000, ..Default::default() });
    out.save_folder(&path);
    let done = out.finish();
    assert_eq!((done.tasks, done.events, done.messages, done.skipped), (4, 0, 1, 0));
    let pim: Pim = s.1.load_account_doc("d1", PIM_DOC).unwrap();
    let tasks: Vec<&mailcraft_model::pim::Task> = pim.tasks.iter().map(|t| &t.task).collect();
    assert_eq!(tasks.iter().map(|t| t.title.as_str()).collect::<Vec<_>>(), ["No day at all", "Due", "Done", "Asked of me"]);
    assert_eq!((tasks[0].due, tasks[0].status, tasks[0].modified), (None, mailcraft_model::pim::TaskStatus::Waiting, 1_700_000_000));
    assert!(tasks[0].extra.iter().any(|l| l.starts_with("ATTACH;") && l.contains("brief.pdf")), "{:?}", tasks[0].extra);
    assert_eq!((tasks[1].due, tasks[1].percent, tasks[1].status), (Some(day), 40, mailcraft_model::pim::TaskStatus::InProgress));
    assert_eq!((tasks[2].status, tasks[2].completed), (mailcraft_model::pim::TaskStatus::Completed, Some(day)));
    assert!(tasks.iter().all(|t| t.priority == mailcraft_model::pim::TaskPriority::High && t.notes == "Notes"));
    assert!(pim.events.is_empty());
}

/// A changed occurrence's item, attached to its appointment (`replaces`: the occurrence), with
/// its own text and files.
fn exception(start: i64, replaces: i64, body: &str, files: Vec<File>) -> File {
    File::Message(Box::new(Item {
        class: "IPM.OLE.CLASS.{00061055-0000-0000-C000-000000000046}".into(),
        subject: "Review".into(),
        body: body.into(),
        files,
        extra: Some(Extra { start: Some(start), end: Some(start + 3600), replaces: Some(replaces), ..Default::default() }),
        ..Default::default()
    }))
}

fn data(name: &str, mime: &str, n: usize) -> File {
    File::Data { name: name.into(), mime: mime.into(), cid: String::new(), data: vec![b'x'; n] }
}

/// A repeating appointment comes in with its changed occurrences, each an event replacing its
/// occurrence with its own text and files, and the series' files on the series; it counts as one
/// calendar item, and an exception item that is no occurrence (it says no time) is counted. A
/// contact comes in with its picture and its files; a file past what the contacts and calendar
/// keep is counted as too big.
#[test]
fn changed_occurrences_and_files_come_in() {
    let s = Scratch::new("pim-files");
    let mut progress = |_| {};
    let mut out = Out::new(&s.1, "d1", &ImportOptions::default(), &mut progress);
    let at = |d: u32, h: i64| mailcraft_model::dates::days_from_civil(2026, 10, d) * 86_400 + h * 3600;
    let (pattern, start) = crate::pst_pim::tests::series(0x3009);
    let own = exception(at(19, 8), at(19, 8), "Bring the proofs.", vec![data("proofs.pdf", "application/pdf", 4)]);
    let timeless = File::Message(Box::new(Item {
        class: "IPM.OLE.CLASS.{00061055-0000-0000-C000-000000000046}".into(),
        files: vec![data("lost.pdf", "application/pdf", 1)],
        extra: Some(Extra { replaces: Some(at(26, 7)), ..Default::default() }),
        ..Default::default()
    }));
    out.add_pim(Item {
        class: "IPM.Appointment".into(),
        subject: "Review".into(),
        body: "Weekly.".into(),
        files: vec![data("agenda.pdf", "application/pdf", 3), own, timeless],
        extra: Some(Extra { start: Some(start), end: Some(start + 3600), recurrence: pattern, ..Default::default() }),
        ..Default::default()
    });
    out.add_pim(Item {
        class: "IPM.Contact".into(),
        photo: Some(("image/jpeg".into(), vec![0xFF, 0xD8, 0xFF])),
        files: vec![data("cv.pdf", "", 2), data("huge.bin", "application/octet-stream", 5 << 20)],
        extra: Some(Extra { given: "Ana".into(), ..Default::default() }),
        ..Default::default()
    });
    let done = out.finish();
    assert_eq!((done.events, done.contacts), (1, 1));
    assert_eq!(done.skipped, 0);
    assert_eq!(done.skips, [(SkipReason::AttachmentTooBig, 1), (SkipReason::ItemNotUsable, 1)].into_iter().collect());
    let pim: Pim = s.1.load_account_doc("d1", PIM_DOC).unwrap();
    let files_of = |extra: &[String]| -> Vec<String> {
        extra.iter().filter(|l| crate::pst_pim::is_file_line(l)).map(|l| l.split(':').next().unwrap_or_default().to_string()).collect()
    };
    let events: Vec<_> = pim.events.iter().map(|e| &e.event).collect();
    assert_eq!(events.len(), 3);
    let series = events[0];
    assert!(series.rrule.is_some() && series.recurrence_id.is_none());
    assert_eq!(files_of(&series.extra), ["ATTACH;FMTTYPE=application/pdf;ENCODING=BASE64;VALUE=BINARY;X-FILENAME=\"agenda.pdf\""]);
    let (moved, quiet) = (events[1], events[2]);
    assert_eq!((moved.uid.as_str(), moved.recurrence_id, moved.summary.as_str()), (series.uid.as_str(), Some(at(12, 7)), "Review (moved) \u{e9}"));
    assert!(files_of(&moved.extra).is_empty());
    assert_eq!((quiet.recurrence_id, quiet.start, quiet.description.as_str()), (Some(at(19, 7)), at(19, 8), "Bring the proofs."));
    assert_eq!(files_of(&quiet.extra), ["ATTACH;FMTTYPE=application/pdf;ENCODING=BASE64;VALUE=BINARY;X-FILENAME=\"proofs.pdf\""]);
    // Written as a calendar file, the series and its changed occurrences read back together.
    let ics = mailcraft_model::ical::write(&pim.events.iter().map(|e| e.event.clone()).collect::<Vec<_>>(), None, 0);
    let back = mailcraft_model::ical::parse(&ics, 0);
    assert_eq!(back.events.iter().map(|e| e.recurrence_id).collect::<Vec<_>>(), [None, Some(at(12, 7)), Some(at(19, 7))]);
    let card = &pim.cards[0].card;
    assert_eq!(card.extra, ["PHOTO:data:image/jpeg;base64,/9j/", "X-MAILCRAFT-ATTACH;X-FILENAME=\"cv.pdf\":data:application/pdf;base64,eHg="]);
}

/// A contact's picture is told by its attachment's flag, in a contact only.
#[test]
fn a_contact_s_picture() {
    use super::super::item::{Got, photo};
    use crate::pst::Val;
    let flagged = |id: u16| match id {
        0x7FFF => Some(Val::Bool(true)),
        0x3707 => Some(Val::Text("ContactPicture.png".into())),
        _ => None,
    };
    assert!(matches!(photo(&flagged, "IPM.Contact", b"png"), Some(Got::Photo(ref mime, ref data)) if mime == "image/png" && data == b"png"));
    assert!(photo(&flagged, "IPM.Note", b"png").is_none(), "a message's file");
    assert!(photo(&|_| None, "IPM.Contact", b"png").is_none(), "a contact's other file");
    // One past the cap is counted, not left out unsaid as a hidden file.
    let big = vec![0u8; super::super::MAX_ATTACHMENT + 1];
    assert!(matches!(photo(&flagged, "IPM.Contact", &big), Some(Got::Left(SkipReason::AttachmentTooBig))));
}

/// The files kept on cards and events stay within what MailCraft reads back: the contacts and
/// calendar written out as one `.vcf` and one `.ics` (as MailCraft writes them) read back whole,
/// every card and event with every file that was kept; those past the cap are counted.
#[test]
fn kept_files_read_back_from_the_exported_files() {
    let s = Scratch::new("pim-files-back");
    let mut progress = |_| {};
    let mut out = Out::new(&s.1, "d1", &ImportOptions::default(), &mut progress);
    // Pictures and files of 700,000 bytes: about 0.93 MB each in base64.
    for i in 0..6 {
        out.add_pim(Item {
            class: "IPM.Contact".into(),
            photo: Some(("image/jpeg".into(), vec![0xFF; 700_000])),
            extra: Some(Extra { given: format!("Person {i}"), ..Default::default() }),
            ..Default::default()
        });
        let start = 1_790_000_000 + i * 86_400;
        out.add_pim(Item {
            class: "IPM.Appointment".into(),
            subject: format!("Review {i}"),
            files: vec![data("proofs.pdf", "application/pdf", 700_000)],
            extra: Some(Extra { start: Some(start), end: Some(start + 3600), ..Default::default() }),
            ..Default::default()
        });
    }
    let done = out.finish();
    assert_eq!((done.contacts, done.events), (6, 6));
    let lost = done.skips.get(&SkipReason::AttachmentTooBig).copied().unwrap_or(0);
    assert_eq!(lost, 12 - 8, "eight fit in what's kept");
    let pim: Pim = s.1.load_account_doc("d1", PIM_DOC).unwrap();
    let vcf: String = pim.cards.iter().map(|c| mailcraft_model::vcard::write(&c.card)).collect();
    let ics = mailcraft_model::ical::write(&pim.events.iter().map(|e| e.event.clone()).collect::<Vec<_>>(), None, 0);
    assert!(vcf.len() > 1 << 20 && ics.len() > 1 << 20, "past the old limits: {} {}", vcf.len(), ics.len());
    let cards = mailcraft_model::vcard::parse(&vcf);
    let events = mailcraft_model::ical::parse(&ics, 0).events;
    assert_eq!((cards.len(), events.len()), (6, 6));
    let files = |extra: &[String]| extra.iter().filter(|l| crate::pst_pim::is_file_line(l)).count();
    let kept = cards.iter().map(|c| files(&c.extra)).sum::<usize>() + events.iter().map(|e| files(&e.extra)).sum::<usize>();
    assert_eq!(kept, 8);
}

/// A file the writer made with a Tasks folder (`IPF.Task`) of Outlook tasks: every one comes in
/// through the whole import, the one with no due day too, with its status, due day and priority
/// from the file's own named properties; the mail beside them stays mail.
#[test]
fn a_data_files_tasks_come_in() {
    use pst_rw::ltp::prop_context::GuidValue;
    use pst_rw::{UnicodePstBatchMessage, UnicodePstFile as Writer, UnicodePstMessage, UnicodePstName, UnicodePstProperty, UnicodePstValue};
    let dir = std::env::temp_dir().join(format!("mailcraft-pst-tasks-file-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let pst = dir.join("tasks.pst");
    let task_set = GuidValue::new(0x0006_2003, 0, 0, [0xC0, 0, 0, 0, 0, 0, 0, 0x46]);
    let day = mailcraft_model::dates::days_from_civil(2026, 11, 5) * 86_400;
    fn filetime(t: i64) -> i64 {
        (t + 11_644_473_600) * 10_000_000
    }
    fn message<'a>(subject: &'a str, id: &'a str, class: &'a str, properties: &'a [UnicodePstProperty<'a>]) -> UnicodePstMessage<'a> {
        UnicodePstMessage {
            subject,
            sender_name: "Riley",
            sender_email: "riley@studio.example",
            recipients: &[],
            body: "Notes",
            html_body: None,
            message_id: id,
            delivery_time: filetime(1_700_000_000),
            message_class: class,
            properties,
        }
    }
    // Due (as Outlook stores the day: its local midnight, two hours east), in progress, high.
    let due = [
        UnicodePstProperty::named(task_set, UnicodePstName::Id(0x8105), UnicodePstValue::Time(filetime(day - 7200))),
        UnicodePstProperty::named(task_set, UnicodePstName::Id(0x8101), UnicodePstValue::Integer32(1)),
        UnicodePstProperty::tagged(0x0017, UnicodePstValue::Integer32(2)),
    ];
    // No day at all, waiting on someone else.
    let waiting = [UnicodePstProperty::named(task_set, UnicodePstName::Id(0x8101), UnicodePstValue::Integer32(3))];
    Writer::create_in_folder_with_attachments(&pst, &["Inbox"], &message("Hello", "<m1@studio.example>", "IPM.Note", &[]), &[]).unwrap();
    drop(Writer::create_folder(&pst, &["Tasks"], "IPF.Task").unwrap());
    let tasks = [
        UnicodePstBatchMessage { message: message("Book the stage", "<t1@studio.example>", "IPM.Task", &due), attachments: &[] },
        UnicodePstBatchMessage { message: message("Renew the licences", "<t2@studio.example>", "IPM.Task", &waiting), attachments: &[] },
    ];
    Writer::append_many_in_folder_with_attachments(&pst, &["Tasks"], &tasks).unwrap();
    let store = Store::open_packed(dir.join("root"), dir.join("files")).unwrap();
    let done = super::super::import(&pst, &store, "t1", &mut |_| {}).unwrap();
    assert_eq!((done.messages, done.tasks, done.events, done.skipped), (1, 2, 0, 0), "{done:?}");
    let pim: Pim = store.load_account_doc("t1", PIM_DOC).unwrap();
    let mut got: Vec<(String, Option<i64>, mailcraft_model::pim::TaskStatus, mailcraft_model::pim::TaskPriority)> =
        pim.tasks.iter().map(|t| (t.task.title.clone(), t.task.due, t.task.status, t.task.priority)).collect();
    got.sort_by(|a, b| a.0.cmp(&b.0));
    use mailcraft_model::pim::{TaskPriority as P, TaskStatus as S};
    assert_eq!(
        got,
        [("Book the stage".to_string(), Some(day), S::InProgress, P::High), ("Renew the licences".to_string(), None, S::Waiting, P::Normal)]
    );
    drop(store);
    let _ = std::fs::remove_dir_all(&dir);
}
