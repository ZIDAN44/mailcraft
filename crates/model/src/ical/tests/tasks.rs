//! To-dos (`VTODO`): read, written and read back, with what MailCraft doesn't read kept.

use super::*;
use crate::pim::{Task, TaskPriority, TaskStatus};

const TODOS: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Example//EN\r\n\
BEGIN:VTODO\r\nUID:t-1\r\nDTSTAMP:20261101T090000Z\r\nSUMMARY:Send the grading notes\r\nDESCRIPTION:Reel 2\\, then 3\r\n\
DTSTART;VALUE=DATE:20261102\r\nDUE;VALUE=DATE:20261105\r\nSTATUS:IN-PROCESS\r\nPERCENT-COMPLETE:40\r\nPRIORITY:1\r\n\
CATEGORIES:Work,Clients\r\nRRULE:FREQ=WEEKLY;BYDAY=TH\r\nATTACH;FMTTYPE=text/plain;ENCODING=BASE64;VALUE=BINARY:aGk=\r\n\
X-UNKNOWN:kept\r\nBEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER;VALUE=DATE-TIME:20261105T080000Z\r\nEND:VALARM\r\nEND:VTODO\r\n\
BEGIN:VTODO\r\nUID:t-2\r\nSUMMARY:Renew the domain\r\nSTATUS:COMPLETED\r\nCOMPLETED:20261020T101500Z\r\nEND:VTODO\r\n\
BEGIN:VTODO\r\nUID:t-3\r\nSUMMARY:Ask about the venue\r\nX-MAILCRAFT-TASK-STATUS:waiting\r\nPRIORITY:9\r\nEND:VTODO\r\n\
BEGIN:VEVENT\r\nUID:e-1\r\nDTSTART:20261103T100000Z\r\nDTEND:20261103T110000Z\r\nSUMMARY:Review\r\nEND:VEVENT\r\n\
END:VCALENDAR\r\n";

/// A to-do's fields come in; a task with no date is a task all the same; events are read beside.
#[test]
fn todos_are_read() {
    let cal = parse(TODOS, 0);
    assert_eq!(cal.events.len(), 1);
    assert_eq!(cal.tasks.len(), 3);
    let t = &cal.tasks[0];
    assert_eq!((t.uid.as_str(), t.title.as_str(), t.notes.as_str()), ("t-1", "Send the grading notes", "Reel 2, then 3"));
    assert_eq!((t.start, t.due), (Some(at(2026, 11, 2, 0, 0)), Some(at(2026, 11, 5, 0, 0))));
    assert_eq!((t.status, t.percent, t.priority), (TaskStatus::InProgress, 40, TaskPriority::High));
    assert_eq!(t.categories, ["Work", "Clients"]);
    assert_eq!(t.rrule.as_deref(), Some("FREQ=WEEKLY;BYDAY=TH"));
    assert_eq!(t.reminder, Some(at(2026, 11, 5, 8, 0)));
    assert!(t.extra.iter().any(|l| l.starts_with("ATTACH;")) && t.extra.iter().any(|l| l == "X-UNKNOWN:kept"));
    assert!(!t.extra.iter().any(|l| l.contains("VALARM")), "the alarm is the reminder");
    let done = &cal.tasks[1];
    assert_eq!((done.status, done.percent, done.completed, done.due), (TaskStatus::Completed, 100, Some(at(2026, 10, 20, 10, 15)), None));
    let waiting = &cal.tasks[2];
    assert_eq!((waiting.status, waiting.priority), (TaskStatus::Waiting, TaskPriority::Low));
}

/// What's written reads back the same; Waiting stays Waiting, a repeating task without a start
/// repeats from its due day, and a due day before the start isn't written.
#[test]
fn todos_read_back_as_written() {
    let cal = parse(TODOS, 0);
    let written = write_calendar(&cal.events, &cal.tasks, at(2026, 11, 3, 0, 0));
    assert!(written.contains("BEGIN:VTODO") && written.contains("X-MAILCRAFT-TASK-STATUS:waiting"), "{written}");
    let back = parse(&written, 0);
    assert_eq!(back.events.len(), 1);
    assert_eq!(back.tasks, cal.tasks);
    let odd = Task {
        uid: "o".into(),
        title: "Odd".into(),
        start: Some(at(2026, 11, 9, 0, 0)),
        due: Some(at(2026, 11, 2, 0, 0)),
        rrule: None,
        ..Default::default()
    };
    let repeating =
        Task { uid: "r".into(), title: "Weekly".into(), due: Some(at(2026, 11, 2, 0, 0)), rrule: Some("FREQ=WEEKLY".into()), ..Default::default() };
    let text = write_calendar(&[], &[odd, repeating], 0);
    let back = parse(&text, 0);
    assert_eq!(back.tasks[0].due, Some(at(2026, 11, 9, 0, 0)));
    assert_eq!(back.tasks[1].start, Some(at(2026, 11, 2, 0, 0)));
    // A file with no tasks writes none.
    assert!(!write(&back.events, None, 0).contains("VTODO"));
}

/// Times for dates fall on the viewer's day; a relative alarm counts from the start (or the due
/// day); a to-do inside an event isn't one of the file's; one with junk in it still comes in.
#[test]
fn todo_times_and_junk() {
    let text = "BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:a\r\nDUE:20261105T230000Z\r\nBEGIN:VALARM\r\nTRIGGER;RELATED=END:-PT1H\r\nEND:VALARM\r\nEND:VTODO\r\n\
BEGIN:VTODO\r\nUID:b\r\nDTSTART;VALUE=DATE:20261102\r\nDURATION:P2D\r\nPERCENT-COMPLETE:900\r\nPRIORITY:x\r\nBEGIN:VALARM\r\nTRIGGER:-P1D\r\nEND:VALARM\r\nEND:VTODO\r\n\
BEGIN:VEVENT\r\nUID:e\r\nDTSTART:20261103T100000Z\r\nBEGIN:VTODO\r\nUID:inside\r\nEND:VTODO\r\nEND:VEVENT\r\n\
BEGIN:VTODO\r\nUID:c\r\nDUE:garbage\r\nSTATUS:\r\nBEGIN:X-THING\r\nX-A:1\r\nEND:X-THING\r\nEND:VTODO\r\nEND:VCALENDAR\r\n";
    let cal = parse(text, 2 * 3600);
    let ids: Vec<&str> = cal.tasks.iter().map(|t| t.uid.as_str()).collect();
    assert_eq!(ids, ["a", "b", "c"]);
    let a = &cal.tasks[0];
    assert_eq!(a.due, Some(at(2026, 11, 6, 0, 0)), "23:00 UTC is the next day two hours east");
    assert_eq!(a.reminder, Some(at(2026, 11, 6, 0, 0) - 2 * 3600 - 3600));
    let b = &cal.tasks[1];
    assert_eq!(b.due, Some(at(2026, 11, 4, 0, 0)));
    assert_eq!((b.percent, b.status, b.priority), (100, TaskStatus::Completed, TaskPriority::Normal));
    assert_eq!(b.reminder, Some(at(2026, 11, 1, 0, 0) - 2 * 3600));
    let c = &cal.tasks[2];
    assert_eq!(c.due, None);
    assert_eq!(c.extra, ["BEGIN:X-THING", "X-A:1", "END:X-THING"]);
    // Cut off mid-task: nothing half-read.
    assert!(parse("BEGIN:VCALENDAR\r\nBEGIN:VTODO\r\nUID:z\r\n", 0).tasks.is_empty());
}
