//! The fake EWS server's tests of calendars, contacts, notes, public folders and shared
//! mailboxes.

use super::*;

#[test]
fn calendars_and_contacts_sync_and_autodiscover_finds_the_server() {
    use mailcraft_model::ical::CalEvent;
    use mailcraft_model::pim::{LOCAL, Pim, StoredCard, StoredEvent, Tombstone};
    use mailcraft_model::vcard::{Card, Typed};
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let cal = d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
    let standup = parse(
        "<t:CalendarItem><t:Subject>Stand-up</t:Subject><t:Body BodyType=\"Text\">Daily</t:Body><t:Start>2026-10-05T09:00:00Z</t:Start>\
         <t:End>2026-10-05T09:15:00Z</t:End><t:IsAllDayEvent>false</t:IsAllDayEvent><t:Location>Room 2</t:Location><t:UID>standup-1</t:UID>\
         <t:Recurrence><t:WeeklyRecurrence><t:Interval>1</t:Interval><t:DaysOfWeek>Monday Wednesday Friday</t:DaysOfWeek></t:WeeklyRecurrence>\
         <t:NoEndRecurrence><t:StartDate>2026-10-05Z</t:StartDate></t:NoEndRecurrence></t:Recurrence></t:CalendarItem>",
    );
    d.pim.push((
        "OLD".into(),
        cal.clone(),
        parse("<t:CalendarItem><t:Subject>Gone</t:Subject><t:Start>2026-10-01T09:00:00Z</t:Start></t:CalendarItem>").kids[0].clone(),
    ));
    d.pim.push(("E1".into(), cal.clone(), standup.kids[0].clone()));
    let server = serve(d);
    let user = format!("{DOMAIN}\\{USER}");
    assert_eq!(crate::ews::autodiscover("riley@contoso.example", &user, PASSWORD, Some(&server.url)).unwrap(), server.url);
    let c = crate::ews::Client::connect(&server.url, &user, PASSWORD, "riley@contoso.example").unwrap();
    let mut pim = Pim::default();
    pim.events.push(StoredEvent {
        calendar: LOCAL.into(),
        event: CalEvent {
            uid: "local-1".into(),
            summary: "Poster review".into(),
            start: 1_792_000_000,
            end: 1_792_003_600,
            rrule: Some("FREQ=DAILY;COUNT=3".into()),
            ..Default::default()
        },
        dirty: true,
        ..Default::default()
    });
    pim.cards.push(StoredCard {
        book: LOCAL.into(),
        card: Card {
            uid: "p1".into(),
            name: "Ben Ito".into(),
            emails: vec![Typed { value: "ben@contoso.example".into(), kind: String::new() }],
            ..Default::default()
        },
        dirty: true,
        ..Default::default()
    });
    pim.deleted.push(Tombstone { href: "OLD".into(), etag: String::new(), ..Default::default() });
    let sync = crate::ews_pim::sync(&c, &pim, 1_792_000_000).unwrap();
    assert!(sync.errors.is_empty(), "{:?}", sync.errors);
    assert_eq!(sync.deleted, ["OLD"]);
    let standup = sync.events.iter().find(|e| e.event.summary == "Stand-up").unwrap();
    assert_eq!(standup.event.rrule.as_deref(), Some("FREQ=WEEKLY;BYDAY=MO,WE,FR"));
    assert_eq!((standup.event.uid.as_str(), standup.event.location.as_str(), standup.event.end - standup.event.start), ("standup-1", "Room 2", 900));
    let poster = sync.events.iter().find(|e| e.event.summary == "Poster review").unwrap();
    assert_eq!(poster.event.rrule.as_deref(), Some("FREQ=DAILY;COUNT=3"), "made on the server with its recurrence");
    assert!(sync.cards.iter().any(|k| k.card.name == "Ben Ito" && k.card.emails[0].value == "ben@contoso.example"));
    assert_eq!(sync.pushed_events.len(), 1);
    // A change goes field by field; the series keeps its id.
    let mut changed = pim.clone();
    changed.deleted.clear();
    changed.cards.clear();
    changed.events = vec![StoredEvent {
        href: standup.href.clone(),
        event: CalEvent { summary: "Stand-up (moved)".into(), rrule: None, ..standup.event.clone() },
        dirty: true,
        ..standup.clone()
    }];
    let sync = crate::ews_pim::sync(&c, &changed, 1_792_000_000).unwrap();
    let moved = sync.events.iter().find(|e| e.href == standup.href).unwrap();
    assert_eq!((moved.event.summary.as_str(), moved.event.rrule.as_deref()), ("Stand-up (moved)", None));
    // No Notes folder on this server: notes stay on this computer.
    assert!(sync.notes.is_none());
}

#[test]
fn sticky_notes_sync_with_the_notes_folder() {
    use mailcraft_model::pim::{Note, Pim, StoredNote};
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
    let notes = d.folder("Notes", None, Some("notes"), "IPF.StickyNote");
    let old = parse(
        "<t:Message><t:ItemClass>IPM.StickyNote</t:ItemClass><t:Subject>Call Ana</t:Subject><t:Body BodyType=\"Text\">Call Ana\r\nabout the mix</t:Body>\
         <t:ExtendedProperty><t:ExtendedFieldURI PropertySetId=\"0006200E-0000-0000-C000-000000000046\" PropertyId=\"35584\" PropertyType=\"Integer\"/><t:Value>2</t:Value></t:ExtendedProperty></t:Message>",
    );
    d.pim.push(("N1".into(), notes, old.kids[0].clone()));
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let mut pim = Pim::default();
    pim.notes.push(StoredNote {
        note: Note { uid: "n-local".into(), text: "Buy paper\n170 gsm".into(), color: 1, modified: 5 },
        dirty: true,
        ..Default::default()
    });
    let sync = crate::ews_pim::sync(&c, &pim, 1_792_000_000).unwrap();
    assert!(sync.errors.is_empty(), "{:?}", sync.errors);
    let list = sync.notes.unwrap();
    let ana = list.iter().find(|n| n.href == "note:N1").unwrap();
    assert_eq!((ana.note.text.as_str(), ana.note.color), ("Call Ana\nabout the mix", 2));
    assert!(list.iter().any(|n| n.note.text == "Buy paper\n170 gsm" && n.note.color == 1), "made in the Notes folder, its colour kept");
    assert_eq!(sync.pushed_notes.len(), 1);
    assert!(sync.pushed_notes[0].href.starts_with("note:"));
}

#[test]
fn someone_elses_calendar_opens_read_only() {
    use mailcraft_model::pim::{Pim, shared_calendar_id};
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
    let review = parse(
        "<t:CalendarItem><t:Subject>Budget review</t:Subject><t:Start>2026-10-06T13:00:00Z</t:Start><t:End>2026-10-06T14:00:00Z</t:End>\
         <t:IsAllDayEvent>false</t:IsAllDayEvent><t:UID>budget-1</t:UID></t:CalendarItem>",
    );
    d.pim.push(("S1".into(), "mailbox:ana@contoso.example".into(), review.kids[0].clone()));
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let pim = Pim { shared: vec!["ana@contoso.example".into(), "nobody@contoso.example".into()], ..Default::default() };
    let sync = crate::ews_pim::sync(&c, &pim, 1_792_000_000).unwrap();
    let id = shared_calendar_id("ana@contoso.example");
    let cals = sync.calendars.clone().unwrap();
    assert!(cals.iter().any(|x| x.id == id && x.read_only && x.name == "ana@contoso.example"));
    assert!(sync.events.iter().any(|e| e.calendar == id && e.event.summary == "Budget review"));
    assert!(sync.synced_calendars.contains(&id));
    // One that can't be read says so, and the rest still syncs.
    assert!(sync.errors.iter().any(|e| e.starts_with("nobody@contoso.example's calendar")), "{:?}", sync.errors);
    assert!(!sync.synced_calendars.contains(&shared_calendar_id("nobody@contoso.example")));
}

/// Three plain appointments carrying one UID: the sync hands the engine three distinct
/// identities, not one.
#[test]
fn meetings_reusing_one_uid_each_reach_the_calendar() {
    use mailcraft_model::pim::Pim;
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let cal = d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
    for (id, day) in [("C1", "06"), ("C2", "13"), ("C3", "20")] {
        let it = parse(&format!(
            "<t:CalendarItem><t:Subject>Safety committee</t:Subject><t:Start>2026-10-{day}T08:00:00Z</t:Start><t:End>2026-10-{day}T10:00:00Z</t:End>\
             <t:IsAllDayEvent>false</t:IsAllDayEvent><t:CalendarItemType>Single</t:CalendarItemType><t:UID>committee-series</t:UID></t:CalendarItem>"
        ));
        d.pim.push((id.into(), cal.clone(), it.kids[0].clone()));
    }
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let sync = crate::ews_pim::sync(&c, &Pim::default(), 1_792_000_000).unwrap();
    // Distinct (UID, instance) pairs: what the engine stores separately.
    let ids: std::collections::HashSet<(String, Option<i64>)> =
        sync.events.iter().filter(|e| e.event.summary == "Safety committee").map(|e| (e.event.uid.clone(), e.event.recurrence_id)).collect();
    assert_eq!(ids.len(), 3, "{ids:?}");
}

/// Someone who made this account an editor of their calendar: events go in, change and go
/// there; someone who didn't keeps theirs read-only.
#[test]
fn someone_elses_calendar_can_be_changed_by_an_editor() {
    use mailcraft_model::ical::CalEvent;
    use mailcraft_model::pim::{Pim, StoredEvent, Tombstone, shared_calendar_id};
    let mut d = Data { editors: vec!["ana@contoso.example".into()], ..Default::default() };
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
    let item = |subject: &str, uid: &str| {
        parse(&format!(
            "<t:CalendarItem><t:Subject>{subject}</t:Subject><t:Start>2026-10-06T13:00:00Z</t:Start><t:End>2026-10-06T14:00:00Z</t:End>\
             <t:IsAllDayEvent>false</t:IsAllDayEvent><t:UID>{uid}</t:UID></t:CalendarItem>"
        ))
        .kids[0]
            .clone()
    };
    d.pim.push(("S1".into(), "mailbox:ana@contoso.example".into(), item("Budget review", "budget-1")));
    d.pim.push(("S2".into(), "mailbox:ana@contoso.example".into(), item("Team lunch", "lunch-1")));
    d.pim.push(("B1".into(), "mailbox:ben@contoso.example".into(), item("Ben's call", "call-1")));
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let (ana, ben) = (shared_calendar_id("ana@contoso.example"), shared_calendar_id("ben@contoso.example"));
    let mut pim = Pim { shared: vec!["ana@contoso.example".into(), "ben@contoso.example".into()], ..Default::default() };
    let first = crate::ews_pim::sync(&c, &pim, 1_792_000_000).unwrap();
    let cals = first.calendars.clone().unwrap();
    let ro = |id: &str| cals.iter().find(|x| x.id == id).map(|x| x.read_only);
    assert_eq!((ro(&ana), ro(&ben)), (Some(false), Some(true)));
    let mut review = first.events.iter().find(|e| e.event.summary == "Budget review").cloned().unwrap();
    let lunch = first.events.iter().find(|e| e.event.summary == "Team lunch").cloned().unwrap();
    review.event.summary = "Budget review (moved)".into();
    review.dirty = true;
    pim.events = vec![
        review,
        StoredEvent {
            calendar: ana.clone(),
            event: CalEvent { uid: "new-1".into(), summary: "Poster check".into(), start: 1_792_000_000, end: 1_792_003_600, ..Default::default() },
            dirty: true,
            ..Default::default()
        },
    ];
    pim.deleted = vec![Tombstone { href: lunch.href.clone(), etag: String::new(), calendar: ana.clone() }];
    let second = crate::ews_pim::sync(&c, &pim, 1_792_000_100).unwrap();
    assert!(second.errors.is_empty(), "{:?}", second.errors);
    assert!(second.pushed_events.iter().all(|e| e.calendar == ana), "they stay in Ana's calendar");
    let d = server.data.lock().unwrap();
    let in_ana: Vec<String> = d
        .pim
        .iter()
        .filter(|(_, f, _)| f == "mailbox:ana@contoso.example")
        .filter_map(|(_, _, el)| el.text_of("Subject").map(str::to_string))
        .collect();
    assert!(in_ana.contains(&"Budget review (moved)".to_string()) && in_ana.contains(&"Poster check".to_string()), "{in_ana:?}");
    assert!(!in_ana.contains(&"Team lunch".to_string()), "deleted");
    assert!(!d.pim.iter().any(|(_, f, el)| !f.starts_with("mailbox:") && el.text_of("Subject") == Some("Poster check")), "not in this mailbox");
}

/// Issue #1, again: a server whose answer about someone's calendar folder doesn't say an
/// editor may change it, while each of its events does (as Outlook finds by trying).
#[test]
fn an_editor_is_known_by_the_events_rights_too() {
    use mailcraft_model::pim::{Pim, shared_calendar_id};
    let mut d = Data { editors: vec!["ana@contoso.example".into()], item_rights_only: true, ..Default::default() };
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
    let item = parse(
        "<t:CalendarItem><t:Subject>Budget review</t:Subject><t:Start>2026-10-06T13:00:00Z</t:Start><t:End>2026-10-06T14:00:00Z</t:End>\
         <t:IsAllDayEvent>false</t:IsAllDayEvent><t:UID>budget-1</t:UID></t:CalendarItem>",
    );
    d.pim.push(("S1".into(), "mailbox:ana@contoso.example".into(), item.kids[0].clone()));
    d.pim.push(("B1".into(), "mailbox:ben@contoso.example".into(), item.kids[0].clone()));
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let pim = Pim { shared: vec!["ana@contoso.example".into(), "ben@contoso.example".into()], ..Default::default() };
    let sync = crate::ews_pim::sync(&c, &pim, 1_792_000_000).unwrap();
    let cals = sync.calendars.clone().unwrap();
    let ro = |id: &str| cals.iter().find(|x| x.id == id).map(|x| x.read_only);
    assert_eq!(
        (ro(&shared_calendar_id("ana@contoso.example")), ro(&shared_calendar_id("ben@contoso.example"))),
        (Some(false), Some(true)),
        "Ana made this account an editor; Ben didn't"
    );
}

#[test]
fn public_folders_list_under_their_own_folder_and_sync() {
    // Exchange 2013 and later: the hierarchy and the content each in a public folder mailbox
    // of their own, reached only when routed there.
    let mut d =
        Data { pf_mailbox: Some("PFContent@contoso.example".into()), pf_hierarchy: Some("PFHierarchy@contoso.example".into()), ..Default::default() };
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
    let root = d.folder("IPM_SUBTREE", None, Some("publicfoldersroot"), "IPF.Note");
    let team = d.folder("Team", Some(&root), None, "IPF.Note");
    let news = d.folder("Announcements", Some(&team), None, "IPF.Note");
    let rooms = d.folder("Rooms", Some(&root), None, "IPF.Appointment");
    let shared = d.folder("Company contacts", Some(&team), None, "IPF.Contact");
    d.pim.push((
        "PC1".into(),
        shared,
        parse("<t:Contact><t:ItemId Id=\"PC1\" ChangeKey=\"k\"/><t:DisplayName>Front desk</t:DisplayName><t:EmailAddresses><t:Entry Key=\"EmailAddress1\">desk@contoso.example</t:Entry></t:EmailAddresses></t:Contact>")
            .kids[0]
            .clone(),
    ));
    d.pim.push((
        "PE1".into(),
        rooms,
        parse("<t:CalendarItem><t:Subject>Room 2 booked</t:Subject><t:Start>2026-10-16T09:00:00Z</t:Start><t:End>2026-10-16T10:00:00Z</t:End><t:UID>room-1</t:UID></t:CalendarItem>")
            .kids[0]
            .clone(),
    ));
    d.deliver(
        &news,
        b"From: hr@contoso.example\r\nSubject: Holiday hours\r\nMessage-ID: <h@x>\r\n\r\nClosed Friday.\r\n",
        false,
        "2026-10-14T09:00:00Z",
    );
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let boxes = c.mailboxes().unwrap();
    let names: Vec<&str> = boxes.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(names, ["Inbox", "Public Folders", "Team", "Announcements"], "the room calendar isn't mail");
    let pf = boxes.iter().find(|m| m.name == "Public Folders").unwrap();
    let team_box = boxes.iter().find(|m| m.name == "Team").unwrap();
    assert_eq!((pf.parent.as_deref(), team_box.parent.as_deref()), (None, Some(pf.id.as_str())));
    assert_eq!(boxes.iter().find(|m| m.name == "Announcements").unwrap().parent.as_deref(), Some(team_box.id.as_str()));
    // Their mail syncs as any folder's.
    let store = crate::fake::tests_store("public-folders");
    let mut st = crate::jmap::State::default();
    let folders = crate::ews::list_folders(&c, &mut st, "a1").unwrap();
    let news_path = folders.iter().find(|f| f.path.ends_with("Announcements")).map(|f| f.path.clone()).unwrap();
    crate::ews::sync_folder(&c, &mut st, &store, "a1", &news_path, &crate::sync::SyncOptions::default()).unwrap();
    let fs = store.load_folder("a1", &news_path).unwrap();
    assert_eq!(fs.messages.len(), 1);
    assert_eq!(fs.messages[0].subject, "Holiday hours");
    let _ = std::fs::remove_dir_all(store.root());
    // Issue #1: the organisation's shared contacts and calendars come in, read-only.
    let sync = crate::ews_pim::sync(&c, &mailcraft_model::pim::Pim::default(), 0).unwrap();
    let books = sync.books.clone().unwrap_or_default();
    let book = books.iter().find(|b| b.id.starts_with("public:")).unwrap();
    assert_eq!((book.name.as_str(), book.read_only), ("Team › Company contacts", true));
    assert!(sync.cards.iter().any(|k| k.book == book.id && k.card.display() == "Front desk"));
    let cals = sync.calendars.clone().unwrap_or_default();
    let room = cals.iter().find(|c| c.id.starts_with("public:")).unwrap();
    assert_eq!((room.name.as_str(), room.read_only), ("Rooms", true));
    assert!(sync.events.iter().any(|e| e.calendar == room.id && e.event.summary == "Room 2 booked"));
    // A server without public folders lists the mailbox's alone.
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    assert_eq!(c.mailboxes().unwrap().len(), 1);
}

/// Issue #1: a shared mailbox's calendar never showed (only calendars opened by hand were
/// read), and every account's own calendar was "Calendar" whatever the server called it.
/// The shared mailbox's calendar is read with its mail, named after it, editable as its
/// rights say; one without a calendar for the account is passed over quietly; the account's
/// own calendar and contacts keep the server's names.
#[test]
fn a_shared_mailboxs_calendar_is_read_with_it() {
    let mut d = Data::default();
    d.folder("Postvak IN", None, Some("inbox"), "IPF.Note");
    d.folder("Agenda", None, Some("calendar"), "IPF.Appointment");
    d.folder("Contactpersonen", None, Some("contacts"), "IPF.Contact");
    d.shared_mailbox(SharedBox { address: "sales@contoso.example".into(), name: "Sales".into(), automap: true, legacy: false, granted: true });
    d.shared_mailbox(SharedBox { address: "boss@contoso.example".into(), name: "Boss".into(), automap: true, legacy: false, granted: false });
    let cal = d.id("S");
    d.folders.push(Fo {
        id: cal.clone(),
        name: "Calendar".into(),
        parent: String::new(),
        class: "IPF.Appointment".into(),
        known: Some("calendar".into()),
        owner: "sales@contoso.example".into(),
    });
    let plan = parse(
        "<t:CalendarItem><t:Subject>Team planning</t:Subject><t:Start>2026-10-20T09:00:00Z</t:Start><t:End>2026-10-20T10:00:00Z</t:End>             <t:IsAllDayEvent>false</t:IsAllDayEvent><t:UID>team-plan-1</t:UID></t:CalendarItem>",
    );
    d.pim.push(("T1".into(), cal, plan.kids[0].clone()));
    // A Teams meeting in the account's own calendar, its join link longer than a text line.
    let agenda = d.folders.iter().find(|f| f.name == "Agenda").map(|f| f.id.clone()).unwrap();
    let long = format!("https://teams.microsoft.com/l/meetup-join/19%3ameeting_{}%40thread.v2/0?context=%7b%22Tid%22%7d", "Nj".repeat(300));
    let teams = parse(&format!(
        "<t:CalendarItem><t:Subject>Sync</t:Subject><t:Body BodyType=\"HTML\">&lt;p&gt;Hi&lt;/p&gt;&lt;a href=\"{}\"&gt;Join the meeting now&lt;/a&gt;</t:Body>             <t:Start>2026-10-20T13:00:00Z</t:Start><t:End>2026-10-20T13:30:00Z</t:End><t:IsAllDayEvent>false</t:IsAllDayEvent><t:UID>sync-1</t:UID></t:CalendarItem>",
        long.replace('&', "&amp;amp;")
    ));
    d.pim.push(("M1".into(), agenda, teams.kids[0].clone()));
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    crate::ews::list_folders(&c, &mut crate::jmap::State::default(), "a1").unwrap();
    let out = crate::ews_pim::sync(&c, &mailcraft_model::pim::Pim::default(), 1_792_000_000).unwrap();
    let cals = out.calendars.clone().unwrap();
    let names: Vec<(&str, &str)> = cals.iter().map(|c| (c.id.as_str(), c.name.as_str())).collect();
    assert_eq!(names, [(crate::ews_pim::CALENDAR, "Agenda"), ("shared:sales@contoso.example", "Sales")], "{names:?}");
    assert!(out.events.iter().any(|e| e.calendar == "shared:sales@contoso.example" && e.event.summary == "Team planning"));
    assert!(out.errors.is_empty(), "the boss's calendar is passed over quietly: {:?}", out.errors);
    assert_eq!(out.books.as_ref().unwrap()[0].name, "Contactpersonen");
    let sync = out.events.iter().find(|e| e.event.summary == "Sync").unwrap();
    assert_eq!(sync.event.join_link().as_deref(), Some(long.as_str()), "the whole join link");
}

/// Shared mailboxes (issue #1): those the server maps to the account open by themselves, one
/// named by its Exchange address only is looked up, one added by hand opens too; each under
/// a folder of its own, routed to it, its Outbox left out, its Inbox not the account's. One
/// without access, and the account's online archive, don't.
#[test]
fn shared_mailboxes_open_by_themselves_or_by_hand() {
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.folder("Sales", None, None, "IPF.Note");
    d.deliver(&inbox, &mail("Own", "o1"), false, "2026-10-19T08:00:00Z");
    let box_ = |address: &str, name: &str, automap: bool, legacy: bool, granted: bool| SharedBox {
        address: address.into(),
        name: name.into(),
        automap,
        legacy,
        granted,
    };
    let sales = d.shared_mailbox(box_("sales@contoso.example", "Sales", true, false, true));
    let quotes = d.shared_folder("sales@contoso.example", "Quotes", &sales);
    d.deliver(&sales, &mail("Order 17", "s1"), false, "2026-10-19T09:00:00Z");
    d.deliver(&quotes, &mail("Quote 4", "s2"), true, "2026-10-19T09:30:00Z");
    d.shared_mailbox(box_("support@contoso.example", "Support", true, true, true));
    d.shared_mailbox(box_("boss@contoso.example", "Boss", true, false, false));
    let team = d.shared_mailbox(box_("team@contoso.example", "Team", false, false, true));
    d.deliver(&team, &mail("Standup", "t1"), false, "2026-10-19T10:00:00Z");
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    c.set_shared(&["team@contoso.example".to_string(), "riley@contoso.example".to_string()]);
    let mut st = crate::jmap::State::default();
    let folders = crate::ews::list_folders(&c, &mut st, "a1").unwrap();
    let paths: Vec<&str> = folders.iter().map(|f| f.path.as_str()).collect();
    // Sales collides with the account's own folder: named with its address.
    let sales_top = "Sales (sales@contoso.example)";
    for want in ["INBOX", "Sales", sales_top, &format!("{sales_top}/Inbox"), &format!("{sales_top}/Inbox/Quotes"), "Support/Inbox", "Team/Inbox"] {
        assert!(paths.contains(&want), "{want} in {paths:?}");
    }
    assert!(!paths.iter().any(|p| p.contains("Boss") || p.contains("Archive") || p.ends_with("Outbox")), "{paths:?}");
    assert!(!paths.iter().any(|p| p.contains("riley@")), "the account's own mailbox isn't a shared one");
    assert_eq!(folders.iter().filter(|f| f.role == mailcraft_model::FolderRole::Inbox).count(), 1, "only the account's own Inbox is its Inbox");
    // Each shared mailbox is a mailbox of its own (issue #1: it showed as one of the
    // account's folders): its folders say whose they are, and what each is there.
    let folder = |path: &str| folders.iter().find(|f| f.path == path).unwrap();
    let top = folder(sales_top);
    assert_eq!((top.mailbox.as_deref(), top.depth()), (Some("sales@contoso.example"), 0));
    let shared_inbox = folder(&format!("{sales_top}/Inbox"));
    assert_eq!(shared_inbox.mailbox.as_deref(), Some("sales@contoso.example"));
    assert_eq!((shared_inbox.role, shared_inbox.mailbox_role), (mailcraft_model::FolderRole::Other, mailcraft_model::FolderRole::Inbox));
    assert!(shared_inbox.is_fixed() && top.is_fixed(), "a shared mailbox's top folder and Inbox can't be renamed or deleted");
    let quotes = folder(&format!("{sales_top}/Inbox/Quotes"));
    assert_eq!((quotes.mailbox.as_deref(), quotes.mailbox_role), (Some("sales@contoso.example"), mailcraft_model::FolderRole::Other));
    assert!(!quotes.is_fixed());
    assert_eq!(folder("Team/Inbox").mailbox.as_deref(), Some("team@contoso.example"));
    assert!(folder("INBOX").mailbox.is_none() && folder("Sales").mailbox.is_none(), "the account's own folders are its own");
    // Shown after the account's own folders, each mailbox together, its Inbox first.
    let mut sorted = folders.clone();
    mailcraft_model::sort_folders(&mut sorted);
    let order: Vec<&str> = sorted.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(
        order,
        [
            "INBOX",
            "Sales",
            sales_top,
            &format!("{sales_top}/Inbox"),
            &format!("{sales_top}/Inbox/Quotes"),
            "Support",
            "Support/Inbox",
            "Team",
            "Team/Inbox"
        ],
    );
    // Their mail syncs as any folder's.
    let store = crate::fake::tests_store("shared-mailboxes");
    let r = crate::ews::sync_folder(&c, &mut st, &store, "a1", &format!("{sales_top}/Inbox"), &crate::sync::SyncOptions::default()).unwrap();
    assert_eq!(r.new, 1);
    assert_eq!(store.load_folder("a1", &format!("{sales_top}/Inbox")).unwrap().messages[0].subject, "Order 17");
    crate::ews::sync_folder(&c, &mut st, &store, "a1", "Team/Inbox", &crate::sync::SyncOptions::default()).unwrap();
    assert_eq!(store.load_folder("a1", "Team/Inbox").unwrap().messages[0].subject, "Standup");
    let anchors = server.data.lock().unwrap().anchors.clone();
    assert!(anchors.iter().any(|a| a == "sales@contoso.example") && anchors.iter().any(|a| a == "team@contoso.example"), "{anchors:?}");
    let _ = std::fs::remove_dir_all(store.root());
}
