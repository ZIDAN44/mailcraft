//! Engine tests: commands against the sample mailbox and scripted network events.

use mailcraft_model::{Event, Flag, FolderRole, Job, Op, SyncStatus};
use mailcraft_store::Store;
use serde_json::{Value, json};

use crate::session::{ListFilter, Row, SearchScope, View};
use crate::{CmdError, Session, sample};

const NOW: i64 = 1_791_979_200;

fn temp(name: &str) -> Store {
    let dir = std::env::temp_dir().join(format!("mailcraft-engine-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    Store::open(dir).unwrap()
}

fn sample_session(name: &str) -> Session {
    let store = temp(name);
    assert!(sample::seed_if_empty(&store, NOW).unwrap());
    assert!(!sample::seed_if_empty(&store, NOW).unwrap());
    let mut s = Session::open(Some(store), mailcraft_model::dates::Clock::new(NOW, 0));
    assert!(s.take_jobs().is_empty(), "local accounts start no workers");
    s
}

fn done(s: Session) {
    if let Some(st) = &s.store {
        let _ = std::fs::remove_dir_all(st.root());
    }
}

fn key_of(s: &Session, subject: &str) -> Value {
    let m = s.list.iter().find(|m| m.subject == subject).unwrap_or_else(|| panic!("no {subject}"));
    json!({"key": m.key.to_id()})
}

#[test]
fn every_command_survives_junk_params() {
    let mut s = sample_session("junk");
    let ids: Vec<&str> = s.registry().all().iter().map(|c| c.id).collect();
    let junk = [
        json!(null),
        json!({}),
        json!({"key": "x"}),
        json!({"key": 5, "folder": [], "account": {"a": 1}}),
        json!({"value": "sideways", "index": -1, "path": "", "data": [999, -4]}),
        json!({"query": "\"unterminated from: -", "scope": 3}),
        json!({"email": "@", "password": "", "imapPort": 99_999_999}),
        json!({"index": 1e300, "limit": -5, "on": "yes"}),
    ];
    for id in &ids {
        for p in &junk {
            let _ = s.run(id, p);
        }
        // Junk params again with a message open and a compose window up.
        let k = s.list.first().map(|m| m.key.clone());
        if let Some(k) = k {
            s.open_message(&k);
        }
        let _ = s.run("mail.new", &json!({}));
        for p in &junk {
            let _ = s.run(id, p);
        }
    }
    assert!(matches!(s.run("no.such", &json!({})), Err(CmdError::Unknown(_))));
    done(s);
}

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
    assert_eq!(r.thread.len(), 2, "the reply joins the conversation");
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
    assert_eq!(s.list.len(), 2);
    done(s);
}

#[test]
fn flags_filters_and_conversations() {
    let mut s = sample_session("flags");
    s.run("list.selectAll", &json!({})).unwrap();
    s.run("mail.markRead", &json!({})).unwrap();
    assert!(s.list.iter().all(|m| m.flags.seen));
    s.run("list.filter", &json!({"value": "unread"})).unwrap();
    assert!(s.list.is_empty());
    s.run("list.filter", &json!({"value": "all"})).unwrap();
    let k = key_of(&s, "Character turnarounds");
    s.run("list.select", &k).unwrap();
    s.run("mail.flag", &json!({})).unwrap();
    s.run("list.filter", &json!({"value": "flagged"})).unwrap();
    assert_eq!(s.list.len(), 2);
    assert_eq!(s.filter, ListFilter::Flagged);
    s.run("list.filter", &json!({"value": "attachments"})).unwrap();
    assert_eq!(s.list.len(), 3);
    s.run("list.filter", &json!({"value": "all"})).unwrap();

    s.run("view.conversations", &json!({"on": true})).unwrap();
    let msgs = s.rows().into_iter().filter(|r| matches!(r, Row::Message { .. })).count();
    assert_eq!(msgs, 6, "the reply folds into its conversation");
    assert!(s.rows().iter().any(|r| matches!(r, Row::Message { count: 2, .. })));
    s.run("list.next", &json!({})).unwrap();
    s.run("list.next", &json!({})).unwrap();
    s.run("list.prev", &json!({})).unwrap();
    assert!(s.focus.is_some());
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
fn compose_reply_and_send_locally() {
    let mut s = sample_session("compose");
    let k = key_of(&s, "Your proof for the festival poster is ready");
    s.run("list.open", &k).unwrap();
    s.run("mail.reply", &json!({})).unwrap();
    let c = s.compose.clone().unwrap();
    assert_eq!(c.to, "Print Shop <orders@printshop.example>");
    assert_eq!(c.subject, "Re: Your proof for the festival poster is ready");
    assert!(c.body.contains("> The proof for order 4471"));
    assert!(c.body.contains("Riley Moreno · Studio Lantern"));
    s.run("compose.update", &json!({"body": "Approved, thank you!\n"})).unwrap();
    s.run("compose.send", &json!({})).unwrap();
    assert!(s.compose.is_none());
    assert!(s.list.iter().find(|m| m.subject.starts_with("Your proof")).unwrap().flags.answered);
    s.run("folder.open", &json!({"account": "sample", "folder": "Sent"})).unwrap();
    assert!(s.list.iter().any(|m| m.subject == "Re: Your proof for the festival poster is ready"));

    // Validation.
    s.run("mail.new", &json!({})).unwrap();
    assert!(matches!(s.run("compose.send", &json!({})), Err(CmdError::Params(_))));
    s.run("compose.update", &json!({"to": "not an address"})).unwrap();
    assert!(matches!(s.run("compose.send", &json!({})), Err(CmdError::Params(m)) if m.contains("not an address")));
    s.run("compose.update", &json!({"to": "ana@soundlab.example"})).unwrap();
    assert!(matches!(s.run("compose.send", &json!({})), Err(CmdError::Params(m)) if m.contains("subject")));
    s.run("compose.attach", &json!({"name": "notes.txt", "data": [104, 105]})).unwrap();
    s.run("compose.saveDraft", &json!({})).unwrap();
    s.run("compose.saveDraft", &json!({})).unwrap();
    s.run("folder.open", &json!({"account": "sample", "folder": "Drafts"})).unwrap();
    assert_eq!(s.list.len(), 1, "saving again replaces the earlier draft");
    s.run("compose.send", &json!({"allowEmptySubject": true})).unwrap();
    assert!(s.list.is_empty(), "the draft goes once sent");

    // Forward carries attachments.
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    let k = key_of(&s, "Character turnarounds");
    s.run("list.open", &k).unwrap();
    s.run("mail.forward", &json!({})).unwrap();
    assert_eq!(s.compose.as_ref().unwrap().attachments.len(), 1);
    s.run("compose.discard", &json!({})).unwrap();
    assert!(s.compose.is_none());
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
    assert!(s.run("folder.empty", &json!({"account": "sample", "folder": "INBOX"})).is_err(), "only Deleted Items and Junk");
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
fn messages_save_as_eml_and_pdf() {
    let mut s = sample_session("export");
    let k = key_of(&s, "Storyboard review on Thursday");
    s.run("list.open", &k).unwrap();
    let dir = s.store.as_ref().unwrap().root().join("export");
    std::fs::create_dir_all(&dir).unwrap();
    let eml = dir.join("m.eml");
    s.run("message.saveAs", &json!({"path": eml.to_string_lossy()})).unwrap();
    assert_eq!(mailcraft_mime::parse(&std::fs::read(&eml).unwrap()).unwrap().subject, "Storyboard review on Thursday");
    if let Some(font) =
        ["C:\\Windows\\Fonts\\segoeui.ttf", "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"].iter().find_map(|p| std::fs::read(p).ok())
    {
        s.print_fonts = Some((font, None));
    }
    let pdf = dir.join("m.pdf");
    s.run("message.exportPdf", &json!({"path": pdf.to_string_lossy()})).unwrap();
    let bytes = std::fs::read(&pdf).unwrap();
    assert!(bytes.starts_with(b"%PDF-") && bytes.ends_with(b"%%EOF\n"));
    if let Ok(keep) = std::env::var("MAILCRAFT_KEEP_PDF") {
        let _ = std::fs::copy(&pdf, keep);
    }
    done(s);
}

#[test]
fn attachments_preview_and_save_all() {
    let mut s = sample_session("attachments");
    let k = key_of(&s, "Your proof for the festival poster is ready");
    s.run("list.open", &k).unwrap();
    let r = s.run("attachment.preview", &json!({"index": 0})).unwrap();
    assert_eq!((r["kind"].as_str(), r["name"].as_str()), (Some("text"), Some("proof-notes.txt")));
    assert!(!r["text"].as_str().unwrap().is_empty());
    let k = key_of(&s, "Character turnarounds");
    s.run("list.open", &k).unwrap();
    assert!(s.run("attachment.preview", &json!({"index": 0})).is_err(), "a picture isn't text");
    let dir = s.store.as_ref().unwrap().root().join("out");
    std::fs::create_dir_all(&dir).unwrap();
    s.run("attachment.saveAll", &json!({"dir": dir.to_string_lossy()})).unwrap();
    let again = s.run("attachment.saveAll", &json!({"dir": dir.to_string_lossy()})).unwrap();
    assert!(again["saved"][0].as_str().unwrap().contains("(2)"), "names already taken get a number");
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2);
    assert!(s.run("attachment.saveAll", &json!({"dir": "/no/such/folder"})).is_err());
    assert_eq!(mailcraft_mime::craft_app_for("application/pdf", "a.pdf"), Some(("pdfcraft", "PdfCraft")));
    assert_eq!(mailcraft_mime::craft_app_for("application/octet-stream", "q.XLSX").map(|a| a.0), Some("gridcraft"));
    assert_eq!(mailcraft_mime::text_preview_kind("message/rfc822", "fwd.eml"), Some("message"));
    done(s);
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
    s.apply(Event::SignedIn { account: id.clone() });
    assert!(matches!(s.take_jobs().as_slice(), [Job::Start { password: None, .. }]));
    s.run("account.signIn", &json!({"account": id})).unwrap();
    assert!(matches!(s.take_jobs().as_slice(), [Job::SignIn { .. }]));
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
    s.apply(Event::Discovered { email: "zoe@example.org".into(), config: Some(c), source: "example.org".into() });
    let r = s.run("account.discover", &json!({"email": "zoe@example.org"})).unwrap();
    assert_eq!((r["found"].as_bool(), r["imap"]["host"].as_str()), (Some(true), Some("imap.example.org")));
}

#[test]
fn formatted_messages_go_out_as_html_with_pictures() {
    let mut s = sample_session("rich");
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "ann@x.example", "subject": "Plan", "body": "Hi **Ann**,\n- frames\n- sound\n"})).unwrap();
    let png = crate::sample::sample_png();
    let r = s.run("compose.insertPicture", &json!({"name": "board.png", "data": png})).unwrap();
    assert!(r["mark"].as_str().unwrap().starts_with("![board](cid:"));
    assert!(s.run("compose.insertPicture", &json!({"name": "notes.txt", "data": [1]})).is_err(), "pictures only");
    s.run("compose.send", &json!({})).unwrap();
    s.run("folder.open", &json!({"account": "sample", "folder": "Sent"})).unwrap();
    let k = key_of(&s, "Plan");
    s.run("list.open", &k).unwrap();
    let p = s.reading.as_ref().unwrap().parsed.clone().unwrap();
    assert!(p.had_html);
    assert!(p.rich.iter().flatten().any(|sp| sp.bold && sp.text == "Ann"));
    assert!(p.text.starts_with("Hi Ann,\n- frames"), "{}", p.text);
    assert_eq!(p.images.len(), 1);
    assert!(p.attachments.iter().any(|a| a.inline && a.content_id.is_some()));

    // Plain text: the marks go as typed, no HTML.
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "ann@x.example", "subject": "Plain", "body": "Keep **these** stars"})).unwrap();
    s.run("compose.format", &json!({"plain": true})).unwrap();
    s.run("compose.send", &json!({})).unwrap();
    s.run("folder.open", &json!({"account": "sample", "folder": "Sent"})).unwrap();
    let k = key_of(&s, "Plain");
    s.run("list.open", &k).unwrap();
    let p = s.reading.as_ref().unwrap().parsed.clone().unwrap();
    assert!(!p.had_html && p.text.contains("**these**"));
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
    store.save_folder(&id, "INBOX", &mailcraft_store::FolderState { validity: 1, uid_next: 3, highest_modseq: 0, messages: msgs }).unwrap();
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
fn categories_pins_and_focused_inbox() {
    let mut s = sample_session("organize");
    let k = key_of(&s, "Character turnarounds");
    s.run("list.select", &k).unwrap();
    s.run("mail.categorize", &json!({"category": "work"})).unwrap();
    assert!(s.selected()[0].flags.has("$label2"), "Work is Thunderbird's $label2");
    assert!(s.run("mail.categorize", &json!({"category": "Nope"})).is_err());
    s.run("category.create", &json!({"name": "Festival"})).unwrap();
    s.run("mail.categorize", &json!({"category": "festival"})).unwrap();
    let names: Vec<String> = s.categories_of(&s.selected()[0]).iter().map(|c| c.name.clone()).collect();
    assert_eq!(names, vec!["Work".to_string(), "Festival".to_string()]);
    // Search by category name or keyword.
    assert_eq!(s.run("search.run", &json!({"query": "category:work"})).unwrap()["results"], 1);
    assert_eq!(s.run("search.run", &json!({"query": "category:festival"})).unwrap()["results"], 1);
    s.run("search.clear", &json!({})).unwrap();
    // Stored with the message (local account: in the cache).
    let st = s.store.as_ref().unwrap().load_folder("sample", "INBOX").unwrap();
    assert!(st.messages.iter().find(|m| m.subject == "Character turnarounds").unwrap().flags.has("festival"));

    // Pinned mail comes first, under its own heading.
    s.run("list.select", &k).unwrap();
    s.run("mail.pin", &json!({})).unwrap();
    let rows = s.rows();
    assert_eq!(rows[0], Row::Header("Pinned"));
    let Row::Message { index, .. } = rows[1] else { panic!() };
    assert_eq!(s.list[index].subject, "Character turnarounds");
    assert_eq!(s.run("search.run", &json!({"query": "is:pinned"})).unwrap()["results"], 1);
    s.run("search.clear", &json!({})).unwrap();

    // Focused Inbox: list mail goes to Other; a sender can be moved for good.
    s.run("view.focused", &json!({"on": true})).unwrap();
    let focused: Vec<String> = s.list.iter().map(|m| m.subject.clone()).collect();
    assert!(!focused.contains(&"Open call: winter group show".to_string()), "{focused:?}");
    s.run("list.focusTab", &json!({"value": "other"})).unwrap();
    assert_eq!(s.list.len(), 1);
    assert_eq!(s.list[0].subject, "Open call: winter group show");
    let k2 = json!({"key": s.list[0].key.to_id()});
    s.run("list.select", &k2).unwrap();
    s.run("mail.moveToFocused", &json!({})).unwrap();
    assert!(s.list.is_empty());
    s.run("list.focusTab", &json!({"value": "focused"})).unwrap();
    assert_eq!(s.list.len(), 7);
    done(s);
}

#[test]
fn snoozed_mail_comes_back_unread() {
    let mut s = sample_session("snooze");
    let k = key_of(&s, "Character turnarounds");
    s.run("list.select", &k).unwrap();
    s.run("mail.markRead", &json!({})).unwrap();
    s.run("mail.snooze", &json!({"until": NOW + 3600})).unwrap();
    assert!(!s.list.iter().any(|m| m.subject == "Character turnarounds"));
    let snoozed = s.account("sample").unwrap().folders.iter().find(|f| f.path == "Snoozed").cloned().expect("a Snoozed folder is made");
    s.run("folder.open", &json!({"account": "sample", "folder": snoozed.path})).unwrap();
    assert_eq!(s.list.len(), 1);
    assert_eq!(s.snoozes.len(), 1);
    // Saved, so a restart still brings it back.
    let root = s.store.as_ref().unwrap().root().to_path_buf();
    let mut s = Session::open(Some(Store::open(&root).unwrap()), mailcraft_model::dates::Clock::new(NOW + 60, 0));
    assert_eq!(s.snoozes.len(), 1);
    s.tick();
    assert_eq!(s.snoozes.len(), 1, "not due yet");
    s.set_clock(NOW + 3601, 0);
    s.tick();
    assert!(s.snoozes.is_empty());
    let back = s.folder_state("sample", "INBOX").messages.iter().find(|m| m.subject == "Character turnarounds").cloned().unwrap();
    assert!(!back.flags.seen, "it comes back unread");
    assert!(s.run("mail.snooze", &json!({"preset": "someday"})).is_err());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn rules_file_new_mail_as_it_arrives() {
    let mut s = sample_session("rules");
    s.run(
        "rules.add",
        &json!({"rule": {"name": "Zoe to Archive", "conditions": [{"if": "from", "value": "zoe@"}], "actions": [{"do": "categorize", "value": "Work"}, {"do": "moveToRole", "value": "archive"}]}}),
    )
    .unwrap();
    assert!(s.run("rules.add", &json!({"rule": {"name": "x", "conditions": [], "actions": [{"do": "categorize", "value": "Nope"}]}})).is_err());
    // A message from Zoe arrives in the Inbox.
    let store = s.store.clone().unwrap();
    let mut st = store.load_folder("sample", "INBOX").unwrap();
    let mut m = st.messages[0].clone();
    m.key.uid = 500;
    m.subject = "Fresh from Zoe".into();
    m.from = mailcraft_model::EmailAddress::new("Zoe Park", "zoe@studio.example");
    m.flags.seen = false;
    m.date = NOW;
    st.messages.insert(0, m);
    store.save_folder("sample", "INBOX", &st).unwrap();
    s.apply(Event::FolderChanged { account: "sample".into(), folder: "INBOX".into() });
    assert!(!s.list.iter().any(|m| m.subject == "Fresh from Zoe"), "filed away by the rule");
    assert!(s.take_new_mail().is_empty(), "filed mail isn't announced");
    s.run("folder.open", &json!({"account": "sample", "folder": "Archive"})).unwrap();
    let filed = s.list.iter().find(|m| m.subject == "Fresh from Zoe").unwrap();
    assert!(filed.flags.has("$label2"));
    // Rules persist, and can run on a folder by hand.
    let s2 = Session::open(Some(Store::open(store.root()).unwrap()), mailcraft_model::dates::Clock::new(NOW, 0));
    assert_eq!(s2.rules.len(), 1);
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    assert_eq!(s.run("rules.run", &json!({})).unwrap()["matched"], 1, "Zoe's older message in the Inbox");
    assert!(!s.list.iter().any(|m| m.from.email == "zoe@studio.example"));
    s.run("rules.delete", &json!({"index": 0})).unwrap();
    assert!(s.rules.is_empty());
    done(s);
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
        store.save_folder(&id, "INBOX", &mailcraft_store::FolderState { validity: 9, uid_next: 99, highest_modseq: 0, messages: msgs }).unwrap();
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

#[test]
fn offline_sends_wait_in_the_outbox() {
    let store = temp("outbox");
    let mut s = Session::open(Some(store.clone()), mailcraft_model::dates::Clock::new(NOW, 0));
    let id = s.run("account.add", &json!({"email": "zoe@example.org", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    s.take_jobs();
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "ann@example.org", "bcc": "boss@example.org", "subject": "Plans", "body": "See you"})).unwrap();
    s.run("compose.attach", &json!({"name": "a.txt", "data": [104, 105]})).unwrap();
    s.run("compose.send", &json!({})).unwrap();
    let Some(Job::Send { id: send_id, raw, from, recipients, .. }) = s.take_jobs().into_iter().find(|j| matches!(j, Job::Send { .. })) else {
        panic!()
    };
    // The network layer couldn't send: it put the message in the Outbox.
    let item = mailcraft_store::OutboxItem { id: send_id, from, recipients, queued: NOW, error: Some("offline".into()), ..Default::default() };
    store.outbox_add(&id, &item, &raw).unwrap();
    s.apply(Event::Queued { account: id.clone(), id: send_id, error: "offline".into() });
    assert!(s.compose.is_none(), "a queued message is done for the user");
    assert!(s.notices.last().unwrap().text.contains("Outbox"));
    assert_eq!(s.outbox.len(), 1);

    s.run("folder.outbox", &json!({})).unwrap();
    assert_eq!(s.view_title(), "Outbox");
    assert_eq!(s.list.len(), 1);
    assert_eq!(s.list[0].subject, "Plans");
    let key = s.list[0].key.clone();
    s.run("list.open", &json!({"key": key.to_id()})).unwrap();
    assert!(s.reading.as_ref().unwrap().parsed.as_ref().unwrap().text.contains("See you"));
    // Mail commands leave Outbox entries alone; Send now asks the network layer.
    s.run("mail.flag", &json!({})).unwrap();
    assert!(s.take_jobs().is_empty());
    s.run("outbox.send", &json!({})).unwrap();
    assert!(matches!(s.take_jobs().as_slice(), [Job::FlushOutbox { .. }]));

    // Edit takes it back out, Bcc and attachment included.
    s.run("outbox.edit", &json!({"key": key.to_id()})).unwrap();
    let c = s.compose.clone().unwrap();
    assert_eq!((c.to.as_str(), c.bcc.as_str(), c.subject.as_str()), ("ann@example.org", "boss@example.org", "Plans"));
    assert_eq!(c.attachments.len(), 1);
    assert!(s.outbox.is_empty() && store.outbox(&id).is_empty());

    // Deleting from the Outbox cancels the send.
    store.outbox_add(&id, &item, &raw).unwrap();
    s.apply(Event::Queued { account: id.clone(), id: send_id, error: "later".into() });
    assert_eq!(s.list.len(), 1);
    s.run("list.selectAll", &json!({})).unwrap();
    s.run("mail.delete", &json!({})).unwrap();
    assert!(s.list.is_empty() && store.outbox(&id).is_empty());
    // A later success from the Outbox is just a notice.
    s.apply(Event::Sent { account: id.clone(), id: 999 });
    assert_eq!(s.notices.last().unwrap().text, "Message sent");
    let _ = std::fs::remove_dir_all(store.root());
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
fn calendar_events_on_this_computer() {
    let mut s = sample_session("calendar");
    s.run("view.module", &json!({"name": "calendar"})).unwrap();
    s.run("calendar.view", &json!({"mode": "week"})).unwrap();
    let week: Vec<Value> = s.run("calendar.events", &json!({})).unwrap().as_array().unwrap().clone();
    let titles: Vec<&str> = week.iter().filter_map(|e| e["summary"].as_str()).collect();
    assert_eq!(titles.iter().filter(|t| **t == "Team stand-up").count(), 3, "Monday, Wednesday and Friday: {titles:?}");
    assert!(titles.contains(&"Storyboard review"));
    let mix = week
        .iter()
        .chain(&s.run("calendar.events", &json!({"from": "2026-10-01", "to": "2026-11-30"})).unwrap().as_array().unwrap().clone())
        .find(|e| e["summary"] == "Sound mix playback")
        .cloned()
        .unwrap();
    assert_eq!(mix["needsReply"], true, "an invitation awaiting a reply");

    // A new event at a local time; the clock is UTC here.
    let uid = s
        .run("event.create", &json!({"summary": "Colour grading", "start": "2026-10-20T10:00", "end": "2026-10-20T12:30", "location": "Suite B"}))
        .unwrap()["uid"]
        .as_str()
        .unwrap()
        .to_string();
    let day = s.run("calendar.events", &json!({"from": "2026-10-20", "to": "2026-10-20"})).unwrap();
    let e = day.as_array().unwrap().iter().find(|e| e["uid"] == uid.as_str()).cloned().unwrap();
    assert_eq!(e["end"].as_i64().unwrap() - e["start"].as_i64().unwrap(), 9000);
    assert!(s.run("event.create", &json!({"summary": " ", "start": "2026-10-20"})).is_err(), "a title is needed");
    assert!(s.run("event.create", &json!({"summary": "x", "start": "2026-10-20T10:00", "end": "2026-10-20T09:00"})).is_err());
    assert!(s.run("event.create", &json!({"summary": "x", "start": "20 October"})).is_err());

    // Make it weekly, skip one week, then move the whole series.
    s.run("event.update", &json!({"uid": uid, "repeat": "weekly"})).unwrap();
    let span = json!({"from": "2026-10-20", "to": "2026-11-10"});
    let count = |s: &mut Session| s.run("calendar.events", &span).unwrap().as_array().unwrap().iter().filter(|e| e["uid"] == uid.as_str()).count();
    assert_eq!(count(&mut s), 4);
    let second =
        s.run("calendar.events", &span).unwrap().as_array().unwrap().iter().filter(|e| e["uid"] == uid.as_str()).nth(1).unwrap()["occurrence"]
            .as_i64()
            .unwrap();
    s.run("event.delete", &json!({"uid": uid, "occurrence": second})).unwrap();
    assert_eq!(count(&mut s), 3, "one occurrence skipped");
    s.run("event.update", &json!({"uid": uid, "summary": "Grading", "allDay": true})).unwrap();
    assert!(s.find_event(&uid).unwrap().1.event.all_day);

    // All of it is kept.
    let root = s.store.as_ref().unwrap().root().to_path_buf();
    let mut again = Session::open(Some(Store::open(&root).unwrap()), mailcraft_model::dates::Clock::new(NOW, 0));
    assert_eq!(again.find_event(&uid).unwrap().1.event.summary, "Grading");
    again.run("event.delete", &json!({"uid": uid})).unwrap();
    assert!(again.find_event(&uid).is_none());

    // Moving through the calendar.
    s.run("calendar.view", &json!({"mode": "month"})).unwrap();
    let r = s.run("calendar.goto", &json!({"date": "2026-01-31", "step": 1})).unwrap();
    assert_eq!(r["date"], "2026-02-28", "a month on from the 31st stays in February");
    assert_eq!(s.cal_days().1 - s.cal_days().0, 42);
    assert_eq!(s.run("calendar.goto", &json!({"date": "today"})).unwrap()["date"].as_str().map(|d| d.len()), Some(10));
    done(s);
}

#[test]
fn people_in_the_address_book() {
    let mut s = sample_session("people");
    let all = s.run("contact.list", &json!({})).unwrap();
    assert!(all.as_array().unwrap().len() >= 5);
    let hits = s.run("contact.list", &json!({"query": "lantern"})).unwrap();
    assert!(hits.as_array().unwrap().iter().all(|p| p["org"] == "Studio Lantern"));
    let uid = s
        .run("contact.create", &json!({"name": "Mira Chen", "emails": "mira@colour.example, mira.chen@home.example", "org": "Colour House"}))
        .unwrap()["uid"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(s.run("contact.create", &json!({"name": "Bad", "emails": ["not an address"]})).is_err());
    assert!(s.run("contact.create", &json!({})).is_err(), "a name or address is needed");
    // Saved people are suggested before people only seen in mail.
    let top: Vec<String> = s.suggest_contacts("mi", 2).into_iter().map(|c| c.address.email).collect();
    assert!(top.contains(&"mira@colour.example".to_string()), "{top:?}");
    s.run("contact.update", &json!({"uid": uid, "phones": ["+1 555 0101"], "title": "Colourist"})).unwrap();
    let p = s.find_person(&uid).unwrap();
    assert_eq!((p.card.phones[0].value.as_str(), p.card.title.as_str(), p.card.emails.len()), ("+1 555 0101", "Colourist", 2));
    s.run("contact.delete", &json!({"uid": uid})).unwrap();
    assert!(s.find_person(&uid).is_none());
    // The sender of a message, once.
    let k = s.list.iter().find(|m| m.from.email == "win@prizes.example").map(|m| m.key.clone());
    if let Some(k) = k {
        s.run("list.open", &json!({"key": k.to_id()})).unwrap();
        let first = s.run("contact.fromMessage", &json!({})).unwrap();
        let second = s.run("contact.fromMessage", &json!({})).unwrap();
        assert_eq!(first["uid"], second["uid"]);
        assert_eq!(second["existing"], true);
    }
    done(s);
}
