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
    d.pim.push(("E1".into(), cal.clone(), review.kids[0].clone()));
    let ana = parse(
        "<t:Contact><t:DisplayName>Ana Silva</t:DisplayName><t:EmailAddresses><t:Entry Key=\"EmailAddress1\">ana@contoso.example</t:Entry>\
         </t:EmailAddresses></t:Contact>",
    );
    d.pim.push(("K1".into(), contacts, ana.kids[0].clone()));
    (d, cal, "E1".into())
}

fn account(url: &str) -> AccountConfig {
    AccountConfig {
        id: "a1".into(),
        email: "riley@contoso.example".into(),
        username: format!("{DOMAIN}\\{USER}"),
        ews: Some(url.into()),
        ..Default::default()
    }
}

fn store(name: &str) -> Store {
    let dir = std::env::temp_dir().join(format!("mailcraft-ews-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    Store::open(&dir).unwrap()
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
    pool.submit(Job::Start { account: account(&server.url), password: Some(Secret(PASSWORD.into())) });
    let said = order(&pool);
    let at = |what: &str| said.iter().position(|s| s == what).unwrap_or_else(|| panic!("{what} isn't in {said:?}"));
    assert_eq!(at("INBOX"), 0, "{said:?}");
    assert_eq!(at("calendar"), 1, "the calendar waited: {said:?}");
    assert!(at("Alpha") > 1 && at("Gamma") > 1, "{said:?}");
    drop(pool);

    // Read before (the engine keeps when): after the mail, as ever.
    store.save_account_doc("a1", PIM_DOC, &Pim { dav_checked: Some(1_792_000_000), ..Default::default() }).unwrap();
    let mut pool = crate::worker::Pool::new(store.clone(), Arc::new(crate::worker::MemoryCredentials::default()));
    pool.submit(Job::Start { account: account(&server.url), password: Some(Secret(PASSWORD.into())) });
    let said = order(&pool);
    assert_eq!(said.last().map(String::as_str), Some("calendar"), "{said:?}");
    assert_eq!(said.len(), 5, "{said:?}");
    drop(pool);
    let _ = std::fs::remove_dir_all(store.root());
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
    pool.submit(Job::Start { account: account(&server.url), password: Some(Secret(PASSWORD.into())) });
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
    let _ = std::fs::remove_dir_all(store.root());
}

/// Appointments the server fails on are counted and said with why (they were left out without a
/// word); the others come. A contact isn't asked for an appointment's recurrence, which Exchange
/// refuses for each contact: they'd all be left out.
#[test]
fn appointments_the_server_fails_on_are_said() {
    let (mut d, cal, first) = mailbox();
    let more = parse("<t:CalendarItem><t:Subject>Plan</t:Subject><t:Start>2026-10-06T09:00:00Z</t:Start><t:UID>plan-1</t:UID></t:CalendarItem>");
    d.pim.push(("E2".into(), cal, more.kids[0].clone()));
    d.unreadable.push(first);
    let server = serve(d);
    let c = crate::ews::Client::connect(&server.url, &format!("{DOMAIN}\\{USER}"), PASSWORD, "riley@contoso.example").unwrap();
    let sync = crate::ews_pim::sync(&c, &Pim::default(), 1_792_000_000).unwrap();
    assert_eq!(sync.events.iter().map(|e| e.event.summary.as_str()).collect::<Vec<_>>(), ["Plan"]);
    assert_eq!(sync.errors, ["Calendar: 1 items couldn't be read (ErrorInternalServerError)"]);
    assert_eq!(sync.cards.iter().map(|k| k.card.display()).collect::<Vec<_>>(), ["Ana Silva"]);
}
