//! Engine tests: accounts of every kind, their settings and what's kept of them.

use super::*;

#[test]
fn jmap_accounts_need_no_imap_servers() {
    let store = temp("jmap-account");
    let mut s = Session::open(Some(store), mailcraft_model::dates::Clock::new(NOW, 0));
    let id = s
        .run("account.add", &json!({"email": "zoe@fastmail.example", "password": "pw", "jmap": " https://api.fastmail.example/jmap/session "}))
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    let a = s.account(&id).unwrap();
    assert_eq!(a.config.jmap.as_deref(), Some("https://api.fastmail.example/jmap/session"));
    assert!(a.config.imap.host.is_empty() && !a.is_local(), "a server account, not the sample");
    assert!(matches!(s.take_jobs().as_slice(), [Job::Start { account, .. }] if account.jmap.is_some()));
    // Saved and read back.
    let root = s.store.as_ref().unwrap().root().to_path_buf();
    let s2 = Session::open(Some(Store::open(&root).unwrap()), mailcraft_model::dates::Clock::new(NOW, 0));
    assert!(s2.account(&id).is_some_and(|a| a.config.jmap.is_some()));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn pop_accounts_download_and_can_start_from_a_data_file() {
    let store = temp("pop-account");
    let mut s = Session::open(Some(store), mailcraft_model::dates::Clock::new(NOW, 0));
    // The POP server is guessed from the address; a copy stays on the server unless asked.
    let id =
        s.run("account.add", &json!({"email": "zoe@example.org", "password": "pw", "pop": true})).unwrap()["account"].as_str().unwrap().to_string();
    let a = s.account(&id).unwrap();
    let pop = a.config.pop.clone().unwrap();
    assert_eq!((pop.host.as_str(), pop.port, pop.security), ("mail.example.org", 995, mailcraft_model::Security::Tls));
    assert!(a.config.imap.host.is_empty() && !a.config.pop_delete && !a.is_local());
    assert_eq!((a.config.pop_keep_days, a.config.pop_delete_with_trash), (Some(14), false), "for 14 days, as Outlook's default");
    assert!(matches!(s.take_jobs().as_slice(), [Job::Start { account, .. }] if account.pop.is_some()));
    assert!(s.run("account.list", &json!({})).unwrap().to_string().contains("mail.example.org:995"));
    // Changed: the other server and removing mail once it's down.
    s.run("account.update", &json!({"account": id, "popHost": "pop.example.org", "popPort": 110, "popSecurity": "starttls", "popDelete": true}))
        .unwrap();
    let a = s.account(&id).unwrap();
    assert_eq!(a.config.pop.as_ref().map(|p| (p.host.as_str(), p.port)), Some(("pop.example.org", 110)));
    assert!(a.config.pop_delete);
    s.run("account.update", &json!({"account": id, "popDelete": false, "popKeepDays": 0, "popDeleteWithTrash": true})).unwrap();
    let a = s.account(&id).unwrap();
    assert_eq!((a.config.pop_delete, a.config.pop_keep_days, a.config.pop_delete_with_trash), (false, None, true), "0: kept for good");
    s.take_jobs();
    assert!(s.run("account.add", &json!({"email": "ana@example.org", "password": "pw", "pop": true, "popHost": " "})).is_err(), "a server is needed");

    // An existing data file comes in first; then the account connects.
    let id = s.run("account.add", &json!({"email": "ana@example.org", "password": "pw", "pop": true, "pst": "C:/Mail/old.pst"})).unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(matches!(s.take_jobs().as_slice(), [Job::ImportPst { path, .. }] if path == "C:/Mail/old.pst"), "held until the file is in");
    s.apply(Event::Imported { account: id.clone(), messages: 40, skipped: 0, done: true, error: None });
    assert!(matches!(s.take_jobs().as_slice(), [Job::Start { account, .. }] if account.id == id));
    // A file that can't be read: the account stays, and connects anyway.
    let id =
        s.run("account.add", &json!({"email": "bo@example.org", "password": "pw", "pop": true, "pst": "C:/Mail/locked.pst"})).unwrap()["account"]
            .as_str()
            .unwrap()
            .to_string();
    s.take_jobs();
    s.apply(Event::Imported { account: id.clone(), messages: 0, skipped: 0, done: true, error: Some("Outlook has this data file open".into()) });
    assert!(s.account(&id).is_some());
    assert!(s.notices.last().is_some_and(|n| n.error && n.text.contains("Outlook has this data file open")));
    assert!(matches!(s.take_jobs().as_slice(), [Job::Start { .. }]));
    let _ = std::fs::remove_dir_all(s.store.as_ref().unwrap().root());
}

#[test]
fn oauth_accounts_sign_in_through_the_browser() {
    let store = temp("oauth");
    let mut s = Session::open(Some(store.clone()), mailcraft_model::dates::Clock::new(NOW, 0));
    let add = json!({"email": "zoe@gmail.com", "oauth": "google"});
    assert!(s.run("account.add", &add).is_err(), "not offered without a registration");
    s.oauth_available = vec!["google".into()];
    let id = s.run("account.add", &add).unwrap()["account"].as_str().unwrap().to_string();
    match s.take_jobs().as_slice() {
        [Job::SignIn { account }] => assert_eq!((account.oauth.as_deref(), account.imap.host.as_str()), (Some("google"), "imap.gmail.com")),
        other => panic!("{other:?}"),
    }
    s.apply(Event::OpenUrl { url: "https://accounts.google.com/o/oauth2/v2/auth?x".into() });
    assert_eq!(s.open_urls.len(), 1);
    s.apply(Event::SignedIn { account: id.clone(), user: None });
    assert!(matches!(s.take_jobs().as_slice(), [Job::Start { password: None, .. }]));
    s.run("account.signIn", &json!({"account": id})).unwrap();
    assert!(matches!(s.take_jobs().as_slice(), [Job::SignIn { .. }]));
    // It has no password: it asks to sign in again.
    s.apply(Event::Status { account: id.clone(), status: SyncStatus::NeedsPassword { error: String::new() } });
    let said = &s.notices.last().unwrap().text;
    assert!(said.contains("sign in again") && !said.contains("password"), "{said}");
    // Saved with the account (no secret in it).
    let saved = std::fs::read_to_string(store.root().join("accounts.json")).unwrap();
    assert!(saved.contains("\"oauth\":\"google\""));
    let _ = std::fs::remove_dir_all(store.root());
}

#[test]
fn server_settings_are_looked_up_once() {
    let mut s = Session::blank();
    assert!(s.run("account.discover", &json!({"email": "nope"})).is_err());
    assert_eq!(s.run("account.discover", &json!({"email": "Zoe@Example.org"})).unwrap()["pending"], true);
    assert!(matches!(s.take_jobs().as_slice(), [Job::Discover { email }] if email == "zoe@example.org"));
    assert_eq!(s.run("account.discover", &json!({"email": "zoe@example.org"})).unwrap()["pending"], true);
    assert!(s.take_jobs().is_empty(), "asked once");
    let mut c = mailcraft_model::AccountConfig::guess("zoe@example.org");
    c.imap.host = "imap.example.org".into();
    s.apply(Event::Discovered { email: "zoe@example.org".into(), config: Some(Box::new(c)), source: "example.org".into() });
    let r = s.run("account.discover", &json!({"email": "zoe@example.org"})).unwrap();
    assert_eq!((r["found"].as_bool(), r["imap"]["host"].as_str()), (Some(true), Some("imap.example.org")));
}

#[test]
fn remote_accounts_go_through_jobs_and_events() {
    let store = temp("remote");
    let mut s = Session::open(Some(store), mailcraft_model::dates::Clock::new(NOW, 0));
    assert_eq!(s.view, View::Empty);
    assert!(s.run("mail.new", &json!({})).is_err());
    assert!(s.run("account.add", &json!({"email": "zoe@example.org"})).is_err(), "password required");
    let r = s.run("account.add", &json!({"email": "zoe@example.org", "password": "pw"})).unwrap();
    let id = r["account"].as_str().unwrap().to_string();
    let jobs = s.take_jobs();
    match jobs.as_slice() {
        [Job::Start { account, password: Some(p) }] => {
            assert_eq!(account.imap.host, "mail.example.org");
            assert_eq!(p.0, "pw");
        }
        other => panic!("{other:?}"),
    }
    assert!(s.run("account.add", &json!({"email": "ZOE@example.org", "password": "x"})).is_err(), "no duplicates");

    // The worker reports folders and a synced Inbox.
    let folders = vec![
        mailcraft_model::Folder { account: id.clone(), path: "INBOX".into(), role: FolderRole::Inbox, selectable: true, ..Default::default() },
        mailcraft_model::Folder { account: id.clone(), path: "Trash".into(), role: FolderRole::Trash, selectable: true, ..Default::default() },
    ];
    s.apply(Event::Folders { account: id.clone(), folders });
    let key = mailcraft_model::MessageKey { account: id.clone(), folder: "INBOX".into(), validity: 9, uid: 1 };
    let st = mailcraft_store::FolderState {
        validity: 9,
        uid_next: 2,
        highest_modseq: 0,
        messages: vec![mailcraft_model::Summary { key: key.clone(), subject: "Hello".into(), date: NOW - 60, ..Default::default() }],
        window: 0,
    };
    s.store.as_ref().unwrap().save_folder(&id, "INBOX", &st).unwrap();
    s.apply(Event::FolderChanged { account: id.clone(), folder: "INBOX".into() });
    assert_eq!(s.list.len(), 1);

    // Opening asks for the body; the event shows it; reading marks it read through a job.
    s.run("list.open", &json!({"key": key.to_id()})).unwrap();
    let jobs = s.take_jobs();
    assert!(jobs.iter().any(|j| matches!(j, Job::FetchBody { .. })));
    assert!(jobs.iter().any(|j| matches!(j, Job::Apply { op: Op::SetFlag { flag: Flag::Seen, on: true, .. }, .. })));
    assert!(s.run("mail.reply", &json!({})).is_err(), "body not here yet");
    s.apply(Event::Body { key: key.clone(), raw: b"From: a@b.example\r\nSubject: Hello\r\nMessage-ID: <h@b>\r\n\r\nHi there\r\n".to_vec() });
    assert!(s.reading.as_ref().unwrap().parsed.as_ref().unwrap().text.contains("Hi there"));

    // Replying and sending queues a Send job; the Sent event closes the message.
    s.run("mail.reply", &json!({})).unwrap();
    s.run("compose.send", &json!({})).unwrap();
    let sent = s.take_jobs();
    let Some(Job::Send { id: send_id, recipients, raw, .. }) = sent.iter().find(|j| matches!(j, Job::Send { .. })) else { panic!("{sent:?}") };
    assert_eq!(recipients, &vec!["a@b.example".to_string()]);
    assert!(String::from_utf8_lossy(raw).contains("In-Reply-To: <h@b>"));
    assert!(s.compose.as_ref().unwrap().sending);
    s.apply(Event::SendFailed { account: id.clone(), id: *send_id, error: "timeout".into() });
    assert_eq!(s.compose.as_ref().unwrap().error.as_deref(), Some("timeout"));
    s.run("compose.send", &json!({})).unwrap();
    let send_id = match s.take_jobs().into_iter().find(|j| matches!(j, Job::Send { .. })) {
        Some(Job::Send { id, .. }) => id,
        _ => panic!(),
    };
    s.apply(Event::Sent { account: id.clone(), id: send_id });
    assert!(s.compose.is_none());
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::Apply { op: Op::SetFlag { flag: Flag::Answered, .. }, .. })));

    // Queued instead of delivered: no Answered flag yet.
    s.run("mail.reply", &json!({})).unwrap();
    s.run("compose.send", &json!({})).unwrap();
    let send_id = match s.take_jobs().into_iter().find(|j| matches!(j, Job::Send { .. })) {
        Some(Job::Send { id, .. }) => id,
        _ => panic!(),
    };
    s.apply(Event::Queued { account: id.clone(), id: send_id, error: "offline".into() });
    assert!(s.compose.is_none(), "the compose closes anyway");
    assert!(!s.take_jobs().iter().any(|j| matches!(j, Job::Apply { op: Op::SetFlag { flag: Flag::Answered, .. }, .. })), "no flag before delivery");
    // The owed flag survives a restart and is set on delivery.
    let mut again = Session::open(s.store.clone(), mailcraft_model::dates::Clock::new(NOW, 0));
    again.take_jobs();
    again.apply(Event::Sent { account: id.clone(), id: send_id });
    assert!(
        again.take_jobs().iter().any(|j| matches!(j, Job::Apply { op: Op::SetFlag { flag: Flag::Answered, keys, .. }, .. } if keys.contains(&key)))
    );
    again.apply(Event::Sent { account: id.clone(), id: send_id });
    assert!(!again.take_jobs().iter().any(|j| matches!(j, Job::Apply { op: Op::SetFlag { .. }, .. })), "set a single time");
    drop(again);
    let mut s = Session::open(s.store.clone(), mailcraft_model::dates::Clock::new(NOW, 0));
    s.apply(Event::Folders {
        account: id.clone(),
        folders: vec![
            mailcraft_model::Folder { account: id.clone(), path: "INBOX".into(), role: FolderRole::Inbox, selectable: true, ..Default::default() },
            mailcraft_model::Folder { account: id.clone(), path: "Trash".into(), role: FolderRole::Trash, selectable: true, ..Default::default() },
        ],
    });
    let answered = |s: &mut Session| s.take_jobs().iter().any(|j| matches!(j, Job::Apply { op: Op::SetFlag { flag: Flag::Answered, .. }, .. }));
    // Queue a reply in the Outbox (as the network layer does when it can't send) and show it.
    let queue_reply = |s: &mut Session| -> mailcraft_model::MessageKey {
        s.run("folder.open", &json!({"account": id, "folder": "INBOX"})).unwrap();
        s.run("list.open", &json!({"key": key.to_id()})).unwrap();
        s.apply(Event::Body { key: key.clone(), raw: b"From: a@b.example\r\nSubject: Hello\r\nMessage-ID: <h@b>\r\n\r\nHi there\r\n".to_vec() });
        s.run("mail.reply", &json!({})).unwrap();
        s.run("compose.send", &json!({})).unwrap();
        let Some(Job::Send { id: send_id, raw, from, recipients, .. }) = s.take_jobs().into_iter().find(|j| matches!(j, Job::Send { .. })) else {
            panic!()
        };
        let item = mailcraft_store::OutboxItem { id: send_id, from, recipients, queued: NOW, error: Some("offline".into()), ..Default::default() };
        s.store.as_ref().unwrap().outbox_add(&id, &item, &raw).unwrap();
        s.apply(Event::Queued { account: id.clone(), id: send_id, error: "offline".into() });
        s.run("folder.outbox", &json!({})).unwrap();
        s.list[0].key.clone()
    };
    // Edited from the Outbox, then delivered: the flag follows the edit.
    let queued = queue_reply(&mut s);
    s.run("outbox.edit", &json!({"key": queued.to_id()})).unwrap();
    s.run("compose.send", &json!({})).unwrap();
    let Some(Job::Send { id: resent, .. }) = s.take_jobs().into_iter().find(|j| matches!(j, Job::Send { .. })) else { panic!() };
    s.apply(Event::Sent { account: id.clone(), id: resent });
    assert!(answered(&mut s), "flag set when the edited reply is delivered");
    // Removed from the Outbox: the owed flag is dropped.
    let queued = queue_reply(&mut s);
    let gone_id = s.outbox.first().map(|(_, i)| i.id).unwrap();
    s.run("list.select", &json!({"key": queued.to_id()})).unwrap();
    s.run("mail.delete", &json!({})).unwrap();
    assert!(s.outbox.is_empty());
    s.apply(Event::Sent { account: id.clone(), id: gone_id });
    assert!(!answered(&mut s), "no flag for an undelivered reply");
    s.run("folder.open", &json!({"account": id, "folder": "INBOX"})).unwrap();

    // Delete moves to Deleted Items through a job.
    s.run("list.select", &json!({"key": key.to_id()})).unwrap();
    s.run("mail.delete", &json!({})).unwrap();
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::Apply { op: Op::Move { to, .. }, .. } if to == "Trash")));
    assert!(s.list.is_empty());

    // Status and password prompts.
    s.apply(Event::Status { account: id.clone(), status: SyncStatus::NeedsPassword { error: "bad".into() } });
    assert!(s.notices.last().unwrap().text.contains("needs its password"));
    s.run("account.setPassword", &json!({"account": id, "password": "new"})).unwrap();
    assert!(matches!(s.take_jobs().as_slice(), [Job::Start { password: Some(_), .. }]));
    assert_eq!(s.run("mail.sync", &json!({})).unwrap()["accounts"], 1);
    s.run("account.remove", &json!({"account": id})).unwrap();
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::Remove { .. })));
    assert!(s.accounts.is_empty());
    let root = s.store.as_ref().unwrap().root().to_path_buf();
    // Accounts persist across sessions (without passwords).
    let s2 = Session::open(Some(Store::open(&root).unwrap()), mailcraft_model::dates::Clock::new(NOW, 0));
    assert!(s2.accounts.is_empty());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn outlook_data_files_open_as_accounts_on_this_computer() {
    let mut s = sample_session("pst");
    assert!(s.run("account.importPst", &json!({"path": "  "})).is_err());
    let id =
        s.run("account.importPst", &json!({"path": r"C:\Users\me\Documents\Archive 2024.pst"})).unwrap()["account"].as_str().unwrap().to_string();
    let a = s.account(&id).unwrap();
    assert_eq!((a.config.name.as_str(), a.config.email.as_str()), ("Archive 2024", "Archive 2024.pst"));
    assert!(a.is_local());
    assert!(matches!(s.take_jobs().as_slice(), [Job::ImportPst { account, path }] if *account == id && path.ends_with("Archive 2024.pst")));

    // The import reports progress, then its folders, then that it's done: the Inbox opens.
    s.apply(Event::Imported { account: id.clone(), messages: 200, skipped: 0, done: false, error: None });
    assert!(s.notices.last().is_some_and(|n| !n.error && n.text.contains("200")));
    let inbox = mailcraft_model::Folder {
        account: id.clone(),
        path: "INBOX".into(),
        delimiter: Some('/'),
        role: FolderRole::Inbox,
        selectable: true,
        ..Default::default()
    };
    s.apply(Event::Folders { account: id.clone(), folders: vec![inbox] });
    // The file's contacts were written next to its mail: they show once it's done.
    let mut pim = mailcraft_model::pim::Pim::default();
    pim.cards.push(mailcraft_model::pim::StoredCard {
        book: mailcraft_model::pim::LOCAL.into(),
        card: mailcraft_model::vcard::Card { uid: "c1".into(), name: "Ana Lima".into(), ..Default::default() },
        ..Default::default()
    });
    s.store.clone().unwrap().save_account_doc(&id, mailcraft_model::pim::PIM_DOC, &pim).unwrap();
    s.apply(Event::Imported { account: id.clone(), messages: 250, skipped: 2, done: true, error: None });
    assert!(s.pim[&id].cards.iter().any(|c| c.card.name == "Ana Lima"));
    assert!(!s.pim[&id].calendars.is_empty(), "with a calendar to add to");
    assert!(s.notices.last().is_some_and(|n| n.text.contains("250 messages imported (2 could not be read)")));
    assert_eq!(s.view, View::Folder { account: id.clone(), folder: "INBOX".into() });
    assert_eq!(s.account(&id).unwrap().status, SyncStatus::Offline);

    // A file that isn't one: the account goes again.
    let bad = s.run("account.importPst", &json!({"path": "/tmp/notes.txt"})).unwrap()["account"].as_str().unwrap().to_string();
    s.apply(Event::Imported { account: bad.clone(), messages: 0, skipped: 0, done: true, error: Some("not an Outlook data file".into()) });
    assert!(s.account(&bad).is_none());
    assert!(s.notices.last().is_some_and(|n| n.error && n.text.contains("notes.txt could not be opened")));

    // Exporting: the current account, or the one asked for, to the file given.
    s.take_jobs();
    assert!(s.run("account.exportPst", &json!({"path": ""})).is_err());
    assert!(s.run("account.exportPst", &json!({"path": "out.pst", "account": "nope"})).is_err());
    s.run("account.exportPst", &json!({"path": "C:/out/Archive.pst", "account": id})).unwrap();
    assert!(
        matches!(s.take_jobs().as_slice(), [Job::ExportPst { account, path, name }] if *account == id && path == "C:/out/Archive.pst" && name == "Archive 2024")
    );
    s.apply(Event::Exported { account: id.clone(), messages: 100, skipped: 0, already: 0, done: false, error: None });
    assert!(s.notices.last().is_some_and(|n| n.text.contains("100 messages exported so far")));
    s.apply(Event::Exported { account: id.clone(), messages: 120, skipped: 3, already: 0, done: true, error: None });
    assert!(s.notices.last().is_some_and(|n| !n.error && n.text.contains("120 messages exported (3 not on this computer")));
    s.apply(Event::Exported { account: id.clone(), messages: 0, skipped: 0, already: 0, done: true, error: Some("disk full".into()) });
    assert!(s.notices.last().is_some_and(|n| n.error && n.text.contains("disk full")));
    done(s);
}

#[test]
fn exchange_accounts_go_through_microsoft_graph() {
    let store = temp("graph-account");
    let mut s = Session::open(Some(store), mailcraft_model::dates::Clock::new(NOW, 0));
    // Graph signs in with Microsoft: refused when this copy has no registration.
    assert!(s.run("account.add", &json!({"email": "ana@contoso.example", "graph": true})).is_err());
    s.oauth_available = vec!["microsoft".into()];
    let id = s.run("account.add", &json!({"email": "ana@contoso.example", "graph": true})).unwrap()["account"].as_str().unwrap().to_string();
    let a = s.account(&id).unwrap();
    assert_eq!(a.config.graph.as_deref(), Some(mailcraft_model::GRAPH_URL));
    assert_eq!(a.config.oauth.as_deref(), Some("microsoft"));
    assert!(a.config.is_api() && !a.is_local() && a.config.imap.host.is_empty(), "no IMAP or SMTP");
    assert!(matches!(s.take_jobs().as_slice(), [Job::SignIn { account }] if account.graph.is_some()));
    // Server settings can't turn it into an IMAP account by accident.
    s.run("account.update", &json!({"account": id, "name": "Ana Ruiz"})).unwrap();
    assert_eq!(s.account(&id).unwrap().config.name, "Ana Ruiz");
    assert_eq!(s.run("account.list", &json!({})).map(|v| v[0]["graph"].clone()).unwrap_or_default(), json!(mailcraft_model::GRAPH_URL));
    let root = s.store.as_ref().unwrap().root().to_path_buf();
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn on_premises_exchange_accounts_go_through_ews() {
    let store = temp("ews-account");
    let mut s = Session::open(Some(store), mailcraft_model::dates::Clock::new(NOW, 0));
    assert!(s.run("account.add", &json!({"email": "riley@contoso.example", "ews": "mail.contoso.example"})).is_err(), "a password is needed");
    let id = s
        .run("account.add", &json!({"email": "riley@contoso.example", "ews": "mail.contoso.example", "username": "CONTOSO\\riley", "password": "pw"}))
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    let a = s.account(&id).unwrap();
    // A server name becomes the usual address.
    assert_eq!(a.config.ews.as_deref(), Some("https://mail.contoso.example/EWS/Exchange.asmx"));
    assert!(a.config.is_api() && !a.is_local() && a.config.imap.host.is_empty() && a.config.oauth.is_none());
    assert!(matches!(s.take_jobs().as_slice(), [Job::Start { account, password: Some(_) }] if account.ews.is_some()));
    // Or found with Autodiscover when it connects.
    let auto =
        s.run("account.add", &json!({"email": "ana@contoso.example", "ews": true, "username": "CONTOSO\\ana", "password": "pw"})).unwrap()["account"]
            .as_str()
            .unwrap()
            .to_string();
    assert_eq!(s.account(&auto).unwrap().config.ews.as_deref(), Some(mailcraft_model::EWS_AUTODISCOVER));
    s.take_jobs();
    // A full address is kept as it is, and can be changed.
    s.run("account.update", &json!({"account": id, "ews": "https://owa.contoso.example/ews/exchange.asmx"})).unwrap();
    assert_eq!(s.account(&id).unwrap().config.ews.as_deref(), Some("https://owa.contoso.example/ews/exchange.asmx"));
    assert_eq!(
        s.run("account.list", &json!({})).map(|v| v[0]["ews"].clone()).unwrap_or_default(),
        json!("https://owa.contoso.example/ews/exchange.asmx")
    );
    let root = s.store.as_ref().unwrap().root().to_path_buf();
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn shared_calendars_open_from_exchange_accounts_and_close() {
    let mut s = sample_session("shared-cal");
    assert!(s.run("calendar.openShared", &json!({"email": "ana@soundlab.example"})).is_err(), "the sample isn't an Exchange account");
    done(s);
    let mut s = Session::open(Some(temp("shared-cal-ews")), mailcraft_model::dates::Clock::new(NOW, 0));
    let id = s
        .run("account.add", &json!({"email": "zoe@contoso.example", "password": "pw", "ews": "https://mail.contoso.example/EWS/Exchange.asmx"}))
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    s.take_jobs();
    assert!(s.run("calendar.openShared", &json!({"email": "not an address"})).is_err());
    assert!(s.run("calendar.openShared", &json!({"email": "zoe@contoso.example"})).is_err(), "not one's own");
    s.run("calendar.openShared", &json!({"email": "Ana@Contoso.example"})).unwrap();
    s.run("calendar.openShared", &json!({"email": "ana@contoso.example"})).unwrap();
    assert_eq!(s.pim[&id].shared, vec!["ana@contoso.example".to_string()], "once, lower-case");
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::SyncDav { .. })));
    // The server shows it: read-only, with its events; closing takes it and them away.
    let cal = mailcraft_model::pim::shared_calendar_id("ana@contoso.example");
    let sync = mailcraft_model::pim::DavSync {
        calendars: Some(vec![
            mailcraft_model::pim::CalendarInfo { id: "ews:calendar".into(), name: "Calendar".into(), ..Default::default() },
            mailcraft_model::pim::CalendarInfo { id: cal.clone(), name: "ana@contoso.example".into(), read_only: true, ..Default::default() },
        ]),
        synced_calendars: vec![cal.clone()],
        events: vec![mailcraft_model::pim::StoredEvent {
            calendar: cal.clone(),
            href: "S1".into(),
            event: mailcraft_model::ical::CalEvent {
                uid: "b1".into(),
                summary: "Budget review".into(),
                start: NOW,
                end: NOW + 3600,
                ..Default::default()
            },
            ..Default::default()
        }],
        ..Default::default()
    };
    s.apply_dav(&id, &sync);
    assert!(s.agenda(NOW - 10, NOW + 10).iter().any(|a| a.calendar == cal && a.summary == "Budget review"));
    s.run("calendar.closeShared", &json!({"email": "ana@contoso.example"})).unwrap();
    assert!(s.pim[&id].shared.is_empty() && s.pim[&id].calendars.iter().all(|c| c.id != cal));
    assert!(s.agenda(NOW - 10, NOW + 10).is_empty());
    assert!(s.run("calendar.closeShared", &json!({"email": "ana@contoso.example"})).is_err());
    done(s);
}

/// With several accounts, the first one's Inbox opens (as Outlook opens on its first mailbox),
/// and accounts can be put in another order, which is kept.
#[test]
fn the_first_account_opens_and_accounts_can_be_reordered() {
    use mailcraft_model::dates::Clock;
    use mailcraft_model::{AccountConfig, Folder};
    let dir = std::env::temp_dir().join(format!("mailcraft-acct-order-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    let config = |id: &str, email: &str| AccountConfig { id: id.into(), email: email.into(), ..Default::default() };
    store.save_accounts(&[config("a1", "zoe@studio.example"), config("a2", "ana@soundlab.example"), config("a3", "sam@studio.example")]).unwrap();
    for id in ["a1", "a2", "a3"] {
        store.save_folders(id, &[Folder { path: "INBOX".into(), role: FolderRole::Inbox, ..Default::default() }]).unwrap();
    }
    let mut s = Session::open(Some(store.clone()), Clock::new(1_700_000_000, 0));
    assert_eq!(s.view, View::Folder { account: "a1".into(), folder: "INBOX".into() });
    s.run("account.move", &json!({"account": "a3", "by": -1})).unwrap();
    s.run("account.move", &json!({"account": "a3", "by": -1})).unwrap();
    s.run("account.move", &json!({"account": "a3", "by": -1})).unwrap();
    let order = |s: &Session| s.accounts.iter().map(|a| a.config.id.clone()).collect::<Vec<_>>();
    assert_eq!(order(&s), ["a3", "a1", "a2"], "at the top it stays there");
    s.run("account.move", &json!({"account": "a1", "by": 1})).unwrap();
    assert_eq!(order(&s), ["a3", "a2", "a1"]);
    assert!(s.run("account.move", &json!({"account": "nobody", "by": 1})).is_err());
    // Kept: the next start has the new order and opens its first account.
    let s = Session::open(Some(store), Clock::new(1_700_000_000, 0));
    assert_eq!(order(&s), ["a3", "a2", "a1"]);
    assert_eq!(s.view, View::Folder { account: "a3".into(), folder: "INBOX".into() });
    let _ = std::fs::remove_dir_all(&dir);
}

/// An account whose folders were kept under another account's name (a data file brought in from
/// it) opens them as its own: opening one said "no folder … in account a1".
#[test]
fn folders_kept_under_another_account_open_as_this_ones() {
    use mailcraft_model::dates::Clock;
    use mailcraft_model::{AccountConfig, Folder};
    let dir = std::env::temp_dir().join(format!("mailcraft-folder-owner-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    store.save_accounts(&[AccountConfig { id: "a2".into(), email: "zidan@garnetbyte.example".into(), ..Default::default() }]).unwrap();
    let f = |path: &str| Folder { account: "a1".into(), path: path.into(), ..Default::default() };
    store.save_folders("a2", &[Folder { role: FolderRole::Inbox, ..f("INBOX") }, f("INBOX/gzamer")]).unwrap();
    let mut s = Session::open(Some(store), Clock::new(1_700_000_000, 0));
    assert!(s.accounts[0].folders.iter().all(|f| f.account == "a2"));
    s.run("folder.open", &json!({"account": "a2", "folder": "INBOX/gzamer"})).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// Collapsing in the folder pane is kept with the settings (as Outlook keeps it).
#[test]
fn collapsing_in_the_folder_pane_is_kept() {
    let mut s = sample_session("collapse");
    s.run("view.collapse", &json!({"key": "favorites"})).unwrap();
    s.run("view.collapse", &json!({"key": "account:sample", "collapsed": true})).unwrap();
    s.run("view.collapse", &json!({"key": "folder:sample/Projects"})).unwrap();
    s.run("view.collapse", &json!({"key": "folder:sample/Projects"})).unwrap();
    let kept: Vec<&str> = s.prefs.collapsed.iter().map(String::as_str).collect();
    assert_eq!(kept, ["account:sample", "favorites"]);
    // Saved with the rest of the settings and read back.
    let back: crate::Prefs = serde_json::from_value(serde_json::to_value(&s.prefs).unwrap()).unwrap();
    assert_eq!(back.collapsed, s.prefs.collapsed);
    assert!(s.run("view.collapse", &json!({"key": ""})).is_err());
}

/// Issue #2: a Hotmail account added with a password (which Microsoft refuses) switches to Sign
/// in with Microsoft in place, and keeps its mail.
#[test]
fn a_microsoft_account_with_a_password_switches_to_microsoft_sign_in() {
    let store = temp("ms-switch");
    let mut s = Session::open(Some(store.clone()), mailcraft_model::dates::Clock::new(NOW, 0));
    s.oauth_available = vec!["microsoft".into()];
    let id = s.run("account.add", &json!({"email": "zoe@hotmail.com", "password": "old"})).unwrap()["account"].as_str().unwrap().to_string();
    s.take_jobs();
    s.apply(Event::Status {
        account: id.clone(),
        status: SyncStatus::Error { error: format!("{} (Basic authentication is disabled)", mailcraft_model::MICROSOFT_NO_PASSWORDS) },
    });
    assert!(s.notices.last().unwrap().text.contains("Sign in with Microsoft"), "said once, as it comes");
    let n = s.notices.len();
    s.apply(Event::Status {
        account: id.clone(),
        status: SyncStatus::Error { error: format!("{} (Basic authentication is disabled)", mailcraft_model::MICROSOFT_NO_PASSWORDS) },
    });
    assert_eq!(s.notices.len(), n, "not again while it stays so");
    s.run("account.signIn", &json!({"account": id})).unwrap();
    match s.take_jobs().as_slice() {
        [Job::SignIn { account }] => assert_eq!(
            (account.id.as_str(), account.oauth.as_deref(), account.imap.host.as_str()),
            (id.as_str(), Some("microsoft"), "outlook.office365.com")
        ),
        other => panic!("{other:?}"),
    }
    assert_eq!(s.account(&id).unwrap().config.oauth.as_deref(), Some("microsoft"));
    // Who the browser signed in as is kept when it isn't the account's address.
    s.apply(Event::SignedIn { account: id.clone(), user: Some("Zoe@Outlook.com".into()) });
    assert_eq!(s.account(&id).unwrap().config.oauth_user.as_deref(), Some("Zoe@Outlook.com"));
    assert!(matches!(s.take_jobs().as_slice(), [Job::Start { account, password: None }] if account.oauth_user.is_some()));
    s.apply(Event::SignedIn { account: id.clone(), user: Some("ZOE@hotmail.com".into()) });
    assert_eq!(s.account(&id).unwrap().config.oauth_user, None, "the same address isn't kept");
    // Other accounts don't switch; nor does a copy without Microsoft sign-in.
    let other = s.run("account.add", &json!({"email": "ann@studio.example", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    assert!(s.run("account.signIn", &json!({"account": other})).is_err());
    let live = s.run("account.add", &json!({"email": "bo@live.com", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    s.oauth_available.clear();
    assert!(s.run("account.signIn", &json!({"account": live})).is_err());
    let _ = std::fs::remove_dir_all(store.root());
}

/// A mailbox Microsoft won't open over IMAP can start again through Microsoft Graph, in the
/// same place among the accounts.
#[test]
fn a_microsoft_account_moves_to_graph() {
    let store = temp("ms-graph");
    let mut s = Session::open(Some(store.clone()), mailcraft_model::dates::Clock::new(NOW, 0));
    s.oauth_available = vec!["microsoft".into()];
    let first = s.run("account.add", &json!({"email": "ann@studio.example", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    let id = s.run("account.add", &json!({"email": "zoe@hotmail.com", "oauth": "microsoft", "name": "Zoe"})).unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    let last = s.run("account.add", &json!({"email": "bo@studio.example", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    s.run("account.setSignature", &json!({"account": id, "signature": "Zoe x"})).unwrap();
    s.take_jobs();
    let closed = format!("{}. Turn on IMAP", mailcraft_model::MICROSOFT_IMAP_CLOSED);
    s.apply(Event::Status { account: id.clone(), status: SyncStatus::Error { error: closed } });
    assert!(s.notices.last().unwrap().text.contains("IMAP"));
    assert!(s.run("account.useGraph", &json!({"account": first})).is_err(), "not a Microsoft account");
    let new = s.run("account.useGraph", &json!({"account": id})).unwrap()["account"].as_str().unwrap().to_string();
    let order: Vec<&str> = s.accounts.iter().map(|a| a.config.id.as_str()).collect();
    assert_eq!(order, [first.as_str(), new.as_str(), last.as_str()]);
    let c = &s.account(&new).unwrap().config;
    assert_eq!((c.email.as_str(), c.name.as_str(), c.signature.as_str()), ("zoe@hotmail.com", "Zoe", "Zoe x"));
    assert_eq!((c.graph.as_deref(), c.oauth.as_deref()), (Some(mailcraft_model::GRAPH_URL), Some("microsoft")));
    let jobs = s.take_jobs();
    assert!(jobs.iter().any(|j| matches!(j, Job::Remove { account } if *account == id)));
    assert!(jobs.iter().any(|j| matches!(j, Job::SignIn { account } if account.id == new && account.graph.is_some())));
    assert!(s.run("account.useGraph", &json!({"account": new})).is_err(), "already on Graph");
    let _ = std::fs::remove_dir_all(store.root());
}

/// Issue #1: how much mail is kept on this computer is a setting (Outlook's "Mail to keep
/// offline"), told to the network side ahead of anything queued.
#[test]
fn how_much_mail_is_kept_is_a_setting() {
    let mut s = sample_session("keep-mail");
    assert_eq!((s.prefs.keep_messages, s.prefs.keep_offline), (1000, 40));
    s.jobs_push_for_test(Job::SyncAll { account: "sample".into() });
    s.run("account.keepMail", &json!({"messages": 0, "offline": 200})).unwrap();
    let jobs = s.take_jobs();
    assert!(matches!(jobs.first(), Some(Job::SetSync { messages: 0, offline: 200 })), "{jobs:?}");
    s.run("account.keepMail", &json!({"messages": 5000})).unwrap();
    s.run("account.keepMail", &json!({"messages": 20000})).unwrap();
    let jobs = s.take_jobs();
    assert_eq!(jobs.iter().filter(|j| matches!(j, Job::SetSync { .. })).count(), 1, "the last one only");
    assert!(matches!(jobs.first(), Some(Job::SetSync { messages: 20000, offline: 200 })));
}

/// Looking through a big mailbox (the address book reads every folder) keeps only so many
/// messages' summaries in memory; the folder on screen stays, and one let go reads back whole.
#[test]
fn the_folder_cache_lets_go_of_folders_past_its_limit() {
    let mut s = sample_session("folder-cache-limit");
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    s.set_cache_limit(3);
    let inbox = s.list.len();
    assert!(inbox > 3, "the sample's Inbox is bigger than the limit");
    // A change: the address book is built again, reading every folder.
    s.run("list.selectAll", &json!({})).unwrap();
    s.run("mail.markRead", &json!({})).unwrap();
    let loads = s.cache_order.len();
    let _ = s.address_book("all", "");
    assert!(s.cache_order.len() >= loads, "every folder was read");
    let cached: usize = s.cache.values().map(|st| st.messages.len()).sum();
    assert!(s.cache.contains_key(&("sample".to_string(), "INBOX".to_string())), "the folder on screen stays");
    // The one on screen, and the one just read (kept even past the limit: it's being used).
    let biggest = s.account("sample").unwrap().folders.iter().filter(|f| f.path != "INBOX").map(|f| f.total as usize).max().unwrap_or(0);
    let held: Vec<(String, usize)> = s.cache.iter().map(|(k, v)| (k.1.clone(), v.messages.len())).collect();
    assert!(cached <= inbox + biggest.max(3), "{cached} summaries kept: {held:?}, inbox {inbox}, biggest {biggest}, view {:?}", s.view);
    let total: usize = s.account("sample").unwrap().folders.iter().map(|f| f.total as usize).sum();
    assert!(cached < total, "not the whole mailbox: {cached} of {total}");
    // A folder let go reads back as it was.
    let sent_before = s.store.as_ref().unwrap().load_folder("sample", "Sent").unwrap().messages.len();
    assert_eq!(s.folder_state("sample", "Sent").messages.len(), sent_before);
    s.refresh_list();
    assert_eq!(s.list.len(), inbox);
}

/// Shared mailboxes are added to an Exchange account by address and removed again; each change
/// connects the account again to list their folders. Junk, duplicates and its own address are
/// refused, and other kinds of accounts have none.
#[test]
fn shared_mailboxes_are_added_and_removed() {
    let mut s = sample_session("shared-mailboxes");
    let id = s
        .run("account.add", &json!({"email": "riley@contoso.example", "password": "pw", "ews": "https://mail.contoso.example/EWS/Exchange.asmx", "username": "CONTOSO\\riley"}))
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    s.take_jobs();
    s.run("account.addSharedMailbox", &json!({"account": id, "address": "Sales <sales@contoso.example>"})).unwrap();
    assert_eq!(s.account(&id).unwrap().config.shared_mailboxes, ["sales@contoso.example"]);
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::Start { account, .. } if account.shared_mailboxes == ["sales@contoso.example"])));
    for bad in ["SALES@contoso.example", "riley@contoso.example", "not an address", "a@x.example, b@x.example"] {
        assert!(s.run("account.addSharedMailbox", &json!({"account": id, "address": bad})).is_err(), "{bad}");
    }
    assert!(s.run("account.addSharedMailbox", &json!({"account": "sample", "address": "x@y.example"})).is_err(), "not Exchange");
    s.run("account.removeSharedMailbox", &json!({"account": id, "address": "sales@CONTOSO.example"})).unwrap();
    assert!(s.account(&id).unwrap().config.shared_mailboxes.is_empty());
    assert!(s.run("account.removeSharedMailbox", &json!({"account": id, "address": "sales@contoso.example"})).is_err());
    done(s);
}

/// Removing an account keeps its data file where it is, as Outlook keeps a removed account's
/// .pst: the same address added again gets a file of its own beside it, and the kept one opens
/// again as a data file. A file that failed to open, or an account starting again, isn't kept.
#[test]
fn a_removed_account_keeps_its_data_file() {
    let dir = std::env::temp_dir().join(format!("mailcraft-engine-keep-file-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let files = dir.join("MailCraft Files");
    let store = Store::open_packed(dir.join("app"), &files).unwrap();
    let mut s = Session::open(Some(store), mailcraft_model::dates::Clock::new(NOW, 0));
    let id = s.run("account.add", &json!({"email": "zoe@example.org", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    let st = s.store.clone().unwrap();
    st.save_folders(&id, &[mailcraft_model::Folder { account: id.clone(), path: "INBOX".into(), selectable: true, ..Default::default() }]).unwrap();
    let kept = files.join("zoe@example.org.mailcraft");
    assert_eq!(st.data_file(&id).as_deref(), Some(kept.as_path()));

    let r = s.run("account.remove", &json!({"account": id})).unwrap();
    assert_eq!(r["dataFile"].as_str(), Some(kept.display().to_string().as_str()));
    assert!(kept.exists(), "the data file stays");
    assert!(s.accounts.is_empty() && st.data_file(&id).is_none());

    // Added again: a file of its own, the kept one untouched.
    let before = std::fs::read(&kept).unwrap();
    let id = s.run("account.add", &json!({"email": "zoe@example.org", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    st.save_folders(&id, &[mailcraft_model::Folder { account: id.clone(), path: "INBOX".into(), selectable: true, ..Default::default() }]).unwrap();
    let second = files.join("zoe@example.org (2).mailcraft");
    assert_eq!(st.data_file(&id).as_deref(), Some(second.as_path()));
    assert_eq!(std::fs::read(&kept).unwrap(), before);

    // The kept file opens again (File › Open & Export) as a mailbox of its own.
    let opened = st.clone();
    let n = opened.import_data_file("a9", &kept).unwrap();
    assert_eq!(n, 0, "no mail in it, but it opens");
    assert_eq!(opened.load_folders("a9").unwrap().len(), 1);
    drop(s);
    let _ = std::fs::remove_dir_all(dir);
}

/// Issue #3: an account whose outgoing server has its own user name and password. The user name
/// is kept with the account, the password goes to the keychain on its own (never in the
/// account's settings), a new one needs its password, and turning it off signs in as IMAP does.
#[test]
fn the_outgoing_server_keeps_a_sign_in_of_its_own() {
    let mut s = sample_session("smtp-sign-in");
    let add = json!({"email": "zoe@example.org", "password": "imap-pw", "imapHost": "imap.example.org", "smtpHost": "mail.relay.example",
        "smtpUsername": "relay-user"});
    assert!(s.run("account.add", &add).unwrap_err().to_string().contains("outgoing server's password"));
    let mut add = add;
    add["smtpPassword"] = json!("smtp-pw");
    let id = s.run("account.add", &add).unwrap()["account"].as_str().unwrap().to_string();
    let jobs = s.take_jobs();
    assert!(
        matches!(jobs.as_slice(), [Job::Start { account, password: Some(p) }, Job::SmtpPassword { account: a2, password: sp }]
            if account.smtp_username.as_deref() == Some("relay-user") && p.0 == "imap-pw" && a2.id == id && sp.0 == "smtp-pw"),
        "{jobs:?}"
    );
    let saved = std::fs::read_to_string(s.store.as_ref().unwrap().root().join("accounts.json")).unwrap();
    assert!(saved.contains("relay-user"));
    assert!(!saved.contains("smtp-pw") && !saved.contains("imap-pw"), "passwords stay out of MailCraft's files");
    // Its password changed alone: kept, no reconnecting.
    s.run("account.update", &json!({"account": id, "smtpUsername": "relay-user", "smtpPassword": "new-pw"})).unwrap();
    assert!(matches!(s.take_jobs().as_slice(), [Job::SmtpPassword { password, .. }] if password.0 == "new-pw"));
    // A new user name with the password kept: it reconnects with it.
    s.run("account.update", &json!({"account": id, "smtpUsername": "relay-2"})).unwrap();
    assert!(matches!(s.take_jobs().as_slice(), [Job::Start { account, .. }] if account.smtp_username.as_deref() == Some("relay-2")));
    // Off: SMTP signs in as IMAP does.
    s.run("account.update", &json!({"account": id, "smtpUsername": null})).unwrap();
    assert!(s.account(&id).unwrap().config.smtp_username.is_none());
    assert!(matches!(s.take_jobs().as_slice(), [Job::Start { .. }]));
    // Accounts that don't send over SMTP have none.
    let ews = json!({"email": "riley@contoso.example", "password": "pw", "ews": "mail.contoso.example", "smtpUsername": "x", "smtpPassword": "y"});
    assert!(s.run("account.add", &ews).is_err());
    done(s);
}

/// Issue #1 (an Update button): MailCraft's releases are test builds of a version; a newer
/// version is an update, and so is a later test build of this one when this build is an earlier
/// test build (known by its commit). A developer's build, ahead or level, is up to date.
#[test]
fn update_checks_tell_newer_releases_and_test_builds() {
    use mailcraft_model::job::Release;
    let r = |tag: &str, version: &str, build: u32, commit: &str| Release {
        tag: tag.into(),
        version: version.into(),
        build,
        commit: commit.into(),
        page: format!("https://github.com/ZIDAN44/mailcraft/releases/tag/{tag}"),
        download: String::new(),
        ..Default::default()
    };
    let sha15 = "8bdba5ceb26a8b910102731c634d6998b8ab25d1";
    let sha14 = "23c7fb53b86cce9856b1b9fc76bf32f9ef397b83";
    let list = vec![r("v0.19.0-test.15", "0.19.0", 15, sha15), r("v0.19.0-test.14", "0.19.0", 14, sha14), r("v0.18.0-test.13", "0.18.0", 13, "")];
    let newer = |version: &str, sha: Option<&str>| crate::newer_release(version, sha, &list).map(|r| r.tag.clone());
    assert_eq!(newer("0.18.0", None).as_deref(), Some("v0.19.0-test.15"), "a newer version");
    assert_eq!(newer("0.19.0", Some(sha14)).as_deref(), Some("v0.19.0-test.15"), "a later test build of this version");
    assert_eq!(newer("0.19.0", Some(&sha14[..9])).as_deref(), Some("v0.19.0-test.15"), "a short commit");
    assert_eq!(newer("0.19.0", Some(sha15)), None, "this is the newest");
    assert_eq!(newer("0.19.0", None), None, "a developer's build of the newest version");
    assert_eq!(newer("0.20.0", Some("0123456789abcdef")), None, "a developer's build ahead");
    assert_eq!(newer("1.0", None), None);
    assert_eq!(crate::newer_release("0.19.0", None, &[]), None);

    // The command asks the network side; the answer decides what File › About shows.
    let mut s = sample_session("updates");
    s.run("app.checkUpdates", &json!({})).unwrap();
    assert_eq!(s.update, crate::UpdateCheck::Checking);
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::CheckUpdates)));
    let ahead = vec![r("v99.0.0-test.1", "99.0.0", 1, "")];
    s.apply(Event::Releases { releases: ahead, error: None });
    assert!(matches!(&s.update, crate::UpdateCheck::Available(r) if r.version == "99.0.0"));
    s.apply(Event::Releases { releases: list.clone(), error: None });
    assert!(matches!(&s.update, crate::UpdateCheck::UpToDate(_)), "{:?}", s.update);
    s.apply(Event::Releases { releases: Vec::new(), error: Some("offline".into()) });
    assert_eq!(s.update, crate::UpdateCheck::Failed("offline".into()));
    done(s);
}

/// Issue #1: every account's own calendar was named "Calendar" in one colour, so they couldn't
/// be told apart. An account's own calendar takes the account's colour (as its mail shows it),
/// its events too; its other calendars keep theirs.
#[test]
fn each_accounts_calendar_has_the_accounts_colour() {
    let mut s = sample_session("calendar-colours");
    let a = s.run("account.add", &json!({"email": "zoe@example.org", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    let mut p = mailcraft_model::pim::Pim::default();
    p.ensure_defaults();
    s.pim.insert(a.clone(), p);
    let colour = |s: &Session, account: &str, cal: &str| s.shown_calendars().into_iter().find(|c| c.0 == account && c.1 == cal).map(|c| c.3);
    let (sample, zoe) = (colour(&s, "sample", mailcraft_model::pim::LOCAL).unwrap(), colour(&s, &a, mailcraft_model::pim::LOCAL).unwrap());
    assert_ne!(sample, zoe, "two accounts' calendars, two colours");
    assert_eq!(zoe, crate::ACCOUNT_COLOR + s.account(&a).unwrap().config.color);
    assert_eq!(colour(&s, "sample", crate::sample::STUDIO_CALENDAR), Some(3), "another calendar keeps its own");
    let (from, to) = (s.clock.now - 7 * 86_400, s.clock.now + 7 * 86_400);
    assert!(s.agenda(from, to).iter().filter(|i| i.calendar == mailcraft_model::pim::LOCAL).all(|i| i.color == sample));
    done(s);
}

/// Issue #4: an account kept online only (from when it's added, or switched in Options) has none
/// of its mail kept on this computer: the store holds it in memory, its worker starts again with
/// the change, and the next start holds it in memory from the outset. POP and the sample mailbox
/// keep their mail here and can't be. A snoozed message of it is remembered without its subject.
#[test]
fn an_account_can_be_kept_online_only() {
    let mut s = sample_session("online-only");
    let store = s.store.clone().unwrap();
    let a = s.run("account.add", &json!({"email": "zoe@example.org", "password": "pw", "onlineOnly": true})).unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(s.account(&a).unwrap().config.keeps_online() && store.is_online_only(&a));
    s.take_jobs();

    assert_eq!(s.run("account.onlineOnly", &json!({"account": a, "on": false})).unwrap()["on"], false);
    assert!(!store.is_online_only(&a));
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::Start { account, .. } if account.id == a && !account.online_only)), "it starts again");
    assert_eq!(s.run("account.onlineOnly", &json!({"account": a})).unwrap()["on"], true, "the other way");
    assert!(store.is_online_only(&a));
    assert!(
        s.run("account.update", &json!({"account": a, "imapHost": "mail.example.org", "smtpHost": "mail.example.org", "onlineOnly": false})).is_ok()
    );
    assert!(!store.is_online_only(&a));
    s.run("account.update", &json!({"account": a, "imapHost": "mail.example.org", "smtpHost": "mail.example.org", "onlineOnly": true})).unwrap();
    assert!(store.is_online_only(&a));

    assert!(s.run("account.onlineOnly", &json!({"account": "sample", "on": true})).is_err(), "the sample mailbox has no server");
    assert!(!store.is_online_only("sample"));
    let pop = json!({"email": "pop@example.org", "password": "pw", "pop": true, "popHost": "pop.example.org", "popPort": 995, "smtpHost": "smtp.example.org", "onlineOnly": true});
    assert!(s.run("account.add", &pop).is_err(), "POP's mail has no other home");
    assert!(s.accounts.iter().all(|x| x.config.email != "pop@example.org"));

    // Snoozed: remembered without the subject (the sample mailbox's keeps it).
    let snooze = |account: &str, subject: &str| mailcraft_model::organize::Snooze {
        account: account.into(),
        message_id: format!("<{}@example.org>", subject.len()),
        folder: "Snoozed".into(),
        back_to: "INBOX".into(),
        until: NOW + 86_400,
        subject: subject.into(),
    };
    // (One of an account that's gone, due: dropped, and the rest saved.)
    let gone = mailcraft_model::organize::Snooze { until: NOW - 60, ..snooze("gone", "Gone") };
    s.snoozes = vec![snooze(&a, "SECRET-SNOOZED"), snooze("sample", "Sample snoozed"), gone];
    s.tick();
    let saved = std::fs::read_to_string(store.root().join("snoozes.json")).unwrap();
    assert!(!saved.contains("SECRET-SNOOZED") && saved.contains("Sample snoozed") && saved.contains(&a), "{saved}");

    // Opened again: held in memory from the start.
    let again = Session::open(Some(Store::open(store.root()).unwrap()), mailcraft_model::dates::Clock::new(NOW, 0));
    assert!(again.store.as_ref().unwrap().is_online_only(&a));
    assert!(!again.store.as_ref().unwrap().is_online_only("sample"));
    done(s);
}

/// Issue #1: a shared mailbox an account opens is a mailbox of its own in the folder pane,
/// after its account at first, and moves among the mailboxes as an account does (the accounts'
/// own order follows); the order is kept with the settings.
#[test]
fn shared_mailboxes_are_mailboxes_of_their_own_in_the_pane() {
    use mailcraft_model::Folder;
    let mut s = sample_session("pane-order");
    let a = s
        .run("account.add", &json!({"email": "riley@contoso.example", "password": "pw", "ews": "http://127.0.0.1:1/EWS/Exchange.asmx"}))
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    let folder = |path: &str, mailbox: Option<&str>| Folder {
        account: a.clone(),
        path: path.into(),
        mailbox: mailbox.map(str::to_string),
        ..Default::default()
    };
    s.apply(Event::Folders {
        account: a.clone(),
        folders: vec![folder("INBOX", None), folder("Sales", Some("Sales@contoso.example")), folder("Sales/Inbox", Some("Sales@contoso.example"))],
    });
    let keys = |s: &Session| s.pane_sections().into_iter().map(|x| x.key).collect::<Vec<_>>();
    let sales = format!("{a}|sales@contoso.example");
    assert_eq!(keys(&s), ["sample".to_string(), a.clone(), sales.clone()]);
    assert_eq!(s.pane_sections()[2].mailbox.as_deref(), Some("Sales"));
    // The shared mailbox to the top, then the sample mailbox below Riley's account.
    s.run("folderPane.move", &json!({"key": sales, "by": -2})).unwrap();
    assert_eq!(keys(&s), [sales.clone(), "sample".to_string(), a.clone()]);
    s.run("account.move", &json!({"account": "sample", "by": 1})).unwrap();
    assert_eq!(keys(&s), [sales.clone(), a.clone(), "sample".to_string()]);
    assert_eq!(s.accounts.iter().map(|x| x.config.id.clone()).collect::<Vec<_>>(), [a.clone(), "sample".to_string()], "the accounts follow");
    assert_eq!(s.prefs.pane_order, keys(&s), "kept with the settings");
    assert!(s.run("folderPane.move", &json!({"key": "nobody", "by": 1})).is_err());
    // A mailbox gone from the server leaves the order; a new one comes after its account.
    s.apply(Event::Folders { account: a.clone(), folders: vec![folder("INBOX", None), folder("Team", Some("team@contoso.example"))] });
    assert_eq!(keys(&s), [a.clone(), format!("{a}|team@contoso.example"), "sample".to_string()]);
    done(s);
}
