//! The mail side of v0.31 together, through real servers on this computer (never a real
//! account), run by hand:
//!
//! ```text
//! MAILCRAFT_REAL_TOGETHER="127.0.0.1:21710 127.0.0.1:21725 127.0.0.1:21743" \
//! MAILCRAFT_REAL_TOGETHER_JMAP="http://127.0.0.1:21780 riley pw" \
//!   cargo test -p mailcraft-cli --test real_mail_together -- --ignored --nocapture
//! ```
//!
//! Dovecot (any user, password `pw`, cleartext allowed) for IMAP, GreenMail (any password) for
//! SMTP and a second account's IMAP; then Stalwart over JMAP. In one mailbox: the sender check
//! and External (#29), other programs' keywords as categories and a server search for one (#28),
//! the message source (#27), Shift selections (#26), an invitation answered while it's read in
//! the middle of the list (#31) and named signatures following From (#11).

#[path = "support/servers.rs"]
mod servers;

use std::time::{Duration, Instant};

use mailcraft_engine::Session;
use mailcraft_engine::model::Event;
use serde_json::json;
use servers::{App, Imap, http_as, now};

/// `ago` minutes before now as a `Date:` header (RFC 5322, in UTC).
fn date(ago: i64) -> String {
    let t = now() - ago * 60;
    let (days, secs) = (t.div_euclid(86_400), t.rem_euclid(86_400));
    // Days since 1970 to a civil date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    format!("{d} {} {y} {:02}:{:02}:{:02} +0000", MONTHS[(m - 1) as usize], secs / 3600, secs / 60 % 60, secs % 60)
}

/// A plain message (`ago`: minutes before now).
fn message(from: &str, to: &str, subject: &str, ago: i64) -> Vec<u8> {
    let date = date(ago);
    format!(
        "From: {from}\r\nTo: {to}\r\nSubject: {subject}\r\nDate: {date}\r\nMessage-ID: <{}.{}@together.example>\r\nMIME-Version: 1.0\r\n\
         Content-Type: text/plain; charset=utf-8\r\n\r\n{subject}.\r\n",
        subject.replace(' ', "-"),
        std::process::id()
    )
    .into_bytes()
}

/// An invitation from Zoe (version `sequence` of meeting `uid`).
fn invitation(user: &str, uid: &str, sequence: u32, subject: &str, ago: i64) -> Vec<u8> {
    let date = date(ago);
    format!(
        "From: Zoe Park <zoe@studio.example>\r\nTo: Riley <{user}>\r\nSubject: {subject}\r\nMessage-ID: <{uid}.{sequence}@studio.example>\r\n\
         Date: {date}\r\nMIME-Version: 1.0\r\nContent-Type: multipart/alternative; boundary=\"b\"\r\n\r\n\
         --b\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nGrade review\r\n--b\r\nContent-Type: text/calendar; charset=utf-8; method=REQUEST\r\n\r\n\
         BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//studio.example//EN\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nUID:{uid}\r\nDTSTAMP:20261031T090000Z\r\n\
         DTSTART:20261110T100000Z\r\nDURATION:PT1H\r\nSUMMARY:Grade review\r\nORGANIZER;CN=Zoe Park:mailto:zoe@studio.example\r\n\
         ATTENDEE;PARTSTAT=NEEDS-ACTION;RSVP=TRUE:mailto:{user}\r\nSEQUENCE:{sequence}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n--b--\r\n"
    )
    .into_bytes()
}

#[test]
#[ignore = "needs real IMAP and SMTP servers in MAILCRAFT_REAL_TOGETHER"]
fn real_mail_side_together() {
    let Ok(v) = std::env::var("MAILCRAFT_REAL_TOGETHER") else {
        eprintln!("MAILCRAFT_REAL_TOGETHER isn't set: skipped");
        return;
    };
    let p: Vec<&str> = v.split_whitespace().collect();
    let [imap, smtp, gm_imap] = p.as_slice() else { panic!("MAILCRAFT_REAL_TOGETHER: imap smtp greenmail-imap") };
    let pid = std::process::id();
    let user = format!("riley{pid}@studio.example");
    let other_user = format!("riley{pid}@studio.test");
    let mut other = Imap::connect(imap, &user, "pw");
    for f in ["Sent", "Trash"] {
        other.create(f);
    }
    // Written to Zoe before: her name is known; the rest of the Inbox.
    other.append("Sent", "\\Seen", &message(&format!("Riley <{user}>"), "Zoe Park <zoe@studio.example>", "Notes for Zoe", 600));
    other.append("INBOX", "", &message("Zoe Park <zoe@studio.example>", &user, "From the real Zoe", 90));
    other.append("INBOX", "", &message("Zoe Park <zoe.park.director@gmail.com>", &user, "Quick favour", 80));
    other.append("INBOX", "$label1 projekt=20x JunkRecorded $MDNSent", &message("Bo <bo@vendor.example>", &user, "Tagged by others", 70));
    other.append("INBOX", "", &message(&format!("Riley <{user}>"), &user, "Note to self", 60));
    let uid = format!("grade-{pid}@studio.example");
    other.append("INBOX", "", &invitation(&user, &uid, 0, "Invitation v0", 50));
    other.append("INBOX", "", &message("Bo <bo@vendor.example>", &user, "Between", 40));
    other.append("INBOX", "", &invitation(&user, &uid, 1, "Invitation v1", 30));
    other.append("INBOX", "", &message("Bo <bo@vendor.example>", &user, "Newest", 20));

    let mut app = App::open("together", "pw");
    let id = app.add_plain(imap, smtp, json!({"email": user, "name": "Riley", "password": "pw"}));
    let id2 = app.add_plain(gm_imap, smtp, json!({"email": other_user, "name": "Riley", "password": "pw"}));
    app.s.run("options.answeredInvites", &json!({"after": "delete"})).unwrap();
    app.s.run("view.externalTag", &json!({"on": true})).unwrap();
    app.pump(120, "the Inbox", |s, _| s.folder_state(&id, "INBOX").messages.len() >= 8);
    app.pump(120, "Sent", |s, _| !s.folder_state(&id, "Sent").messages.is_empty());
    // The people written to are counted again once the check is ten minutes old (it was
    // gathered before Sent came).
    app.s.clock.now += 601;
    app.s.run("folder.open", &json!({"account": id, "folder": "INBOX"})).unwrap();

    // #29 and #28 in one list: the warning and the address, External, other programs' keywords
    // as categories (not Apple's or the IANA's own).
    let r = app.s.run("list.messages", &json!({"limit": 50})).unwrap();
    let row = |subject: &str| r["messages"].as_array().unwrap().iter().find(|m| m["subject"] == subject).cloned().unwrap();
    eprintln!("{}", serde_json::to_string_pretty(&r).unwrap());
    let favour = row("Quick favour");
    assert_eq!(
        (favour["senderWarning"].as_bool(), favour["fromAddress"].as_bool(), favour["external"].as_bool()),
        (Some(true), Some(true), Some(true))
    );
    let real = row("From the real Zoe");
    assert_eq!((real["senderWarning"].as_bool(), real["external"].as_bool()), (Some(false), Some(false)));
    let own = row("Note to self");
    assert_eq!((own["senderWarning"].as_bool(), own["external"].as_bool()), (Some(false), Some(false)), "own mail");
    let tagged = row("Tagged by others");
    assert_eq!(tagged["external"].as_bool(), Some(true));
    let cats: Vec<&str> = tagged["categories"].as_array().unwrap().iter().filter_map(|c| c.as_str()).collect();
    assert!(cats.contains(&"Important") && cats.iter().any(|c| c.eq_ignore_ascii_case("projekt x")), "{cats:?}");
    assert!(!cats.iter().any(|c| c.contains("Junk") || c.contains("MDN")), "{cats:?}");

    // #27: the source of the one that doesn't add up, downloaded.
    let favour = app.key(&id, "INBOX", "Quick favour");
    let src = app.s.run("message.source", &json!({"key": favour.to_id(), "headers": true})).unwrap();
    if src["pending"].as_bool() == Some(true) {
        app.pump(60, "the source", |s, _| s.sources.iter().any(|v| v.key == favour && !v.pending));
    }
    let src = app.s.run("message.source", &json!({"key": favour.to_id(), "headers": true})).unwrap();
    assert!(src["text"].as_str().unwrap().contains("From: Zoe Park <zoe.park.director@gmail.com>"), "{src}");

    // #28 on the server: a search for the Thunderbird tag by name finds it there.
    app.s.run("search.run", &json!({"query": "category:\"Projekt X\"", "scope": "mailbox"})).unwrap();
    app.s.run("search.server", &json!({})).unwrap();
    let mut asked = (String::new(), 0, None);
    app.pump(60, "the server's search", |_, e| match e {
        Event::ServerSearched { query, keys, error, done, .. } => {
            asked = (query.clone(), asked.1 + keys.len(), error.clone());
            *done
        }
        _ => false,
    });
    assert!(asked.0.contains("keyword:projekt=20x") && asked.1 >= 1 && asked.2.is_none(), "the server asked by keyword: {asked:?}");
    let found: Vec<String> = app.s.list.iter().map(|m| m.subject.clone()).collect();
    assert_eq!(found, vec!["Tagged by others".to_string()], "{found:?}");
    app.s.run("search.clear", &json!({})).ok();
    app.s.run("folder.open", &json!({"account": id, "folder": "INBOX"})).unwrap();

    // #26 then #31: the newest invitation read, between plain mail; answered, it and the earlier
    // version go to Deleted Items on the server, and the list goes on to the next message, from
    // which Shift+Down selects.
    let v1 = app.key(&id, "INBOX", "Invitation v1");
    let v0 = app.key(&id, "INBOX", "Invitation v0");
    for k in [&v0, &v1] {
        app.s.run("list.open", &json!({"key": k.to_id()})).unwrap();
        let k = k.clone();
        app.pump(60, "the invitation's body", |s, _| s.reading.as_ref().is_some_and(|r| r.key == k && r.parsed.is_some()));
    }
    let rows = app.s.row_keys();
    let at = rows.iter().position(|k| *k == v1).unwrap();
    let after = rows[at + 1].clone();
    let r = app.s.run("invite.respond", &json!({"answer": "accepted"})).unwrap();
    assert_eq!(r["moved"], 2, "{r}");
    assert_eq!(app.s.reading.as_ref().map(|r| app.subject(&r.key)), Some(app.subject(&after)), "the next one is read");
    app.s.run("list.extendNext", &json!({})).unwrap();
    let picked: Vec<String> = app.s.selection.iter().map(|k| app.subject(k)).collect();
    eprintln!("selected after answering: {picked:?}");
    assert_eq!(picked.len(), 2);
    assert!(!picked.iter().any(|p| p.starts_with("Invitation")), "{picked:?}");
    // Ctrl+A, then a category on all of them: kept on the server as its keyword.
    app.s.run("list.selectAll", &json!({})).unwrap();
    assert!(app.s.reading.is_none(), "several selected: none read");
    app.s.run("mail.categorize", &json!({"category": "Work"})).unwrap();
    app.settle(5);
    let trash = other.subjects("Trash");
    let inbox = other.messages("INBOX");
    eprintln!("Trash: {trash:?}\nInbox: {inbox:?}");
    assert!(trash.contains(&"Invitation v0".to_string()) && trash.contains(&"Invitation v1".to_string()));
    assert!(!inbox.iter().any(|m| m.subject.starts_with("Invitation")));
    let work = app.s.category("Work").map(|c| c.keyword.clone()).unwrap();
    assert!(inbox.iter().all(|m| m.flags.iter().any(|f| f.eq_ignore_ascii_case(&work))), "{inbox:?}");

    // #11: each account's signature; From changed, the other's follows, and it's what's sent.
    app.s.run("account.setSignature", &json!({"account": id, "name": "Dovecot sig", "signature": "Riley at the Dovecot desk"})).unwrap();
    app.s.run("account.setSignature", &json!({"account": id2, "name": "GreenMail sig", "signature": "Riley at the GreenMail desk"})).unwrap();
    app.s.run("mail.new", &json!({"account": id})).unwrap();
    assert!(app.s.compose.as_ref().unwrap().body.contains("Dovecot desk"));
    app.s.run("compose.update", &json!({"to": other_user, "subject": format!("Signed {pid}"), "account": id2})).unwrap();
    let body = app.s.compose.as_ref().unwrap().body.clone();
    assert!(body.contains("GreenMail desk") && !body.contains("Dovecot desk"), "{body}");
    app.s.run("compose.send", &json!({})).unwrap();
    app.pump(60, "sent", |_, e| matches!(e, Event::Sent { .. }));
    let mut gm = Imap::connect(gm_imap, &other_user, "pw");
    let end = Instant::now() + Duration::from_secs(30);
    while !gm.subjects("INBOX").contains(&format!("Signed {pid}")) {
        assert!(Instant::now() < end, "not delivered");
        std::thread::sleep(Duration::from_millis(500));
    }
    let text = gm.newest_text("INBOX");
    assert!(text.contains("GreenMail desk") && !text.contains("Dovecot desk"), "{text}");
    eprintln!("all of it on Dovecot and GreenMail");
}

/// The same over JMAP (Stalwart: `MAILCRAFT_REAL_TOGETHER_JMAP="http://127.0.0.1:21780 riley
/// pw"`): another program's keyword shows as a category and a search for it by name asks the
/// server for that keyword; the source of a message is downloaded as it came.
#[test]
#[ignore = "needs a real JMAP server in MAILCRAFT_REAL_TOGETHER_JMAP"]
fn real_jmap_categories_and_source() {
    let Ok(v) = std::env::var("MAILCRAFT_REAL_TOGETHER_JMAP") else {
        eprintln!("MAILCRAFT_REAL_TOGETHER_JMAP isn't set: skipped");
        return;
    };
    let p: Vec<&str> = v.split_whitespace().collect();
    let [base, user, password] = p.as_slice() else { panic!("MAILCRAFT_REAL_TOGETHER_JMAP: base user password") };
    let auth = format!("{user}:{password}");
    let pid = std::process::id();
    // Put in by another program, with its tag and a junk filter's note.
    let session: serde_json::Value =
        serde_json::from_slice(&http_as("GET", &format!("{base}/jmap/session"), &auth, "application/json", b"").body).unwrap();
    let account = session["primaryAccounts"]["urn:ietf:params:jmap:mail"].as_str().unwrap().to_string();
    let api = session["apiUrl"].as_str().unwrap().replace("https://", "http://");
    let api = format!("{base}/{}", api.split_once("/jmap").map(|(_, rest)| format!("jmap{rest}")).unwrap_or_else(|| "jmap/".into()));
    let call = |calls: serde_json::Value| -> serde_json::Value {
        let body = json!({"using": ["urn:ietf:params:jmap:core", "urn:ietf:params:jmap:mail"], "methodCalls": calls});
        serde_json::from_slice(&http_as("POST", &api, &auth, "application/json", body.to_string().as_bytes()).body).unwrap()
    };
    let r = call(json!([["Mailbox/query", {"accountId": account, "filter": {"role": "inbox"}}, "q"]]));
    let inbox = r["methodResponses"][0][1]["ids"][0].as_str().unwrap().to_string();
    let subject = format!("Tagged over JMAP {pid}");
    let raw = message("Bo <bo@vendor.example>", "riley@studio.example", &subject, 5);
    let up: serde_json::Value =
        serde_json::from_slice(&http_as("POST", &format!("{base}/jmap/upload/{account}/"), &auth, "message/rfc822", &raw).body).unwrap();
    let blob = up["blobId"].as_str().unwrap();
    let r = call(json!([["Email/import", {"accountId": account, "emails": {"m": {"blobId": blob, "mailboxIds": {inbox.clone(): true},
        "keywords": {"projekt=20x": true, "junkrecorded": true}}}}, "i"]]));
    assert!(r["methodResponses"][0][1]["created"]["m"].is_object(), "{r}");

    let mut app = App::open("together-jmap", password);
    let id = app.add(json!({"email": "riley@studio.example", "username": user, "password": password, "jmap": format!("{base}/jmap/session")}));
    let folder = |s: &Session| s.account(&id).and_then(|a| a.folder(mailcraft_engine::model::FolderRole::Inbox)).map(|f| f.path.clone());
    app.pump(120, "the Inbox", |s, _| folder(s).is_some_and(|f| s.folder_state(&id, &f).messages.iter().any(|m| m.subject == subject)));
    let inbox_path = folder(&app.s).unwrap();
    app.s.run("folder.open", &json!({"account": id, "folder": inbox_path})).unwrap();
    let key = app.key(&id, &inbox_path, &subject);
    let m = app.s.summary(&key).cloned().unwrap();
    let cats: Vec<String> = app.s.categories_of(&m).into_iter().map(|c| c.name).collect();
    assert_eq!(cats, vec!["projekt x".to_string()], "{:?}", m.flags.keywords);
    // The source, downloaded.
    let src = app.s.run("message.source", &json!({"key": key.to_id()})).unwrap();
    if src["pending"].as_bool() == Some(true) {
        app.pump(60, "the source", |s, _| s.sources.iter().any(|v| v.key == key && !v.pending));
    }
    let src = app.s.run("message.source", &json!({"key": key.to_id()})).unwrap();
    assert!(src["error"].is_null(), "{src}");
    assert!(src["text"].as_str().unwrap().contains(&format!("Subject: {subject}")), "{src}");
    // A search for the tag by name: the server is asked for its keyword, and finds it.
    app.s.run("search.run", &json!({"query": "category:\"Projekt X\"", "scope": "mailbox"})).unwrap();
    app.s.run("search.server", &json!({})).unwrap();
    let mut asked = (String::new(), 0, None);
    app.pump(60, "the server's search", |_, e| match e {
        Event::ServerSearched { query, keys, error, done, .. } => {
            asked = (query.clone(), asked.1 + keys.len(), error.clone());
            *done
        }
        _ => false,
    });
    assert!(asked.0.contains("keyword:projekt=20x") && asked.1 >= 1 && asked.2.is_none(), "the server asked by keyword: {asked:?}");
    let found: Vec<String> = app.s.list.iter().map(|m| m.subject.clone()).collect();
    assert!(found.contains(&subject), "{found:?}");
    eprintln!("JMAP: categories {cats:?}, the source and a search for the tag");
}
