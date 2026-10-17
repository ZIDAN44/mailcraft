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
    assert_eq!(s.list.len(), 3);
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
    assert_eq!(s.list.len(), 4, "a picture, a proof, a turnaround and an invitation");
    s.run("list.filter", &json!({"value": "all"})).unwrap();

    // Follow Up: flagged with a day it's due; Mark Complete and unflagging forget it.
    let turnaround = |s: &Session| s.list.iter().find(|m| m.subject == "Character turnarounds").cloned().unwrap();
    s.run("list.select", &k).unwrap();
    let today = mailcraft_model::dates::today(&s.clock);
    s.run("mail.flag", &json!({"due": "tomorrow"})).unwrap();
    assert_eq!(s.follow_up_due(&turnaround(&s)), Some(today + 1));
    s.run("mail.flag", &json!({"due": "2026-12-01"})).unwrap();
    assert_eq!(s.follow_up_due(&turnaround(&s)).map(mailcraft_model::dates::day_string).as_deref(), Some("2026-12-01"));
    assert!(s.run("mail.flag", &json!({"due": "someday"})).is_err());
    s.run("mail.flag", &json!({"complete": true})).unwrap();
    assert!(!turnaround(&s).flags.flagged && s.prefs.follow_up.is_empty());
    s.run("mail.flag", &json!({"due": "thisWeek"})).unwrap();
    s.run("mail.flag", &json!({"on": false})).unwrap();
    assert!(s.follow_up_due(&turnaround(&s)).is_none() && s.prefs.follow_up.is_empty());
    s.run("mail.flag", &json!({"due": "none"})).unwrap();
    assert!(turnaround(&s).flags.flagged && s.follow_up_due(&turnaround(&s)).is_none());

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
    assert!(c.body.contains("\nFrom: Print Shop <orders@printshop.example>\nSent: "));
    assert!(c.body.contains("The proof for order 4471"));
    assert!(c.body.contains("Riley Moreno · Studio Lantern"));
    assert!(!c.window, "replies start in the reading pane");
    s.run("compose.popOut", &json!({})).unwrap();
    assert!(s.compose.as_ref().unwrap().window);
    s.run("compose.popIn", &json!({})).unwrap();
    assert!(!s.compose.as_ref().unwrap().window);
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

    // Forward as attachment: the whole original as an .eml, sent as it is (not base64).
    s.run("mail.forwardAsAttachment", &json!({})).unwrap();
    let c = s.compose.clone().unwrap();
    assert_eq!(c.subject, "Fwd: Character turnarounds");
    assert!(!c.body.contains("Here are the first turnarounds"), "the original isn't quoted");
    let a = &c.attachments[0];
    assert_eq!((c.attachments.len(), a.name.as_str(), a.mime.as_str()), (1, "Character turnarounds.eml", "message/rfc822"));
    assert!(String::from_utf8_lossy(&a.data).contains("Subject: Character turnarounds"));
    s.run("compose.update", &json!({"to": "ana@soundlab.example"})).unwrap();
    s.run("compose.send", &json!({})).unwrap();
    s.run("folder.open", &json!({"account": "sample", "folder": "Sent"})).unwrap();
    let k = s.list.iter().find(|m| m.subject == "Fwd: Character turnarounds").unwrap().key.clone();
    let sent = s.store.as_ref().unwrap().read_body(&k).unwrap();
    let sent = String::from_utf8_lossy(&sent);
    let part = sent.split("Content-Type: message/rfc822").nth(1).unwrap();
    assert!(part.contains("Subject: Character turnarounds") && !part[..300.min(part.len())].contains("base64"));
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
fn formatted_messages_go_out_as_html_with_pictures() {
    let mut s = sample_session("rich");
    s.run("mail.new", &json!({})).unwrap();
    // Agents and the command line can write with marks.
    s.run("compose.update", &json!({"to": "ann@x.example", "subject": "Plan", "body": "Hi **Ann**,\n- frames\n- sound\n", "marks": true})).unwrap();
    let c = s.compose.as_ref().unwrap();
    assert_eq!(c.body, "Hi Ann,\nframes\nsound\n");
    assert!(c.styles.style(3).bold && c.styles.para(1).list == mailcraft_mime::rich::List::Bullet);
    let png = crate::sample::sample_png();
    let r = s.run("compose.insertPicture", &json!({"name": "board.png", "data": png})).unwrap();
    assert!(r["cid"].as_str().is_some());
    assert!(s.compose.as_ref().unwrap().body.ends_with(mailcraft_mime::rich::PICTURE));
    assert!(s.run("compose.insertPicture", &json!({"name": "notes.txt", "data": [1]})).is_err(), "pictures only");
    s.run("compose.send", &json!({})).unwrap();
    s.run("folder.open", &json!({"account": "sample", "folder": "Sent"})).unwrap();
    let k = key_of(&s, "Plan");
    s.run("list.open", &k).unwrap();
    let p = s.reading.as_ref().unwrap().parsed.clone().unwrap();
    assert!(p.had_html);
    assert!(p.rich.iter().flatten().any(|sp| sp.bold && sp.text == "Ann"));
    assert!(p.text.starts_with("Hi Ann,\n• frames"), "{}", p.text);
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
    // Filed in a folder, it's still new mail: announced, as Outlook does.
    assert!(s.take_new_mail().iter().any(|m| m.subject == "Fresh from Zoe"), "filed mail is announced");
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

/// The calendar carried by the newest message in a folder (as `METHOD`, and the text).
fn sent_calendar(s: &Session, folder: &str, subject: &str) -> (String, String) {
    let st = s.store.as_ref().unwrap().load_folder("sample", folder).unwrap();
    let m = st
        .messages
        .iter()
        .find(|m| m.subject == subject)
        .unwrap_or_else(|| panic!("no {subject}: {:?}", st.messages.iter().map(|m| &m.subject).collect::<Vec<_>>()));
    let cal = s.message_calendar(&m.key).unwrap();
    let raw = String::from_utf8_lossy(&s.store.as_ref().unwrap().read_body(&m.key).unwrap()).to_string();
    (cal.method.unwrap_or_default(), raw)
}

#[test]
fn invitations_are_answered_and_sent() {
    let mut s = sample_session("invites");
    let k = s.list.iter().find(|m| m.subject == "Sound mix, version 3").map(|m| m.key.clone()).unwrap();
    s.run("list.open", &json!({"key": k.to_id()})).unwrap();
    let inv = s.invite().expect("an invitation");
    assert_eq!((inv.method.as_str(), inv.answer.as_str(), inv.in_calendar), ("REQUEST", "NEEDS-ACTION", true));
    assert_eq!(inv.event.summary, "Sound mix playback");
    assert!(s.run("invite.respond", &json!({"answer": "maybe"})).is_err());

    s.run("invite.respond", &json!({"answer": "accepted"})).unwrap();
    assert_eq!(s.invite().unwrap().answer, "ACCEPTED");
    let (method, raw) = sent_calendar(&s, "Sent", "Accepted: Sound mix playback");
    assert_eq!(method, "REPLY");
    assert!(raw.contains("PARTSTAT=ACCEPTED") && raw.contains("mailto:riley@studio.example"), "{raw}");
    assert!(raw.to_ascii_lowercase().contains("text/calendar"), "sent as a calendar part");
    assert!(raw.contains("To: \"Ana Lima\" <ana@soundlab.example>") || raw.contains("ana@soundlab.example"));

    // Declining takes it out of the calendar.
    s.run("invite.respond", &json!({"answer": "declined"})).unwrap();
    assert!(s.find_event("mix@sample.mailcraft").is_none());
    assert_eq!(sent_calendar(&s, "Sent", "Declined: Sound mix playback").0, "REPLY");
    // Accepting again puts it back.
    s.run("invite.respond", &json!({"answer": "tentative"})).unwrap();
    let e = s.find_event("mix@sample.mailcraft").unwrap().1.event.clone();
    assert!(e.attendees.iter().any(|a| a.email == "riley@studio.example" && a.partstat == "TENTATIVE"));
    done(s);
}

#[test]
fn own_events_invite_and_cancel_guests() {
    let mut s = sample_session("meetings");
    let uid = s
        .run(
            "event.create",
            &json!({"summary": "Trailer review", "start": "2026-10-21T15:00", "attendees": "Zoe Park <zoe@studio.example>, sam@studio.example"}),
        )
        .unwrap()["uid"]
        .as_str()
        .unwrap()
        .to_string();
    let e = s.find_event(&uid).unwrap().1.event.clone();
    assert_eq!(e.organizer.as_ref().map(|o| o.email.as_str()), Some("riley@studio.example"));
    assert!(e.attendees.iter().all(|a| a.partstat == "NEEDS-ACTION" && a.rsvp));
    let (method, raw) = sent_calendar(&s, "Sent", "Invitation: Trailer review");
    assert_eq!(method, "REQUEST");
    assert!(raw.contains("zoe@studio.example") && raw.contains("sam@studio.example"));
    assert!(s.run("event.create", &json!({"summary": "x", "start": "2026-10-21", "attendees": "not an address"})).is_err());

    // A guest's answer comes back and is recorded.
    let mut reply = e.clone();
    reply.attendees = vec![mailcraft_model::ical::Attendee {
        email: "zoe@studio.example".into(),
        name: "Zoe Park".into(),
        partstat: "ACCEPTED".into(),
        rsvp: false,
    }];
    let ics = mailcraft_model::ical::write(&[reply], Some("REPLY"), NOW);
    let d = mailcraft_mime::Draft {
        to: vec![mailcraft_model::EmailAddress::new("Riley", "riley@studio.example")],
        subject: "Accepted: Trailer review".into(),
        body: "Zoe accepted.".into(),
        attachments: vec![mailcraft_mime::OutAttachment {
            name: "invite.ics".into(),
            mime: "text/calendar; method=REPLY".into(),
            data: ics.into_bytes(),
        }],
        ..Default::default()
    };
    let raw = mailcraft_mime::build(&d, &mailcraft_model::EmailAddress::new("Zoe Park", "zoe@studio.example"), NOW, "<r1@x>").unwrap();
    let store = s.store.clone().unwrap();
    let mut inbox = store.load_folder("sample", "INBOX").unwrap();
    let key = mailcraft_model::MessageKey { account: "sample".into(), folder: "INBOX".into(), validity: inbox.validity, uid: 999 };
    inbox.messages.insert(0, mailcraft_mime::summary_from_full(&raw, key.clone(), mailcraft_model::Flags::default(), 0, NOW));
    store.save_folder("sample", "INBOX", &inbox).unwrap();
    store.write_body(&key, &raw).unwrap();
    s.apply(Event::FolderChanged { account: "sample".into(), folder: "INBOX".into() });
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    s.run("list.open", &json!({"key": key.to_id()})).unwrap();
    let inv = s.invite().unwrap();
    assert_eq!((inv.method.as_str(), inv.replier.as_ref().map(|r| r.partstat.as_str())), ("REPLY", Some("ACCEPTED")));
    s.run("invite.apply", &json!({})).unwrap();
    let zoe = s.find_event(&uid).unwrap().1.event.attendees.iter().find(|a| a.email == "zoe@studio.example").cloned().unwrap();
    assert_eq!(zoe.partstat, "ACCEPTED");

    // Called off: the guests are told.
    s.run("event.delete", &json!({"uid": uid})).unwrap();
    let (method, raw) = sent_calendar(&s, "Sent", "Cancelled: Trailer review");
    assert_eq!(method, "CANCEL");
    assert!(raw.contains("STATUS:CANCELLED"));
    done(s);
}

#[test]
fn dav_results_merge_without_losing_local_changes() {
    use mailcraft_model::ical::CalEvent;
    use mailcraft_model::pim::{CalendarInfo, DavSync, StoredEvent};
    let mut s = Session::open(Some(temp("dav-merge")), mailcraft_model::dates::Clock::new(NOW, 0));
    let id = s.run("account.add", &json!({"email": "zoe@example.org", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    let cal = "https://dav.example.org/cal/work/".to_string();
    let ev = |uid: &str, summary: &str, href: &str| StoredEvent {
        calendar: cal.clone(),
        href: href.into(),
        etag: "\"1\"".into(),
        event: CalEvent { uid: uid.into(), summary: summary.into(), start: NOW + 3600, end: NOW + 7200, ..Default::default() },
        dirty: false,
    };
    let first = DavSync {
        caldav_home: Some("https://dav.example.org/cal/".into()),
        calendars: Some(vec![CalendarInfo { id: cal.clone(), name: "Work".into(), color: 2, read_only: false, ctag: "a".into() }]),
        events: vec![ev("a", "Review", &format!("{cal}a.ics")), ev("b", "Lunch", &format!("{cal}b.ics"))],
        synced_calendars: vec![cal.clone()],
        checked: NOW,
        ..Default::default()
    };
    s.apply(Event::DavSynced { account: id.clone(), sync: Box::new(first) });
    let p = s.pim.get(&id).unwrap();
    assert_eq!(p.calendars.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["Work"], "the empty calendar on this computer gives way");
    assert_eq!(p.events.len(), 2);
    assert!(s.agenda(NOW, NOW + 86_400).iter().any(|i| i.summary == "Review" && i.color == 2));

    // An edit here goes to the server; the server meanwhile dropped "Lunch".
    s.take_jobs();
    s.run("event.update", &json!({"uid": "a", "summary": "Review (moved)"})).unwrap();
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::SyncDav { .. })), "a sync is asked for");
    let mut pushed = s.find_event("a").unwrap().1.clone();
    pushed.etag = "\"2\"".into();
    pushed.dirty = false;
    let second = DavSync {
        caldav_home: Some("https://dav.example.org/cal/".into()),
        calendars: Some(vec![CalendarInfo { id: cal.clone(), name: "Work".into(), color: 2, read_only: false, ctag: "b".into() }]),
        events: vec![pushed.clone()],
        synced_calendars: vec![cal.clone()],
        pushed_events: vec![pushed],
        checked: NOW,
        ..Default::default()
    };
    s.apply(Event::DavSynced { account: id.clone(), sync: Box::new(second) });
    let p = s.pim.get(&id).unwrap();
    assert_eq!(p.events.len(), 1, "Lunch is gone from the server");
    assert!(!p.events[0].dirty && p.events[0].etag == "\"2\"" && p.events[0].event.summary == "Review (moved)");

    // A change made while a sync was out stays to be sent.
    s.run("event.update", &json!({"uid": "a", "summary": "Review (moved again)"})).unwrap();
    let stale = DavSync {
        calendars: Some(vec![CalendarInfo { id: cal.clone(), name: "Work".into(), color: 2, read_only: false, ctag: "c".into() }]),
        events: vec![ev("a", "Review (moved)", &format!("{cal}a.ics"))],
        synced_calendars: vec![cal.clone()],
        caldav_home: Some("https://dav.example.org/cal/".into()),
        ..Default::default()
    };
    s.apply(Event::DavSynced { account: id.clone(), sync: Box::new(stale) });
    let e = s.find_event("a").unwrap().1.clone();
    assert!(e.dirty && e.event.summary == "Review (moved again)", "{e:?}");
    // A server without calendars: the one on this computer stays.
    let none = DavSync { checked: NOW, ..Default::default() };
    let mut s2 = Session::open(Some(temp("dav-none")), mailcraft_model::dates::Clock::new(NOW, 0));
    let id2 = s2.run("account.add", &json!({"email": "ann@example.org", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    s2.apply(Event::DavSynced { account: id2.clone(), sync: Box::new(none) });
    assert_eq!(s2.pim.get(&id2).unwrap().calendars[0].id, mailcraft_model::pim::LOCAL);
    assert_eq!(s2.pim.get(&id2).unwrap().caldav_home.as_deref(), Some(""), "remembered as none");
    // An event made on this computer, stored in the server's calendar (Exchange gives it a new
    // uid): one event after the merge, the server's.
    s2.run("event.create", &json!({"summary": "Poster review", "start": NOW + 3600, "end": NOW + 7200})).unwrap();
    let local = s2.pim.get(&id2).unwrap().events[0].clone();
    let server_cal = "CAL1".to_string();
    let pushed = StoredEvent { calendar: server_cal.clone(), href: "EV1".into(), etag: "ck".into(), event: local.event.clone(), dirty: false };
    let read = StoredEvent { event: CalEvent { uid: "ical-EV1".into(), ..local.event.clone() }, ..pushed.clone() };
    let merged = DavSync {
        caldav_home: Some("graph".into()),
        calendars: Some(vec![CalendarInfo { id: server_cal.clone(), name: "Calendar".into(), color: 0, read_only: false, ctag: String::new() }]),
        events: vec![read],
        synced_calendars: vec![server_cal],
        pushed_events: vec![pushed],
        checked: NOW,
        ..Default::default()
    };
    s2.apply(Event::DavSynced { account: id2.clone(), sync: Box::new(merged) });
    let p = s2.pim.get(&id2).unwrap();
    assert_eq!(p.events.iter().filter(|e| e.event.summary == "Poster review").count(), 1, "{:?}", p.events);
    assert_eq!(p.events[0].event.uid, "ical-EV1");
    done(s);
    done(s2);
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
        unread: 0,
        total: 0,
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
    s.apply(Event::Exported { account: id.clone(), messages: 100, skipped: 0, done: false, error: None });
    assert!(s.notices.last().is_some_and(|n| n.text.contains("100 messages exported so far")));
    s.apply(Event::Exported { account: id.clone(), messages: 120, skipped: 3, done: true, error: None });
    assert!(s.notices.last().is_some_and(|n| !n.error && n.text.contains("120 messages exported (3 not on this computer")));
    s.apply(Event::Exported { account: id.clone(), messages: 0, skipped: 0, done: true, error: Some("disk full".into()) });
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
fn new_mail_opens_in_a_window_and_closing_keeps_a_draft() {
    let mut s = sample_session("compose-window");
    s.run("mail.new", &json!({})).unwrap();
    assert!(s.compose.as_ref().unwrap().window, "new mail opens in its own window");
    // Nothing written (just the signature): closing leaves nothing behind.
    let v = s.run("compose.close", &json!({})).unwrap();
    assert_eq!(v["saved"], false);
    assert!(s.compose.is_none());

    s.run("view.newMailWindow", &json!({"on": false})).unwrap();
    s.run("mail.new", &json!({})).unwrap();
    assert!(!s.compose.as_ref().unwrap().window, "unless the option is off");
    s.run("compose.update", &json!({"to": "zoe@studio.example", "subject": "Plans"})).unwrap();
    let v = s.run("compose.close", &json!({})).unwrap();
    assert_eq!(v["saved"], true);
    assert!(s.compose.is_none());
    assert!(s.notices.last().is_some_and(|n| n.text == "Saved in Drafts"));
    s.run("folder.open", &json!({"account": "sample", "folder": "Drafts"})).unwrap();
    assert!(s.list.iter().any(|m| m.subject == "Plans"));
    // Closing without keeping it.
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"subject": "Gone"})).unwrap();
    assert_eq!(s.run("compose.close", &json!({"save": false})).unwrap()["saved"], false);
    done(s);
}

/// View › Arrange by, as Outlook's: sorted and grouped by sender, size, flag…, and the other way
/// round.
#[test]
fn the_list_is_arranged_by_sender_size_flag_and_reversed() {
    let mut s = sample_session("arrange");
    let dates: Vec<i64> = s.list.iter().map(|m| m.date).collect();
    assert!(dates.windows(2).all(|w| w[0] >= w[1]), "newest first by default");
    s.run("view.arrange", &json!({"by": "from"})).unwrap();
    let names: Vec<String> =
        s.list.iter().map(|m| if m.from.name.is_empty() { m.from.email.to_lowercase() } else { m.from.name.to_lowercase() }).collect();
    assert!(names.windows(2).all(|w| w[0] <= w[1]), "senders A to Z: {names:?}");
    assert!(s.rows().iter().any(|r| matches!(r, Row::Group(g) if g == "Zoe Park")), "under each sender");
    s.run("view.reverseSort", &json!({})).unwrap();
    let names: Vec<String> =
        s.list.iter().map(|m| if m.from.name.is_empty() { m.from.email.to_lowercase() } else { m.from.name.to_lowercase() }).collect();
    assert!(names.windows(2).all(|w| w[0] >= w[1]), "Z to A reversed");
    s.run("view.arrange", &json!({"by": "size"})).unwrap();
    assert!(!s.prefs.reverse_sort, "a new arrangement starts in its own order");
    assert!(s.list.windows(2).all(|w| w[0].size >= w[1].size), "largest first");
    assert!(s.rows().iter().any(|r| matches!(r, Row::Header(h) if crate::ARRANGE_HEADINGS.contains(h))));
    s.run("view.arrange", &json!({"by": "flag"})).unwrap();
    let rows = s.rows();
    let at = |h: &'static str| rows.iter().position(|r| *r == Row::Header(h));
    assert!(matches!((at("Flagged"), at("Not flagged")), (Some(a), Some(b)) if a < b), "{rows:?}");
    assert!(s.list.first().is_some_and(|m| m.flags.flagged), "flagged first");
    assert!(s.run("view.arrange", &json!({"by": "colour"})).is_err());
    s.run("view.arrange", &json!({"by": "date"})).unwrap();
    assert_eq!(s.rows()[0], Row::Header("Today"));
}

/// Send / Receive › Work Offline: ahead of anything queued, kept in the settings, and nothing
/// connects meanwhile.
#[test]
fn working_offline_comes_first_and_stops_send_receive() {
    let store = temp("offline");
    let mut s = Session::open(Some(store), mailcraft_model::dates::Clock::new(NOW, 0));
    s.run("account.add", &json!({"email": "zoe@example.org", "password": "pw"})).unwrap();
    s.run("mail.workOffline", &json!({})).unwrap();
    assert!(s.prefs.work_offline);
    assert!(s.run("mail.sync", &json!({})).is_err(), "nothing connects");
    assert!(matches!(s.take_jobs().first(), Some(Job::WorkOffline { on: true })), "before the account starts");
    s.run("mail.workOffline", &json!({"on": false})).unwrap();
    assert!(matches!(s.take_jobs().as_slice(), [Job::WorkOffline { on: false }]));
    assert!(s.run("mail.sync", &json!({})).is_ok());
    let _ = std::fs::remove_dir_all(s.store.as_ref().unwrap().root());
}

/// Undo (Ctrl+Z) puts back the last move, delete, flag or category: moved mail is found again
/// by its Message-ID, as a move gives it a new key.
#[test]
fn undo_puts_back_moves_flags_and_categories() {
    let mut s = sample_session("undo");
    assert!(s.run("edit.undo", &json!({})).is_err(), "nothing to undo yet");
    let k = key_of(&s, "Storyboard review on Thursday");
    s.run("list.select", &k).unwrap();
    let was_flagged = s.selected()[0].flags.flagged;
    s.run("mail.flag", &json!({})).unwrap();
    s.run("edit.undo", &json!({})).unwrap();
    let m = s.list.iter().find(|m| m.subject == "Storyboard review on Thursday").unwrap();
    assert_eq!(m.flags.flagged, was_flagged, "the flag is back");
    s.run("list.select", &k).unwrap();
    s.run("mail.delete", &json!({})).unwrap();
    assert!(!s.list.iter().any(|m| m.subject == "Storyboard review on Thursday"), "gone from the Inbox");
    s.run("edit.undo", &json!({})).unwrap();
    assert!(s.list.iter().any(|m| m.subject == "Storyboard review on Thursday"), "back in the Inbox");
    let trash = s.account("sample").and_then(|a| a.folder(FolderRole::Trash)).map(|f| f.path.clone()).unwrap();
    assert!(!s.folder_state("sample", &trash).messages.iter().any(|m| m.subject == "Storyboard review on Thursday"));
    assert!(s.undo.is_empty());
}

/// Filter Email, as Outlook's: categorized mail, and mail sent or copied to the account.
#[test]
fn filters_for_categorized_mail_and_mail_to_me() {
    let mut s = sample_session("filters");
    let all = s.list.len();
    s.run("list.filter", &json!({"value": "categorized"})).unwrap();
    let before = s.list.len();
    s.run("list.filter", &json!({"value": "all"})).unwrap();
    let k = key_of(&s, "Character turnarounds");
    s.run("list.select", &k).unwrap();
    let cat = s.prefs.categories[0].name.clone();
    s.run("mail.categorize", &json!({"category": cat})).unwrap();
    s.run("list.filter", &json!({"value": "categorized"})).unwrap();
    assert_eq!(s.list.len(), before + 1);
    assert!(s.list.iter().any(|m| m.subject == "Character turnarounds"));
    s.run("list.filter", &json!({"value": "toMe"})).unwrap();
    assert!(!s.list.is_empty() && s.list.len() <= all);
    assert!(s.list.iter().all(|m| m.to.iter().chain(&m.cc).any(|r| r.email.eq_ignore_ascii_case("riley@studio.example"))));
    assert!(s.run("list.filter", &json!({"value": "nonsense"})).is_err());
}

/// Delete › Ignore, as Outlook's: the conversation goes to Deleted Items, and so do replies to it
/// that arrive later; ignoring it again stops that.
#[test]
fn ignored_conversations_stay_in_deleted_items() {
    let mut s = sample_session("ignore");
    let trash = s.account("sample").and_then(|a| a.folder(FolderRole::Trash)).map(|f| f.path.clone()).unwrap();
    let k = key_of(&s, "Storyboard review on Thursday");
    s.run("list.select", &k).unwrap();
    assert_eq!(s.run("mail.ignore", &json!({})).unwrap()["moved"], 2, "Zoe's message and Sam's reply");
    assert!(!s.list.iter().any(|m| m.subject.contains("Storyboard review")));
    // A reply in it arrives.
    let store = s.store.clone().unwrap();
    let mut st = store.load_folder("sample", "INBOX").unwrap();
    let mut m = st.messages[0].clone();
    let sam = s.folder_state("sample", &trash).messages.iter().find(|m| m.subject.starts_with("Re:")).cloned().unwrap();
    m.key.uid = 600;
    m.subject = "Re: Storyboard review on Thursday".into();
    m.message_id = "late-reply@studio.example".into();
    m.in_reply_to = sam.message_id.clone();
    m.references = vec![sam.message_id.clone()];
    m.flags.seen = false;
    m.date = NOW;
    st.messages.insert(0, m);
    store.save_folder("sample", "INBOX", &st).unwrap();
    s.apply(Event::FolderChanged { account: "sample".into(), folder: "INBOX".into() });
    assert!(!s.list.iter().any(|m| m.subject.contains("Storyboard review")), "went to Deleted Items");
    assert!(s.take_new_mail().is_empty(), "not announced");
    assert!(s.folder_state("sample", &trash).messages.iter().any(|m| m.message_id == "late-reply@studio.example"));
    // Stop ignoring (from Deleted Items).
    s.run("folder.open", &json!({"account": "sample", "folder": trash})).unwrap();
    let k = key_of(&s, "Storyboard review on Thursday");
    s.run("list.select", &k).unwrap();
    assert_eq!(s.run("mail.ignore", &json!({})).unwrap()["ignoring"], false);
    assert!(!s.selected().iter().any(|m| s.is_ignored(m)), "no longer ignored");
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

/// A rule can run right away on the mail already in the Inbox, whatever folder is on screen, as
/// Outlook's "run this rule now".
#[test]
fn a_rule_runs_now_on_the_inbox() {
    let mut s = sample_session("rule-now");
    s.run("rules.add", &json!({"rule": {"name": "Zoe to Archive", "conditions": [{"if": "from", "value": "zoe@"}], "actions": [{"do": "moveToRole", "value": "archive"}]}})).unwrap();
    s.run("folder.open", &json!({"account": "sample", "folder": "Archive"})).unwrap();
    let r = s.run("rules.run", &json!({"index": 0, "inbox": true})).unwrap();
    assert_eq!(r["matched"], 1);
    assert!(s.notices.last().is_some_and(|n| n.text.contains("1 message matched")));
    assert!(!s.folder_state("sample", "INBOX").messages.iter().any(|m| m.from.email == "zoe@studio.example"), "moved out of the Inbox");
    assert!(s.list.iter().any(|m| m.from.email == "zoe@studio.example"), "and into Archive, on screen");
}

/// Mail a rule deletes isn't announced (mail it files in a folder is).
#[test]
fn mail_a_rule_deletes_is_not_announced() {
    let mut s = sample_session("rule-quiet");
    s.run("rules.add", &json!({"rule": {"name": "Zoe away", "conditions": [{"if": "from", "value": "zoe@"}], "actions": [{"do": "delete"}]}}))
        .unwrap();
    let store = s.store.clone().unwrap();
    let mut st = store.load_folder("sample", "INBOX").unwrap();
    let mut m = st.messages[0].clone();
    m.key.uid = 700;
    m.subject = "Deleted on arrival".into();
    m.from = mailcraft_model::EmailAddress::new("Zoe Park", "zoe@studio.example");
    m.flags.seen = false;
    m.date = NOW;
    st.messages.insert(0, m);
    store.save_folder("sample", "INBOX", &st).unwrap();
    s.apply(Event::FolderChanged { account: "sample".into(), folder: "INBOX".into() });
    assert!(!s.take_new_mail().iter().any(|m| m.subject == "Deleted on arrival"));
}

#[test]
fn delay_delivery_holds_mail_in_the_outbox_until_its_time() {
    let mut s = Session::open(Some(temp("delay")), mailcraft_model::dates::Clock::new(NOW, 0));
    s.run("account.add", &json!({"email": "zoe@example.org", "password": "pw"})).unwrap();
    s.take_jobs();
    s.run("mail.new", &json!({})).unwrap();
    s.run(
        "compose.update",
        &json!({"to": "ann@example.org", "subject": "Later", "body": "Hi", "replyTo": "Desk <desk@example.org>", "sendAfter": NOW + 3600}),
    )
    .unwrap();
    assert!(s.run("compose.update", &json!({"replyTo": "not an address"})).is_ok());
    assert!(s.run("compose.send", &json!({})).is_err(), "a bad Replies to address is caught");
    s.run("compose.update", &json!({"replyTo": "Desk <desk@example.org>"})).unwrap();
    assert_eq!(s.run("compose.send", &json!({})).unwrap()["sendAfter"], NOW + 3600);
    assert!(s.compose.is_none());
    assert!(!s.take_jobs().iter().any(|j| matches!(j, Job::Send { .. })), "held back, not sent");
    let (account, item) = s.outbox.first().cloned().unwrap();
    assert_eq!(item.not_before, NOW + 3600);
    let raw = s.store.as_ref().unwrap().outbox_raw(&account, item.id).unwrap();
    assert!(String::from_utf8_lossy(&raw).contains("Reply-To: \"Desk\" <desk@example.org>"));
    // Not yet: nothing asks for it. Its time: the account is asked, once.
    s.tick();
    assert!(s.take_jobs().is_empty());
    s.set_clock(NOW + 3601, 0);
    s.tick();
    s.tick();
    assert_eq!(s.take_jobs().iter().filter(|j| matches!(j, Job::FlushOutbox { .. })).count(), 1);
    // Send now sends a held-back message at once.
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "ann@example.org", "subject": "Tomorrow", "body": "x", "sendAfter": NOW + 86_400})).unwrap();
    s.run("compose.send", &json!({})).unwrap();
    s.run("folder.outbox", &json!({})).unwrap();
    let k = s.list.iter().find(|m| m.subject == "Tomorrow").unwrap().key.clone();
    assert!(s.list.iter().find(|m| m.subject == "Tomorrow").unwrap().preview.starts_with("Will be sent"));
    s.run("outbox.send", &json!({"key": k.to_id()})).unwrap();
    assert!(s.outbox.iter().all(|(_, i)| i.not_before == 0 || i.not_before <= NOW + 3601));
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::FlushOutbox { .. })));
    done(s);
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

#[test]
fn notes_are_written_coloured_and_deleted() {
    let mut s = sample_session("notes");
    assert_eq!(s.notes("").len(), 3, "the sample's notes");
    let uid = s.run("note.create", &json!({"text": "Call the print shop", "color": 9})).unwrap()["uid"].as_str().unwrap().to_string();
    let n = s.notes("").into_iter().find(|n| n.note.uid == uid).unwrap();
    assert_eq!((n.note.color, n.note.text.as_str()), (4, "Call the print shop"), "a colour past the last is the last");
    s.set_clock(NOW + 60, 0);
    s.run("note.update", &json!({"uid": uid, "text": "Call the print shop\nabout paper", "color": 0})).unwrap();
    let first = &s.notes("")[0];
    assert_eq!((first.note.uid.as_str(), first.note.title(), first.note.color), (uid.as_str(), "Call the print shop", 0), "the latest first");
    assert_eq!(s.notes("paper").len(), 2);
    assert!(s.run("note.update", &json!({"uid": "nope", "text": "x"})).is_err());
    s.run("note.delete", &json!({"uid": uid})).unwrap();
    assert!(s.notes("").iter().all(|n| n.note.uid != uid));
    // Kept with the account's calendar and contacts, across starts.
    let root = s.store.as_ref().unwrap().root().to_path_buf();
    drop(s);
    let s = Session::open(Some(Store::open(&root).unwrap()), mailcraft_model::dates::Clock::new(NOW, 0));
    assert_eq!(s.notes("").len(), 3);
    done(s);

    // An Exchange account: notes go to its Notes folder, and what the server says follows.
    let mut s = Session::open(Some(temp("notes-ews")), mailcraft_model::dates::Clock::new(NOW, 0));
    let id = s
        .run("account.add", &json!({"email": "zoe@contoso.example", "password": "pw", "ews": "https://mail.contoso.example/EWS/Exchange.asmx"}))
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    s.take_jobs();
    let uid = s.run("note.create", &json!({"text": "Sync me"})).unwrap()["uid"].as_str().unwrap().to_string();
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::SyncDav { .. })), "asks for a sync");
    assert!(s.pim[&id].notes[0].dirty);
    let stored = mailcraft_model::pim::StoredNote { href: "note:X1".into(), note: s.pim[&id].notes[0].note.clone(), ..Default::default() };
    let server_copy =
        mailcraft_model::pim::StoredNote { note: mailcraft_model::pim::Note { uid: "note:X1".into(), ..stored.note.clone() }, ..stored.clone() };
    let other = mailcraft_model::pim::StoredNote {
        href: "note:X2".into(),
        note: mailcraft_model::pim::Note { uid: "note:X2".into(), text: "From Outlook".into(), color: 2, modified: 1 },
        ..Default::default()
    };
    let sync = mailcraft_model::pim::DavSync { pushed_notes: vec![stored], notes: Some(vec![server_copy, other]), ..Default::default() };
    s.apply_dav(&id, &sync);
    let texts: Vec<String> = s.notes("").into_iter().map(|n| n.note.text).collect();
    assert_eq!(texts.len(), 2, "{texts:?}: the note sent isn't doubled");
    assert!(texts.contains(&"From Outlook".to_string()) && texts.contains(&"Sync me".to_string()));
    let _ = uid;
    s.run("note.delete", &json!({"uid": "note:X2"})).unwrap();
    assert!(s.pim[&id].deleted.iter().any(|t| t.href == "note:X2"), "deleted on the server next time");
    done(s);
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
