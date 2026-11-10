//! Issue #61: an account with calendars and contacts only, at a CalDAV and CardDAV server (a
//! Nextcloud, a Radicale), and no mail server. Its mail side is a local account's; what concerns
//! its calendars and contacts goes to its server.

use super::*;

const SERVER: &str = "https://cloud.example.org/remote.php/dav";

fn add(s: &mut Session) -> String {
    let added = s.run("account.add", &json!({"email": "zoe@example.org", "username": "zoe", "password": "app-pw", "dav": SERVER})).unwrap();
    added["account"].as_str().unwrap().to_string()
}

#[test]
fn a_calendar_only_account_starts_with_its_server_and_no_mail_one() {
    let mut s = Session::open(Some(temp("calendar-only-add")), mailcraft_model::dates::Clock::new(NOW, 0));
    let id = add(&mut s);
    let c = s.account(&id).unwrap().config.clone();
    assert!(c.is_calendar_only() && c.is_local() && c.has_server());
    assert_eq!((c.username.as_str(), c.imap.host.as_str(), c.smtp.host.as_str()), ("zoe", "", ""));
    let dav = c.dav.clone().unwrap();
    assert_eq!((dav.caldav.as_str(), dav.carddav.as_str(), dav.username), (SERVER, SERVER, None), "one address for both, its own sign-in");
    // Connected with its password; what it brings is shown.
    assert!(matches!(s.take_jobs().as_slice(), [Job::Start { password: Some(p), .. }] if p.0 == "app-pw"));
    assert_eq!(s.module, crate::Module::Calendar);
    // No mail goes from it, and it isn't the sample mailbox.
    assert_eq!(s.send_block(&id), Some(crate::session::SendBlock::NoServer));
    assert!(!s.account(&id).unwrap().is_data_file());
    // Opened again, it connects again.
    let store = s.store.clone();
    let mut again = Session::open(store, mailcraft_model::dates::Clock::new(NOW, 0));
    assert!(again.take_jobs().iter().any(|j| matches!(j, Job::Start { account, .. } if account.id == id)));
    // Send / Receive and working offline take it in; a plain local account isn't.
    let local = again.run("account.add", &json!({"email": "me@example.org", "local": true})).unwrap()["account"].as_str().unwrap().to_string();
    again.take_jobs();
    assert_eq!(again.sync_all(), 1);
    assert!(matches!(again.take_jobs().as_slice(), [Job::SyncAll { account }] if *account == id));
    again.set_work_offline(true);
    assert_eq!(again.account(&id).unwrap().status, mailcraft_model::SyncStatus::Offline);
    assert!(again.account(&local).is_some());
    done(again);
}

#[test]
fn what_it_needs_is_asked_for() {
    let mut s = Session::open(Some(temp("calendar-only-asks")), mailcraft_model::dates::Clock::new(NOW, 0));
    let add = |s: &mut Session, v: serde_json::Value| s.run("account.add", &v).err().map(|e| e.to_string());
    assert!(add(&mut s, json!({"email": "zoe@example.org", "dav": SERVER})).is_some_and(|e| e.contains("password")));
    assert!(add(&mut s, json!({"dav": SERVER, "password": "pw"})).is_some_and(|e| e.contains("email")));
    assert!(add(&mut s, json!({"email": "zoe@example.org", "password": "pw", "dav": "   "})).is_some_and(|e| e.contains("required")));
    // The password never crosses the network unprotected (this computer's own server aside).
    assert!(
        add(&mut s, json!({"email": "zoe@example.org", "password": "pw", "dav": "http://cloud.example.org/dav"}))
            .is_some_and(|e| e.contains("https"))
    );
    assert!(add(&mut s, json!({"email": "zoe@example.org", "password": "pw", "dav": "http://localhost:8080/dav"})).is_none());
    assert!(
        add(&mut s, json!({"email": "zo@example.org", "username": "z\no","password": "pw", "dav": SERVER})).is_some_and(|e| e.contains("user name"))
    );
    assert!(add(&mut s, json!({"email": "zoe@example.org", "password": "pw", "dav": SERVER})).is_some_and(|e| e.contains("already")));
    done(s);
}

#[test]
fn its_calendar_and_contacts_go_to_its_server() {
    let mut s = Session::open(Some(temp("calendar-only-changes")), mailcraft_model::dates::Clock::new(NOW, 0));
    let id = add(&mut s);
    s.take_jobs();
    // Waiting for its calendars, as an invitation's arrival does for any account's.
    assert!(s.calendars_pending(&id));
    let cal = "https://cloud.example.org/remote.php/dav/calendars/zoe/personal/".to_string();
    let book = "https://cloud.example.org/remote.php/dav/addressbooks/users/zoe/contacts/".to_string();
    let sync = mailcraft_model::pim::DavSync {
        caldav_home: Some("https://cloud.example.org/remote.php/dav/calendars/zoe/".into()),
        calendars: Some(vec![mailcraft_model::pim::CalendarInfo {
            id: cal.clone(),
            name: "Personal".into(),
            ctag: "1".into(),
            ..Default::default()
        }]),
        carddav_home: Some("https://cloud.example.org/remote.php/dav/addressbooks/users/zoe/".into()),
        books: Some(vec![mailcraft_model::pim::BookInfo { id: book.clone(), name: "Contacts".into(), ..Default::default() }]),
        checked: NOW,
        ..Default::default()
    };
    s.apply(Event::DavSynced { account: id.clone(), sync: Box::new(sync) });
    assert!(!s.calendars_pending(&id));
    s.module = crate::Module::Calendar;
    s.run("event.create", &json!({"summary": "Dentist", "start": "2026-10-20T10:00", "end": "2026-10-20T11:00", "account": id})).unwrap();
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::SyncDav { account } if *account == id)), "the server hears of a new event");
    assert_eq!(s.pim[&id].events.iter().map(|e| e.calendar.as_str()).collect::<Vec<_>>(), [cal.as_str()]);
    s.run("contact.create", &json!({"name": "Ana Lee", "account": id})).unwrap();
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::SyncDav { account } if *account == id)), "nor of a new contact");
    assert_eq!(s.pim[&id].cards.iter().map(|c| c.book.as_str()).collect::<Vec<_>>(), [book.as_str()]);
    done(s);
}

#[test]
fn its_server_and_sign_in_can_change_but_not_go() {
    let mut s = Session::open(Some(temp("calendar-only-settings")), mailcraft_model::dates::Clock::new(NOW, 0));
    let id = add(&mut s);
    s.take_jobs();
    // Its server's address: changed through Calendars and contacts, never emptied.
    assert!(s.run("account.dav", &json!({"account": id, "caldav": "", "carddav": ""})).is_err());
    s.run("account.dav", &json!({"account": id, "caldav": "https://dav.example.org/", "carddav": ""})).unwrap();
    assert!(s.account(&id).unwrap().config.is_calendar_only());
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::Start { .. })), "connects to the new server");
    // A new password, and another user name: the other user's calendars move out of the way.
    s.run("account.setPassword", &json!({"account": id, "password": "new-pw"})).unwrap();
    assert!(matches!(s.take_jobs().as_slice(), [Job::Start { password: Some(p), .. }] if p.0 == "new-pw"));
    s.pim_mut(&id).caldav_home = Some("https://dav.example.org/zoe/".into());
    s.run("account.update", &json!({"account": id, "name": "Zoe", "username": "zoe2"})).unwrap();
    let c = &s.account(&id).unwrap().config;
    assert_eq!((c.name.as_str(), c.username.as_str()), ("Zoe", "zoe2"));
    assert_eq!(s.pim[&id].caldav_home, None, "looked for again as the new user");
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::Start { .. })));
    // It has no mail server to set, nor to keep mail online on, nor automatic replies.
    assert!(s.run("account.update", &json!({"account": id, "onlineOnly": true})).is_err());
    assert!(s.run("account.update", &json!({"account": id, "smtpUsername": "x", "smtpPassword": "y"})).is_err());
    assert_eq!(s.account(&id).unwrap().config.imap.host, "");
    s.run("account.vacation", &json!({"account": id})).unwrap();
    assert!(!s.take_jobs().iter().any(|j| matches!(j, Job::GetVacation { .. })));
    // Removed, the keychain's password goes with it (the pool's `Remove`).
    s.run("account.remove", &json!({"account": id})).unwrap();
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::Remove { account } if *account == id)));
    done(s);
}

/// Meetings with no mail to send their invitations with: a server that schedules (Nextcloud)
/// sends them, and the answers; one that doesn't (Radicale) can't, and the user is told so.
#[test]
fn a_meetings_invitations_go_through_a_server_that_schedules() {
    for schedules in [true, false] {
        let mut s = Session::open(Some(temp(&format!("calendar-only-meetings-{schedules}"))), mailcraft_model::dates::Clock::new(NOW, 0));
        let id = add(&mut s);
        let cal = "https://cloud.example.org/remote.php/dav/calendars/zoe/personal/".to_string();
        let sync = mailcraft_model::pim::DavSync {
            caldav_home: Some("https://cloud.example.org/remote.php/dav/calendars/zoe/".into()),
            calendars: Some(vec![mailcraft_model::pim::CalendarInfo { id: cal.clone(), name: "Personal".into(), ..Default::default() }]),
            auto_schedule: Some(schedules),
            checked: NOW,
            ..Default::default()
        };
        s.apply(Event::DavSynced { account: id.clone(), sync: Box::new(sync) });
        s.take_jobs();
        s.notices.clear();
        let made = s.run(
            "event.create",
            &json!({"summary": "Planning", "start": "2026-10-20T10:00", "attendees": "Ana Lee <ana@example.org>", "account": id}),
        );
        let uid = made.unwrap()["uid"].as_str().unwrap().to_string();
        let jobs = s.take_jobs();
        assert!(!jobs.iter().any(|j| matches!(j, Job::Send { .. })), "no mail goes from it");
        assert!(jobs.iter().any(|j| matches!(j, Job::SyncDav { .. })), "the server has the meeting");
        let not_sent = s.notices.iter().any(|n| n.text.contains("not sent"));
        if schedules {
            assert_eq!(s.meeting_route(&id, &uid), crate::Route::Server);
            assert!(!not_sent && s.pim[&id].notices.is_empty(), "the server sends them: {:?}", s.notices);
        } else {
            assert!(not_sent, "said it couldn't go: {:?}", s.notices);
        }
        done(s);
    }
}
