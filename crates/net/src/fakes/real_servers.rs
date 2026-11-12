//! Against real servers on this computer (Dovecot, GreenMail in Docker; never a real account),
//! run by hand: what the in-memory servers can't prove about a real one's answers.
//!
//! ```text
//! MAILCRAFT_REAL_IMAP="127.0.0.1:1143 user password" \
//! MAILCRAFT_REAL_SMTP="127.0.0.1:3025 user password 127.0.0.1:3143" \
//!   cargo test -p mailcraft-net real_ -- --ignored --nocapture
//! ```
//!
//! The IMAP server's Inbox holds `Message 1`…`Message 3000` (every hundredth's body says
//! "needle"), and one message with a 30 MB attachment, newest.

use std::time::{Duration, Instant};

use mailcraft_model::job::Secret;
use mailcraft_model::{AccountConfig, Event, Job, Op, Security, ServerConfig, SyncStatus};

use crate::worker::{MemoryCredentials, Pool};

/// `host:port user password [more…]` from the environment, or `None` (the test is skipped).
fn server(var: &str) -> Option<(ServerConfig, String, String, Vec<String>)> {
    let v = std::env::var(var).ok()?;
    let mut parts = v.split_whitespace().map(str::to_string);
    let (addr, user, pass) = (parts.next()?, parts.next()?, parts.next()?);
    let (host, port) = addr.rsplit_once(':')?;
    let cfg = ServerConfig { host: host.into(), port: port.parse().ok()?, security: Security::None };
    Some((cfg, user, pass, parts.collect()))
}

/// Events until `f` says yes (or `secs` pass): all of them, the last the one wanted.
fn until(pool: &Pool, secs: u64, what: &str, mut f: impl FnMut(&Event) -> bool) -> Vec<Event> {
    let end = Instant::now() + Duration::from_secs(secs);
    let mut seen = Vec::new();
    while Instant::now() < end {
        if let Some(e) = pool.wait(Duration::from_millis(100)) {
            let hit = f(&e);
            seen.push(e);
            if hit {
                return seen;
            }
        }
    }
    panic!("timed out waiting for {what}; last events: {:?}", seen.iter().rev().take(5).collect::<Vec<_>>());
}

/// A real IMAP server through the worker: a window of 25, Show more over QRESYNC, a search of
/// the whole Inbox a range at a time (its finds as they come), a search stopped, a 30 MB message
/// read whole past the per-command caps, and mail put in by another client noticed at once.
#[test]
#[ignore]
fn real_imap_server() {
    let Some((imap, user, pass, _)) = server("MAILCRAFT_REAL_IMAP") else {
        eprintln!("MAILCRAFT_REAL_IMAP isn't set: skipped");
        return;
    };
    let store = crate::fake::store("real-imap");
    let mut pool = Pool::new(store.clone(), std::sync::Arc::new(MemoryCredentials::default()));
    pool.submit(Job::SetSync { messages: 25, offline: 0 });
    let account = AccountConfig {
        id: "r1".into(),
        email: user.clone(),
        username: user.clone(),
        imap: imap.clone(),
        smtp: ServerConfig { host: "127.0.0.1".into(), port: 1, security: Security::None },
        ..Default::default()
    };
    let began = Instant::now();
    pool.submit(Job::Start { account, password: Some(Secret(pass.clone())) });
    until(&pool, 120, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    let kept = || store.load_folder("r1", "INBOX").map(|f| f.messages.len()).unwrap_or(0);
    eprintln!("online in {:?}, {} kept", began.elapsed(), kept());
    assert_eq!(kept(), 25);

    // Show more: 25 more, through the QRESYNC sync (Dovecot has it).
    pool.submit(Job::MoreMail { account: "r1".into(), folder: "INBOX".into(), more: 25 });
    let end = Instant::now() + Duration::from_secs(60);
    while kept() < 50 {
        assert!(Instant::now() < end, "Show more brought {} of 50", kept());
        let _ = pool.wait(Duration::from_millis(100));
    }

    // Search the server: newest first, its finds as they come.
    let began = Instant::now();
    pool.submit(Job::SearchServer { account: "r1".into(), folders: vec!["INBOX".into()], query: "needle".into(), search: 1 });
    let seen = until(&pool, 120, "the search's end", |e| matches!(e, Event::ServerSearched { done: true, .. }));
    let batches: Vec<usize> =
        seen.iter().filter_map(|e| if let Event::ServerSearched { keys, done: false, .. } = e { Some(keys.len()) } else { None }).collect();
    let found: usize = batches.iter().sum();
    eprintln!("search: {found} found in {:?}, in batches {batches:?}", began.elapsed());
    assert_eq!(found, 30);
    assert!(batches.len() > 1, "shown as found, not all at the end");

    // A search stopped before it starts its next range answers that it's done, with no error.
    pool.submit(Job::SearchServer { account: "r1".into(), folders: vec!["INBOX".into()], query: "nowhere-at-all".into(), search: 2 });
    pool.submit(Job::StopSearch);
    until(&pool, 60, "the stopped search's end", |e| matches!(e, Event::ServerSearched { done: true, error: None, .. }));

    // The 30 MB message, whole: the per-command caps leave real mail alone.
    let big =
        store.load_folder("r1", "INBOX").unwrap().messages.into_iter().find(|m| m.subject == "Big attachment").expect("the big message is kept");
    let began = Instant::now();
    pool.submit(Job::FetchBody { key: big.key.clone() });
    // Its text first (issue #24), then the whole of it.
    let whole = |raw: &[u8]| !mailcraft_mime::is_text_first(raw);
    let seen = until(&pool, 180, "the big message", |e| {
        matches!(e, Event::Body { key, raw } if *key == big.key && whole(raw)) || matches!(e, Event::BodyFailed { key, .. } if *key == big.key)
    });
    match seen.last() {
        Some(Event::Body { raw, .. }) => {
            eprintln!("big message: {} bytes in {:?}", raw.len(), began.elapsed());
            assert!(raw.len() > 40_000_000, "{}", raw.len());
        }
        other => panic!("the big message didn't come: {other:?}"),
    }

    // Another client puts a message in the Inbox: noticed without waiting for the next sync.
    let mut other = crate::imap::Client::connect(&imap).unwrap();
    other.login(&user, &pass).unwrap();
    let subject = format!("Arrived {}", std::process::id());
    let raw =
        format!("From: x@studio.test\r\nTo: {user}\r\nSubject: {subject}\r\nMessage-ID: <late{}@studio.test>\r\n\r\nhi\r\n", std::process::id());
    let began = Instant::now();
    other.append("INBOX", "", raw.as_bytes()).unwrap();
    let end = Instant::now() + Duration::from_secs(45);
    while !store.load_folder("r1", "INBOX").unwrap().messages.iter().any(|m| m.subject == subject) {
        assert!(Instant::now() < end, "the new message wasn't noticed in 45 s");
        let _ = pool.wait(Duration::from_millis(100));
    }
    eprintln!("new mail noticed in {:?}", began.elapsed());
    pool.submit(Job::Remove { account: "r1".into() });
    let _ = std::fs::remove_dir_all(store.root());
}

/// Issue #24 on a real IMAP server (`MAILCRAFT_REAL_TEXT_FIRST="127.0.0.1:3143 user password"`,
/// a mailbox of its own): a message with a 10 MB file and a picture shows its text from its
/// structure first, the files listed with no bytes; the whole of it comes after, as it was put.
#[test]
#[ignore]
fn real_imap_text_first() {
    let Some((imap, user, pass, _)) = server("MAILCRAFT_REAL_TEXT_FIRST") else {
        eprintln!("MAILCRAFT_REAL_TEXT_FIRST isn't set: skipped");
        return;
    };
    let id = std::process::id();
    let file =
        "QUJDRA".repeat(10 << 20).chars().collect::<Vec<_>>().chunks(76).map(|l| l.iter().collect::<String>()).collect::<Vec<_>>().join("\r\n");
    let raw = format!(
        "From: Sam <sam@studio.test>\r\nTo: {user}\r\nSubject: Rushes {id}\r\nMessage-ID: <rushes{id}@studio.test>\r\nDate: Tue, 3 Nov 2026 10:00:00 +0000\r\n\
         MIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"B{id}\"\r\n\r\n\
         --B{id}\r\nContent-Type: multipart/related; boundary=\"R{id}\"\r\n\r\n\
         --R{id}\r\nContent-Type: text/html; charset=iso-8859-1\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\n<p>Les notes, =E0 lire tout de suite.</p><img src=3D\"cid:still{id}@x\">\r\n\
         --R{id}\r\nContent-Type: image/png\r\nContent-ID: <still{id}@x>\r\nContent-Transfer-Encoding: base64\r\n\r\niVBORw0KGgo=\r\n--R{id}--\r\n\
         --B{id}\r\nContent-Type: video/quicktime\r\nContent-Disposition: attachment; filename*=utf-8''r%C3%BCsh%20reel.mov\r\n\
         Content-Transfer-Encoding: base64\r\n\r\n{file}\r\n--B{id}--\r\n"
    );
    let mut c = crate::imap::Client::connect(&imap).unwrap();
    c.login(&user, &pass).unwrap();
    c.append("INBOX", "", raw.as_bytes()).unwrap();
    c.logout();
    let store = crate::fake::store("real-text-first");
    let mut pool = Pool::new(store.clone(), std::sync::Arc::new(MemoryCredentials::default()));
    pool.opts.prefetch = 0;
    let account = AccountConfig {
        id: "r1".into(),
        email: user.clone(),
        username: user.clone(),
        imap,
        smtp: ServerConfig { host: "127.0.0.1".into(), port: 1, security: Security::None },
        ..Default::default()
    };
    pool.submit(Job::Start { account, password: Some(Secret(pass)) });
    until(&pool, 120, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    let subject = format!("Rushes {id}");
    let m = store.load_folder("r1", "INBOX").unwrap().messages.into_iter().find(|m| m.subject == subject).expect("the message is listed");
    let began = Instant::now();
    pool.submit(Job::FetchBody { key: m.key.clone() });
    let seen = until(&pool, 60, "its text", |e| matches!(e, Event::Body { key, .. } if *key == m.key));
    let first_after = began.elapsed();
    let Some(Event::Body { raw: first, .. }) = seen.last() else { panic!("no body") };
    assert!(mailcraft_mime::is_text_first(first), "its text first");
    let p = mailcraft_mime::parse(first).unwrap();
    assert!(p.text.contains("Les notes, à lire tout de suite."), "{}", p.text);
    let files: Vec<(String, usize, bool)> = p.attachments.iter().map(|a| (a.name.clone(), a.size, a.inline)).collect();
    eprintln!("text first: {} bytes in {first_after:?}; files {files:?}", first.len());
    assert!(files.iter().any(|(n, s, inline)| n == "rüsh reel.mov" && *s > 40_000_000 && !inline), "{files:?}");
    assert!(files.iter().any(|f| f.2), "the picture, inline: {files:?}");
    assert!(first.len() < 8192);
    let seen =
        until(&pool, 180, "the whole of it", |e| matches!(e, Event::Body { key, raw } if *key == m.key && !mailcraft_mime::is_text_first(raw)));
    let Some(Event::Body { raw: whole, .. }) = seen.last() else { panic!("no body") };
    eprintln!("whole: {} bytes in {:?}", whole.len(), began.elapsed());
    assert_eq!(mailcraft_mime::parse(whole).unwrap().subject, subject);
    assert!(store.has_body(&m.key));
}

/// Issue #38 on a real IMAP server (`MAILCRAFT_REAL_FILING="127.0.0.1:3143 user password"`): an
/// Outlook `.msg` and an `.eml` dropped on a folder go to the server as read mail, and come back
/// in the folder's list; a file that isn't a message is said.
#[test]
#[ignore]
fn real_imap_message_files() {
    let Some((imap, user, pass, _)) = server("MAILCRAFT_REAL_FILING") else {
        eprintln!("MAILCRAFT_REAL_FILING isn't set: skipped");
        return;
    };
    let dir = std::env::temp_dir().join(format!("mailcraft-real-filing-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let id = std::process::id();
    let (msg, eml, png) = (dir.join("figures.msg"), dir.join("hello.eml"), dir.join("photo.png"));
    std::fs::write(&msg, crate::data_files::msg::tests::sample()).unwrap();
    std::fs::write(
        &eml,
        format!("From: Zoe <zoe@studio.test>\r\nTo: {user}\r\nSubject: Saved hello {id}\r\nMessage-ID: <saved{id}@studio.test>\r\n\r\nHi.\r\n"),
    )
    .unwrap();
    std::fs::write(&png, b"\x89PNG\r\n\x1a\n").unwrap();
    let store = crate::fake::store("real-filing");
    let mut pool = Pool::new(store.clone(), std::sync::Arc::new(MemoryCredentials::default()));
    pool.opts.prefetch = 0;
    let account = AccountConfig {
        id: "r1".into(),
        email: user.clone(),
        username: user.clone(),
        imap: imap.clone(),
        smtp: ServerConfig { host: "127.0.0.1".into(), port: 1, security: Security::None },
        ..Default::default()
    };
    pool.submit(Job::Start { account, password: Some(Secret(pass.clone())) });
    until(&pool, 120, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    let paths = [&msg, &eml, &png].iter().map(|p| p.to_string_lossy().into_owned()).collect();
    pool.submit(Job::FileMessages { account: "r1".into(), folder: "INBOX".into(), paths });
    let seen = until(&pool, 60, "filed", |e| matches!(e, Event::MessagesFiled { .. }));
    let Some(Event::MessagesFiled { added, failed, .. }) = seen.last() else { panic!() };
    eprintln!("added {added}, failed {failed:?}");
    assert_eq!((*added, failed.len()), (2, 1));
    let listed = store.load_folder("r1", "INBOX").unwrap();
    let subject = format!("Saved hello {id}");
    assert!(listed.messages.iter().any(|m| m.subject == subject && m.flags.seen), "the .eml, read");
    assert!(listed.messages.iter().any(|m| m.subject == "Quarterly figures — räder" && m.has_attachments), "the .msg, with its files");
    // As the server keeps it: the .msg's files and attached message there.
    let mut c = crate::imap::Client::connect(&imap).unwrap();
    c.login(&user, &pass).unwrap();
    c.select("INBOX", true).unwrap();
    let found = c.uid_search("SUBJECT \"Quarterly figures\"").unwrap();
    let last = *found.last().expect("on the server");
    let raw = c.uid_fetch(&last.to_string(), "UID BODY.PEEK[]", "").unwrap().into_iter().find_map(|f| f.body).unwrap();
    let p = mailcraft_mime::parse(&raw).unwrap();
    let names: Vec<&str> = p.attachments.iter().map(|a| a.name.as_str()).collect();
    eprintln!("on the server: {} bytes, files {names:?}", raw.len());
    assert!(names.contains(&"figures.csv") && p.attachments.iter().any(|a| a.mime == "message/rfc822"), "{names:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// Issue #39 through a real server (`MAILCRAFT_REAL_REACTION="127.0.0.1:3025 ana@studio.test
/// anapw 127.0.0.1:3143 ben@studio.test benpw"`: SMTP and the sender, then IMAP and the
/// recipient): a reaction sent to a message arrives marked a reaction, threaded under it, and
/// its summary, from the recipient's sync, says so.
#[test]
#[ignore]
fn real_reaction_round_trip() {
    let Some((smtp, from, from_pw, more)) = server("MAILCRAFT_REAL_REACTION") else {
        eprintln!("MAILCRAFT_REAL_REACTION isn't set: skipped");
        return;
    };
    let [imap_at, to, to_pw] = more.as_slice() else { panic!("…: smtp user password imap user password") };
    let (host, port) = imap_at.rsplit_once(':').unwrap();
    let imap = ServerConfig { host: host.into(), port: port.parse().unwrap(), security: Security::None };
    let id = std::process::id();
    let original = format!(
        "From: Ben <{to}>\r\nTo: Ana <{from}>\r\nSubject: Launch plan {id}\r\nMessage-ID: <plan{id}@studio.test>\r\n\r\nHere's the plan.\r\n"
    );
    let parsed = mailcraft_mime::parse(original.as_bytes()).unwrap();
    let me = mailcraft_model::EmailAddress::new("Ana", from.as_str());
    let raw = mailcraft_mime::reaction(&parsed, "🎉", &me, std::slice::from_ref(&from), 1_793_700_000, &format!("<react{id}@studio.test>")).unwrap();
    crate::smtp::send(&smtp, &from, &from_pw, &from, std::slice::from_ref(to), &raw).unwrap();
    let store = crate::fake::store("real-reaction");
    let mut pool = Pool::new(store.clone(), std::sync::Arc::new(MemoryCredentials::default()));
    // Nothing downloaded ahead: the reaction's body comes because its header says what it is.
    pool.opts.prefetch = 0;
    let account = AccountConfig {
        id: "r1".into(),
        email: to.clone(),
        username: to.clone(),
        imap,
        smtp: ServerConfig { host: "127.0.0.1".into(), port: 1, security: Security::None },
        ..Default::default()
    };
    pool.submit(Job::Start { account, password: Some(Secret(to_pw.clone())) });
    until(&pool, 120, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    let subject = format!("Re: Launch plan {id}");
    let end = Instant::now() + Duration::from_secs(30);
    let got = loop {
        let found = store.load_folder("r1", "INBOX").unwrap().messages.into_iter().find(|m| m.subject == subject);
        match found {
            Some(m) if !m.reaction.is_empty() => break m,
            _ if Instant::now() > end => panic!("the reaction didn't arrive marked as one"),
            _ => {
                pool.submit(Job::SyncFolder { account: "r1".into(), folder: "INBOX".into() });
                let _ = pool.wait(Duration::from_millis(500));
            }
        }
    };
    eprintln!("arrived: {:?} answering {}", got.reaction, got.in_reply_to);
    assert_eq!((got.reaction.as_str(), got.in_reply_to.as_str()), ("🎉", format!("plan{id}@studio.test").as_str()));
}

/// Issue #40 on a real CalDAV server (`MAILCRAFT_REAL_CALDAV`, as below): an event's reminder
/// dismissed here is stored as its alarm acknowledged (RFC 9074), and read back so, from a copy
/// another program would read; the alarm itself stays.
#[test]
#[ignore]
fn real_caldav_dismissed_reminder() {
    use mailcraft_model::ical::{CalEvent, acknowledge, acknowledged, reminder_minutes, set_reminder};
    use mailcraft_model::pim::{Pim, StoredEvent};
    let Ok(v) = std::env::var("MAILCRAFT_REAL_CALDAV") else {
        eprintln!("MAILCRAFT_REAL_CALDAV isn't set: skipped");
        return;
    };
    let p: Vec<&str> = v.split_whitespace().collect();
    let [url, user, password] = p.as_slice() else { panic!("MAILCRAFT_REAL_CALDAV: url user password") };
    let dav = crate::dav::Dav::new(user, password);
    let at = crate::dav::Where { email: user, username: "", others: &[], imap_host: "127.0.0.1", caldav: Some(url), carddav: None, mails: true };
    let zone = mailcraft_model::zone::Zone::Fixed(0);
    let now = 1_793_782_000;
    let sync = |pim: &Pim| {
        let s = crate::dav::sync(&dav, &at, pim, now, zone).unwrap();
        assert!(s.errors.is_empty(), "{:?}", s.errors);
        s
    };
    let first = sync(&Pim::default());
    let cal = first.calendars.as_ref().and_then(|c| c.iter().find(|c| !c.read_only)).map(|c| c.id.clone()).expect("a calendar to write to");
    let uid = format!("mc32-reminder-{}@mailcraft", std::process::id());
    let mut e = CalEvent { uid: uid.clone(), summary: "Dentist".into(), start: now + 900, end: now + 4500, dtstamp: now, ..Default::default() };
    set_reminder(&mut e, Some(15));
    let mut pim = Pim { calendars: first.calendars.clone().unwrap_or_default(), caldav_home: first.caldav_home.clone(), ..Default::default() };
    pim.events.push(StoredEvent { calendar: cal.clone(), event: e, dirty: true, ..Default::default() });
    let s = sync(&pim);
    let mut stored = s.pushed_events.iter().find(|x| x.event.uid == uid).cloned().expect("stored");
    // Dismissed here, as the engine does: acknowledged now, sent again.
    acknowledge(&mut stored.event, now);
    stored.dirty = true;
    pim.events = vec![stored.clone()];
    let s = sync(&pim);
    assert!(s.pushed_events.iter().any(|x| x.event.uid == uid), "sent again");
    let got = dav.multiget(crate::dav::Kind::Calendars, &cal, std::slice::from_ref(&stored.href)).unwrap();
    let text = got.first().map(|g| g.2.clone()).unwrap_or_default();
    eprintln!("--- stored:\n{text}");
    let alarm = text.split("BEGIN:VALARM").nth(1).and_then(|a| a.split("END:VALARM").next()).unwrap_or_default();
    assert!(alarm.contains("ACKNOWLEDGED:") && alarm.contains("TRIGGER"), "{text}");
    // Read back from scratch, as another program reads it.
    let fresh = sync(&Pim::default());
    let back = fresh.events.iter().find(|x| x.event.uid == uid).expect("read back");
    assert_eq!((acknowledged(&back.event), reminder_minutes(&back.event)), (Some(now), Some(15)));
    let _ = dav.delete(&stored.href, "");
}

/// A real SMTP server: a message sent, then found in the recipient's mailbox over IMAP; a
/// refused sign-in, where the server refuses one, is a sign-in failure, not a refusal of the
/// message.
#[test]
#[ignore]
fn real_smtp_server() {
    let Some((smtp, user, pass, more)) = server("MAILCRAFT_REAL_SMTP") else {
        eprintln!("MAILCRAFT_REAL_SMTP isn't set: skipped");
        return;
    };
    let to = if user.contains('@') { user.clone() } else { format!("{user}@gm.test") };
    let subject = format!("Sent {}", std::process::id());
    let raw = format!("From: {to}\r\nTo: {to}\r\nSubject: {subject}\r\nMessage-ID: <sent{}@studio.test>\r\n\r\nhello\r\n", std::process::id());
    crate::smtp::send(&smtp, &user, &pass, &to, std::slice::from_ref(&to), raw.as_bytes()).unwrap();
    // A refused sign-in is a sign-in failure, not a refusal of the message (some test servers,
    // GreenMail among them, take any password: then there's nothing to see).
    let copies = match crate::smtp::send(&smtp, &user, "wrong-password", &to, std::slice::from_ref(&to), raw.as_bytes()) {
        Ok(()) => {
            eprintln!("this server takes any password: a refused sign-in isn't tried");
            2
        }
        Err(e) => {
            eprintln!("wrong password: {e}");
            assert!(matches!(e, crate::NetError::Auth(_)), "{e:?}");
            1
        }
    };
    // Read back over the same server's IMAP, when given.
    if let Some((host, port)) = more.first().and_then(|a| a.rsplit_once(':')) {
        let cfg = ServerConfig { host: host.into(), port: port.parse().unwrap(), security: Security::None };
        let mut c = crate::imap::Client::connect(&cfg).unwrap();
        c.login(&user, &pass).unwrap();
        let info = c.select("INBOX", true).unwrap();
        let found = c.uid_search(&format!("SUBJECT \"{subject}\"")).unwrap();
        eprintln!("inbox has {}, the sent one: {found:?}", info.exists);
        assert_eq!(found.len(), copies);
    }
}

/// S/MIME through a real server (`MAILCRAFT_REAL_SMIME="127.0.0.1:3025 127.0.0.1:3143"`:
/// GreenMail's SMTP and IMAP, any password), with the smime crate's test certificates: Ana's
/// encrypted mail (from OpenSSL) delivered to Riley and decrypted by the pool with the key it
/// keeps; Riley's reply signed and encrypted by the pool, sent, fetched back over IMAP as the
/// server delivered it, decrypted and its signature checked. That copy is written to
/// `$MAILCRAFT_SMIME_OUT/mc-delivered.eml` too, for OpenSSL to verify and decrypt.
#[test]
#[ignore]
fn real_smime_round_trip() {
    let Ok(v) = std::env::var("MAILCRAFT_REAL_SMIME") else {
        eprintln!("MAILCRAFT_REAL_SMIME isn't set: skipped");
        return;
    };
    let mut parts = v.split_whitespace();
    let cfg = |a: Option<&str>| {
        let (host, port) = a.and_then(|a| a.rsplit_once(':')).unwrap();
        ServerConfig { host: host.into(), port: port.parse().unwrap(), security: Security::None }
    };
    let (smtp, imap) = (cfg(parts.next()), cfg(parts.next()));
    let data = |n: &str| std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../smime/testdata").join(n)).unwrap();
    let pem = |n: &str| der::pem::decode_vec(&data(n)).unwrap().1;
    let riley = "riley@studio.example";
    let store = crate::fake::store("real-smime");
    let mut pool = Pool::new(store.clone(), std::sync::Arc::new(MemoryCredentials::default()));
    let account =
        AccountConfig { id: "r1".into(), email: riley.into(), username: riley.into(), imap: imap.clone(), smtp: smtp.clone(), ..Default::default() };
    // The certificate, imported: its key into the keychain (here, memory).
    pool.submit(Job::ImportIdentity { account: account.clone(), data: data("riley-legacy.p12"), password: Secret("secret".into()) });
    let seen = until(&pool, 30, "the certificate", |e| matches!(e, Event::Identity { .. }));
    assert!(matches!(seen.last(), Some(Event::Identity { error: None, chain, .. }) if chain.len() == 2), "{seen:?}");
    // Ana's encrypted mail, as OpenSSL wrote it, delivered to Riley.
    let tag = std::process::id();
    let from_ana = String::from_utf8(data("encrypted-cbc.eml")).unwrap().replace("Subject: Encrypted CBC", &format!("Subject: From Ana {tag}"));
    crate::smtp::send(&smtp, "ana@posteo.example", "pw", "ana@posteo.example", &[riley.to_string()], from_ana.as_bytes()).unwrap();
    // Riley's reply, signed and encrypted to Ana and to Riley (the Sent copy).
    let reply = format!(
        "From: Riley <{riley}>\r\nTo: ana@posteo.example, {riley}\r\nSubject: Reply {tag}\r\nDate: Sat, 24 Oct 2026 10:00:00 +0000\r\nMessage-ID: <r{tag}@studio.example>\r\n\
MIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\nApproved =E2=9C=93, thanks.\r\n"
    );
    pool.submit(Job::Protect {
        account: account.clone(),
        id: 2,
        raw: mailcraft_model::Private(reply.into_bytes()),
        sign: true,
        encrypt_to: vec![pem("ana.pem"), pem("riley.pem")],
        gcm: false,
        hide_subject: false,
    });
    let seen = until(&pool, 30, "the protected reply", |e| matches!(e, Event::Protected { .. }));
    let Some(Event::Protected { raw: Some(sealed), error: None, .. }) = seen.last() else { panic!("{seen:?}") };
    crate::smtp::send(&smtp, riley, "pw", riley, &["ana@posteo.example".to_string(), riley.to_string()], &sealed.0).unwrap();
    // Both, as the server delivered them to Riley.
    let mut c = crate::imap::Client::connect(&imap).unwrap();
    c.login(riley, "pw").unwrap();
    c.select("INBOX", true).unwrap();
    // A real server delivers a moment after it accepts (Postfix to Dovecot); GreenMail at once.
    let mut raw_of = |subject: &str| -> Vec<u8> {
        let end = Instant::now() + Duration::from_secs(30);
        let uid = loop {
            c.select("INBOX", true).unwrap();
            if let Some(u) = c.uid_search(&format!("SUBJECT \"{subject}\"")).unwrap().last().copied() {
                break u;
            }
            assert!(Instant::now() < end, "{subject} wasn't delivered");
            std::thread::sleep(Duration::from_millis(300));
        };
        c.uid_fetch(&uid.to_string(), "BODY.PEEK[]", "").unwrap().into_iter().find_map(|f| f.body).unwrap()
    };
    let key = |uid: u32| mailcraft_model::MessageKey { account: "r1".into(), folder: "INBOX".into(), validity: 1, uid };
    for (n, subject) in [(1, format!("From Ana {tag}")), (2, format!("Reply {tag}"))] {
        let raw = raw_of(&subject);
        let mailcraft_mime::smime::Protection::Encrypted(cms) = mailcraft_mime::smime::protection(&raw) else { panic!("{subject}: not encrypted") };
        pool.submit(Job::Decrypt { account: account.clone(), key: key(n), cms });
        let seen = until(&pool, 30, "decrypted", |e| matches!(e, Event::Decrypted { key: k, .. } if k.uid == n));
        let Some(Event::Decrypted { content: Some(entity), error: None, .. }) = seen.last() else { panic!("{subject}: {seen:?}") };
        let inner = mailcraft_mime::smime::with_entity(&raw, &entity.0);
        let text = match mailcraft_mime::smime::protection(&inner) {
            mailcraft_mime::smime::Protection::Signed { content, signature } => {
                let (signed, _) = mailcraft_smime::verify(&signature, Some(&content)).unwrap_or_else(|e| panic!("{subject}: {e}"));
                assert_eq!(
                    mailcraft_smime::cert::check(&signed.signer, &signed.certs, &[pem("ca.pem")], 1_792_900_000, riley).unwrap(),
                    vec![],
                    "{subject}"
                );
                mailcraft_mime::parse(&mailcraft_mime::smime::with_entity(&inner, &content)).unwrap().text
            }
            _ => mailcraft_mime::parse(&inner).unwrap().text,
        };
        eprintln!("{subject}: {}", text.trim());
        assert!(text.contains(if n == 1 { "the proof is approved" } else { "Approved \u{2713}, thanks." }), "{subject}: {text}");
        if n == 2
            && let Some(dir) = std::env::var_os("MAILCRAFT_SMIME_OUT")
        {
            std::fs::write(std::path::Path::new(&dir).join("mc-delivered.eml"), &raw).unwrap();
        }
    }
    let _ = std::fs::remove_dir_all(store.root());
}

/// Issues #6 and #7 through a real mail server and a real calendar server
/// (`MAILCRAFT_REAL_DAV="127.0.0.1:3143 http://127.0.0.1:5232/zoe@posteo.example/work/ zoe@posteo.example app-pw"`:
/// GreenMail's IMAP, any password; Radicale, its own password; a calendar "Work" with "Posteo
/// planning" in it and an address book with Ana Lima). The calendar address is given by hand as a
/// Posteo user copies it (one calendar's), the contacts address as the server's root, the
/// password the calendar server's own: both are found and read. A wrong one is said. Then a
/// renewed certificate: the old one, imported after, is kept for reading, and mail sent to it
/// opens with its key.
#[test]
#[ignore]
fn real_calendar_by_hand_and_a_renewed_certificate() {
    let Ok(v) = std::env::var("MAILCRAFT_REAL_DAV") else {
        eprintln!("MAILCRAFT_REAL_DAV isn't set: skipped");
        return;
    };
    let p: Vec<&str> = v.split_whitespace().collect();
    let [imap, calendar, user, password] = p.as_slice() else { panic!("MAILCRAFT_REAL_DAV: imap calendar user password") };
    let (host, port) = imap.rsplit_once(':').unwrap();
    let imap = ServerConfig { host: host.into(), port: port.parse().unwrap(), security: Security::None };
    let root = calendar.split("/zoe").next().unwrap().to_string() + "/";
    let store = crate::fake::store("real-dav");
    let mut pool = Pool::new(store.clone(), std::sync::Arc::new(MemoryCredentials::default()));
    let account = |id: &str| AccountConfig {
        id: id.into(),
        email: (*user).into(),
        username: (*user).into(),
        imap: imap.clone(),
        smtp: imap.clone(),
        dav: Some(mailcraft_model::DavSettings { caldav: (*calendar).into(), carddav: root.clone(), username: Some((*user).into()) }),
        ..Default::default()
    };
    let a = account("d1");
    pool.submit(Job::DavPassword { account: a.clone(), password: Secret((*password).into()) });
    pool.submit(Job::Start { account: a.clone(), password: Some(Secret("mail-pw".into())) });
    let seen = until(&pool, 60, "calendars and contacts", |e| matches!(e, Event::DavSynced { account, .. } if account == "d1"));
    let Some(Event::DavSynced { sync, .. }) = seen.last() else { panic!() };
    assert!(sync.errors.is_empty(), "{:?}", sync.errors);
    let home = calendar.trim_end_matches('/').rsplit_once('/').unwrap().0.to_string() + "/";
    assert_eq!(sync.caldav_home.as_deref().map(|h| h.replace("%40", "@")), Some(home), "one calendar's address leads to its home");
    assert!(sync.calendars.as_ref().is_some_and(|c| c.iter().any(|c| c.name == "Work")), "{:?}", sync.calendars);
    assert!(sync.events.iter().any(|e| e.event.summary == "Posteo planning"), "{:?}", sync.events);
    assert!(sync.cards.iter().any(|c| c.card.display() == "Ana Lima"), "{:?}", sync.cards);
    // The wrong password for the calendar server: said, not taken for "none here".
    let b = account("d2");
    pool.submit(Job::DavPassword { account: b.clone(), password: Secret("wrong".into()) });
    pool.submit(Job::Start { account: b, password: Some(Secret("mail-pw".into())) });
    let seen =
        until(&pool, 60, "the refusal", |e| matches!(e, Event::Notice { account, message } if account == "d2" && message.contains("calendar")));
    eprintln!("{:?}", seen.last());

    // A renewed certificate: the new one first, then last year's, kept for reading.
    let data = |n: &str| std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../smime/testdata").join(n)).unwrap();
    for name in ["riley-aes.p12", "riley-2025.p12"] {
        pool.submit(Job::ImportIdentity { account: a.clone(), data: data(name), password: Secret("secret".into()) });
        let seen = until(&pool, 30, "the certificate", |e| matches!(e, Event::Identity { .. }));
        let Some(Event::Identity { error: None, older, .. }) = seen.last() else { panic!("{seen:?}") };
        assert_eq!(older.len(), usize::from(name == "riley-2025.p12"), "{name}");
    }
    let old = mailcraft_smime::read_p12(&data("riley-2025.p12"), "secret").unwrap();
    let body = b"Content-Type: text/plain\r\n\r\nSent before the renewal.\r\n";
    let cms = mailcraft_smime::encrypt(body, std::slice::from_ref(&old.chain[0]), mailcraft_smime::Cipher::Aes256Gcm, &mut rand_core::OsRng).unwrap();
    let key = mailcraft_model::MessageKey { account: "d1".into(), folder: "INBOX".into(), validity: 1, uid: 9 };
    pool.submit(Job::Decrypt { account: a, key, cms });
    let seen = until(&pool, 30, "decrypted", |e| matches!(e, Event::Decrypted { .. }));
    let Some(Event::Decrypted { content: Some(c), error: None, .. }) = seen.last() else { panic!("{seen:?}") };
    assert_eq!(c.0, body);
    let _ = std::fs::remove_dir_all(store.root());
}

/// Issue #32 on a real CalDAV server (Radicale, Stalwart): an event made private, out of the
/// office, is stored with `CLASS:PRIVATE`, `TRANSP:OPAQUE` and Outlook's busy status, and read
/// back so; one another calendar wrote "show time only" (as Thunderbird does, with its own
/// lines) keeps its classification and lines through an edit here; made public and free, its
/// `CLASS` goes and `TRANSP` says transparent. Then the calendar's search (issue #33) finds it
/// in what was read.
///
/// ```text
/// MAILCRAFT_REAL_CALDAV="http://127.0.0.1:21432/riley/ riley pw" \
///   cargo test -p mailcraft-net real_caldav_private -- --ignored --nocapture
/// ```
#[test]
#[ignore]
fn real_caldav_private_and_show_as() {
    use mailcraft_model::ical::{EventQuery, Privacy, ShowAs, privacy, set_privacy, set_show_as, show_as};
    use mailcraft_model::pim::{Pim, StoredEvent};
    let Ok(v) = std::env::var("MAILCRAFT_REAL_CALDAV") else {
        eprintln!("MAILCRAFT_REAL_CALDAV isn't set: skipped");
        return;
    };
    let p: Vec<&str> = v.split_whitespace().collect();
    let [url, user, password] = p.as_slice() else { panic!("MAILCRAFT_REAL_CALDAV: url user password") };
    let dav = crate::dav::Dav::new(user, password);
    let at = crate::dav::Where { email: user, username: "", others: &[], imap_host: "127.0.0.1", caldav: Some(url), carddav: None, mails: true };
    let zone = mailcraft_model::zone::Zone::Fixed(0);
    let now = 1_792_000_000;
    // Each sync merged as the engine does, roughly: what the server has, what was pushed.
    let merge = |pim: &mut Pim, s: mailcraft_model::pim::DavSync| {
        assert!(s.errors.is_empty(), "{:?}", s.errors);
        pim.caldav_home = s.caldav_home.clone();
        pim.carddav_home = Some(String::new());
        pim.dav_checked = Some(s.checked);
        if let Some(c) = s.calendars.clone() {
            pim.calendars = c;
        }
        for c in &s.synced_calendars {
            pim.events.retain(|e| &e.calendar != c || s.kept.contains(&e.href));
        }
        pim.events.retain(|e| !s.pushed_events.iter().any(|x| x.event.uid == e.event.uid));
        pim.events.extend(s.pushed_events.clone());
        for e in s.events {
            pim.events.retain(|x| !(x.href == e.href && x.event.uid == e.event.uid));
            pim.events.push(e);
        }
        pim.deleted.clear();
    };
    let mut pim = Pim::default();
    let s = crate::dav::sync(&dav, &at, &pim, now, zone).unwrap();
    merge(&mut pim, s);
    let cal = pim.calendars.iter().find(|c| !c.read_only).map(|c| c.id.clone()).expect("a calendar to write to");
    eprintln!("calendar: {cal}");
    let raw = |pim: &Pim, uid: &str| -> String {
        let href = pim.events.iter().find(|e| e.event.uid == uid).map(|e| e.href.clone()).unwrap();
        let got = dav.multiget(crate::dav::Kind::Calendars, &cal, &[href]).unwrap();
        got.first().map(|g| g.2.clone()).unwrap_or_default()
    };
    // Made here: private, out of office.
    let uid = format!("mc31e-private-{}@mailcraft", std::process::id());
    let mut e = mailcraft_model::ical::CalEvent {
        uid: uid.clone(),
        summary: "Doctor".into(),
        location: "Clinic 4".into(),
        start: now,
        end: now + 3600,
        ..Default::default()
    };
    set_privacy(&mut e, Privacy::Private);
    set_show_as(&mut e, ShowAs::OutOfOffice);
    pim.events.push(StoredEvent { calendar: cal.clone(), event: e, dirty: true, ..Default::default() });
    let s = crate::dav::sync(&dav, &at, &pim, now, zone).unwrap();
    merge(&mut pim, s);
    let stored = raw(&pim, &uid);
    eprintln!("--- stored:\n{stored}");
    for line in ["CLASS:PRIVATE", "TRANSP:OPAQUE", "X-MICROSOFT-CDO-BUSYSTATUS:OOF"] {
        assert!(stored.contains(line), "{line} in {stored}");
    }
    pim.calendars.iter_mut().for_each(|c| c.ctag.clear());
    let s = crate::dav::sync(&dav, &at, &pim, now, zone).unwrap();
    merge(&mut pim, s);
    let back = pim.events.iter().find(|e| e.event.uid == uid).unwrap();
    assert_eq!((privacy(&back.event), show_as(&back.event)), (Privacy::Private, ShowAs::OutOfOffice));

    // Written by another calendar: show time only, with lines of its own.
    let other = format!("mc31e-other-{}@mailcraft", std::process::id());
    let ics = format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Another calendar//EN\r\nBEGIN:VEVENT\r\nUID:{other}\r\nDTSTAMP:20261020T080000Z\r\n\
         DTSTART:20261022T090000Z\r\nDTEND:20261022T100000Z\r\nSUMMARY:Board meeting\r\nLOCATION:Room 2.14\r\nCLASS:CONFIDENTIAL\r\n\
         TRANSP:TRANSPARENT\r\nX-MOZ-GENERATION:4\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
    );
    let href = format!("{}/{}.ics", cal.trim_end_matches('/'), other.replace('@', "-"));
    dav.put(&href, "text/calendar; charset=utf-8", &ics, None).unwrap();
    pim.calendars.iter_mut().for_each(|c| c.ctag.clear());
    let s = crate::dav::sync(&dav, &at, &pim, now, zone).unwrap();
    merge(&mut pim, s);
    let theirs = pim.events.iter().find(|e| e.event.uid == other).cloned().unwrap();
    assert_eq!((privacy(&theirs.event), show_as(&theirs.event)), (Privacy::Confidential, ShowAs::Free));
    // The calendar's search finds both, offline, in what was read.
    let found: Vec<&str> =
        pim.events.iter().filter(|e| EventQuery::parse("is:private").matches(&e.event)).map(|e| e.event.summary.as_str()).collect();
    assert!(found.contains(&"Doctor") && found.contains(&"Board meeting"), "{found:?}");
    assert!(pim.events.iter().any(|e| EventQuery::parse("location:2.14").matches(&e.event) && e.event.uid == other));
    // Edited here (its title): the classification and the other calendar's lines stay.
    for x in pim.events.iter_mut().filter(|x| x.event.uid == other) {
        x.event.summary = "Board meeting (moved)".into();
        x.event.sequence += 1;
        x.dirty = true;
    }
    let s = crate::dav::sync(&dav, &at, &pim, now, zone).unwrap();
    merge(&mut pim, s);
    let stored = raw(&pim, &other);
    eprintln!("--- edited:\n{stored}");
    assert!(stored.contains("SUMMARY:Board meeting (moved)"));
    assert_eq!(stored.matches("CLASS:").count(), 1, "{stored}");
    assert!(stored.contains("CLASS:CONFIDENTIAL") && stored.contains("X-MOZ-GENERATION:4") && stored.contains("TRANSP:TRANSPARENT"), "{stored}");
    // Made public and free here.
    for x in pim.events.iter_mut().filter(|x| x.event.uid == other) {
        set_privacy(&mut x.event, Privacy::Public);
        set_show_as(&mut x.event, ShowAs::Free);
        x.dirty = true;
    }
    for x in pim.events.iter_mut().filter(|x| x.event.uid == uid) {
        set_privacy(&mut x.event, Privacy::Public);
        set_show_as(&mut x.event, ShowAs::Free);
        x.dirty = true;
    }
    let s = crate::dav::sync(&dav, &at, &pim, now, zone).unwrap();
    merge(&mut pim, s);
    let stored = raw(&pim, &uid);
    eprintln!("--- public and free:\n{stored}");
    assert!(!stored.contains("CLASS:") && stored.contains("TRANSP:TRANSPARENT") && stored.contains("X-MICROSOFT-CDO-BUSYSTATUS:FREE"), "{stored}");
    assert_eq!(stored.matches("TRANSP:").count(), 1, "{stored}");
    // An invitation as Outlook sends it (the guest's copy tentative, meant to be busy), accepted
    // here: busy on the server, Outlook's other lines kept.
    let invite = format!("mc31e-invite-{}@mailcraft", std::process::id());
    let ics = format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Another calendar//EN\r\nBEGIN:VEVENT\r\nUID:{invite}\r\nDTSTAMP:20261020T080000Z\r\n\
         DTSTART:20261023T090000Z\r\nDTEND:20261023T100000Z\r\nSUMMARY:Planning\r\nTRANSP:OPAQUE\r\n\
         X-MICROSOFT-CDO-BUSYSTATUS:TENTATIVE\r\nX-MICROSOFT-CDO-INTENDEDSTATUS:BUSY\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
    );
    let href = format!("{}/{}.ics", cal.trim_end_matches('/'), invite.replace('@', "-"));
    dav.put(&href, "text/calendar; charset=utf-8", &ics, None).unwrap();
    pim.calendars.iter_mut().for_each(|c| c.ctag.clear());
    let s = crate::dav::sync(&dav, &at, &pim, now, zone).unwrap();
    merge(&mut pim, s);
    for x in pim.events.iter_mut().filter(|x| x.event.uid == invite) {
        assert_eq!(show_as(&x.event), ShowAs::Tentative);
        mailcraft_model::ical::show_as_answered(&mut x.event, "ACCEPTED");
        x.dirty = true;
    }
    let s = crate::dav::sync(&dav, &at, &pim, now, zone).unwrap();
    merge(&mut pim, s);
    let stored = raw(&pim, &invite);
    eprintln!("--- accepted:\n{stored}");
    assert!(stored.contains("X-MICROSOFT-CDO-BUSYSTATUS:BUSY") && stored.contains("X-MICROSOFT-CDO-INTENDEDSTATUS:BUSY"), "{stored}");
    assert_eq!(stored.matches("BUSYSTATUS").count(), 1, "{stored}");
    // Cleaned up.
    for u in [&uid, &other, &invite] {
        let e = pim.events.iter().find(|e| &e.event.uid == u).unwrap();
        dav.delete(&e.href, "").unwrap();
    }
}

/// Categories as keywords on a real server (issue #28), in a folder another user shares:
/// Thunderbird's tags (`r=c3=bcckfrage`, `to=20do`) and SOGo's (`$label6`) come down as they are,
/// one set here goes up as it is; a user who may only read the folder (PERMANENTFLAGS without
/// `\*`, which Dovecot answers a STORE to with OK and forgets) is refused.
///
/// ```text
/// MAILCRAFT_REAL_KEYWORDS="127.0.0.1:1143 bob pw shared/alice dave" \
///   cargo test -p mailcraft-net real_keywords -- --ignored --nocapture
/// ```
///
/// `shared/alice` holds a message with those three keywords; `bob` may write to it (`lrwsite`),
/// `dave` only read (`lrs`); every password is the same.
#[test]
#[ignore]
fn real_keywords_in_a_shared_folder() {
    let Some((imap, user, pass, rest)) = server("MAILCRAFT_REAL_KEYWORDS") else {
        eprintln!("MAILCRAFT_REAL_KEYWORDS isn't set: skipped");
        return;
    };
    let (Some(folder), Some(reader)) = (rest.first().cloned(), rest.get(1).cloned()) else {
        panic!("MAILCRAFT_REAL_KEYWORDS: the shared folder and the read-only user are missing");
    };
    let store = crate::fake::store("real-keywords");
    let mut pool = Pool::new(store.clone(), std::sync::Arc::new(MemoryCredentials::default()));
    let start = |pool: &mut Pool, id: &str, who: &str| {
        let account = AccountConfig {
            id: id.into(),
            email: who.into(),
            username: who.into(),
            imap: imap.clone(),
            smtp: ServerConfig { host: "127.0.0.1".into(), port: 1, security: Security::None },
            ..Default::default()
        };
        pool.submit(Job::Start { account, password: Some(Secret(pass.clone())) });
        until(pool, 120, "online", |e| matches!(e, Event::Status { account, status: SyncStatus::Online { .. } } if account == id));
    };
    start(&mut pool, "w", &user);
    let m = store.load_folder("w", &folder).ok().and_then(|f| f.messages.into_iter().next()).expect("the shared folder's message");
    eprintln!("came down: {:?}", m.flags.keywords);
    for k in ["r=c3=bcckfrage", "to=20do", "$label6"] {
        assert!(m.flags.has(k), "{k} in {:?}", m.flags.keywords);
    }

    // One set here, one taken off: the server has them so for every client.
    pool.submit(Job::Apply { account: "w".into(), op: Op::SetKeyword { keys: vec![m.key.clone()], keyword: "$label7".into(), on: true } });
    pool.submit(Job::Apply { account: "w".into(), op: Op::SetKeyword { keys: vec![m.key.clone()], keyword: "to=20do".into(), on: false } });
    let mut other = crate::imap::Client::connect(&imap).unwrap();
    other.login(&user, &pass).unwrap();
    let end = Instant::now() + Duration::from_secs(30);
    loop {
        other.select(&folder, true).unwrap();
        let f: Vec<String> =
            other.uid_fetch(&m.key.uid.to_string(), "FLAGS", "").unwrap().into_iter().flat_map(|x| x.flags.unwrap_or_default()).collect();
        let has = |k: &str| f.iter().any(|x| x.eq_ignore_ascii_case(k));
        if has("$label7") && !has("to=20do") {
            eprintln!("on the server: {:?}", f);
            break;
        }
        assert!(Instant::now() < end, "not on the server: {f:?}");
        let _ = pool.wait(Duration::from_millis(200));
    }

    // The server searched for a Thunderbird keyword (it was left out of the search before).
    let q = "keyword:r=c3=bcckfrage";
    pool.submit(Job::SearchServer { account: "w".into(), folders: vec![folder.clone()], query: q.into(), search: 1 });
    let mut found = Vec::new();
    until(&pool, 60, "the search", |e| match e {
        Event::ServerSearched { query, keys, done, .. } if query == q => {
            found.extend(keys.iter().cloned());
            *done
        }
        _ => false,
    });
    assert_eq!(found, std::slice::from_ref(&m.key));

    // Read-only: refused, not taken and forgotten.
    start(&mut pool, "r", &reader);
    let mine = store.load_folder("r", &folder).ok().and_then(|f| f.messages.into_iter().next()).expect("the reader's copy");
    pool.submit(Job::Apply {
        account: "r".into(),
        op: Op::SetKeyword { keys: vec![mine.key.clone()], keyword: "dave=20was=20here".into(), on: true },
    });
    let seen = until(&pool, 60, "the refusal", |e| matches!(e, Event::OpFailed { account, .. } if account == "r"));
    eprintln!("refused: {:?}", seen.last());
    // Taking one off too: Dovecot answers the reader's `-FLAGS` with OK and keeps it.
    pool.submit(Job::Apply { account: "r".into(), op: Op::SetKeyword { keys: vec![mine.key.clone()], keyword: "$label6".into(), on: false } });
    let seen = until(&pool, 60, "the refusal to take one off", |e| matches!(e, Event::OpFailed { account, .. } if account == "r"));
    eprintln!("refused: {:?}", seen.last());
    pool.submit(Job::Remove { account: "w".into() });
    pool.submit(Job::Remove { account: "r".into() });
    let _ = std::fs::remove_dir_all(store.root());
}
