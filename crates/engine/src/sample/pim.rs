//! The sample's calendar, tasks, notes and address book.

use mailcraft_model::pim::{StoredTask, Task, TaskPriority, TaskStatus};

use super::*;

/// The sample calendar (a week around `now`, in local time) and address book.
pub fn sample_pim(now: i64, offset: i64) -> mailcraft_model::pim::Pim {
    use mailcraft_model::ical::{Attendee, CalEvent};
    use mailcraft_model::pim::{LOCAL, Pim, StoredCard, StoredEvent};
    use mailcraft_model::vcard::{Card, Typed};
    let today = (now + offset).div_euclid(86_400);
    let monday = today - i64::from(mailcraft_model::dates::weekday_index(today));
    let at = |day: i64, h: i64, m: i64| day * 86_400 + h * 3600 + m * 60 - offset;
    let who = |name: &str, email: &str, partstat: &str| Attendee {
        email: email.into(),
        name: name.into(),
        partstat: partstat.into(),
        rsvp: false,
        ..Default::default()
    };
    let ev = |uid: &str, summary: &str, start: i64, end: i64| CalEvent {
        uid: format!("{uid}@sample.mailcraft"),
        summary: summary.into(),
        start,
        end,
        ..Default::default()
    };
    let events = vec![
        CalEvent {
            rrule: Some("FREQ=WEEKLY;BYDAY=MO,WE,FR".into()),
            location: "Studio floor".into(),
            ..ev("standup", "Team stand-up", at(monday - 14, 9, 30), at(monday - 14, 9, 45))
        },
        CalEvent {
            location: "Screening room".into(),
            description: "Revised frames for scenes 4 to 7, and the colour key.".into(),
            organizer: Some(who("Zoe Park", "zoe@studio.example", "ACCEPTED")),
            attendees: vec![who("Riley Moreno", ME.1, "ACCEPTED"), who("Sam Okafor", "sam@studio.example", "ACCEPTED")],
            ..ev("review", "Storyboard review", at(monday + 3, 14, 0), at(monday + 3, 15, 30))
        },
        CalEvent { location: "Café on the corner".into(), ..ev("lunch", "Lunch with Jun", at(today, 12, 30), at(today, 13, 30)) },
        CalEvent {
            location: "Sound Lab".into(),
            organizer: Some(who("Ana Lima", "ana@soundlab.example", "ACCEPTED")),
            attendees: vec![who("Riley Moreno", ME.1, "NEEDS-ACTION")],
            ..ev("mix", "Sound mix playback", at(today + 1, 11, 0), at(today + 1, 12, 0))
        },
        CalEvent { all_day: true, ..ev("poster", "Poster to the print shop", (today + 2) * 86_400, (today + 3) * 86_400) },
        CalEvent { location: "Community Gallery".into(), ..ev("opening", "Winter group show opening", at(today + 9, 18, 0), at(today + 9, 21, 0)) },
    ];
    let person = |uid: &str, name: &str, email: &str, org: &str, title: &str, phone: &str| Card {
        uid: format!("{uid}@sample.mailcraft"),
        name: name.into(),
        emails: vec![Typed { value: email.into(), kind: "work".into() }],
        phones: if phone.is_empty() { Vec::new() } else { vec![Typed { value: phone.into(), kind: "cell".into() }] },
        org: org.into(),
        title: title.into(),
        ..Default::default()
    };
    let cards = vec![
        person("zoe", "Zoe Park", "zoe@studio.example", "Studio Lantern", "Director", "+1 555 0142"),
        person("sam", "Sam Okafor", "sam@studio.example", "Studio Lantern", "Editor", "+1 555 0177"),
        person("ana", "Ana Lima", "ana@soundlab.example", "Sound Lab", "Sound designer", ""),
        person("jun", "Jun Watanabe", "jun@studio.example", "Studio Lantern", "Animator", "+1 555 0108"),
        person("print", "Print Shop", "orders@printshop.example", "Print Shop", "", "+1 555 0190"),
    ];
    // The studio's own calendar beside the personal one (to see side by side, or hide).
    let studio = vec![
        CalEvent { location: "Edit suite".into(), ..ev("edit", "Edit session: act two", at(today, 10, 0), at(today, 12, 0)) },
        CalEvent { location: "Machine room".into(), ..ev("render", "Render farm maintenance", at(today + 1, 9, 0), at(today + 1, 13, 0)) },
    ];
    let mut p = Pim::default();
    p.ensure_defaults();
    p.calendars.push(mailcraft_model::pim::CalendarInfo { id: STUDIO_CALENDAR.into(), name: "Studio".into(), color: 3, ..Default::default() });
    p.events = events
        .into_iter()
        .map(|event| StoredEvent { calendar: LOCAL.into(), event, ..Default::default() })
        .chain(studio.into_iter().map(|event| StoredEvent { calendar: STUDIO_CALENDAR.into(), event, ..Default::default() }))
        .collect();
    // Categories colour some of them (as in Outlook).
    for e in &mut p.events {
        match e.event.uid.split('@').next().unwrap_or("") {
            "review" | "edit" => e.event.categories = vec!["Work".into()],
            "lunch" => {
                e.event.categories = vec!["Personal".into()];
                // Private (issue #32): the calendar marks it with a lock.
                mailcraft_model::ical::set_privacy(&mut e.event, mailcraft_model::ical::Privacy::Private);
            }
            // Out of the studio, as Outlook's Show As has it.
            "opening" => mailcraft_model::ical::set_show_as(&mut e.event, mailcraft_model::ical::ShowAs::OutOfOffice),
            _ => {}
        }
    }
    p.cards = cards.into_iter().map(|card| StoredCard { book: LOCAL.into(), card, ..Default::default() }).collect();
    // Notes (Outlook's Notes), kept on this computer.
    let note = |uid: &str, text: &str, color: u8, ago: i64| mailcraft_model::pim::StoredNote {
        note: mailcraft_model::pim::Note { uid: format!("{uid}@sample.mailcraft"), text: text.into(), color, modified: now - ago, files: Vec::new() },
        ..Default::default()
    };
    p.notes = vec![
        note("note-poster", "Festival poster\nApprove the proof by Thursday; ask about 170 gsm paper.", 3, 3600),
        note("note-mix", "Sound mix v3\nRain under the café scene: a little quieter.", 0, 86_400),
        note("note-fox", "Fox turnarounds\nSide view: longer tail.", 1, 3 * 86_400),
    ];
    // Tasks (Outlook's Tasks): some due, one late, one done, one with no day at all.
    let day = today * 86_400;
    let task = |uid: &str, title: &str, due: Option<i64>, status: TaskStatus, percent: u8, priority: TaskPriority| StoredTask {
        task: Task {
            uid: format!("{uid}@sample.mailcraft"),
            title: title.into(),
            due,
            status,
            percent,
            priority,
            modified: now - 3600,
            ..Default::default()
        },
        ..Default::default()
    };
    p.tasks = vec![
        task("task-grade", "Send the grading notes to the client", Some(day), TaskStatus::InProgress, 50, TaskPriority::High),
        task("task-stage", "Book the sound stage", Some(day + 3 * 86_400), TaskStatus::NotStarted, 0, TaskPriority::Normal),
        task("task-posters", "Order the festival posters", Some(day - 86_400), TaskStatus::NotStarted, 0, TaskPriority::Normal),
        task("task-fonts", "Renew the font licences", None, TaskStatus::Waiting, 0, TaskPriority::Low),
        task("task-backup", "Back up the edit drives", Some(day + 2 * 86_400), TaskStatus::NotStarted, 0, TaskPriority::Normal),
        task("task-kit", "Return the camera kit", Some(day - 2 * 86_400), TaskStatus::Completed, 100, TaskPriority::Normal),
    ];
    for t in &mut p.tasks {
        match t.task.uid.split('@').next().unwrap_or("") {
            "task-grade" => {
                t.task.categories = vec!["Work".into()];
                t.task.notes = "Reels 2 and 3; the café scene is warmer now.".into();
            }
            "task-backup" => t.task.rrule = Some("FREQ=WEEKLY".into()),
            "task-kit" => t.task.completed = Some(now - 2 * 86_400),
            _ => {}
        }
    }
    p
}
