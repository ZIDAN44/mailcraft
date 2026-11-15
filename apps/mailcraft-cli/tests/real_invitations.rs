//! Meeting invitations through a real mail server and a real calendar server on this computer
//! (issues #30 and #31; never a real account), run by hand:
//!
//! ```text
//! MAILCRAFT_REAL_INVITES="127.0.0.1:21510 http://127.0.0.1:21532/riley@studio.example/personal/" \
//! MAILCRAFT_REAL_INVITES_JMAP="http://127.0.0.1:21508 riley pw" \
//! MAILCRAFT_REAL_INVITES_TOGETHER="127.0.0.1:21510 http://127.0.0.1:21532/riley@studio.example/personal/" \
//!   cargo test -p mailcraft-cli --test real_invitations -- --ignored --nocapture
//! ```
//!
//! An IMAP server (Dovecot: any user, password `pw`, with a folder `Trash`) and a CalDAV server
//! (Radicale, any password, with a calendar `personal` made for the user), reached through a
//! proxy here that refuses connections, then answers slowly; the calendar is read straight from
//! the server to check what was stored. Then Stalwart, mail over JMAP and its CalDAV calendar.
//! The engine and the network layer run together, as in the app.

#[path = "support/servers.rs"]
mod servers;

use std::time::{Duration, Instant};

use mailcraft_engine::model::pim::LOCAL;
use mailcraft_engine::model::{Event, Job, SyncStatus};
use serde_json::json;
use servers::{App, Imap, basic, http, http_as, proxy};

const USER: &str = "riley@studio.example";

/// An invitation from Zoe to the user, as a calendar service would send it.
fn invitation(uid: &str, summary: &str, start: &str) -> Vec<u8> {
    format!(
        "From: Zoe Park <zoe@organizer.example>\r\nTo: Riley <{USER}>\r\nSubject: Invitation: {summary}\r\nMessage-ID: <{uid}.mail@organizer.example>\r\n\
         Date: Sat, 31 Oct 2026 09:00:00 +0000\r\nMIME-Version: 1.0\r\nContent-Type: multipart/alternative; boundary=\"b\"\r\n\r\n\
         --b\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{summary}\r\n--b\r\nContent-Type: text/calendar; charset=utf-8; method=REQUEST\r\n\r\n\
         BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//organizer.example//EN\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nUID:{uid}\r\nDTSTAMP:20261031T090000Z\r\n\
         DTSTART:{start}\r\nDURATION:PT1H\r\nSUMMARY:{summary}\r\nORGANIZER;CN=Zoe Park:mailto:zoe@organizer.example\r\n\
         ATTENDEE;PARTSTAT=NEEDS-ACTION;RSVP=TRUE:mailto:{USER}\r\nSEQUENCE:0\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n--b--\r\n"
    )
    .into_bytes()
}

/// The calendar's items, read straight from the server (no proxy).
fn stored(calendar: &str) -> String {
    let body = "<?xml version=\"1.0\"?><c:calendar-query xmlns:d=\"DAV:\" xmlns:c=\"urn:ietf:params:xml:ns:caldav\"><d:prop><c:calendar-data/></d:prop><c:filter><c:comp-filter name=\"VCALENDAR\"/></c:filter></c:calendar-query>";
    let auth = basic(&format!("{USER}:pw"));
    http("REPORT", calendar, &[("Depth", "1"), ("Authorization", &auth), ("Content-Type", "application/xml")], body.as_bytes()).text()
}

/// Each event as it comes; a calendar sync that went wrong stops the test (each item is stored
/// once: "changed on the server meanwhile" said a second sync sent them again).
fn watch(e: &Event) {
    match e {
        Event::Status { status, .. } => eprintln!("status: {status:?}"),
        Event::DavSynced { sync, .. } => {
            eprintln!("calendars: {:?}, {} events stored", sync.calendars.as_ref().map(Vec::len), sync.pushed_events.len());
            assert!(sync.errors.is_empty(), "{:?}", sync.errors);
        }
        Event::FolderChanged { folder, .. } => eprintln!("changed: {folder}"),
        Event::Body { key, .. } => eprintln!("body: {}", key.to_id()),
        _ => {}
    }
}

#[test]
#[ignore = "needs real IMAP and CalDAV servers in MAILCRAFT_REAL_INVITES"]
fn real_invitations_wait_for_the_calendar_and_leave_the_inbox() {
    let Ok(v) = std::env::var("MAILCRAFT_REAL_INVITES") else {
        eprintln!("MAILCRAFT_REAL_INVITES isn't set: skipped");
        return;
    };
    let p: Vec<&str> = v.split_whitespace().collect();
    let [imap, calendar] = p.as_slice() else { panic!("MAILCRAFT_REAL_INVITES: imap calendar") };
    let target = calendar.trim_start_matches("http://").split('/').next().unwrap().to_string();
    let (port, mode) = proxy(&target);
    let set_mode = |m: Option<u64>| *mode.lock().unwrap() = m;
    let home = calendar.trim_end_matches('/').rsplit_once('/').unwrap().0.to_string() + "/";
    let proxied = home.replace(&target, &format!("127.0.0.1:{port}"));
    let mut app = App::open("invites", "pw");
    app.watch = watch;
    let id = app.add_plain(imap, "127.0.0.1:1", json!({"email": USER, "password": "pw"}));
    app.s.run("options.answeredInvites", &json!({"after": "delete"})).unwrap();
    let mut other = Imap::connect(imap, USER, "pw");
    // Deleted Items, as a new mailbox may not have it yet (made once; "already exists" after).
    other.create("Trash");

    // 1. No calendar server found from the address (studio.example has none): the account's
    //    calendar is on this computer, and an invitation that comes goes there, as it should.
    set_mode(None);
    app.pump(120, "calendars looked for", |_, e| matches!(e, Event::DavSynced { .. }));
    assert_eq!(app.s.pim[&id].caldav_home.as_deref(), Some(""), "none found");
    other.append("INBOX", "", &invitation("one@organizer.example", "Colour pass", "20261105T100000Z"));
    app.pump(120, "the first invitation", |s, _| s.find_event("one@organizer.example").is_some());
    assert_eq!(app.s.find_event("one@organizer.example").unwrap().1.calendar, LOCAL);
    eprintln!("1: the first invitation is in the calendar on this computer");

    // 2. The calendar server given by hand (issue #7), but it can't be reached yet: what
    //    arrives now is held, then after a while put on this computer too.
    app.s.run("account.dav", &json!({"account": id, "caldav": proxied})).unwrap();
    app.pump(120, "online again", |_, e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    assert!(app.s.calendars_pending(&id), "still to be found");
    other.append("INBOX", "", &invitation("two@organizer.example", "Grade review", "20261106T140000Z"));
    app.pump(120, "the second invitation held", |s, _| !s.pim[&id].held.is_empty());
    assert!(app.s.find_event("two@organizer.example").is_none(), "held, not on this computer's calendar");
    eprintln!("2: the second invitation is held");
    app.s.clock.now += 181;
    app.s.tick();
    assert_eq!(app.s.find_event("two@organizer.example").unwrap().1.calendar, LOCAL, "waited long enough: here");
    other.append("INBOX", "", &invitation("three@organizer.example", "Mix review", "20261107T150000Z"));
    app.pump(120, "the third invitation held", |s, _| !s.pim[&id].held.is_empty());
    // One declined in its message while held: it doesn't come back once the calendars are found.
    other.append("INBOX", "", &invitation("four@organizer.example", "Bounce", "20261108T150000Z"));
    app.pump(120, "the fourth invitation held", |s, _| s.pim[&id].held.len() == 2);
    let key = app.s.folder_state(&id, "INBOX").messages.iter().find(|m| m.subject == "Invitation: Bounce").map(|m| m.key.to_id()).unwrap();
    app.s.run("folder.open", &json!({"account": id, "folder": "INBOX"})).unwrap();
    app.s.run("list.open", &json!({"key": key})).unwrap();
    let r = app.s.run("invite.respond", &json!({"answer": "declined"})).unwrap();
    assert_eq!(
        r["moved"],
        1,
        "{r} to {:?} of {:?}",
        app.s.answered_folder(&id),
        app.s.account(&id).map(|a| a.folders.iter().map(|f| (f.path.clone(), f.role)).collect::<Vec<_>>())
    );
    assert_eq!(app.s.pim[&id].held.len(), 1, "let go");

    // 3. The server answers, slowly: the held one goes to its calendar, those put here move
    //    there, and both are stored on the server.
    set_mode(Some(3));
    app.pool.submit(Job::SyncDav { account: id.clone() });
    app.pump(180, "the calendars found", |_, e| matches!(e, Event::DavSynced { sync, .. } if sync.calendars.is_some()));
    for uid in ["one@organizer.example", "two@organizer.example", "three@organizer.example"] {
        let (_, e) = app.s.find_event(uid).unwrap();
        assert!(e.calendar.ends_with("/personal/"), "{uid} in {}", e.calendar);
    }
    assert!(app.s.pim[&id].held.is_empty() && app.s.pim[&id].placed.is_empty());
    app.pump(180, "stored on the server", |s, _| s.pim[&id].events.iter().all(|e| !e.dirty && !e.href.is_empty()));
    let ics = stored(calendar);
    for summary in ["Colour pass", "Grade review", "Mix review"] {
        assert!(ics.contains(&format!("SUMMARY:{summary}")), "{summary} not on the server:\n{ics}");
    }
    assert!(!ics.contains("SUMMARY:Bounce") && app.s.find_event("four@organizer.example").is_none(), "declined while held: not back");
    eprintln!("3: all three are in the server's calendar, the one declined while held isn't");

    // 4. Issue #31: answered from the calendar, the invitation leaves the Inbox on the server
    //    (Deleted Items); answered in the message, likewise; the answer is stored.
    app.s.run("event.respond", &json!({"uid": "one@organizer.example", "answer": "accepted"})).unwrap();
    app.pump(120, "the move done", |s, _| s.folder_state(&id, "Trash").messages.iter().any(|m| m.subject == "Invitation: Colour pass"));
    let key = app.s.folder_state(&id, "INBOX").messages.iter().find(|m| m.subject == "Invitation: Grade review").map(|m| m.key.to_id()).unwrap();
    app.s.run("folder.open", &json!({"account": id, "folder": "INBOX"})).unwrap();
    app.s.run("list.open", &json!({"key": key})).unwrap();
    let r = app.s.run("invite.respond", &json!({"answer": "tentative"})).unwrap();
    assert_eq!(r["moved"], 1, "{r}");
    app.pump(120, "the answers stored", |s, _| s.pim[&id].events.iter().all(|e| !e.dirty));
    let end = Instant::now() + Duration::from_secs(30);
    loop {
        let inbox = other.subjects("INBOX");
        let trash = other.subjects("Trash");
        if trash.len() >= 3 {
            assert!(inbox.iter().all(|s| !s.contains("Colour pass") && !s.contains("Grade review")), "{inbox:?}");
            assert!(inbox.iter().any(|s| s.contains("Mix review")), "the unanswered one stays: {inbox:?}");
            eprintln!("4: on the server, Inbox {inbox:?}, Trash {trash:?}");
            break;
        }
        assert!(Instant::now() < end, "not moved on the server: Inbox {inbox:?}, Trash {trash:?}");
        let _ = app.pool.wait(Duration::from_millis(500));
    }
    let ics = stored(calendar);
    assert!(ics.contains("PARTSTAT=ACCEPTED") && ics.contains("PARTSTAT=TENTATIVE"), "{ics}");
}

/// A JMAP server's mail, from the other side: put a message in the Inbox, list a mailbox.
struct Jmap {
    api: String,
    upload: String,
    account: String,
    auth: String,
}

impl Jmap {
    fn connect(base: &str, auth: &str) -> Jmap {
        let session: serde_json::Value =
            serde_json::from_slice(&http_as("GET", &format!("{base}/jmap/session"), auth, "application/json", b"").body).unwrap();
        let account = session["primaryAccounts"]["urn:ietf:params:jmap:mail"].as_str().unwrap().to_string();
        let upload = session["uploadUrl"].as_str().unwrap().replace("{accountId}", &account);
        Jmap { api: session["apiUrl"].as_str().unwrap().to_string(), upload, account, auth: auth.into() }
    }

    fn call(&self, calls: serde_json::Value) -> serde_json::Value {
        let req = json!({"using": ["urn:ietf:params:jmap:core", "urn:ietf:params:jmap:mail"], "methodCalls": calls});
        serde_json::from_slice(&http_as("POST", &self.api, &self.auth, "application/json", req.to_string().as_bytes()).body).unwrap()
    }

    fn mailbox(&self, role: &str) -> String {
        let r = self.call(json!([["Mailbox/get", {"accountId": self.account, "ids": null}, "m"]]));
        r["methodResponses"][0][1]["list"].as_array().unwrap().iter().find(|m| m["role"] == role).unwrap()["id"].as_str().unwrap().to_string()
    }

    fn deliver(&self, raw: &[u8]) {
        let blob: serde_json::Value = serde_json::from_slice(&http_as("POST", &self.upload, &self.auth, "message/rfc822", raw).body).unwrap();
        let inbox = self.mailbox("inbox");
        let r = self.call(
            json!([["Email/import", {"accountId": self.account, "emails": {"a": {"blobId": blob["blobId"], "mailboxIds": {inbox: true}}}}, "i"]]),
        );
        assert!(r["methodResponses"][0][1]["created"]["a"].is_object(), "{r}");
    }

    fn subjects(&self, role: &str) -> Vec<String> {
        let mb = self.mailbox(role);
        let r = self.call(json!([
            ["Email/query", {"accountId": self.account, "filter": {"inMailbox": mb}}, "q"],
            ["Email/get", {"accountId": self.account, "#ids": {"resultOf": "q", "name": "Email/query", "path": "/ids"}, "properties": ["subject"]}, "g"]
        ]));
        r["methodResponses"][1][1]["list"].as_array().unwrap().iter().filter_map(|e| e["subject"].as_str().map(str::to_string)).collect()
    }
}

/// The same through JMAP and a CalDAV calendar on one server (Stalwart:
/// `MAILCRAFT_REAL_INVITES_JMAP="http://127.0.0.1:21508 riley pw"`): the invitation goes to the
/// account's calendar there; answered from the calendar, it's moved to Deleted Items on the
/// server by JMAP.
#[test]
#[ignore = "needs a real JMAP server in MAILCRAFT_REAL_INVITES_JMAP"]
fn real_jmap_answered_invitation_leaves_the_inbox() {
    let Ok(v) = std::env::var("MAILCRAFT_REAL_INVITES_JMAP") else {
        eprintln!("MAILCRAFT_REAL_INVITES_JMAP isn't set: skipped");
        return;
    };
    let p: Vec<&str> = v.split_whitespace().collect();
    let [base, user, password] = p.as_slice() else { panic!("MAILCRAFT_REAL_INVITES_JMAP: base user password") };
    let auth = format!("{user}:{password}");
    let other = Jmap::connect(base, &auth);
    let mut app = App::open("invites-jmap", password);
    app.watch = watch;
    let id = app.add(json!({"email": USER, "username": user, "password": password, "jmap": format!("{base}/jmap/session")}));
    app.s.run("account.dav", &json!({"account": id, "caldav": format!("{base}/dav/cal/"), "username": user, "password": password})).unwrap();
    app.s.run("options.answeredInvites", &json!({"after": "delete"})).unwrap();
    app.pump(120, "calendars found", |_, e| matches!(e, Event::DavSynced { sync, .. } if sync.calendars.is_some()));
    let calendar = app.s.pim[&id].default_calendar().map(|c| c.id.clone()).unwrap();
    eprintln!("calendars: {:?}", app.s.pim[&id].calendars.iter().map(|c| (&c.id, &c.name, c.foreign)).collect::<Vec<_>>());
    let uid = format!("jmap-{}@organizer.example", std::process::id());
    let summary = format!("Sound check {}", std::process::id());
    let subject = format!("Invitation: {summary}");
    other.deliver(&invitation(&uid, &summary, "20261108T100000Z"));
    // Stalwart pushes no change for a message imported (as one delivered would be): looked at as
    // Send/Receive would.
    let inbox = app.s.account(&id).and_then(|a| a.folder(mailcraft_engine::model::FolderRole::Inbox)).map(|f| f.path.clone()).unwrap();
    app.pool.submit(Job::SyncFolder { account: id.clone(), folder: inbox });
    app.pump(120, "the invitation", |s, _| s.find_event(&uid).is_some());
    assert_eq!(app.s.find_event(&uid).unwrap().1.calendar, calendar);
    app.pump(120, "stored on the server", |s, _| s.find_event(&uid).is_some_and(|(_, e)| !e.dirty && !e.href.is_empty()));
    app.s.run("event.respond", &json!({"uid": uid, "answer": "accepted"})).unwrap();
    let end = Instant::now() + Duration::from_secs(60);
    loop {
        for j in app.s.take_jobs() {
            app.pool.submit(j);
        }
        if let Some(e) = app.pool.wait(Duration::from_millis(300)) {
            app.s.apply(e);
        }
        let trash = other.subjects("trash");
        if trash.contains(&subject) {
            assert!(!other.subjects("inbox").contains(&subject));
            eprintln!("moved on the server: Trash {trash:?}");
            break;
        }
        assert!(Instant::now() < end, "not moved: Inbox {:?}", other.subjects("inbox"));
    }
    let (_, e) = app.s.find_event(&uid).unwrap();
    let ics = http_as("GET", &e.href, &auth, "text/plain", b"").text();
    assert!(ics.contains(&format!("SUMMARY:{summary}")) && ics.contains("PARTSTAT=ACCEPTED"), "{ics}");
    eprintln!(
        "stored on the server, accepted:
{ics}"
    );
}

/// An Outlook invitation with an online meeting (`CONFERENCE`), sent tentative but meant busy,
/// with the organizer's alarm: the calendar of v0.31 together (issues #25, #30, #31, #32, #33).
fn outlook_invitation(uid: &str, summary: &str, start: &str) -> Vec<u8> {
    let raw = String::from_utf8(invitation(uid, summary, start)).unwrap();
    raw.replace(
        "SEQUENCE:0\r\n",
        "SEQUENCE:0\r\nCONFERENCE;VALUE=URI;FEATURE=AUDIO,VIDEO;LABEL=Join:https://meet.example.edu/rooms/abc-def-ghi\r\n\
         TRANSP:OPAQUE\r\nX-MICROSOFT-CDO-BUSYSTATUS:TENTATIVE\r\nX-MICROSOFT-CDO-INTENDEDSTATUS:BUSY\r\n\
         BEGIN:VALARM\r\nACTION:DISPLAY\r\nDESCRIPTION:Reminder\r\nTRIGGER:-PT15M\r\nEND:VALARM\r\n",
    )
    .into_bytes()
}

/// The server's copy of an accepted meeting made private and out of office: each of its
/// classification, busy status and meeting once.
fn check_stored(ics: &str, uid: &str) {
    let unfolded = ics.replace("\r\n ", "").replace("\n ", "");
    let item = unfolded.split("BEGIN:VEVENT").find(|v| v.contains(&format!("UID:{uid}"))).unwrap_or_else(|| panic!("{uid} not stored:\n{ics}"));
    for want in
        ["CONFERENCE", "meet.example.edu/rooms/abc-def-ghi", "PARTSTAT=ACCEPTED", "CLASS:PRIVATE", "X-MICROSOFT-CDO-BUSYSTATUS:OOF", "TRANSP:OPAQUE"]
    {
        assert!(item.contains(want), "{want} not stored:\n{item}");
    }
    for once in ["CLASS:", "BUSYSTATUS:", "CONFERENCE", "TRANSP:"] {
        assert_eq!(item.matches(once).count(), 1, "{once}: {item}");
    }
}

/// Dovecot and Radicale (`MAILCRAFT_REAL_INVITES_TOGETHER="imap calendar"`, as
/// MAILCRAFT_REAL_INVITES; not `real_mail_together.rs`'s MAILCRAFT_REAL_TOGETHER, three servers):
/// the Outlook invitation arrives while the calendar server can't be reached, is held, accepted
/// in its message (out of the Inbox), then stored in the server's calendar busy, with its
/// meeting; made private and out of office, it's stored so, and found by its meeting's link.
#[test]
#[ignore = "needs real IMAP and CalDAV servers in MAILCRAFT_REAL_INVITES_TOGETHER"]
fn real_outlook_meeting_held_accepted_private_and_found() {
    let Ok(v) = std::env::var("MAILCRAFT_REAL_INVITES_TOGETHER") else {
        eprintln!("MAILCRAFT_REAL_INVITES_TOGETHER isn't set: skipped");
        return;
    };
    let p: Vec<&str> = v.split_whitespace().collect();
    let [imap, calendar] = p.as_slice() else { panic!("MAILCRAFT_REAL_INVITES_TOGETHER: imap calendar") };
    let target = calendar.trim_start_matches("http://").split('/').next().unwrap().to_string();
    let (proxy_port, mode) = proxy(&target);
    let home = calendar.trim_end_matches('/').rsplit_once('/').unwrap().0.to_string() + "/";
    let proxied = home.replace(&target, &format!("127.0.0.1:{proxy_port}"));
    let mut app = App::open("together", "pw");
    app.watch = watch;
    let id = app.add_plain(imap, "127.0.0.1:1", json!({"email": USER, "password": "pw"}));
    app.s.run("account.dav", &json!({"account": id, "caldav": proxied})).unwrap();
    app.s.run("options.answeredInvites", &json!({"after": "delete"})).unwrap();
    let mut other = Imap::connect(imap, USER, "pw");
    other.create("Trash");
    *mode.lock().unwrap() = None;
    app.pump(120, "online", |_, e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    let tag = std::process::id();
    let uid = format!("together-{tag}@organizer.example");
    let summary = format!("Planning {tag}");
    other.append("INBOX", "", &outlook_invitation(&uid, &summary, "20261110T100000Z"));
    app.pump(120, "the invitation held", |s, _| !s.pim[&id].held.is_empty());
    assert!(app.s.calendars_pending(&id));
    let subject = format!("Invitation: {summary}");
    let key = app.s.folder_state(&id, "INBOX").messages.iter().find(|m| m.subject == subject).map(|m| m.key.to_id()).unwrap();
    app.s.run("folder.open", &json!({"account": id, "folder": "INBOX"})).unwrap();
    app.s.run("list.open", &json!({"key": key})).unwrap();
    let r = app.s.run("invite.respond", &json!({"answer": "accepted"})).unwrap();
    assert_eq!(r["moved"], 1, "{r}");
    assert!(app.s.pim[&id].held.is_empty(), "dealt with");
    // The calendar server answers: the accepted meeting goes there, busy, with its meeting.
    *mode.lock().unwrap() = Some(1);
    app.pool.submit(Job::SyncDav { account: id.clone() });
    app.pump(180, "the calendars found", |_, e| matches!(e, Event::DavSynced { sync, .. } if sync.calendars.is_some()));
    app.pump(180, "stored on the server", |s, _| s.find_event(&uid).is_some_and(|(_, e)| !e.dirty && !e.href.is_empty()));
    let (_, e) = app.s.find_event(&uid).unwrap();
    assert!(e.calendar.ends_with("/personal/"), "{}", e.calendar);
    assert_eq!(mailcraft_engine::model::ical::show_as(&e.event), mailcraft_engine::model::ical::ShowAs::Busy);
    let ics = stored(calendar);
    assert!(ics.contains("X-MICROSOFT-CDO-BUSYSTATUS:BUSY"), "{ics}");
    // Made private and out of office here: stored so on the server, each line once.
    app.s.run("event.update", &json!({"uid": uid, "privacy": "private", "showAs": "oof"})).unwrap();
    app.pump(120, "the change stored", |s, _| s.find_event(&uid).is_some_and(|(_, e)| !e.dirty));
    check_stored(&stored(calendar), &uid);
    let hits = app.s.run("calendar.search", &json!({"query": "is:private meet.example.edu/rooms/abc-def-ghi"})).unwrap();
    assert!(hits.as_array().unwrap().iter().any(|h| h["uid"] == uid.as_str()), "{hits}");
    let end = Instant::now() + Duration::from_secs(30);
    while !other.subjects("Trash").iter().any(|x| x.contains(&summary)) {
        assert!(Instant::now() < end, "not moved on the server");
        let _ = app.pool.wait(Duration::from_millis(500));
    }
    eprintln!("held, accepted, moved, stored busy then private and out of office, found");
}

/// Stalwart (`MAILCRAFT_REAL_INVITES_JMAP`): an event made private, out of office and with a
/// room's meeting at once is stored with all of them; the Outlook invitation delivered by JMAP,
/// accepted from the calendar and made private, likewise, and moved out of the Inbox.
#[test]
#[ignore = "needs a real JMAP server in MAILCRAFT_REAL_INVITES_JMAP"]
fn real_jmap_private_meeting_and_outlook_invitation() {
    let Ok(v) = std::env::var("MAILCRAFT_REAL_INVITES_JMAP") else {
        eprintln!("MAILCRAFT_REAL_INVITES_JMAP isn't set: skipped");
        return;
    };
    let p: Vec<&str> = v.split_whitespace().collect();
    let [base, user, password] = p.as_slice() else { panic!("MAILCRAFT_REAL_INVITES_JMAP: base user password") };
    let auth = format!("{user}:{password}");
    let other = Jmap::connect(base, &auth);
    let mut app = App::open("together-jmap", password);
    app.watch = watch;
    let id = app.add(json!({"email": USER, "username": user, "password": password, "jmap": format!("{base}/jmap/session")}));
    app.s.run("account.dav", &json!({"account": id, "caldav": format!("{base}/dav/cal/"), "username": user, "password": password})).unwrap();
    app.s.run("options.answeredInvites", &json!({"after": "delete"})).unwrap();
    app.s.run("meetings.addRoom", &json!({"name": "BBB", "url": "https://meet.example.edu/rooms/abc-def-ghi", "code": "123"})).unwrap();
    app.pump(120, "calendars found", |_, e| matches!(e, Event::DavSynced { sync, .. } if sync.calendars.is_some()));
    let tag = std::process::id();
    let made = app
        .s
        .run(
            "event.create",
            &json!({"account": id, "summary": format!("Doctor {tag}"), "start": "2026-11-12T09:00", "privacy": "private", "showAs": "oof", "onlineMeeting": "BBB", "reminder": 10}),
        )
        .unwrap()["uid"]
        .as_str()
        .unwrap()
        .to_string();
    app.pump(120, "made and stored", |s, _| s.find_event(&made).is_some_and(|(_, e)| !e.dirty && !e.href.is_empty()));
    let (_, e) = app.s.find_event(&made).unwrap();
    let ics = http_as("GET", &e.href, &auth, "text/plain", b"").text().replace("\r\n ", "");
    for want in ["CLASS:PRIVATE", "X-MICROSOFT-CDO-BUSYSTATUS:OOF", "CONFERENCE", "---- Online meeting ----", "TRIGGER:-PT10M"] {
        assert!(ics.contains(want), "{want}: {ics}");
    }
    let uid = format!("together-jmap-{tag}@organizer.example");
    let summary = format!("Planning {tag}");
    other.deliver(&outlook_invitation(&uid, &summary, "20261113T100000Z"));
    let inbox = app.s.account(&id).and_then(|a| a.folder(mailcraft_engine::model::FolderRole::Inbox)).map(|f| f.path.clone()).unwrap();
    app.pool.submit(Job::SyncFolder { account: id.clone(), folder: inbox });
    app.pump(120, "the invitation", |s, _| s.find_event(&uid).is_some());
    app.pump(120, "stored on the server", |s, _| s.find_event(&uid).is_some_and(|(_, e)| !e.dirty && !e.href.is_empty()));
    app.s.run("event.respond", &json!({"uid": uid, "answer": "accepted"})).unwrap();
    app.s.run("event.update", &json!({"uid": uid, "privacy": "private", "showAs": "oof"})).unwrap();
    app.pump(120, "the answer stored", |s, _| s.find_event(&uid).is_some_and(|(_, e)| !e.dirty));
    let (_, e) = app.s.find_event(&uid).unwrap();
    check_stored(&http_as("GET", &e.href, &auth, "text/plain", b"").text(), &uid);
    let end = Instant::now() + Duration::from_secs(60);
    while !other.subjects("trash").iter().any(|x| x.contains(&summary)) {
        for j in app.s.take_jobs() {
            app.pool.submit(j);
        }
        if let Some(e) = app.pool.wait(Duration::from_millis(300)) {
            app.s.apply(e);
        }
        assert!(Instant::now() < end, "not moved: Inbox {:?}", other.subjects("inbox"));
    }
    let hits = app.s.run("calendar.search", &json!({"query": "is:private meet.example.edu"})).unwrap();
    assert_eq!(hits.as_array().map(Vec::len), Some(2), "{hits}");
    eprintln!("made and stored with all three; the invitation accepted, private, moved, found");
}
