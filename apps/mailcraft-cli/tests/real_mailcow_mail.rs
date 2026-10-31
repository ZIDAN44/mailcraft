//! MailCraft against a real Mailcow (Dovecot, Postfix, SOGo) on this computer: setting an account
//! up, and categories shared with SOGo webmail and other users (issue #28). Never a real
//! account; run by hand:
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
#[ignore]
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
#[ignore]
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
        if let Some(x) = imap.messages("INBOX").into_iter().find(|x| x.2 == subject) {
            break x.0;
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
    let archived = imap.messages("Archive").into_iter().find(|x| x.2 == subject).expect("moved by SOGo");
    eprintln!("in Archive: {:?}", archived.1);
    sogo.post("Mail/0/folderArchive/moveMessages", &json!({"uids": [archived.0], "folder": "/0/folderINBOX"}));
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
    let now_uid = imap.messages("INBOX").into_iter().find(|x| x.2 == subject).map(|x| x.0).unwrap();
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
