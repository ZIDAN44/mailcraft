//! Engine tests: the list, reading, changing and searching mail.

use super::*;

#[test]
fn sample_inbox_lists_newest_first_with_groups() {
    let s = sample_session("inbox");
    assert_eq!(s.view_title(), "Inbox");
    assert_eq!(s.list.len(), 7);
    assert_eq!(s.list[0].subject, "Re: Storyboard review on Thursday");
    let rows = s.rows();
    assert_eq!(rows[0], Row::Header("Today"));
    assert!(rows.contains(&Row::Header("Yesterday")));
    let inbox = s.account(sample::SAMPLE_ACCOUNT).unwrap().folder(FolderRole::Inbox).unwrap();
    assert_eq!((inbox.unread, inbox.total), (3, 7));
    done(s);
}

#[test]
fn open_marks_read_and_shows_text_and_attachments() {
    let mut s = sample_session("open");
    let k = key_of(&s, "Storyboard review on Thursday");
    s.run("list.open", &k).unwrap();
    let r = s.reading.clone().unwrap();
    let p = r.parsed.unwrap();
    assert!(p.text.contains("revised frames"));
    assert_eq!(p.attachments[0].name, "colour-key.png");
    assert!(r.thread.is_empty(), "one message shown, as the list isn't grouped by conversation (issue #1)");
    assert!(s.list.iter().find(|m| m.subject == "Storyboard review on Thursday").unwrap().flags.seen);
    // Persisted for local accounts.
    let st = s.store.as_ref().unwrap().load_folder(sample::SAMPLE_ACCOUNT, "INBOX").unwrap();
    assert!(st.messages.iter().find(|m| m.subject == "Storyboard review on Thursday").unwrap().flags.seen);
    let inbox = s.account(sample::SAMPLE_ACCOUNT).unwrap().folder(FolderRole::Inbox).unwrap();
    assert_eq!(inbox.unread, 2);
    // Counts survive a restart.
    let reopened = Session::open(Some(Store::open(s.store.as_ref().unwrap().root()).unwrap()), mailcraft_model::dates::Clock::new(NOW, 0));
    assert_eq!(reopened.account(sample::SAMPLE_ACCOUNT).unwrap().folder(FolderRole::Inbox).unwrap().unread, 2);
    let read = s.run("message.read", &json!({})).unwrap();
    assert_eq!(read["attachments"][0]["mime"], "image/png");
    let out = s.store.as_ref().unwrap().root().join("saved.png");
    s.run("attachment.save", &json!({"index": 0, "path": out.to_string_lossy()})).unwrap();
    assert_eq!(&std::fs::read(&out).unwrap()[..4], b"\x89PNG");
    done(s);
}

#[test]
fn delete_archive_move_and_permanent_delete() {
    let mut s = sample_session("delete");
    let k = key_of(&s, "Sound mix, version 3");
    s.run("list.open", &k).unwrap();
    s.run("mail.delete", &json!({})).unwrap();
    assert!(!s.list.iter().any(|m| m.subject == "Sound mix, version 3"));
    // The next message opens without being marked read.
    assert!(s.reading.is_some());
    s.run("folder.open", &json!({"account": "sample", "folder": "Trash"})).unwrap();
    assert_eq!(s.list.len(), 1);
    let k = key_of(&s, "Sound mix, version 3");
    s.run("list.select", &k).unwrap();
    s.run("mail.delete", &json!({})).unwrap();
    assert!(s.list.is_empty(), "deleting in Deleted Items is permanent");

    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    let k = key_of(&s, "Character turnarounds");
    s.run("list.select", &k).unwrap();
    s.run("mail.archive", &json!({})).unwrap();
    s.run("folder.open", &json!({"account": "sample", "folder": "Archive"})).unwrap();
    assert!(s.list.iter().any(|m| m.subject == "Character turnarounds"));
    let moved = s.list.iter().find(|m| m.subject == "Character turnarounds").unwrap();
    assert!(s.store.as_ref().unwrap().has_body(&moved.key), "the source moves with the message");

    let k = key_of(&s, "Kick-off notes");
    s.run("list.select", &k).unwrap();
    s.run("mail.move", &json!({"folder": "Projects/Short film"})).unwrap();
    assert!(s.run("mail.move", &json!({"folder": "Nowhere"})).is_err());
    s.run("folder.open", &json!({"account": "sample", "folder": "Projects/Short film"})).unwrap();
    assert_eq!(s.list.len(), 3);
    done(s);
}

/// Issue #1: a message's whole conversation showed above it in the reading pane though the
/// list wasn't grouped by conversation; as Outlook, it shows only while grouped, and turning
/// conversations on or off changes the open message at once.
#[test]
fn the_conversation_shows_above_a_message_only_when_grouped() {
    let mut s = sample_session("conversation-pane");
    assert!(!s.prefs.conversations);
    let k = key_of(&s, "Storyboard review on Thursday");
    s.run("list.open", &k).unwrap();
    assert!(s.reading.as_ref().unwrap().thread.is_empty());
    s.run("view.conversations", &json!({"on": true})).unwrap();
    assert_eq!(s.reading.as_ref().unwrap().thread.len(), 2, "the reply joins the conversation");
    // Arranged by something other than date, conversations still show above the message.
    s.run("view.arrange", &json!({"by": "from"})).unwrap();
    s.run("list.open", &k).unwrap();
    assert_eq!(s.reading.as_ref().unwrap().thread.len(), 2);
    s.run("view.conversations", &json!({"on": false})).unwrap();
    assert!(s.reading.as_ref().unwrap().thread.is_empty());
    done(s);
}

#[test]
fn search_finds_bodies_fields_and_attachment_names() {
    let mut s = sample_session("search");
    assert_eq!(s.run("search.run", &json!({"query": "projector"})).unwrap()["results"], 1);
    assert_eq!(s.list[0].subject, "Re: Storyboard review on Thursday");
    assert_eq!(s.run("search.run", &json!({"query": "from:zoe"})).unwrap()["results"], 2);
    assert_eq!(s.run("search.run", &json!({"query": "turnaround"})).unwrap()["results"], 1);
    assert_eq!(s.run("search.run", &json!({"query": "shot list"})).unwrap()["results"], 1, "other folders are searched too");
    assert_eq!(s.run("search.run", &json!({"query": "shot list", "scope": "current"})).unwrap()["results"], 0);
    assert_eq!(s.run("search.run", &json!({"query": "has:attachment from:jun"})).unwrap()["results"], 1);
    assert!(s.view_title().contains("turnaround") || s.view_title().contains("jun"));
    s.run("search.clear", &json!({})).unwrap();
    assert!(s.search.is_none());
    assert_eq!(s.list.len(), 7);
    let _ = SearchScope::All;
    done(s);
}

#[test]
fn folders_create_rename_move_delete_and_favorites() {
    let mut s = sample_session("folders");
    let a = sample::SAMPLE_ACCOUNT;
    let paths = |s: &Session| s.account(a).unwrap().folders.iter().map(|f| f.path.clone()).collect::<Vec<_>>();
    // Default favorites: Inbox, Sent Items, Drafts.
    assert_eq!(s.favorites().len(), 3);

    s.run("folder.create", &json!({"name": "Clients"})).unwrap();
    assert!(s.folder(a, "Clients").is_some());
    assert!(matches!(s.run("folder.create", &json!({"name": "Clients"})), Err(CmdError::Params(_))), "no duplicates");
    assert!(s.run("folder.create", &json!({"name": "a/b"})).is_err(), "no delimiters in names");
    s.run("folder.create", &json!({"name": "Café Acme", "parent": "Clients"})).unwrap();
    let acme = "Clients/Caf&AOk- Acme";
    assert!(s.folder(a, acme).is_some(), "{:?}", paths(&s));
    assert_eq!(s.folder(a, acme).unwrap().display_name(), "Café Acme");

    // File a message there, then rename the parent: the folder, its mail and the view follow.
    let k = key_of(&s, "Character turnarounds");
    s.run("list.select", &k).unwrap();
    s.run("mail.move", &json!({"folder": acme})).unwrap();
    s.run("folder.open", &json!({"account": a, "folder": acme})).unwrap();
    assert_eq!(s.list.len(), 1);
    s.run("folder.favorite", &json!({"folder": "Clients"})).unwrap();
    assert_eq!(s.favorites().len(), 4);
    s.run("folder.rename", &json!({"folder": "Clients", "name": "Customers"})).unwrap();
    let acme = "Customers/Caf&AOk- Acme";
    assert_eq!(s.view, View::Folder { account: a.into(), folder: acme.into() });
    assert_eq!(s.list.len(), 1, "the mail moved with the folder");
    assert!(s.favorites().iter().any(|f| f.folder == "Customers"), "favorites follow a rename");
    assert!(s.store.as_ref().unwrap().has_body(&s.list[0].key));

    // Move to the top level, and back inside; never into itself.
    s.run("folder.move", &json!({"folder": acme})).unwrap();
    assert!(s.folder(a, "Caf&AOk- Acme").is_some());
    assert!(s.run("folder.move", &json!({"folder": "Customers", "parent": "Customers"})).is_err());
    // Special folders stay put.
    assert!(s.run("folder.rename", &json!({"folder": "INBOX", "name": "x"})).is_err());
    assert!(s.run("folder.delete", &json!({"folder": "Sent"})).is_err());

    // Deleting goes to Deleted Items first, then for good.
    s.run("folder.delete", &json!({"folder": "Caf&AOk- Acme"})).unwrap();
    assert!(s.folder(a, "Trash/Caf&AOk- Acme").is_some(), "{:?}", paths(&s));
    s.run("folder.open", &json!({"account": a, "folder": "Trash/Caf&AOk- Acme"})).unwrap();
    s.run("folder.delete", &json!({"folder": "Trash/Caf&AOk- Acme"})).unwrap();
    assert!(s.folder(a, "Trash/Caf&AOk- Acme").is_none());
    assert_ne!(s.view, View::Folder { account: a.into(), folder: "Trash/Caf&AOk- Acme".into() }, "the view leaves a deleted folder");

    // Changes persist.
    let root = s.store.as_ref().unwrap().root().to_path_buf();
    let s2 = Session::open(Some(Store::open(&root).unwrap()), mailcraft_model::dates::Clock::new(NOW, 0));
    assert!(s2.folder(a, "Customers").is_some());
    assert!(s2.folder(a, "Clients").is_none());
    done(s);
}

#[test]
fn remote_folder_changes_are_queued_as_jobs() {
    let store = temp("remote-folders");
    let mut s = Session::open(Some(store), mailcraft_model::dates::Clock::new(NOW, 0));
    let id = s.run("account.add", &json!({"email": "zoe@example.org", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    let f = |p: &str, role| mailcraft_model::Folder {
        account: id.clone(),
        path: p.into(),
        delimiter: Some('.'),
        role,
        selectable: true,
        ..Default::default()
    };
    // A server that keeps every folder under INBOX.
    s.apply(Event::Folders { account: id.clone(), folders: vec![f("INBOX", FolderRole::Inbox), f("INBOX.Trash", FolderRole::Trash)] });
    s.take_jobs();
    s.run("folder.create", &json!({"name": "Plans"})).unwrap();
    assert!(matches!(s.take_jobs().as_slice(), [Job::Apply { op: Op::CreateFolder { path }, .. }] if path == "INBOX.Plans"));
    s.run("folder.delete", &json!({"folder": "INBOX.Plans"})).unwrap();
    assert!(matches!(s.take_jobs().as_slice(), [Job::Apply { op: Op::RenameFolder { to, .. }, .. }] if to == "INBOX.Trash.Plans"));
    s.run("folder.delete", &json!({"folder": "INBOX.Trash.Plans"})).unwrap();
    assert!(matches!(s.take_jobs().as_slice(), [Job::Apply { op: Op::DeleteFolder { path }, .. }] if path == "INBOX.Trash.Plans"));
    let _ = std::fs::remove_dir_all(s.store.as_ref().unwrap().root());
}

#[test]
fn messages_open_in_windows_of_their_own() {
    let mut s = sample_session("windows");
    let k = s.list.iter().find(|m| !m.flags.seen).map(|m| m.key.clone()).unwrap();
    s.run("message.popOut", &json!({"key": k.to_id()})).unwrap();
    s.run("message.popOut", &json!({"key": k.to_id()})).unwrap();
    assert_eq!(s.windows.len(), 1, "opening again doesn't add a window");
    assert!(s.windows[0].parsed.is_some(), "cached: shown at once");
    assert!(s.summary(&k).unwrap().flags.seen, "opening marks it read");
    assert!(s.reading.is_none(), "the reading pane is left alone");
    // Not downloaded yet: fetched, then filled in when it arrives.
    let k2 = s.list.iter().find(|m| m.key != k).map(|m| m.key.clone()).unwrap();
    s.store.as_ref().unwrap().remove_body(&k2);
    s.run("message.popOut", &json!({"key": k2.to_id()})).unwrap();
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::FetchBody { key } if *key == k2)));
    assert!(s.windows[1].parsed.is_none());
    s.apply(Event::Body {
        key: k2.clone(),
        raw: b"Subject: Arrived

hi
"
        .to_vec(),
    });
    assert_eq!(s.windows[1].parsed.as_ref().unwrap().subject, "Arrived");
    s.run("message.closeWindow", &json!({"key": k.to_id()})).unwrap();
    assert_eq!(s.windows.len(), 1);
    // The focused message without a key; and never more than a handful of windows.
    let keys: Vec<mailcraft_model::MessageKey> = s.list.iter().map(|m| m.key.clone()).collect();
    for k in &keys {
        s.run("list.select", &json!({"key": k.to_id()})).unwrap();
        s.run("message.popOut", &json!({})).unwrap();
    }
    assert_eq!(s.windows.len(), keys.len().min(crate::session::MAX_WINDOWS));
    done(s);
}

#[test]
fn mailbox_settings_commands() {
    // Emptying Deleted Items (the sample mailbox: straight to the cache).
    let mut s = sample_session("mailbox");
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    let first = s.list[0].key.clone();
    s.run("list.select", &json!({"key": first.to_id()})).unwrap();
    s.run("mail.delete", &json!({})).unwrap();
    s.run("folder.open", &json!({"account": "sample", "folder": "Trash"})).unwrap();
    assert!(!s.list.is_empty());
    // Delete All elsewhere moves everything to Deleted Items, as Outlook's.
    let inbox = s.store.as_ref().unwrap().load_folder("sample", "INBOX").unwrap().messages.len();
    let moved = s.run("folder.empty", &json!({"account": "sample", "folder": "INBOX"})).unwrap()["moved"].as_u64().unwrap();
    assert!(inbox > 0 && moved == u64::try_from(inbox).unwrap());
    assert!(s.store.as_ref().unwrap().load_folder("sample", "INBOX").unwrap().messages.is_empty());
    s.run("folder.open", &json!({"account": "sample", "folder": "Trash"})).unwrap();
    assert!(s.list.len() > inbox);
    s.run("folder.empty", &json!({"account": "sample", "role": "trash"})).unwrap();
    assert!(s.list.is_empty() && s.store.as_ref().unwrap().load_folder("sample", "Trash").unwrap().messages.is_empty());
    assert!(s.run("account.mailbox", &json!({"account": "sample"})).unwrap()["cached"].as_u64().unwrap() > 0);
    assert!(s.run("account.setVacation", &json!({"account": "sample", "text": "x"})).is_err(), "no server");
    assert!(s.run("account.update", &json!({"account": "sample", "imapHost": "x"})).is_err());
    done(s);

    // A server account: replies, quota and settings go through jobs and events.
    let mut s = Session::open(Some(temp("mailbox-remote")), mailcraft_model::dates::Clock::new(NOW, 0));
    let id = s.run("account.add", &json!({"email": "zoe@example.org", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    s.take_jobs();
    assert_eq!(s.run("account.vacation", &json!({"account": id})).unwrap()["pending"], true);
    assert!(matches!(s.take_jobs().as_slice(), [Job::GetVacation { .. }]));
    s.apply(Event::Vacation { account: id.clone(), vacation: None, error: Some("no ManageSieve".into()) });
    assert_eq!(s.run("account.vacation", &json!({"account": id, "refresh": false})).unwrap()["error"], "no ManageSieve");
    for bad in [
        json!({"text": "x", "from": "tomorrow"}),
        json!({"text": "x", "from": "2026-10-20", "until": "2026-10-19"}),
        json!({"enabled": true, "text": " "}),
    ] {
        let mut p = bad.clone();
        p["account"] = json!(id);
        assert!(s.run("account.setVacation", &p).is_err(), "{bad}");
    }
    s.run("account.setVacation", &json!({"account": id, "subject": "Away", "text": "Back Monday", "until": "2026-10-20"})).unwrap();
    assert!(
        matches!(s.take_jobs().as_slice(), [Job::SetVacation { vacation, .. }] if vacation.enabled && vacation.until.as_deref() == Some("2026-10-20"))
    );
    let v = mailcraft_model::job::Vacation { enabled: true, text: "Back Monday".into(), ..Default::default() };
    s.apply(Event::Vacation { account: id.clone(), vacation: Some(v.clone()), error: Some("refused".into()) });
    assert_eq!(s.run("account.vacation", &json!({"account": id, "refresh": false})).unwrap()["vacation"]["text"], "Back Monday");
    assert!(s.notices.last().is_some_and(|n| n.error && n.text.contains("refused")));
    s.apply(Event::Quota { account: id.clone(), used: 5 << 20, limit: 1 << 30 });
    assert_eq!(s.run("account.mailbox", &json!({"account": id})).unwrap()["quota"]["used"], 5 << 20);
    // Server settings: a change reconnects; JMAP replaces IMAP and back.
    s.run("account.update", &json!({"account": id, "imapHost": "imap.example.org", "imapPort": 993, "name": "Zoe"})).unwrap();
    assert!(matches!(s.take_jobs().as_slice(), [Job::Start { account, .. }] if account.imap.host == "imap.example.org" && account.name == "Zoe"));
    s.run("account.update", &json!({"account": id, "jmap": "https://api.example.org/jmap/session"})).unwrap();
    assert!(s.account(&id).unwrap().config.jmap.is_some());
    s.run("account.update", &json!({"account": id, "jmap": null})).unwrap();
    assert!(s.account(&id).unwrap().config.jmap.is_none());
    assert!(s.run("account.update", &json!({"account": id, "imapHost": "bad host"})).is_err());
    done(s);
}

#[test]
fn web_pictures_load_only_when_asked() {
    let store = temp("pictures");
    let mut s = Session::open(Some(store.clone()), mailcraft_model::dates::Clock::new(NOW, 0));
    let id = s.run("account.add", &json!({"email": "zoe@example.org", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    let inbox =
        mailcraft_model::Folder { account: id.clone(), path: "INBOX".into(), role: FolderRole::Inbox, selectable: true, ..Default::default() };
    s.apply(Event::Folders { account: id.clone(), folders: vec![inbox] });
    let key = |uid| mailcraft_model::MessageKey { account: id.clone(), folder: "INBOX".into(), validity: 1, uid };
    let msgs = vec![
        mailcraft_model::Summary { key: key(1), subject: "News".into(), date: NOW, ..Default::default() },
        mailcraft_model::Summary { key: key(2), subject: "More".into(), date: NOW - 1, ..Default::default() },
    ];
    store
        .save_folder(&id, "INBOX", &mailcraft_store::FolderState { validity: 1, uid_next: 3, highest_modseq: 0, messages: msgs, window: 0 })
        .unwrap();
    let html = |n: u32| {
        format!(
            "From: Shop <news@shop.example>\r\nSubject: n\r\nContent-Type: text/html\r\n\r\n<p>Sale</p><img src=\"https://cdn.shop.example/{n}.png\">"
        )
    };
    store.write_body(&key(1), html(1).as_bytes()).unwrap();
    store.write_body(&key(2), html(2).as_bytes()).unwrap();
    s.apply(Event::FolderChanged { account: id.clone(), folder: "INBOX".into() });
    s.take_jobs();
    s.run("list.open", &json!({"key": key(1).to_id()})).unwrap();
    assert!(!s.take_jobs().iter().any(|j| matches!(j, Job::FetchUrl { .. })), "nothing loads by itself");
    assert!(!s.pictures_allowed());
    assert_eq!(s.remote_pictures(), vec!["https://cdn.shop.example/1.png".to_string()]);
    s.run("message.showPictures", &json!({})).unwrap();
    assert!(matches!(s.take_jobs().as_slice(), [Job::FetchUrl { url }] if url == "https://cdn.shop.example/1.png"));
    s.apply(Event::Fetched { url: "https://cdn.shop.example/1.png".into(), data: Some(vec![1, 2]), error: None });
    assert!(matches!(s.pictures.get("https://cdn.shop.example/1.png"), Some(crate::Picture::Ready(_))));
    // Once only: the next message from the sender still asks…
    s.run("list.open", &json!({"key": key(2).to_id()})).unwrap();
    assert!(!s.take_jobs().iter().any(|j| matches!(j, Job::FetchUrl { .. })));
    // …until the sender is trusted; then pictures load on opening, and blocking stops that.
    s.run("message.showPictures", &json!({"always": true})).unwrap();
    s.take_jobs();
    s.run("list.open", &json!({"key": key(1).to_id()})).unwrap();
    assert!(s.pictures_allowed());
    s.run("message.blockPictures", &json!({})).unwrap();
    assert!(!s.pictures_allowed());
    let _ = std::fs::remove_dir_all(store.root());
}

#[test]
fn only_mail_that_arrives_later_is_announced() {
    let store = temp("notify");
    let mut s = Session::open(Some(store.clone()), mailcraft_model::dates::Clock::new(NOW, 0));
    let id = s.run("account.add", &json!({"email": "zoe@example.org", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    let inbox =
        mailcraft_model::Folder { account: id.clone(), path: "INBOX".into(), role: FolderRole::Inbox, selectable: true, ..Default::default() };
    s.apply(Event::Folders { account: id.clone(), folders: vec![inbox] });
    let m = |uid: u32, seen: bool, age: i64| mailcraft_model::Summary {
        key: mailcraft_model::MessageKey { account: id.clone(), folder: "INBOX".into(), validity: 9, uid },
        subject: format!("m{uid}"),
        date: NOW - age,
        flags: mailcraft_model::Flags { seen, ..Default::default() },
        ..Default::default()
    };
    let save = |msgs: Vec<mailcraft_model::Summary>| {
        store
            .save_folder(&id, "INBOX", &mailcraft_store::FolderState { validity: 9, uid_next: 99, highest_modseq: 0, messages: msgs, window: 0 })
            .unwrap();
    };
    // The first sync's backlog is not news.
    save(vec![m(2, false, 60), m(1, false, 120)]);
    s.apply(Event::FolderChanged { account: id.clone(), folder: "INBOX".into() });
    assert!(s.take_new_mail().is_empty());
    // Then: one new unread, one new but already read elsewhere, one old one finally synced.
    save(vec![m(5, false, 10), m(4, true, 20), m(3, false, 5 * 86_400), m(2, false, 60), m(1, false, 120)]);
    s.apply(Event::FolderChanged { account: id.clone(), folder: "INBOX".into() });
    let news: Vec<String> = s.take_new_mail().into_iter().map(|m| m.subject).collect();
    assert_eq!(news, vec!["m5".to_string()]);
    assert!(s.take_new_mail().is_empty(), "announced once");
    // Switched off: nothing.
    s.run("view.notifications", &json!({"on": false})).unwrap();
    save(vec![m(6, false, 1), m(5, false, 10)]);
    s.apply(Event::FolderChanged { account: id.clone(), folder: "INBOX".into() });
    assert!(s.take_new_mail().is_empty());
    let _ = std::fs::remove_dir_all(store.root());
}

#[test]
fn recipients_are_suggested_from_mail_seen() {
    let mut s = sample_session("contacts");
    let r = s.run("contacts.suggest", &json!({"query": "zo"})).unwrap();
    assert_eq!(r[0]["email"], "zoe@studio.example");
    assert_eq!(r[0]["address"], "Zoe Park <zoe@studio.example>");
    // Someone you wrote to (the print shop, in Sent Items) is suggested by any part of the name.
    let r = s.run("contacts.suggest", &json!({"query": "shop"})).unwrap();
    assert_eq!(r[0]["email"], "orders@printshop.example");
    assert!(r[0]["sent"].as_u64().unwrap() >= 1);
    // Your own address isn't.
    assert_eq!(s.run("contacts.suggest", &json!({"query": "riley"})).unwrap().as_array().unwrap().len(), 0);
    assert!(s.run("contacts.suggest", &json!({})).is_err());
    done(s);
}

/// A folder made in the Inbox shows under it, as Outlook shows one.
#[test]
fn a_folder_made_in_the_inbox_shows_under_it() {
    let mut s = sample_session("subfolder");
    let r = s.run("folder.create", &json!({"account": "sample", "parent": "INBOX", "name": "Clients"})).unwrap();
    let path = r["created"].as_str().unwrap().to_string();
    let order: Vec<String> = s.account("sample").unwrap().folders.iter().map(|f| f.path.clone()).collect();
    let inbox = order.iter().position(|p| p == "INBOX").unwrap();
    assert_eq!(order.get(inbox + 1), Some(&path), "right under the Inbox: {order:?}");
    assert_eq!(s.folder("sample", &path).map(mailcraft_model::Folder::depth), Some(1));
}

#[test]
fn the_samples_office_files_read_as_documents() {
    use mailcraft_mime::office::{Block, Preview, preview};
    let Some(Preview::Document(b)) = preview("order-4471.docx", "", &sample::sample_docx()) else { panic!("not a document") };
    assert!(matches!(&b[0], Block::Paragraph { heading: true, .. }));
    assert!(b.iter().any(|x| matches!(x, Block::Table(rows) if rows.len() == 3 && rows[1][1] == "A2")));
    let Some(Preview::Workbook(sheets)) = preview("order-4471.xlsx", "", &sample::sample_xlsx()) else { panic!("not a workbook") };
    assert_eq!(sheets[0].0, "Order 4471");
    assert_eq!(sheets[0].1[3], vec!["Total".to_string(), String::new(), String::new(), "802.5".into()]);
}

/// Typing in the search box stays quick on a big mailbox: a search after mail changes (which
/// rebuilds the index) doesn't parse every message again.
#[test]
fn searching_a_big_mailbox_stays_quick() {
    let store = temp("big-search");
    store.save_accounts(&[mailcraft_model::AccountConfig { id: "a1".into(), email: "me@studio.example".into(), ..Default::default() }]).unwrap();
    store
        .save_folders(
            "a1",
            &[mailcraft_model::Folder {
                account: "a1".into(),
                path: "INBOX".into(),
                role: FolderRole::Inbox,
                selectable: true,
                ..Default::default()
            }],
        )
        .unwrap();
    let mut st = store.load_folder("a1", "INBOX").unwrap();
    for uid in 1..=3000u32 {
        let key = mailcraft_model::MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid };
        let html = format!("<html><body><table><tr><td><h1>Report {uid}</h1></td><td>{}</td></tr></table></body></html>", "word ".repeat(400));
        let raw = format!("From: a@x.example\r\nSubject: Report {uid}\r\nContent-Type: text/html\r\n\r\n{html}\r\n");
        store.write_body(&key, raw.as_bytes()).unwrap();
        st.messages.push(mailcraft_model::Summary { key, subject: format!("Report {uid}"), ..Default::default() });
    }
    st.messages[1234].subject = "needle here".into();
    store.save_folder("a1", "INBOX", &st).unwrap();
    let mut s = Session::open(Some(store.clone()), mailcraft_model::dates::Clock::new(NOW, 0));
    s.take_jobs();
    let t = std::time::Instant::now();
    assert_eq!(s.run("search.run", &json!({"query": "needle"})).unwrap()["results"], 1);
    let first = t.elapsed();
    // Mail changes (as a sync does), and the next key is typed: the index is built again.
    s.open_view(View::Folder { account: "a1".into(), folder: "INBOX".into() });
    let t = std::time::Instant::now();
    assert_eq!(s.run("search.run", &json!({"query": "needl"})).unwrap()["results"], 1);
    let again = t.elapsed();
    println!("first {first:?}, again {again:?}");
    assert!(again < first / 2 || again < std::time::Duration::from_millis(300), "first {first:?}, again {again:?}");
    let _ = std::fs::remove_dir_all(store.root());
}

/// Issue #1: the reading pane marks mail read as Outlook's Reading Pane options say: when shown
/// (after a wait), once another message is chosen, or not at all; a message opened in its own
/// window (a double-click) is always read.
#[test]
fn the_reading_pane_marks_mail_read_as_chosen() {
    let mut s = sample_session("read-options");
    let unread: Vec<mailcraft_model::MessageKey> = s.list.iter().take(4).map(|m| m.key.clone()).collect();
    assert_eq!(unread.len(), 4, "the sample has mail");
    s.selection = unread.clone();
    s.run("mail.markUnread", &json!({})).unwrap();
    let seen = |s: &Session, k: &mailcraft_model::MessageKey| s.summary(k).is_some_and(|m| m.flags.seen);
    // After a wait: not read until its time, and not at all when left before.
    s.run("view.readingOptions", &json!({"when": "shown", "after": 3})).unwrap();
    s.open_message(&unread[0]);
    assert!(!seen(&s, &unread[0]));
    s.open_message(&unread[1]);
    s.clock.now += 5;
    s.tick();
    assert!(!seen(&s, &unread[0]), "left before its time");
    s.open_message(&unread[1]);
    s.clock.now += 3;
    s.tick();
    assert!(seen(&s, &unread[1]), "read once shown long enough");
    // Once another is chosen.
    s.run("view.readingOptions", &json!({"when": "left"})).unwrap();
    s.open_message(&unread[2]);
    assert!(!seen(&s, &unread[2]));
    s.open_message(&unread[3]);
    assert!(seen(&s, &unread[2]) && !seen(&s, &unread[3]));
    // Not in the pane at all: only a double-click (its own window) marks it.
    s.run("view.readingOptions", &json!({"markRead": false})).unwrap();
    s.open_message(&unread[0]);
    assert!(!seen(&s, &unread[0]));
    s.run("message.popOut", &json!({"key": unread[0].to_id()})).unwrap();
    assert!(seen(&s, &unread[0]), "opened in its own window");
    assert!(s.run("view.readingOptions", &json!({"when": "sometime"})).is_err());
    let back: crate::Prefs = serde_json::from_value(json!({"markReadOnOpen": false})).unwrap();
    assert_eq!(
        (back.mark_read_on_open, back.read_in_pane, back.read_after_secs),
        (false, crate::ReadInPane::Shown, 0),
        "older settings read as before"
    );
}

/// A big folder (tens of thousands of messages, as a 60 GB data file brings): selecting all and
/// marking them read, then deleting them, takes moments, not minutes (per message, a delete read
/// its whole folder again, and lookups went through whole lists: 8,000 took 65 s).
#[test]
fn a_big_folder_changes_all_at_once_quickly() {
    let store = temp("big-folder-ops");
    sample::seed_if_empty(&store, NOW).unwrap();
    let n = 20_000u32;
    let mut st = store.load_folder("sample", "Projects").unwrap();
    st.validity = 7;
    st.messages = (1..=n)
        .map(|uid| mailcraft_model::Summary {
            key: mailcraft_model::MessageKey { account: "sample".into(), folder: "Projects".into(), validity: 7, uid },
            subject: format!("Report {uid}"),
            date: NOW - i64::from(uid) * 60,
            ..Default::default()
        })
        .collect();
    store.save_folder("sample", "Projects", &st).unwrap();
    let mut s = Session::open(Some(store), mailcraft_model::dates::Clock::new(NOW, 0));
    s.run("folder.open", &json!({"account": "sample", "folder": "Projects"})).unwrap();
    assert_eq!(s.list.len(), n as usize);
    let began = std::time::Instant::now();
    s.run("list.selectAll", &json!({})).unwrap();
    s.run("mail.markRead", &json!({})).unwrap();
    let took = began.elapsed();
    assert!(s.list.iter().all(|m| m.flags.seen));
    assert!(s.store.as_ref().unwrap().load_folder("sample", "Projects").unwrap().messages.iter().all(|m| m.flags.seen));
    assert!(took < std::time::Duration::from_secs(15), "marking {n} read took {took:?}");
    let began = std::time::Instant::now();
    s.run("list.selectAll", &json!({})).unwrap();
    s.run("mail.delete", &json!({})).unwrap();
    let took = began.elapsed();
    assert!(s.list.is_empty(), "{}", s.list.len());
    assert!(took < std::time::Duration::from_secs(15), "deleting {n} took {took:?}");
}

/// Issue #4: mail past what is kept. A folder with more on its server says how many and shows
/// more when asked; a search asks the servers, and what they find is in the results whether or
/// not its words are in what is downloaded here.
#[test]
fn more_on_the_server_and_searching_it() {
    use mailcraft_model::{Folder, FolderRole, MessageKey, Summary};
    let mut s = sample_session("server-search");
    let a = s
        .run("account.add", &json!({"email": "zoe@example.org", "password": "pw", "imapHost": "mail.example.org", "smtpHost": "mail.example.org"}))
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    s.take_jobs();
    s.apply(Event::Folders {
        account: a.clone(),
        folders: vec![Folder { account: a.clone(), path: "INBOX".into(), role: FolderRole::Inbox, total: 50, ..Default::default() }],
    });
    let store = s.store.clone().unwrap();
    let key = |uid| MessageKey { account: a.clone(), folder: "INBOX".into(), validity: 1, uid };
    let mut st = store.load_folder(&a, "INBOX").unwrap();
    st.validity = 1;
    st.messages = (41..=50).map(|n| Summary { key: key(n), subject: format!("Message {n}"), ..Default::default() }).collect();
    store.save_folder(&a, "INBOX", &st).unwrap();
    s.view = crate::View::Folder { account: a.clone(), folder: "INBOX".into() };
    s.apply(Event::FolderChanged { account: a.clone(), folder: "INBOX".into() });
    assert_eq!(s.more_on_server(), Some((a.clone(), "INBOX".to_string(), 40)));
    s.run("folder.more", &json!({})).unwrap();
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::MoreMail { account, folder, more: 1000 } if *account == a && folder == "INBOX")));
    assert_eq!(s.showing_more, Some((a.clone(), "INBOX".to_string())));
    s.apply(Event::Status { account: a.clone(), status: SyncStatus::Online { at: 0 } });
    assert_eq!(s.showing_more, None, "settled, it can be asked again");
    assert_eq!(s.more_on_server().map(|m| m.2), Some(40));

    // Searching: nothing here has "needle"; the server finds an old message.
    assert!(s.run("search.server", &json!({})).is_err(), "search for something first");
    s.run("search.run", &json!({"query": "needle", "scope": "all"})).unwrap();
    assert!(s.list.is_empty());
    s.run("search.server", &json!({})).unwrap();
    let jobs = s.take_jobs();
    assert!(
        jobs.iter().any(
            |j| matches!(j, Job::SearchServer { account, query, folders } if *account == a && query == "needle" && folders == &["INBOX".to_string()])
        ),
        "{jobs:?}"
    );
    assert!(!jobs.iter().any(|j| matches!(j, Job::SearchServer { account, .. } if account == "sample")), "the sample mailbox has no server");
    assert_eq!(s.server_search.as_ref().map(|x| x.pending), Some(1));
    let mut st = store.load_folder(&a, "INBOX").unwrap();
    st.messages.push(Summary { key: key(7), subject: "Old one".into(), ..Default::default() });
    store.save_folder(&a, "INBOX", &st).unwrap();
    s.apply(Event::FolderChanged { account: a.clone(), folder: "INBOX".into() });
    s.apply(Event::ServerSearched { account: a.clone(), query: "needle".into(), keys: vec![key(7)], error: None });
    assert_eq!(s.list.iter().map(|m| m.subject.as_str()).collect::<Vec<_>>(), ["Old one"]);
    assert_eq!(s.server_search.as_ref().map(|x| (x.pending, x.found.len())), Some((0, 1)));
    // An answer for another query is not this search's; a new query starts again.
    s.apply(Event::ServerSearched { account: a.clone(), query: "other".into(), keys: vec![key(41)], error: None });
    assert_eq!(s.list.len(), 1);
    s.run("search.run", &json!({"query": "something else", "scope": "all"})).unwrap();
    assert!(s.server_search.is_none());
    done(s);
}

/// Issue #4: closing loses what online-only accounts hold in memory only (unsent mail, changes
/// not on the server yet): counted, to ask first. A start without a kept password asks for it,
/// once while MailCraft runs.
#[test]
fn closing_counts_whats_lost_and_a_missing_password_is_asked_for() {
    let mut s = sample_session("lost-on-close");
    let a = s
        .run("account.add", &json!({"email": "zoe@example.org", "password": "pw", "imapHost": "mail.example.org", "smtpHost": "mail.example.org", "onlineOnly": true}))
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(s.lost_on_close(), (0, 0));
    let store = s.store.clone().unwrap();
    store.save_ops(&a, &[mailcraft_model::Op::Expunge { keys: Vec::new() }]).unwrap();
    s.outbox.push((a.clone(), mailcraft_store::OutboxItem { id: 3, ..Default::default() }));
    s.outbox.push(("sample".into(), mailcraft_store::OutboxItem { id: 4, ..Default::default() }));
    assert_eq!(s.lost_on_close(), (1, 1), "only the online-only account's");
    s.run("account.onlineOnly", &json!({"account": a, "on": false})).unwrap();
    assert_eq!(s.lost_on_close(), (0, 0), "kept on this computer: nothing lost");

    let needs = |s: &mut Session| s.apply(Event::Status { account: a.clone(), status: SyncStatus::NeedsPassword { error: String::new() } });
    needs(&mut s);
    assert_eq!(s.ask_password, std::slice::from_ref(&a));
    s.ask_password.clear();
    needs(&mut s);
    assert!(s.ask_password.is_empty(), "once while MailCraft runs");
    done(s);
}
