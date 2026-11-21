//! The fake EWS server's tests of when an account's calendar is read, and of what is said when
//! the server won't give it (issue #80: the calendar stayed empty, and nothing said why).

use mailcraft_model::pim::{PIM_DOC, Pim};

use super::*;

/// A mailbox with an Inbox, three more folders of mail, a calendar with one appointment and a
/// contact; the calendar's and the appointment's ids.
fn mailbox() -> (Data, String, String) {
    let mut d = Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    d.deliver(&inbox, &mail("Storyboard", "m1"), true, "2024-05-06T09:00:00Z");
    for name in ["Alpha", "Beta", "Gamma"] {
        let f = d.folder(name, None, None, "IPF.Note");
        d.deliver(&f, &mail(name, name), true, "2024-05-01T10:00:00Z");
    }
    let cal = d.folder("Calendar", None, Some("calendar"), "IPF.Appointment");
    let contacts = d.folder("Contacts", None, Some("contacts"), "IPF.Contact");
    let review =
        parse("<t:CalendarItem><t:Subject>Review</t:Subject><t:Start>2026-10-05T09:00:00Z</t:Start><t:UID>review-1</t:UID></t:CalendarItem>");
    d.pim.push(PimItem::new("E1", cal.clone(), review.kids[0].clone()));
    let ana = parse(
        "<t:Contact><t:DisplayName>Ana Silva</t:DisplayName><t:EmailAddresses><t:Entry Key=\"EmailAddress1\">ana@contoso.example</t:Entry>\
         </t:EmailAddresses></t:Contact>",
    );
    d.pim.push(PimItem::new("K1", contacts, ana.kids[0].clone()));
    (d, cal, "E1".into())
}

fn store(name: &str) -> Store {
    Store::open(crate::data_files::scratch::scratch(&format!("ews-{name}"))).unwrap()
}

/// What was said until the account was online and its calendar read: the folders in the order
/// their sync began, with `calendar` where the calendar and contacts came.
fn order(pool: &crate::worker::Pool) -> Vec<String> {
    let (mut online, mut read) = (false, false);
    let seen = until(pool, "online with its calendar", |e| {
        online |= matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. });
        read |= matches!(e, Event::DavSynced { .. });
        online && read
    });
    let mut said: Vec<String> = Vec::new();
    for e in seen {
        let what = match e {
            Event::Status { status: SyncStatus::Syncing { folder }, .. } => folder,
            Event::DavSynced { .. } => "calendar".into(),
            _ => continue,
        };
        if !said.contains(&what) {
            said.push(what);
        }
    }
    said
}

/// An account's calendar and contacts are read once its Inbox is in, the first time: after
/// every folder, a big mailbox's calendar stayed empty for as long as its mail took. Read
/// before, they wait for the mail as they did.
#[test]
fn the_calendar_comes_with_the_inbox_the_first_time() {
    let (d, ..) = mailbox();
    let server = serve(d);
    let store = store("calendar-first");
    let mut pool = crate::worker::Pool::new(store.clone(), Arc::new(crate::worker::MemoryCredentials::default()));
    pool.submit(Job::Start { account: account("a1", &server.url), password: Some(Secret(PASSWORD.into())) });
    let said = order(&pool);
    let at = |what: &str| said.iter().position(|s| s == what).unwrap_or_else(|| panic!("{what} isn't in {said:?}"));
    assert_eq!(at("INBOX"), 0, "{said:?}");
    assert_eq!(at("calendar"), 1, "the calendar waited: {said:?}");
    assert!(at("Alpha") > 1 && at("Gamma") > 1, "{said:?}");
    drop(pool);

    // Read before (the engine keeps when): after the mail, as ever.
    store.save_account_doc("a1", PIM_DOC, &Pim { dav_checked: Some(1_792_000_000), ..Default::default() }).unwrap();
    let mut pool = crate::worker::Pool::new(store.clone(), Arc::new(crate::worker::MemoryCredentials::default()));
    pool.submit(Job::Start { account: account("a1", &server.url), password: Some(Secret(PASSWORD.into())) });
    let said = order(&pool);
    assert_eq!(said.last().map(String::as_str), Some("calendar"), "{said:?}");
    assert_eq!(said.len(), 5, "{said:?}");
    drop(pool);
}

/// A calendar the server won't list is said, with why, and the contacts still come; said once,
/// not at every sync.
#[test]
fn a_calendar_the_server_refuses_says_so_once() {
    let (mut d, cal, _) = mailbox();
    d.unreadable.push(cal);
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let sync = crate::ews_pim::sync(&c, &Pim::default(), 1_792_000_000).unwrap();
    assert!(sync.events.is_empty());
    assert_eq!(sync.errors, ["Calendar: ErrorAccessDenied"]);
    assert_eq!(sync.cards.len(), 1, "the contacts are read all the same");

    let store = store("calendar-refused");
    let mut pool = crate::worker::Pool::new(store.clone(), Arc::new(crate::worker::MemoryCredentials::default()));
    pool.submit(Job::Start { account: account("a1", &server.url), password: Some(Secret(PASSWORD.into())) });
    let errors = |pool: &crate::worker::Pool| {
        let seen = until(pool, "the calendar's answer", |e| matches!(e, Event::DavSynced { .. }));
        match seen.into_iter().next_back() {
            Some(Event::DavSynced { sync, .. }) => sync.errors,
            other => panic!("{other:?}"),
        }
    };
    assert_eq!(errors(&pool), ["Calendar: ErrorAccessDenied"]);
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    pool.submit(Job::SyncDav { account: "a1".into() });
    assert!(errors(&pool).is_empty(), "said again");
    // Given again, then refused again: said again.
    let cal = std::mem::take(&mut server.data.lock().unwrap().unreadable);
    pool.submit(Job::SyncDav { account: "a1".into() });
    assert!(errors(&pool).is_empty());
    server.data.lock().unwrap().unreadable = cal;
    pool.submit(Job::SyncDav { account: "a1".into() });
    assert_eq!(errors(&pool), ["Calendar: ErrorAccessDenied"]);
    drop(pool);
}

/// Appointments the server fails on are counted and said with why (they were left out without a
/// word); the others come. A contact isn't asked for an appointment's recurrence, which Exchange
/// refuses for each contact: they'd all be left out.
#[test]
fn appointments_the_server_fails_on_are_said() {
    let (mut d, cal, first) = mailbox();
    let more = parse("<t:CalendarItem><t:Subject>Plan</t:Subject><t:Start>2026-10-06T09:00:00Z</t:Start><t:UID>plan-1</t:UID></t:CalendarItem>");
    d.pim.push(PimItem::new("E2", cal, more.kids[0].clone()));
    d.unreadable.push(first);
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let sync = crate::ews_pim::sync(&c, &Pim::default(), 1_792_000_000).unwrap();
    assert_eq!(sync.events.iter().map(|e| e.event.summary.as_str()).collect::<Vec<_>>(), ["Plan"]);
    assert_eq!(sync.errors, ["Calendar: 1 items couldn't be read (ErrorInternalServerError)"]);
    assert_eq!(sync.cards.iter().map(|k| k.card.display()).collect::<Vec<_>>(), ["Ana Silva"]);
}

/// Issue #204: a series' changed occurrence, which the listing doesn't give, is read by the id
/// the series names and comes as the occurrence it stands for; one taken out is the series'
/// exception. One gone meanwhile is left to its series, and the calendar is read all the same.
#[test]
fn a_series_changed_occurrences_are_read() {
    let (mut d, cal, _) = mailbox();
    let series = parse(
        "<t:CalendarItem><t:Subject>Weekly review</t:Subject><t:UID>weekly-1</t:UID>\
         <t:Start>2026-10-05T09:00:00Z</t:Start><t:End>2026-10-05T10:00:00Z</t:End>\
         <t:CalendarItemType>RecurringMaster</t:CalendarItemType>\
         <t:Recurrence><t:WeeklyRecurrence><t:Interval>1</t:Interval><t:DaysOfWeek>Monday</t:DaysOfWeek></t:WeeklyRecurrence>\
         <t:NoEndRecurrence><t:StartDate>2026-10-05Z</t:StartDate></t:NoEndRecurrence></t:Recurrence>\
         <t:ModifiedOccurrences>\
         <t:Occurrence><t:ItemId Id=\"X1\"/><t:Start>2026-10-13T14:00:00Z</t:Start><t:End>2026-10-13T15:00:00Z</t:End><t:OriginalStart>2026-10-12T09:00:00Z</t:OriginalStart></t:Occurrence>\
         <t:Occurrence><t:ItemId Id=\"X2\"/><t:Start>2026-11-02T11:00:00Z</t:Start><t:End>2026-11-02T12:00:00Z</t:End><t:OriginalStart>2026-11-02T09:00:00Z</t:OriginalStart></t:Occurrence>\
         </t:ModifiedOccurrences>\
         <t:DeletedOccurrences><t:DeletedOccurrence><t:Start>2026-10-19T09:00:00Z</t:Start></t:DeletedOccurrence></t:DeletedOccurrences>\
         </t:CalendarItem>",
    );
    d.pim.push(PimItem::new("S1", cal.clone(), series.kids[0].clone()));
    let moved = parse(
        "<t:CalendarItem><t:Subject>Weekly review, Tuesday</t:Subject><t:UID>weekly-1</t:UID>\
         <t:Start>2026-10-13T14:00:00Z</t:Start><t:End>2026-10-13T15:00:00Z</t:End>\
         <t:CalendarItemType>Exception</t:CalendarItemType></t:CalendarItem>",
    );
    d.pim.push(PimItem::new("X1", cal, moved.kids[0].clone()));
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let sync = crate::ews_pim::sync(&c, &Pim::default(), 1_792_000_000).unwrap();
    assert!(sync.errors.is_empty(), "{:?}", sync.errors);
    let t = |s: &str| mailcraft_jmap::utc_date(s).unwrap();
    let mut weekly: Vec<_> = sync.events.iter().filter(|e| e.event.uid == "weekly-1").map(|e| &e.event).collect();
    weekly.sort_by_key(|e| e.recurrence_id);
    let [series, moved] = weekly[..] else { panic!("{weekly:?}") };
    assert_eq!((series.recurrence_id, series.exdates.as_slice()), (None, [t("2026-10-19T09:00:00Z")].as_slice()));
    assert_eq!(
        (moved.recurrence_id, moved.start, moved.summary.as_str()),
        (Some(t("2026-10-12T09:00:00Z")), t("2026-10-13T14:00:00Z"), "Weekly review, Tuesday")
    );
    assert!(moved.rrule.is_none());
    assert!(sync.events.iter().any(|e| e.event.summary == "Review"), "the rest of the calendar");
}

/// Issue #229: a series whose every occurrence was deleted, which Exchange won't give
/// (`ErrorRecurrenceHasNoOccurrence`), has nothing to show and isn't said to be unreadable: it
/// was counted as an item the server refused, in a red notice at every start.
#[test]
fn a_series_with_no_occurrences_left_is_no_error() {
    let (mut d, cal, _) = mailbox();
    let empty = parse(
        "<t:CalendarItem><t:Subject>Old stand-up</t:Subject><t:UID>old-1</t:UID>\
         <t:Start>2026-10-05T09:00:00Z</t:Start><t:End>2026-10-05T09:15:00Z</t:End>\
         <t:CalendarItemType>RecurringMaster</t:CalendarItemType>\
         <t:Recurrence><t:DailyRecurrence><t:Interval>1</t:Interval></t:DailyRecurrence>\
         <t:NumberedRecurrence><t:StartDate>2026-10-05Z</t:StartDate><t:NumberOfOccurrences>2</t:NumberOfOccurrences></t:NumberedRecurrence></t:Recurrence>\
         <t:DeletedOccurrences><t:DeletedOccurrence><t:Start>2026-10-05T09:00:00Z</t:Start></t:DeletedOccurrence>\
         <t:DeletedOccurrence><t:Start>2026-10-06T09:00:00Z</t:Start></t:DeletedOccurrence></t:DeletedOccurrences></t:CalendarItem>",
    );
    d.pim.push(PimItem::new("S9", cal, empty.kids[0].clone()));
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let sync = crate::ews_pim::sync(&c, &Pim::default(), 1_792_000_000).unwrap();
    assert!(sync.errors.is_empty(), "{:?}", sync.errors);
    assert_eq!(sync.events.iter().map(|e| e.event.summary.as_str()).collect::<Vec<_>>(), ["Review"]);
}

/// Issue #144: a series the user accepted, sent to another of the account's addresses than its
/// own, comes accepted: it came still waiting for their answer (and drawn as one), Exchange's
/// answer being put only on the guest with the account's own address.
#[test]
fn an_answer_to_a_meeting_sent_to_another_address_is_the_users() {
    let (mut d, cal, _) = mailbox();
    let series = parse(
        "<t:CalendarItem><t:Subject>Sprint planning</t:Subject><t:UID>sprint-1</t:UID>\
         <t:Start>2026-10-05T13:00:00Z</t:Start><t:End>2026-10-05T14:00:00Z</t:End>\
         <t:CalendarItemType>RecurringMaster</t:CalendarItemType><t:MyResponseType>Accept</t:MyResponseType>\
         <t:Recurrence><t:WeeklyRecurrence><t:Interval>1</t:Interval><t:DaysOfWeek>Monday</t:DaysOfWeek></t:WeeklyRecurrence>\
         <t:NoEndRecurrence><t:StartDate>2026-10-05Z</t:StartDate></t:NoEndRecurrence></t:Recurrence>\
         <t:RequiredAttendees>\
         <t:Attendee><t:Mailbox><t:EmailAddress>Riley.Lee@contoso.example</t:EmailAddress></t:Mailbox><t:ResponseType>Unknown</t:ResponseType></t:Attendee>\
         <t:Attendee><t:Mailbox><t:EmailAddress>ana@contoso.example</t:EmailAddress></t:Mailbox><t:ResponseType>Unknown</t:ResponseType></t:Attendee>\
         </t:RequiredAttendees></t:CalendarItem>",
    );
    d.pim.push(PimItem::new("S2", cal, series.kids[0].clone()));
    let server = serve(d);
    let store = store("answer-elsewhere");
    let mut pool = crate::worker::Pool::new(store, Arc::new(crate::worker::MemoryCredentials::default()));
    let mut account = account("a1", &server.url);
    account.identities.push(mailcraft_model::Identity { email: "riley.lee@contoso.example".into(), ..Default::default() });
    pool.submit(Job::Start { account, password: Some(Secret(PASSWORD.into())) });
    let seen = until(&pool, "the calendar", |e| matches!(e, Event::DavSynced { .. }));
    let Some(Event::DavSynced { sync, .. }) = seen.into_iter().next_back() else { panic!("no calendar") };
    let sprint = sync.events.iter().find(|e| e.event.uid == "sprint-1").expect("the series");
    let answers: Vec<(&str, &str)> = sprint.event.attendees.iter().map(|a| (a.email.as_str(), a.partstat.as_str())).collect();
    assert_eq!(answers, [("Riley.Lee@contoso.example", "ACCEPTED"), ("ana@contoso.example", "NEEDS-ACTION")]);
}
