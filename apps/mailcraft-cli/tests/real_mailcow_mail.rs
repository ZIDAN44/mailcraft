//! MailCraft against a real Mailcow (Dovecot, Postfix, SOGo) on this computer: setting an account
//! up, categories shared with SOGo webmail and other users (issue #28), and what an AI agent
//! reads through `mailcraft-cli mcp` (issue #255). Never a real account; run by hand:
//!
//! ```text
//! MAILCRAFT_REAL_MAILCOW="127.0.0.1:22143 127.0.0.1:22588 http://127.0.0.1:22080 \
//!   ann@mc31.test:<password> bob@mc31.test:<password> carol@mc31.test:<password>" \
//!   cargo test -p mailcraft-cli --test real_mailcow_mail -- --ignored --nocapture --test-threads 1
//! ```
//!
//! Mailcow from `mailcow-dockerized` (`generate_config.sh`, then in `mailcow.conf`: the ports
//! moved to 22000 and up on 127.0.0.1, `HTTP_REDIRECT=n`, `SKIP_LETS_ENCRYPT=y`, an `API_KEY`),
//! with what MailCraft needs to reach a server that has no real certificate, on this computer
//! only: `data/conf/dovecot/extra.conf` with `disable_plaintext_auth = no`, and a
//! `docker-compose.override.yml` publishing Postfix's port 588 (SOGo's own submission, which
//! takes a password without TLS) as 127.0.0.1:22588. Three mailboxes of one domain made with
//! Mailcow's API (`POST /api/v1/add/domain`, `/add/mailbox`). SOGo's web endpoints are reached
//! signed in through Mailcow's own sign-in page (see `support/mailcow.rs`). The tests change only
//! what they make, and can run again.

#[path = "support/mailcow.rs"]
mod mailcow;

use mailcow::{App, Mailcow, http, message};
use mailcraft_engine::Session;
use mailcraft_engine::model::{Event, FolderRole, Security, SyncStatus};
use serde_json::json;

/// Mailcow's autoconfig file (it serves Thunderbird's and Outlook's), read as MailCraft reads
/// it when an address is typed: the encrypted ports first. Then the calendar and contacts
/// found from Mailcow's own address, as `/.well-known/caldav` leads to SOGo.
#[test]
#[ignore = "needs a Mailcow on this computer in MAILCRAFT_REAL_MAILCOW"]
fn real_mailcow_account_setup() {
    let Some(m) = Mailcow::from_env() else { return };
    let (email, _) = m.user(0);
    let domain = email.split('@').nth(1).unwrap().to_string();
    // The domain's names aren't in DNS here: Mailcow is asked with the name it would be asked by.
    let web = m.web.clone();
    let mut fetch = |url: &str| -> Option<String> {
        let rest = url.strip_prefix("https://")?;
        let (host, path) = rest.split_once('/')?;
        if !host.ends_with(&domain) {
            return None;
        }
        let r = http("GET", &format!("{web}/{path}"), &[("Host", host)], b"");
        eprintln!("{url}: {}", r.status);
        (r.status == 200).then(|| r.text())
    };
    let mut mx = |_: &str| Vec::new();
    let mut srv = |_: &str| None;
    let found =
        mailcraft_net::autoconfig::discover_with(email, &mut mailcraft_net::autoconfig::Lookups { fetch: &mut fetch, mx: &mut mx, srv: &mut srv })
            .expect("Mailcow's autoconfig");
    let c = &found.config;
    eprintln!("found at {}: imap {:?} smtp {:?} pop {:?} as {}", found.source, c.imap, c.smtp, c.pop, c.username);
    assert_eq!(found.source, domain);
    assert_eq!((c.imap.port, c.imap.security), (22993, Security::Tls), "IMAPS before STARTTLS");
    assert_eq!((c.smtp.port, c.smtp.security), (22465, Security::Tls), "SMTPS before submission");
    assert_eq!(c.username, email);
    assert!(c.imap.host.starts_with("mail."), "{}", c.imap.host);

    // The account itself (without TLS, as the test server has no real certificate), its
    // folders by their special uses, and SOGo's calendar and address book from the server's
    // address alone.
    let (mut app, id) = App::new(&m, 0, "setup");
    app.pump(120, "online", |_, e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    app.s.run("account.dav", &json!({"account": id, "caldav": format!("{}/", m.web), "carddav": format!("{}/", m.web)})).unwrap();
    let sync = app.sync_dav(&id);
    assert!(sync.errors.is_empty(), "{:?}", sync.errors);
    eprintln!("homes: {:?} {:?}", sync.caldav_home, sync.carddav_home);
    assert_eq!(sync.caldav_home.as_deref(), Some(format!("{}/SOGo/dav/{email}/Calendar/", m.web).as_str()));
    assert_eq!(sync.carddav_home.as_deref(), Some(format!("{}/SOGo/dav/{email}/Contacts/", m.web).as_str()));
    let calendars = sync.calendars.clone().unwrap_or_default();
    eprintln!("calendars: {:?}", calendars.iter().map(|c| (&c.name, &c.id, c.read_only, c.foreign)).collect::<Vec<_>>());
    assert!(calendars.iter().any(|c| c.id.ends_with("/Calendar/personal/") && !c.read_only && !c.foreign), "SOGo's personal calendar");
    let books = sync.books.clone().unwrap_or_default();
    assert!(books.iter().any(|b| b.id.ends_with("/Contacts/personal/") && !b.read_only), "{books:?}");
    assert!(app.s.pim[&id].default_calendar().is_some_and(|c| c.id.ends_with("/Calendar/personal/")));
    let roles = app.s.account(&id).map(|a| a.folders.iter().map(|f| (f.path.clone(), f.role)).collect::<Vec<_>>()).unwrap_or_default();
    eprintln!("folders: {roles:?}");
    for role in [FolderRole::Inbox, FolderRole::Sent, FolderRole::Drafts, FolderRole::Trash, FolderRole::Junk, FolderRole::Archive] {
        assert!(roles.iter().any(|(_, r)| *r == role), "{role:?} in {roles:?}");
    }
}

/// Issue #28 with SOGo webmail and Dovecot as Mailcow sets them up: SOGo's labels come down as
/// MailCraft's categories (its five as the defaults, others shown as they are), categories set
/// here show in SOGo, a Thunderbird keyword survives SOGo touching the message, and in ann's
/// Inbox shared through SOGo, bob (who may change it) and carol (who may only read it) see the
/// same; carol's change is refused, not taken and forgotten.
#[test]
#[ignore = "needs a Mailcow on this computer in MAILCRAFT_REAL_MAILCOW"]
fn real_mailcow_categories_with_sogo() {
    let Some(m) = Mailcow::from_env() else { return };
    let tag = std::process::id();
    let (ann, _) = m.user(0);
    let (bob, _) = m.user(1);
    let (carol, _) = m.user(2);
    let sogo = m.sogo(0);
    // SOGo's labels: its five, and two of the user's own (a number on, as organisations do,
    // and a name), kept in SOGo's preferences.
    let labels = sogo.add_labels(&[("$label6", "Review", "#aa00aa"), ("projekt", "Projekt X", "#00aaaa")]);
    eprintln!("SOGo's labels: {labels}");
    let subject = format!("Labels {tag}");
    m.send(1, &[ann], &message(&format!("Bob <{bob}>"), ann, &subject, &format!("labels-{tag}@mc31.test")));
    let mut imap = m.imap_as(0);
    let end = std::time::Instant::now() + std::time::Duration::from_secs(60);
    let uid = loop {
        if let Some(x) = imap.messages("INBOX").into_iter().find(|x| x.subject == subject) {
            break x.uid;
        }
        assert!(std::time::Instant::now() < end, "not delivered");
        std::thread::sleep(std::time::Duration::from_secs(1));
    };
    // Labelled in SOGo webmail.
    for l in ["$label1", "$label6", "projekt"] {
        sogo.label("folderINBOX", &[uid], l, true);
    }
    eprintln!("on the server after SOGo: {:?}", imap.flags("INBOX", &subject));

    let (mut app, id) = App::new(&m, 0, "categories");
    app.pump(120, "online", |s, _| s.folder_state(&id, "INBOX").messages.iter().any(|x| x.subject == subject));
    app.s.run("folder.open", &json!({"account": id, "folder": "INBOX"})).unwrap();
    let names = |app: &mut App| -> Vec<String> {
        let k = app.s.folder_state(&id, "INBOX").messages.iter().find(|x| x.subject == subject).map(|x| x.key.clone()).unwrap();
        let s: &Session = &app.s;
        s.summary(&k).map(|x| s.categories_of(x).into_iter().map(|c| c.name).collect()).unwrap_or_default()
    };
    let shown = names(&mut app);
    eprintln!("MailCraft shows: {shown:?}");
    assert_eq!(shown, ["Important", "$label6", "projekt"], "SOGo's first label is MailCraft's Important; the others as they are");
    let unlisted = app.s.run("category.unlisted", &json!({})).unwrap();
    eprintln!("not in the list: {unlisted}");
    for k in ["$label6", "projekt"] {
        assert!(unlisted.as_array().unwrap().iter().any(|u| u["keyword"] == k), "{k}: {unlisted}");
    }
    // SOGo's $label6 named as there: the same keyword, MailCraft's name.
    app.s.run("category.create", &json!({"name": "Review", "keyword": "$label6", "color": 5})).unwrap();
    assert_eq!(names(&mut app), ["Important", "Review", "projekt"]);

    // Set here: Thunderbird's keyword for Rückfrage and SOGo's Work; Important taken off.
    app.s.run("category.create", &json!({"name": "Rückfrage", "color": 3})).unwrap();
    let key = app.s.folder_state(&id, "INBOX").messages.iter().find(|x| x.subject == subject).map(|x| x.key.to_id()).unwrap();
    app.s.run("list.open", &json!({"key": key})).unwrap();
    app.s.run("mail.categorize", &json!({"category": "Rückfrage"})).unwrap();
    app.s.run("mail.categorize", &json!({"category": "Work"})).unwrap();
    app.s.run("mail.categorize", &json!({"category": "Important", "on": false})).unwrap();
    let end = std::time::Instant::now() + std::time::Duration::from_secs(60);
    let flags = loop {
        app.settle(1);
        let f = imap.flags("INBOX", &subject);
        if f.iter().any(|x| x == "r=c3=bcckfrage") && f.iter().any(|x| x == "$label2") && !f.iter().any(|x| x == "$label1") {
            break f;
        }
        assert!(std::time::Instant::now() < end, "not on the server: {f:?}");
    };
    eprintln!("on the server after MailCraft: {flags:?}");
    // SOGo's message list shows them (its rows' `Flags`).
    let list = sogo.mail_list("folderINBOX");
    let heads: Vec<String> = list["headers"][0].as_array().unwrap().iter().map(|h| h.as_str().unwrap_or_default().to_string()).collect();
    let (fi, ui) = (heads.iter().position(|h| h == "Flags").unwrap(), heads.iter().position(|h| h == "uid").unwrap());
    let row = list["headers"].as_array().unwrap().iter().skip(1).find(|r| r[ui] == uid).cloned().unwrap();
    eprintln!("SOGo lists its flags as {}", row[fi]);
    for k in ["$label2", "$label6", "projekt", "r=c3=bcckfrage"] {
        assert!(row[fi].as_array().unwrap().iter().any(|f| f == k), "{k} in SOGo's {}", row[fi]);
    }

    // SOGo touches the message: read, flagged, moved to Archive and back (as its list's menu
    // does). The keywords stay, MailCraft still shows them.
    let r = sogo.get(&format!("Mail/0/folderINBOX/{uid}/markMessageRead"));
    assert!((200..300).contains(&r.status), "{}", r.text());
    sogo.post("Mail/0/folderINBOX/addOrRemoveLabel", &json!({"operation": "add", "msgUIDs": [uid], "flags": "\\Flagged"}));
    sogo.post("Mail/0/folderINBOX/moveMessages", &json!({"uids": [uid], "folder": "/0/folderArchive"}));
    let archived = imap.messages("Archive").into_iter().find(|x| x.subject == subject).expect("moved by SOGo");
    eprintln!("in Archive: {:?}", archived.flags);
    sogo.post("Mail/0/folderArchive/moveMessages", &json!({"uids": [archived.uid], "folder": "/0/folderINBOX"}));
    let back = imap.flags("INBOX", &subject);
    eprintln!("back in the Inbox: {back:?}");
    for k in ["r=c3=bcckfrage", "$label2", "$label6", "projekt", "\\Seen", "\\Flagged"] {
        assert!(back.iter().any(|f| f == k), "{k} after SOGo moved it: {back:?}");
    }
    app.pump(120, "the move seen", |s, _| {
        s.folder_state(&id, "INBOX").messages.iter().any(|x| x.subject == subject && x.flags.flagged && x.key.uid != uid)
    });
    assert_eq!(names(&mut app), ["Work", "Review", "Rückfrage", "projekt"]);
    // SOGo takes one off: gone here too.
    let now_uid = imap.messages("INBOX").into_iter().find(|x| x.subject == subject).map(|x| x.uid).unwrap();
    sogo.label("folderINBOX", &[now_uid], "projekt", false);
    app.pool.submit(mailcraft_engine::model::Job::SyncFolder { account: id.clone(), folder: "INBOX".into() });
    app.pump(60, "projekt taken off", |s, _| s.folder_state(&id, "INBOX").messages.iter().any(|x| x.subject == subject && !x.flags.has("projekt")));
    eprintln!("after SOGo took projekt off: {:?}", names(&mut app));

    // ann's Inbox shared in SOGo: bob may change it (SOGo's "modify" rights), carol only read.
    let rw = json!({"userCanReadMails": 1, "userCanMarkMailsRead": 1, "userCanWriteMails": 1, "userCanInsertMails": 1, "userCanPostMails": 1,
                    "userCanEraseMails": 1, "userCanExpungeFolder": 1, "userCanViewFolder": 1, "userCanCreateSubfolders": 0, "userCanRemoveFolder": 0, "userIsAdministrator": 0});
    let ro = json!({"userCanReadMails": 1, "userCanViewFolder": 1, "userCanMarkMailsRead": 0, "userCanWriteMails": 0, "userCanInsertMails": 0, "userCanPostMails": 0,
                    "userCanEraseMails": 0, "userCanExpungeFolder": 0, "userCanCreateSubfolders": 0, "userCanRemoveFolder": 0, "userIsAdministrator": 0});
    sogo.share("Mail/0/folderINBOX", bob, rw, false);
    sogo.share("Mail/0/folderINBOX", carol, ro, false);
    eprintln!("{}", imap.cmd("GETACL INBOX").lines().next().unwrap_or_default());
    let shared = format!("Shared/{ann}/INBOX");

    let (mut b, bid) = App::new(&m, 1, "categories");
    b.pump(120, "bob's copy of the shared Inbox", |s, _| s.folder_state(&bid, &shared).messages.iter().any(|x| x.subject == subject));
    b.s.run("folder.open", &json!({"account": bid, "folder": shared})).unwrap();
    let bk = b.s.folder_state(&bid, &shared).messages.iter().find(|x| x.subject == subject).map(|x| x.key.clone()).unwrap();
    let seen: Vec<String> = b.s.summary(&bk).map(|x| b.s.categories_of(x).into_iter().map(|c| c.name).collect()).unwrap_or_default();
    eprintln!("bob sees: {seen:?}");
    for want in ["Work", "$label6", "rückfrage"] {
        assert!(seen.iter().any(|s| s == want), "{want} in {seen:?}");
    }
    b.s.run("list.open", &json!({"key": bk.to_id()})).unwrap();
    b.s.run("mail.categorize", &json!({"keyword": "$label7"})).unwrap();
    let end = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while !imap.flags("INBOX", &subject).iter().any(|f| f == "$label7") {
        b.settle(1);
        assert!(std::time::Instant::now() < end, "bob's keyword didn't reach ann's Inbox");
    }
    eprintln!("bob's $label7 is on ann's message: {:?}", imap.flags("INBOX", &subject));

    let (mut c, cid) = App::new(&m, 2, "categories");
    c.pump(120, "carol's copy of the shared Inbox", |s, _| s.folder_state(&cid, &shared).messages.iter().any(|x| x.subject == subject));
    c.s.run("folder.open", &json!({"account": cid, "folder": shared})).unwrap();
    let ck = c.s.folder_state(&cid, &shared).messages.iter().find(|x| x.subject == subject).map(|x| x.key.clone()).unwrap();
    let seen: Vec<String> = c.s.summary(&ck).map(|x| c.s.categories_of(x).into_iter().map(|c| c.name).collect()).unwrap_or_default();
    eprintln!("carol sees: {seen:?}");
    assert!(seen.iter().any(|s| s == "$label7"), "{seen:?}");
    c.s.run("list.open", &json!({"key": ck.to_id()})).unwrap();
    let r = c.s.run("mail.categorize", &json!({"category": "Later"}));
    eprintln!("carol's Later: {r:?}");
    let mut refused = String::new();
    if r.is_ok() {
        c.pump(60, "the refusal", |_, e| match e {
            mailcraft_engine::model::Event::OpFailed { error, .. } => {
                refused = error.clone();
                true
            }
            _ => false,
        });
    } else {
        refused = format!("{r:?}");
    }
    eprintln!("carol is told: {refused}");
    assert!(!refused.is_empty());
    // Undone here once the folder is read again (the refusal makes the worker read it).
    c.pump(60, "Later taken back", |s, _| s.folder_state(&cid, &shared).messages.iter().any(|x| x.subject == subject && !x.flags.has("$label5")));
    assert!(!imap.flags("INBOX", &subject).iter().any(|f| f == "$label5"), "not on the server");
}

/// Everything with someone (issue #178), from real mail: ann writes to bob, carol on copy,
/// through Postfix, with a file, a link and her number in her signature; bob answers from
/// MailCraft. Bob's MailCraft puts the two together from his Inbox and Sent Items, with ann's
/// file and link (or, while her message isn't downloaded, says so), her number to offer, and
/// carol beside her (never bob himself).
#[test]
#[ignore = "needs a Mailcow on this computer in MAILCRAFT_REAL_MAILCOW"]
fn real_mailcow_everything_with_someone() {
    let Some(m) = Mailcow::from_env() else { return };
    let tag = std::process::id();
    let (ann, _) = m.user(0);
    let (bob, _) = m.user(1);
    let (carol, _) = m.user(2);
    let subject = format!("The cut {tag}");
    let raw = format!(
        "From: Ann Example <{ann}>\r\nTo: Bob <{bob}>\r\nCc: Carol <{carol}>\r\nSubject: {subject}\r\nMessage-ID: <cut-{tag}@mc31.test>\r\n\
         Date: Sat, 21 Nov 2026 09:00:00 +0000\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"b\"\r\n\r\n\
         --b\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nThe second cut is at https://cut.example/{tag} for you, with the rushes{tag}.\r\n\r\n\
         -- \r\nAnn Example\r\nTel +44 20 7946 0958\r\n\r\n\
         --b\r\nContent-Type: application/pdf; name=\"notes-{tag}.pdf\"\r\nContent-Disposition: attachment; filename=\"notes-{tag}.pdf\"\r\n\
         Content-Transfer-Encoding: base64\r\n\r\nJVBERi0xLjQK\r\n--b--\r\n"
    );
    m.send(0, &[bob, carol], raw.as_bytes());

    let (mut b, bid) = App::new(&m, 1, "person");
    let key = b.wait_for_mail(&bid, &subject);
    let ours = |a: &mailcraft_engine::PersonActivity| a.threads.iter().find(|t| t.subject == subject).cloned();
    let first = b.s.person_activity(ann);
    let file = format!("notes-{tag}.pdf");
    eprintln!("before it's opened: {} messages, {} not downloaded, files {:?}", first.message_count(), first.not_downloaded, first.files);
    assert!(ours(&first).is_some(), "her message, from the Inbox");
    assert!(first.not_downloaded > 0 || first.files.iter().any(|f| f.name == file), "a file not counted is said to be missing");

    // Read, and answered from MailCraft: the answer goes out and into Sent Items on Mailcow.
    b.s.run("list.open", &json!({"key": key.to_id()})).unwrap();
    b.pump(60, "the message read", |s, _| s.reading.as_ref().is_some_and(|r| r.parsed.is_some()));
    b.s.run("mail.reply", &json!({})).unwrap();
    b.s.run("compose.update", &json!({"body": "Looks good, Ann."})).unwrap();
    b.s.run("compose.send", &json!({})).unwrap();
    b.pump(60, "sent", |_, e| matches!(e, Event::Sent { .. }));
    let sent = b.s.account(&bid).and_then(|a| a.folder(FolderRole::Sent)).map(|f| f.path.clone()).expect("Sent Items");
    let answer = format!("Re: {subject}");
    let end = std::time::Instant::now() + std::time::Duration::from_secs(90);
    while !b.s.folder_state(&bid, &sent).messages.iter().any(|x| x.subject == answer) {
        assert!(std::time::Instant::now() < end, "the answer didn't reach Sent Items");
        b.pool.submit(mailcraft_engine::model::Job::SyncFolder { account: bid.clone(), folder: sent.clone() });
        b.settle(2);
    }

    let a = b.s.person_activity(&ann.to_uppercase());
    let thread = ours(&a).expect("the conversation");
    let subjects: Vec<&str> = thread.messages.iter().map(|x| x.subject.as_str()).collect();
    eprintln!("bob and ann: {} conversations; ours: {subjects:?}", a.threads.len());
    assert_eq!(subjects, [answer.as_str(), subject.as_str()], "his answer from Sent Items, the newest first");
    assert_eq!(a.name, "Ann Example");
    assert!(a.files.iter().any(|f| f.name == file && f.from_them && f.key == key), "{:?}", a.files);
    let link = format!("https://cut.example/{tag}");
    assert!(a.links.iter().any(|l| l.url == link), "{:?}", a.links);
    assert_eq!(a.found.phones, ["+44 20 7946 0958"], "her number, from her signature");
    assert!(a.people.iter().any(|p| p.email.eq_ignore_ascii_case(carol)), "{:?}", a.people);
    assert!(!a.people.iter().any(|p| p.email.eq_ignore_ascii_case(bob) || p.email.eq_ignore_ascii_case(ann)), "{:?}", a.people);
    // Narrowed by a word of her message's text alone (the other tests here share the run's
    // number): her message, read from its text (files and links go by their own names).
    let word = format!("rushes{tag}");
    let r = a.refined(&word);
    eprintln!("narrowed to {word}: {} conversations, files {:?}, links {:?}", r.threads.len(), r.files.len(), r.links.len());
    assert_eq!(r.threads.iter().map(|t| t.subject.as_str()).collect::<Vec<_>>(), [subject.as_str()]);
    assert_eq!(r.message_count(), 1, "her message, not his answer");
    // The command, as the CLI runs it.
    let v = b.s.run("person.activity", &json!({"email": ann, "query": format!("subject:\"{subject}\"")})).unwrap();
    eprintln!("person.activity: {v}");
    assert_eq!(v["threadCount"], 1);
    assert_eq!(v["threads"][0]["messages"].as_array().map(Vec::len), Some(2));
}

/// What an AI agent reads through `mailcraft-cli mcp` (issue #255), MailCraft closed: ann's mail
/// as synced from Mailcow, found by a word of its text and read whole, and still unread on the
/// server after. The CLI itself, as an assistant starts it: the account added and synced with it,
/// then its MCP server on stdin and stdout.
#[test]
#[ignore = "needs a Mailcow on this computer in MAILCRAFT_REAL_MAILCOW"]
fn real_mailcow_mail_read_through_mcp() {
    use std::io::{BufRead, Write};
    use std::process::{Command, Stdio};
    let Some(m) = Mailcow::from_env() else { return };
    let ((ann, password), (bob, _)) = (m.user(0), m.user(1));
    let tag = mailcow::now();
    let subject = format!("Mix notes {tag}");
    let word = format!("kestrel{tag}");
    let raw = format!(
        "From: {bob}\r\nTo: {ann}\r\nSubject: {subject}\r\nMessage-ID: <mcp-{tag}@mc31.test>\r\nDate: Sat, 31 Oct 2026 09:00:00 +0000\r\n\
         MIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nThe reel to send is {word}.\r\n"
    );
    m.send(1, &[ann], raw.as_bytes());
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("real-mcp-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let cli = |args: &[&str]| -> String {
        let out = Command::new(env!("CARGO_BIN_EXE_mailcraft-cli"))
            .arg("--data")
            .arg(&dir)
            .args(args)
            .env("MAILCRAFT_PASSWORD", password)
            .output()
            .unwrap();
        assert!(out.status.success(), "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    cli(&["add", "--email", ann, "--imap", &m.imap, "--imap-security", "none", "--smtp", &m.smtp, "--smtp-security", "none"]);
    // Delivery takes a moment.
    let end = std::time::Instant::now() + std::time::Duration::from_secs(120);
    loop {
        cli(&["sync", "--timeout", "60"]);
        if cli(&["search", &format!("subject:\"{subject}\"")]).contains(&subject) {
            break;
        }
        assert!(std::time::Instant::now() < end, "bob's message didn't arrive");
        std::thread::sleep(std::time::Duration::from_secs(2));
    }

    // Agents let in, as File › Options › AI agents saves it (MailCraft closed, nothing else does).
    let settings = |prefs: serde_json::Value| std::fs::write(dir.join("ui.json"), json!({"prefs": prefs}).to_string()).unwrap();
    settings(json!({"agents": true}));
    let mut mcp = Command::new(env!("CARGO_BIN_EXE_mailcraft-cli"))
        .arg("--data")
        .arg(&dir)
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = mcp.stdin.take().unwrap();
    let mut output = std::io::BufReader::new(mcp.stdout.take().unwrap());
    let mut ask = |id: u64, method: &str, params: serde_json::Value| -> serde_json::Value {
        writeln!(input, "{}", json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})).unwrap();
        let mut line = String::new();
        output.read_line(&mut line).unwrap();
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["id"], id, "{v}");
        v
    };
    let tool = |v: &serde_json::Value| -> serde_json::Value {
        assert!(v["result"]["isError"] != true, "{v}");
        serde_json::from_str(v["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
    };
    ask(1, "initialize", json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "1"}}));
    let accounts = tool(&ask(2, "tools/call", json!({"name": "list_accounts", "arguments": {}})));
    eprintln!("list_accounts: {accounts}");
    assert!(accounts.to_string().contains(ann), "{accounts}");
    let found = tool(&ask(3, "tools/call", json!({"name": "search_mail", "arguments": {"query": word}})));
    eprintln!("search_mail {word}: {found}");
    assert_eq!(found["found"], 1, "{found}");
    let key = found["messages"][0]["key"].as_str().unwrap().to_string();
    let read = tool(&ask(4, "tools/call", json!({"name": "read_message", "arguments": {"key": key}})));
    eprintln!("read_message: {read}");
    assert_eq!(read["subject"], subject.as_str());
    assert!(read["from"].as_str().unwrap().contains(bob) && read["text"].as_str().unwrap().contains(&word), "{read}");
    assert_eq!(read["unread"], true);
    // Kept from agents: the same server stays connected, and reads nothing of it now.
    let id = accounts[0]["account"].as_str().unwrap().to_string();
    settings(json!({"agents": true, "agentsHidden": [id]}));
    assert_eq!(tool(&ask(5, "tools/call", json!({"name": "search_mail", "arguments": {"query": word}})))["found"], 0);
    let refused = ask(6, "tools/call", json!({"name": "read_message", "arguments": {"key": key}}));
    assert_eq!(refused["result"]["isError"], true, "{refused}");
    drop(input);
    assert!(mcp.wait().unwrap().success());
    // Read by the agent, and nothing told the server.
    let flags = m.imap_as(0).flags("INBOX", &subject);
    assert!(!flags.iter().any(|f| f.eq_ignore_ascii_case("\\Seen")), "{flags:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// An organisation's profile (issue #245) against Mailcow: its servers for the domain make an
/// address and a password all an account needs; a locked signature filled from the person's
/// details, and the disclaimer, reach the other side in what's sent; another domain's account
/// isn't allowed; and with encrypted connections required, this test server (which has none)
/// is held back, said.
#[test]
#[ignore = "needs a Mailcow on this computer in MAILCRAFT_REAL_MAILCOW"]
fn real_mailcow_organisation_profile() {
    use mailcraft_engine::policy::Organisation;
    let Some(m) = Mailcow::from_env() else { return };
    let ((ann, password), (bob, _)) = (m.user(0), m.user(1));
    let domain = ann.split('@').nth(1).unwrap().to_string();
    let tag = mailcow::now();
    let profile = |encrypted: bool| {
        let v = json!({
            "organisation": "MC31",
            "requireEncryption": encrypted,
            "accounts": {"onlyOrganisationDomains": true,
                         "servers": {domain.clone(): {"imap": format!("{}:none", m.imap), "smtp": format!("{}:none", m.smtp)}}},
            "signature": {"name": "MC31", "text": "{name}\n{title}", "locked": true},
            "disclaimer": {"text": format!("Confidential to MC31 ({tag}).")}
        });
        Organisation::from_sources(vec![("test".into(), v, true)])
    };
    let mut a = App::open("org-ann", password);
    a.s.set_organisation(profile(false));
    assert!(a.s.run("account.add", &json!({"email": "someone@gmail.example", "password": "pw"})).is_err(), "only its domains");
    let id = a.add(json!({"email": ann, "password": password}));
    assert_eq!(a.s.account(&id).map(|x| format!("{}:{}", x.config.imap.host, x.config.imap.port)), Some(m.imap.clone()));
    a.pump(120, "online", |_, e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    a.s.run("account.details", &json!({"account": id, "title": "Sound editor"})).unwrap();
    let subject = format!("Mix review {tag}");
    a.s.run("mail.new", &json!({})).unwrap();
    a.s.run("compose.update", &json!({"to": bob, "subject": subject})).unwrap();
    a.s.run("compose.send", &json!({})).unwrap();
    a.pump(60, "sent", |_, e| matches!(e, Event::Sent { .. }));
    // On Mailcow, in bob's Inbox: the signature and the disclaimer, as sent.
    let end = std::time::Instant::now() + std::time::Duration::from_secs(90);
    let text = loop {
        let t = m.imap_as(1).newest_text("INBOX");
        if t.contains(&tag.to_string()) {
            break t;
        }
        assert!(std::time::Instant::now() < end, "the message didn't reach bob");
        std::thread::sleep(std::time::Duration::from_secs(2));
    };
    eprintln!("as bob got it: {text}");
    assert!(text.contains("Sound editor") && text.contains(&format!("Confidential to MC31 ({tag}).")), "{text}");
    // Encrypted connections required: this server has none, so the account is held back.
    a.s.set_organisation(profile(true));
    let config = a.s.account(&id).unwrap().config.clone();
    a.s.push_job(mailcraft_engine::model::Job::Start { account: config, password: None });
    assert!(a.s.take_jobs().is_empty());
    let status = format!("{:?}", a.s.account(&id).unwrap().status);
    assert!(status.contains("encrypted"), "{status}");
}
