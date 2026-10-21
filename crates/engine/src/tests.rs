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
fn compose_reply_and_send_locally() {
    let mut s = sample_session("compose");
    let k = key_of(&s, "Your proof for the festival poster is ready");
    s.run("list.open", &k).unwrap();
    s.run("mail.reply", &json!({})).unwrap();
    let c = s.compose.clone().unwrap();
    assert_eq!(c.to, "Print Shop <orders@printshop.example>; ", "ready for another address");
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
    // Only the ones asked for (those selected).
    let only = s.run("attachment.saveAll", &json!({"dir": dir.to_string_lossy(), "indices": [9999]})).unwrap();
    assert_eq!(only["saved"].as_array().map(Vec::len), Some(0));
    let index = s.reading.as_ref().and_then(|r| r.parsed.as_ref()).map(|p| p.attachments[0].index).unwrap();
    let only = s.run("attachment.saveAll", &json!({"dir": dir.to_string_lossy(), "indices": [index]})).unwrap();
    assert_eq!(only["saved"].as_array().map(Vec::len), Some(1));
    assert!(s.run("attachment.saveAll", &json!({"dir": dir.to_string_lossy(), "indices": "junk"})).is_ok(), "junk means all");
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
    assert_eq!((c.to.as_str(), c.bcc.as_str(), c.subject.as_str()), ("ann@example.org; ", "boss@example.org; ", "Plans"));
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

/// Issue #1: notes sort A to Z (by their first line), by colour, or the last changed first.
#[test]
fn notes_can_be_arranged() {
    let mut s = sample_session("notes-order");
    for (text, color) in [("banana bread", 3), ("Apple pie\nwith cream", 1), ("cherry jam", 1)] {
        s.run("note.create", &json!({"text": text, "color": color})).unwrap();
    }
    let titles = |s: &Session| {
        s.notes("")
            .iter()
            .filter(|n| ["Apple", "banana", "cherry"].iter().any(|w| n.note.text.starts_with(w)))
            .map(|n| n.note.text.lines().next().unwrap_or("").to_string())
            .collect::<Vec<_>>()
    };
    s.run("note.arrange", &json!({"by": "title"})).unwrap();
    assert_eq!(titles(&s), ["Apple pie", "banana bread", "cherry jam"]);
    s.run("note.arrange", &json!({"by": "color"})).unwrap();
    assert_eq!(titles(&s), ["Apple pie", "cherry jam", "banana bread"]);
    assert!(s.run("note.arrange", &json!({"by": "size"})).is_err());
}

/// Issue #1: a signature goes into the message being written from the message itself.
#[test]
fn a_signature_goes_in_from_the_message() {
    let mut s = sample_session("insert-signature");
    s.run("account.setSignature", &json!({"account": "sample", "signature": ""})).unwrap();
    s.run("mail.new", &json!({})).unwrap();
    s.compose.as_mut().unwrap().body = "Hello there".into();
    assert!(s.run("compose.insertSignature", &json!({})).is_err(), "no signature yet");
    s.run("account.setSignature", &json!({"account": "sample", "signature": "Riley\nStudio"})).unwrap();
    let r = s.run("compose.insertSignature", &json!({"at": 5})).unwrap();
    assert_eq!(s.compose.as_ref().unwrap().body, "Hello\n-- \nRiley\nStudio\n there");
    assert_eq!(r["end"], 23);
    // One already there is replaced, wherever the cursor is.
    s.run("account.setSignature", &json!({"account": "sample", "signature": "R."})).unwrap();
    s.run("compose.insertSignature", &json!({"at": 9999})).unwrap();
    assert_eq!(s.compose.as_ref().unwrap().body, "Hello\n-- \nR.\n there");
    // A reply made with the old signature: that one is replaced.
    let c = s.compose.as_mut().unwrap();
    c.body = "Hi\n\n-- \nOld\n\n> quoted".into();
    c.signature = "-- \nOld".into();
    s.run("compose.insertSignature", &json!({})).unwrap();
    assert_eq!(s.compose.as_ref().unwrap().body, "Hi\n\n-- \nR.\n\n> quoted");
}

/// Issue #1: a signature pasted from Outlook keeps its look: kept as HTML, and put into new
/// messages, replies and Insert signature with it; its text is the plain-text twin.
#[test]
fn a_formatted_signature_keeps_its_look() {
    let mut s = sample_session("formatted-signature");
    let html = "<p class=MsoNormal><b><span style='color:#BFBFBF'>Company bv</span></b></p>\
                <p class=MsoNormal>An street 1<br><a href=\"http://www.company.com/\">www.company.com</a></p>\
                <p class=MsoNormal><i>Confidential.</i></p>";
    s.run("account.setSignature", &json!({"account": "sample", "signature": "ignored", "html": html})).unwrap();
    let config = s.account("sample").unwrap().config.clone();
    assert_eq!(config.signature, "Company bv\nAn street 1\nwww.company.com\nConfidential.");
    assert!(!config.signature_html.is_empty());
    let check = |s: &Session| {
        let c = s.compose.as_ref().unwrap();
        let at = c.body.find("Company bv").map(|b| c.body[..b].chars().count()).unwrap();
        let st = c.styles.style(at);
        assert!(st.bold && st.color == Some([0xBF, 0xBF, 0xBF]), "{st:?}");
        let link = c.body.find("www.company").map(|b| c.body[..b].chars().count()).unwrap();
        assert_eq!(c.styles.style(link).link.as_deref(), Some("http://www.company.com/"));
        assert!(!c.styles.style(0).bold, "the text before it stays as it was");
        assert_eq!(c.styles.chars.len(), c.body.chars().count());
        let html = mailcraft_mime::rich::to_html(&c.body, &c.styles);
        assert!(html.contains("http://www.company.com/"), "{html}");
    };
    s.run("mail.new", &json!({})).unwrap();
    assert!(s.compose.as_ref().unwrap().body.ends_with("-- \nCompany bv\nAn street 1\nwww.company.com\nConfidential."));
    check(&s);
    assert!(!s.compose_written(), "a message with only its signature has nothing written");
    // Insert signature replaces it, keeping the look.
    s.run("compose.insertSignature", &json!({})).unwrap();
    check(&s);
    s.run("compose.discard", &json!({})).ok();
    s.compose = None;
    let k = s.list.first().map(|m| m.key.clone()).unwrap();
    s.run("list.open", &json!({"key": k.to_id()})).unwrap();
    s.run("mail.reply", &json!({})).unwrap();
    check(&s);
    // Saved plain again, the look goes.
    s.run("account.setSignature", &json!({"account": "sample", "signature": "R."})).unwrap();
    assert!(s.account("sample").unwrap().config.signature_html.is_empty());
    assert!(s.run("account.setSignature", &json!({"account": "sample", "signature": "x", "html": "<p><b>\u{0}"})).is_ok(), "junk HTML");
}

/// Issue #1: a contact's numbers keep what they are (business, home, mobile…), and its
/// addresses and birthday are fields of their own, as in Outlook.
#[test]
fn contacts_keep_typed_numbers_addresses_and_birthdays() {
    let mut s = sample_session("contact-fields");
    let uid = s
        .run(
            "contact.create",
            &json!({
                "name": "Jos Peeters",
                "emails": ["jos@company.example", "jos@home.example"],
                "phones": [{"value": "+32 2 123 45 67", "kind": "work"}, {"value": "+32 490 12 34 56", "kind": "cell"}, {"value": "+32 9 765 43 21", "kind": "home"}],
                "addresses": [{"kind": "work", "street": "An street 1", "postcode": "1000", "city": "Brussel", "country": "Belgium"}, {"kind": "home", "city": "Gent"}, {"street": " "}],
                "birthday": "1980-02-29",
            }),
        )
        .unwrap()["uid"]
        .as_str()
        .unwrap()
        .to_string();
    let c = s.find_person(&uid).unwrap().card;
    let kinds: Vec<(&str, &str)> = c.phones.iter().map(|p| (p.kind.as_str(), p.value.as_str())).collect();
    assert_eq!(kinds, [("work", "+32 2 123 45 67"), ("cell", "+32 490 12 34 56"), ("home", "+32 9 765 43 21")]);
    assert_eq!(c.addresses.len(), 2, "an empty one isn't kept");
    assert_eq!(c.addresses[0].one_line(), "An street 1, 1000 Brussel, Belgium");
    assert_eq!(c.birthday, "1980-02-29");
    assert_eq!(c.emails.len(), 2);
    // Changing one thing leaves the rest; a birthday that isn't a date is refused.
    s.run("contact.update", &json!({"uid": uid, "title": "Owner"})).unwrap();
    assert_eq!(s.find_person(&uid).unwrap().card.phones.len(), 3);
    assert!(s.run("contact.update", &json!({"uid": uid, "birthday": "next tuesday"})).is_err());
    s.run("contact.update", &json!({"uid": uid, "birthday": ""})).unwrap();
    assert!(s.find_person(&uid).unwrap().card.birthday.is_empty());
    // Strings still work for phones (kind unknown).
    s.run("contact.update", &json!({"uid": uid, "phones": "+1 555 0100, +1 555 0101"})).unwrap();
    assert_eq!(s.find_person(&uid).unwrap().card.phones.len(), 2);
}

/// Issue #1: Select Names opens with the address book chosen (Outlook's "Show this address list
/// first"), and one address book can be shown alone.
#[test]
fn the_address_book_opens_with_the_one_chosen() {
    let mut s = sample_session("default-book");
    let all = s.run("contact.addressBook", &json!({})).unwrap();
    let people = s.run("contact.addressBook", &json!({"source": "people"})).unwrap();
    assert!(all.as_array().unwrap().len() >= people.as_array().unwrap().len());
    let books = s.address_books();
    let (book, _, _) = books.first().cloned().expect("the sample has an address book");
    assert!(s.run("contact.defaultBook", &json!({"source": "book:nobody:x"})).is_err());
    s.run("contact.defaultBook", &json!({"source": book})).unwrap();
    assert_eq!(s.prefs.address_book, book);
    let mine = s.run("contact.addressBook", &json!({})).unwrap();
    assert_eq!(mine, s.run("contact.addressBook", &json!({"source": book})).unwrap(), "it opens with the one chosen");
    assert!(!mine.as_array().unwrap().is_empty());
    let (account, id) = book.strip_prefix("book:").and_then(|b| b.split_once(':')).unwrap();
    let in_book = s.people("").into_iter().filter(|p| p.account == account && p.book == id).flat_map(|p| p.card.emails).count();
    assert_eq!(mine.as_array().unwrap().len(), in_book);
    let back: crate::session::Prefs = serde_json::from_value(serde_json::to_value(&s.prefs).unwrap()).unwrap();
    assert_eq!(back.address_book, book, "kept with the settings");
}

/// Issue #1: events are drawn in their category's colour (shared with mail's categories), so a
/// calendar isn't all one colour.
#[test]
fn events_take_their_categorys_colour() {
    let mut s = sample_session("event-colours");
    s.run("category.create", &json!({"name": "Clients", "color": 5})).unwrap();
    let mut add = |summary: &str, categories: Value| {
        s.run("event.create", &json!({"summary": summary, "start": "2026-10-16T10:00", "categories": categories})).unwrap();
    };
    add("Pitch", json!(["clients"]));
    add("Mix", json!("Studio, Clients"));
    add("Plain", json!([]));
    let day = mailcraft_engine_day(&s);
    let item = |name: &str| s.agenda(day, day + 86_400).into_iter().find(|i| i.summary == name).unwrap();
    assert_eq!(item("Pitch").category, Some(5), "a category's own colour, whatever its case");
    let studio = item("Mix").category.unwrap();
    assert_eq!(studio, s.category_color_of("Studio"), "the first category; one from elsewhere gets one from its name");
    assert_eq!(s.category_color_of("Studio"), s.category_color_of(" studio "));
    assert_eq!(item("Plain").category, None, "no category: its calendar's colour");
    let (_, stored) = s.find_event(&item("Mix").uid).unwrap();
    assert_eq!(stored.event.categories, ["Studio", "Clients"]);
}

/// The start of 2026-10-16, local, as the sample's clock has it.
fn mailcraft_engine_day(s: &Session) -> i64 {
    crate::parse_local("2026-10-16", i64::from(s.clock.offset)).map(|(t, _)| t).unwrap()
}

/// Issue #1: calendars can be hidden (Outlook's ticks) and shown side by side, and the side
/// pane can show several months; all kept with the settings.
#[test]
fn calendars_can_be_hidden_and_shown_side_by_side() {
    let mut s = sample_session("calendar-views");
    let studio = crate::sample::STUDIO_CALENDAR;
    let names = |s: &Session| s.shown_calendars().into_iter().map(|c| c.1).collect::<Vec<_>>();
    assert_eq!(names(&s), [mailcraft_model::pim::LOCAL, studio]);
    let (from, to) = (s.clock.now - 7 * 86_400, s.clock.now + 7 * 86_400);
    let in_studio = |s: &Session| s.agenda(from, to).iter().filter(|i| i.calendar == studio).count();
    assert_eq!(in_studio(&s), 2);
    s.run("calendar.show", &json!({"account": "sample", "calendar": studio})).unwrap();
    assert_eq!(names(&s), [mailcraft_model::pim::LOCAL]);
    assert_eq!(in_studio(&s), 0, "a hidden calendar's events aren't shown");
    assert!(s.agenda(from, to).iter().any(|i| i.summary == "Lunch with Jun"));
    s.run("calendar.show", &json!({"account": "sample", "calendar": studio, "shown": true})).unwrap();
    assert_eq!(in_studio(&s), 2);
    assert!(s.run("calendar.show", &json!({"account": "sample", "calendar": "nope"})).is_err());
    s.run("calendar.sideBySide", &json!({})).unwrap();
    assert!(s.prefs.calendars_side_by_side);
    s.run("calendar.months", &json!({"count": 9})).unwrap();
    assert_eq!(s.prefs.calendar_months, 6, "at most six");
    s.run("calendar.show", &json!({"account": "sample", "calendar": studio, "shown": false})).unwrap();
    let back: crate::Prefs = serde_json::from_value(serde_json::to_value(&s.prefs).unwrap()).unwrap();
    assert_eq!((back.calendars_side_by_side, back.calendar_months, back.hidden_calendars.len()), (true, 6, 1));
    // The sample's events in categories take their colours.
    let edit = s.agenda(from, to).into_iter().find(|i| i.summary == "Lunch with Jun").unwrap();
    assert_eq!(edit.category, Some(s.category_color_of("Personal")));
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

/// Issue #1: an organisation's shared contacts (Exchange public folders) can be read, not
/// changed or deleted.
#[test]
fn shared_contacts_are_read_only() {
    use mailcraft_model::pim::{BookInfo, DavSync, StoredCard};
    let mut s = sample_session("shared-contacts");
    let card = mailcraft_model::vcard::Card { uid: "desk".into(), name: "Front desk".into(), ..Default::default() };
    let mut books = s.pim.get("sample").unwrap().books.clone();
    books.push(BookInfo { id: "public:pc".into(), name: "Company contacts".into(), read_only: true, ctag: String::new() });
    let sync = DavSync {
        books: Some(books),
        cards: vec![StoredCard { book: "public:pc".into(), href: "public:desk".into(), card, ..Default::default() }],
        synced_books: vec!["public:pc".into()],
        ..Default::default()
    };
    s.apply(Event::DavSynced { account: "sample".into(), sync: Box::new(sync) });
    assert!(s.find_person("desk").is_some(), "shown with the others");
    assert!(s.person_read_only("desk"));
    assert!(s.run("contact.update", &json!({"uid": "desk", "name": "Reception"})).is_err());
    assert!(s.run("contact.delete", &json!({"uid": "desk"})).is_err());
    assert!(s.find_person("desk").is_some());
    // New contacts go to a book that can take them.
    let made = s.run("contact.create", &json!({"name": "New one"})).unwrap();
    assert!(!s.person_read_only(made["uid"].as_str().unwrap()));
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

/// Rules go to a file and come back, on this computer or another: an account by its address,
/// a missing category made, a rule already here not doubled, junk refused.
#[test]
fn rules_export_and_import() {
    let mut s = sample_session("rules-io");
    s.run("category.create", &json!({"name": "Clients"})).unwrap();
    let rule = |name: &str, account: Option<&str>| json!({"rule": {"name": name, "conditions": [{"if": "from", "value": "printshop"}], "actions": [{"do": "categorize", "value": "Clients"}], "account": account}});
    s.run("rules.add", &rule("Print shop", Some("sample"))).unwrap();
    s.run("rules.add", &rule("Everywhere", None)).unwrap();
    let file = s.store.as_ref().unwrap().root().join("rules.mailcraft-rules");
    s.run("rules.export", &json!({"path": file.to_string_lossy()})).unwrap();
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains("\"accountEmail\"") && !text.contains("\"account\": \"sample\""), "an account by its address: {text}");

    // Here again: nothing doubled.
    let r = s.run("rules.import", &json!({"path": file.to_string_lossy()})).unwrap();
    assert_eq!((r["added"].as_u64(), s.rules.len()), (Some(0), 2));

    // Another computer: the same address, no Clients category yet.
    let mut other = sample_session("rules-io-2");
    assert!(other.category("Clients").is_none());
    let r = other.run("rules.import", &json!({"path": file.to_string_lossy()})).unwrap();
    assert_eq!((r["added"].as_u64(), r["turnedOff"].as_u64()), (Some(2), Some(0)));
    assert!(other.category("Clients").is_some(), "the category is made");
    assert_eq!(other.rules[0].account.as_deref(), Some("sample"));
    assert!(other.rules[0].enabled);

    // An account this computer doesn't have: the rule comes in turned off. (Saved again by an
    // editor that starts it with a byte order mark.)
    let foreign = text.replacen("riley@studio.example", "nobody@elsewhere.example", 1);
    std::fs::write(&file, [&b"\xEF\xBB\xBF"[..], foreign.as_bytes()].concat()).unwrap();
    let r = other.run("rules.import", &json!({"path": file.to_string_lossy(), "replace": true})).unwrap();
    assert_eq!((r["turnedOff"].as_u64(), other.rules.len()), (Some(1), 2));
    assert!(!other.rules[0].enabled && other.rules[0].account.is_none());

    // Junk, and a file that isn't one.
    for junk in [&b"{\"rules\": []}"[..], b"not json", b"{\"mailcraftRules\": 1, \"rules\": [{\"name\": 3}]}", &[0xff; 64][..]] {
        std::fs::write(&file, junk).unwrap();
        assert!(other.run("rules.import", &json!({"path": file.to_string_lossy()})).is_err());
    }
    assert_eq!(other.rules.len(), 2, "a bad file changes nothing");
    assert!(other.run("rules.import", &json!({"path": "/no/such/file"})).is_err());
    done(other);
    done(s);
}

/// A new message to someone (a contact card's Send email): To filled in, ready for more.
#[test]
fn new_mail_to_someone() {
    let mut s = sample_session("new-mail-to");
    s.run("mail.new", &json!({"to": "Zoe Park <zoe@studio.example>"})).unwrap();
    assert_eq!(s.compose.as_ref().unwrap().to, "Zoe Park <zoe@studio.example>; ");
    s.compose = None;
    assert!(s.run("mail.new", &json!({"to": "not an address"})).is_err());
    assert!(s.compose.is_none(), "nothing opens for a bad address");
    s.run("mail.new", &json!({"to": "  "})).unwrap();
    assert_eq!(s.compose.as_ref().unwrap().to, "");
    done(s);
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

/// Reminders: an event's reminder comes due at its time, snoozes, comes back, and goes when
/// dismissed; each occurrence of a series has its own; one without a reminder, or cancelled,
/// never comes; a hidden calendar's still does; and bad params are refused.
#[test]
fn reminders_come_snooze_and_go() {
    let mut s = sample_session("reminders");
    let tomorrow = s.clock.now - s.clock.now.rem_euclid(86_400) + 86_400;
    let (start, review_at) = (tomorrow + 9 * 3600, tomorrow + 14 * 3600);
    let day = mailcraft_model::dates::day_string(start.div_euclid(86_400));
    let make = |s: &mut Session, summary: &str, time: &str, extra: Value| -> String {
        let mut p = json!({"summary": summary, "start": format!("{day}T{time}"), "end": format!("{day}T{}", if time == "09:00" { "10:00" } else { "15:00" })});
        if let (Some(o), Some(x)) = (p.as_object_mut(), extra.as_object()) {
            o.extend(x.clone());
        }
        s.run("event.create", &p).unwrap()["uid"].as_str().unwrap().to_string()
    };
    let standup = make(&mut s, "Stand-up", "09:00", json!({"reminder": 15, "repeat": "daily"}));
    let quiet = make(&mut s, "No reminder", "09:00", json!({}));
    let review = make(&mut s, "Review", "14:00", json!({"reminder": "60"}));
    let due = |s: &Session| s.due_reminders(s.clock.now).into_iter().map(|r| r.summary).collect::<Vec<_>>();
    // A day before: nothing yet.
    s.clock.now = start - 86_400;
    assert!(due(&s).is_empty(), "{:?}", due(&s));
    // At 08:45 the stand-up's is due; the quiet one never.
    s.clock.now = start - 15 * 60;
    assert_eq!(due(&s), ["Stand-up"]);
    let key = s.due_reminders(s.clock.now)[0].key.clone();
    assert!(key.starts_with("local|") || key.contains(&standup), "{key}");
    // Snoozed five minutes: gone, then back.
    s.run("reminders.snooze", &json!({"key": key, "minutes": 5})).unwrap();
    assert!(due(&s).is_empty());
    s.clock.now += 5 * 60;
    assert_eq!(due(&s), ["Stand-up"]);
    // Snooze until "0 minutes before start" when that's still ahead; refused once it's passed.
    s.run("reminders.snooze", &json!({"key": key, "beforeStart": 0})).unwrap();
    assert!(due(&s).is_empty());
    s.clock.now = start;
    assert_eq!(due(&s), ["Stand-up"]);
    assert!(s.run("reminders.snooze", &json!({"key": key, "beforeStart": 5})).is_err());
    // Dismissed: gone for good; tomorrow's occurrence has its own.
    s.run("reminders.dismiss", &json!({"key": key})).unwrap();
    assert!(due(&s).is_empty());
    s.clock.now = start + 86_400 - 10 * 60;
    assert_eq!(due(&s), ["Review", "Stand-up"], "the next day's stand-up, and yesterday's review still unanswered (overdue)");
    assert_eq!(s.run("reminders.dismiss", &json!({})).unwrap()["dismissed"], 2);
    assert!(due(&s).is_empty());
    // The 14:00 review's an hour before; hiding its calendar doesn't silence it.
    s.prefs.reminders.clear();
    s.clock.now = review_at - 3600;
    s.prefs.hidden_calendars.insert("local:local".into());
    assert_eq!(due(&s), ["Stand-up", "Review"], "the stand-up's of 09:00 is still unanswered too (soonest first)");
    s.run("event.update", &json!({"uid": standup, "reminder": "none"})).unwrap();
    assert_eq!(due(&s), ["Review"]);
    // Changing its reminder to none: gone; a cancelled event never comes.
    s.run("event.update", &json!({"uid": review, "reminder": "none"})).unwrap();
    assert!(due(&s).is_empty());
    s.run("event.update", &json!({"uid": review, "reminder": 60})).unwrap();
    assert_eq!(due(&s), ["Review"]);
    s.update_event(&review, |e| e.status = "CANCELLED".into()).unwrap();
    assert!(due(&s).is_empty());
    // Long after it ended, an unanswered reminder no longer waits.
    s.update_event(&review, |e| e.status = String::new()).unwrap();
    s.clock.now = review_at + 3 * 86_400;
    assert!(!due(&s).contains(&"Review".to_string()));
    // Junk.
    for bad in
        [json!({"key": "nope", "minutes": 5}), json!({"key": key}), json!({"key": key, "minutes": 0}), json!({"key": key, "minutes": 99_999_999})]
    {
        assert!(s.run("reminders.snooze", &bad).is_err(), "{bad}");
    }
    assert!(s.run("reminders.dismiss", &json!({"key": "nope"})).is_err());
    for bad in [json!(-1), json!(999_999), json!("soon"), json!(true)] {
        assert!(s.run("event.update", &json!({"uid": quiet, "reminder": bad})).is_err(), "{bad}");
    }
    assert!(s.run("reminders.list", &json!({})).unwrap().is_array());
    done(s);
}

/// Junk Email lists, as Outlook's: a blocked sender's mail goes to Junk as it arrives, a safe
/// sender's comes back out of it; a domain covers every address at it; safe wins over blocked.
#[test]
fn blocked_and_safe_senders_sort_mail_as_it_arrives() {
    let mut s = sample_session("junk-lists");
    let junk = s.account("sample").and_then(|a| a.folder(FolderRole::Junk)).map(|f| f.path.clone()).unwrap();
    let store = s.store.clone().unwrap();
    let arrive = |s: &mut Session, folder: &str, uid: u32, from: &str, subject: &str| {
        let mut st = store.load_folder("sample", folder).unwrap();
        let mut m = store.load_folder("sample", "INBOX").unwrap().messages[0].clone();
        m.key.folder = folder.into();
        m.key.uid = uid;
        m.key.validity = st.validity;
        m.from = mailcraft_model::EmailAddress::new("", from);
        m.subject = subject.into();
        m.message_id = format!("{uid}@junk.example");
        m.in_reply_to.clear();
        m.references.clear();
        m.flags.seen = false;
        m.date = NOW;
        st.messages.insert(0, m);
        store.save_folder("sample", folder, &st).unwrap();
        s.apply(Event::FolderChanged { account: "sample".into(), folder: folder.into() });
    };
    let inbox_has = |s: &mut Session, subject: &str| s.folder_state("sample", "INBOX").messages.iter().any(|m| m.subject == subject);
    let junk_has = |s: &mut Session, subject: &str| s.folder_state("sample", &junk).messages.iter().any(|m| m.subject == subject);
    // Block Zoe from the selection: her mail goes to Junk now, and as it comes.
    let k = key_of(&s, "Storyboard review on Thursday");
    s.run("list.select", &k).unwrap();
    let r = s.run("junk.blockSender", &json!({})).unwrap();
    assert_eq!(r["added"], json!(["zoe@studio.example"]));
    assert!(junk_has(&mut s, "Storyboard review on Thursday"));
    s.take_new_mail();
    arrive(&mut s, "INBOX", 700, "ZOE@Studio.Example", "Again");
    assert!(junk_has(&mut s, "Again") && !inbox_has(&mut s, "Again"), "an address matches whatever its case");
    assert!(s.take_new_mail().is_empty(), "blocked mail isn't announced");
    // A domain: every address at it, not its look-alikes.
    s.run("junk.edit", &json!({"list": "blocked", "entry": "Spam.Example"})).unwrap();
    arrive(&mut s, "INBOX", 701, "a@spam.example", "Offer");
    arrive(&mut s, "INBOX", 702, "b@notspam.example", "Fine");
    arrive(&mut s, "INBOX", 703, "c@spam.example.org", "Fine too");
    assert!(junk_has(&mut s, "Offer") && inbox_has(&mut s, "Fine") && inbox_has(&mut s, "Fine too"));
    // Safe wins: never blocking a@ lets just that address through.
    s.run("junk.edit", &json!({"list": "safe", "entry": "<a@spam.example>"})).unwrap();
    arrive(&mut s, "INBOX", 704, "a@spam.example", "Through");
    arrive(&mut s, "INBOX", 705, "d@spam.example", "Still junk");
    assert!(inbox_has(&mut s, "Through") && junk_has(&mut s, "Still junk"));
    // A safe sender's mail filed in Junk by the server comes back to the Inbox: Junk's mark is
    // set as soon as its folder is known, so the first change it sees is already an arrival
    // (found against a real server: Junk empty at the first sync sent no change).
    s.seen_uid.clear();
    let folders = s.account("sample").unwrap().folders.clone();
    s.apply(Event::Folders { account: "sample".into(), folders });
    arrive(&mut s, &junk, 900, "a@spam.example", "Misfiled");
    assert!(inbox_has(&mut s, "Misfiled") && !junk_has(&mut s, "Misfiled"));
    // Never block from Junk: on the safe list, off the blocked one, and back to the Inbox.
    s.run("folder.open", &json!({"account": "sample", "folder": junk})).unwrap();
    let k = key_of(&s, "Again");
    s.run("list.select", &k).unwrap();
    s.run("junk.neverBlockSender", &json!({})).unwrap();
    assert!(inbox_has(&mut s, "Again"));
    assert!(s.prefs.safe_senders.contains("zoe@studio.example") && !s.prefs.blocked_senders.contains("zoe@studio.example"));
    // Not junk, always trusting the domain's sender; the domain itself.
    let k = key_of(&s, "Still junk");
    s.run("list.select", &k).unwrap();
    s.run("junk.neverBlockDomain", &json!({})).unwrap();
    assert!(s.prefs.safe_senders.contains("@spam.example") && !s.prefs.blocked_senders.contains("@spam.example"));
    assert!(inbox_has(&mut s, "Still junk"));
    // Junk entries and params.
    for bad in ["", "@", "nobody", "a@", "@-x.example", "x..example", "a@b@c.example", &"x".repeat(400)] {
        assert!(s.run("junk.edit", &json!({"list": "blocked", "entry": bad})).is_err(), "{bad:?}");
    }
    assert!(s.run("junk.edit", &json!({"list": "nope", "entry": "a@b.example"})).is_err());
    assert!(s.run("junk.edit", &json!({"entry": "a@b.example"})).is_err());
    let r = s.run("junk.edit", &json!({"list": "safe", "entry": "a@spam.example", "on": false})).unwrap();
    assert!(!r["safe"].as_array().unwrap().iter().any(|e| e == "a@spam.example"));
    // The lists are kept with the settings.
    let lists = s.run("junk.lists", &json!({})).unwrap();
    assert!(lists["safe"].as_array().unwrap().iter().any(|e| e == "@spam.example"));
    let back: crate::session::Prefs = serde_json::from_value(serde_json::to_value(&s.prefs).unwrap()).unwrap();
    assert_eq!((back.safe_senders, back.blocked_senders), (s.prefs.safe_senders.clone(), s.prefs.blocked_senders.clone()));
}

/// Clean Up: in a conversation, a message a later reply quotes in full goes to Deleted Items
/// (undoably); one that isn't, isn't downloaded, or is flagged, stays.
#[test]
fn clean_up_moves_only_what_a_reply_quotes() {
    let mut s = sample_session("clean-up");
    let trash = s.account("sample").and_then(|a| a.folder(FolderRole::Trash)).map(|f| f.path.clone()).unwrap();
    let store = s.store.clone().unwrap();
    let mut st = store.load_folder("sample", "INBOX").unwrap();
    let template = st.messages[0].clone();
    let thread = [
        (801, "Frames for Thursday", "Can you send the frames?\r\n\r\nZoe", true),
        (802, "Re: Frames for Thursday", "Sure.\r\n\r\n> Can you send the frames?\r\n>\r\n> Zoe", true),
        (803, "Re: Frames for Thursday", "Thanks!\r\n\r\n> Sure.\r\n>\r\n>> Can you send the frames?\r\n>>\r\n>> Zoe", true),
        // Another conversation: one not downloaded, and a flagged one.
        (811, "Budget", "Numbers attached.", false),
        (812, "Re: Budget", "Got them.\r\n> Numbers attached.", true),
    ];
    for (i, (uid, subject, body, downloaded)) in thread.iter().enumerate() {
        let mut m = template.clone();
        m.key.uid = *uid;
        m.subject = subject.to_string();
        m.message_id = format!("c{uid}@studio.example");
        let parent = if *uid == 801 || *uid == 811 { None } else { Some(format!("c{}@studio.example", uid - 1)) };
        m.in_reply_to = parent.clone().unwrap_or_default();
        m.references = if *uid == 803 { vec!["c801@studio.example".into(), "c802@studio.example".into()] } else { parent.into_iter().collect() };
        m.date = NOW - 1000 + i as i64;
        m.flags.seen = true;
        m.has_attachments = false;
        st.messages.insert(0, m.clone());
        if *downloaded {
            let raw = format!(
                "From: a@b.example\r\nSubject: {subject}\r\nMessage-ID: <c{uid}@studio.example>\r\nContent-Type: text/plain\r\n\r\n{body}\r\n"
            );
            store.write_body(&m.key, raw.as_bytes()).unwrap();
        }
    }
    store.save_folder("sample", "INBOX", &st).unwrap();
    s.apply(Event::FolderChanged { account: "sample".into(), folder: "INBOX".into() });
    let in_trash = |s: &mut Session, uid_subject: &str| s.folder_state("sample", &trash).messages.iter().any(|m| m.message_id == uid_subject);
    // The selection's conversation only.
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    let k = s.list.iter().find(|m| m.message_id == "c803@studio.example").map(|m| json!({"key": m.key.to_id()})).unwrap();
    s.run("list.select", &k).unwrap();
    assert_eq!(s.run("mail.cleanUp", &json!({})).unwrap()["moved"], 2);
    assert!(in_trash(&mut s, "c801@studio.example") && in_trash(&mut s, "c802@studio.example") && !in_trash(&mut s, "c803@studio.example"));
    // Undone: back.
    s.run("edit.undo", &json!({})).unwrap();
    assert!(!in_trash(&mut s, "c801@studio.example"));
    // The folder: the budget thread's first isn't downloaded, so it stays; flag the first of
    // the frames thread and only the reply in the middle goes.
    let k = s.list.iter().find(|m| m.message_id == "c801@studio.example").map(|m| json!({"key": m.key.to_id()})).unwrap();
    s.run("list.select", &k).unwrap();
    s.run("mail.flag", &json!({})).unwrap();
    let r = s.run("folder.cleanUp", &json!({})).unwrap();
    assert_eq!(r["moved"], 1, "{r}");
    assert!(in_trash(&mut s, "c802@studio.example") && !in_trash(&mut s, "c801@studio.example") && !in_trash(&mut s, "c811@studio.example"));
    // Again: nothing more; Deleted Items isn't cleaned; bad params.
    assert_eq!(s.run("folder.cleanUp", &json!({"account": "sample", "folder": "INBOX"})).unwrap()["moved"], 0);
    assert_eq!(s.run("folder.cleanUp", &json!({"account": "sample", "folder": trash})).unwrap()["moved"], 0);
    assert!(s.run("folder.cleanUp", &json!({"account": "sample", "folder": "Nope"})).is_err());
    assert!(s.run("folder.cleanUp", &json!({"account": "nobody", "folder": "INBOX"})).is_err());
}

/// The work week: Monday to Friday of the week the day is in, moving a week at a time.
#[test]
fn the_work_week_is_monday_to_friday() {
    let mut s = sample_session("work-week");
    // Saturday 2026-10-24.
    s.run("calendar.goto", &json!({"date": "2026-10-24"})).unwrap();
    let r = s.run("calendar.view", &json!({"mode": "workWeek"})).unwrap();
    assert_eq!(r["mode"], "workweek");
    let (d0, d1) = s.cal_days();
    assert_eq!((mailcraft_model::dates::day_string(d0), d1 - d0), ("2026-10-19".to_string(), 5));
    s.run("calendar.goto", &json!({"step": 1})).unwrap();
    assert_eq!(mailcraft_model::dates::day_string(s.cal_days().0), "2026-10-26");
    let events = s.run("calendar.events", &json!({})).unwrap();
    assert!(events.as_array().is_some());
    assert!(s.run("calendar.view", &json!({"mode": "fortnight"})).is_err());
}

/// Search Folders: saved searches over every folder but Deleted Items and Junk, kept with the
/// settings; renamed, changed, deleted (its mail stays); names and queries checked.
#[test]
fn search_folders_find_mail_everywhere_but_deleted_and_junk() {
    let mut s = sample_session("search-folders");
    let r = s.run("searchFolder.create", &json!({"name": "Unread mail", "query": "is:unread"})).unwrap();
    let unread_everywhere: Vec<String> = s.list.iter().map(|m| m.subject.clone()).collect();
    assert_eq!(r["messages"], unread_everywhere.len());
    assert!(!unread_everywhere.is_empty());
    assert_eq!(s.open_search_folder_name(), Some("Unread mail"));
    // Junk's unread one isn't in it; the plain search finds it.
    let junk = s.account("sample").and_then(|a| a.folder(FolderRole::Junk)).map(|f| f.path.clone()).unwrap();
    let junk_unread = s.folder_state("sample", &junk).messages.iter().filter(|m| !m.flags.seen).count();
    assert!(junk_unread > 0, "the sample's Junk has unread mail");
    assert!(s.list.iter().all(|m| m.key.folder != junk));
    s.run("search.run", &json!({"query": "is:unread"})).unwrap();
    assert!(s.list.iter().any(|m| m.key.folder == junk), "a search of its own isn't the Search Folder");
    assert_eq!(s.open_search_folder_name(), None);
    // From certain people, any of them; larger than.
    s.run("searchFolder.create", &json!({"name": "From Zoe or Ana", "query": "from:zoe,ana"})).unwrap();
    let froms: Vec<String> = s.list.iter().map(|m| m.from.email.clone()).collect();
    assert!(froms.iter().any(|f| f.starts_with("zoe")) && froms.iter().any(|f| f.starts_with("ana")), "{froms:?}");
    assert!(froms.iter().all(|f| f.starts_with("zoe") || f.starts_with("ana")));
    // Changed and renamed while open: shows what it finds now.
    let r = s.run("searchFolder.edit", &json!({"name": "from zoe or ana", "newName": "Zoe", "query": "from:zoe"})).unwrap();
    assert_eq!(r["name"], "Zoe");
    assert!(s.list.iter().all(|m| m.from.email.starts_with("zoe")));
    assert_eq!(s.open_search_folder_name(), Some("Zoe"));
    // Checked: names unique whatever their case, a query that searches, limits.
    for bad in [
        json!({"name": "", "query": "is:unread"}),
        json!({"name": "  ", "query": "is:unread"}),
        json!({"name": "UNREAD MAIL", "query": "is:flagged"}),
        json!({"name": "Nothing", "query": "   "}),
        json!({"name": "x".repeat(101), "query": "is:unread"}),
        json!({"name": "Long", "query": "a".repeat(1001)}),
        json!({"name": "Ctl\u{7}", "query": "is:unread"}),
        json!({"name": "No query"}),
    ] {
        assert!(s.run("searchFolder.create", &bad).is_err(), "{bad}");
    }
    assert!(s.run("searchFolder.edit", &json!({"name": "Zoe", "newName": "Unread mail"})).is_err(), "taken");
    assert!(s.run("searchFolder.open", &json!({"name": "Nope"})).is_err());
    assert!(s.run("searchFolder.delete", &json!({"name": "Nope"})).is_err());
    // Deleted while open: the search goes, the mail stays.
    let before = s.folder_state("sample", "INBOX").messages.len();
    s.run("searchFolder.delete", &json!({"name": "zoe"})).unwrap();
    assert!(s.search.is_none());
    assert_eq!(s.folder_state("sample", "INBOX").messages.len(), before);
    // Kept with the settings; capped.
    let back: crate::session::Prefs = serde_json::from_value(serde_json::to_value(&s.prefs).unwrap()).unwrap();
    assert_eq!(back.search_folders, s.prefs.search_folders);
    for n in 0..200 {
        let _ = s.run("searchFolder.create", &json!({"name": format!("F{n}"), "query": "is:flagged"}));
    }
    assert_eq!(s.prefs.search_folders.len(), crate::session::search_folders::SEARCH_FOLDERS_MAX);
}

/// Quick Steps: their marks, then their one move (undoable); by name, place or shortcut;
/// checked as they're made; a category deleted since is passed over.
#[test]
fn quick_steps_mark_then_move_the_selection() {
    let mut s = sample_session("quick-steps");
    s.run("category.create", &json!({"name": "Done"})).unwrap();
    let archive = s.account("sample").and_then(|a| a.folder(FolderRole::Archive)).map(|f| f.path.clone()).unwrap();
    let r = s
        .run(
            "quickStep.create",
            &json!({"name": "Done", "actions": [{"do": "moveToRole", "value": "archive"}, {"do": "markRead"}, {"do": "categorize", "value": "Done"}, {"do": "markRead"}]}),
        )
        .unwrap();
    assert_eq!(r["index"], 0);
    assert_eq!(s.prefs.quick_steps[0].actions.len(), 3, "the same action twice is done once");
    s.run("quickStep.create", &json!({"name": "Flag it", "actions": [{"do": "flag"}, {"do": "pin"}]})).unwrap();
    // Ctrl+Shift+1: the first, on Ana's unread message.
    let k = key_of(&s, "Sound mix, version 3");
    s.run("list.select", &k).unwrap();
    let id = s.summary(&crate::parse_key(&k).unwrap()).unwrap().message_id.clone();
    let by_key = crate::cmd::registry().all().iter().find(|c| c.shortcut == "Mod+Shift+1").map(|c| c.id);
    assert_eq!(by_key, Some("quickStep.1"));
    s.run("quickStep.1", &json!({})).unwrap();
    let moved = s.folder_state("sample", &archive).messages.iter().find(|m| m.message_id == id).cloned().unwrap();
    assert!(moved.flags.seen && !moved.flags.keywords.is_empty(), "read and categorized before it left: {:?}", moved.flags);
    // Undone: back in the Inbox.
    s.run("edit.undo", &json!({})).unwrap();
    assert!(s.folder_state("sample", "INBOX").messages.iter().any(|m| m.message_id == id));
    // By name: flagged and pinned, staying where it is.
    let k = key_of(&s, "Character turnarounds");
    s.run("list.select", &k).unwrap();
    s.run("quickStep.run", &json!({"name": "flag IT"})).unwrap();
    let m = s.folder_state("sample", "INBOX").messages.iter().find(|m| m.subject == "Character turnarounds").cloned().unwrap();
    assert!(m.flags.flagged && m.flags.pinned());
    // A category deleted since: the rest is still done.
    s.run("category.delete", &json!({"name": "Done"})).unwrap();
    let k = key_of(&s, "Open call: winter group show");
    s.run("list.select", &k).unwrap();
    s.run("quickStep.run", &json!({"index": 0})).unwrap();
    assert!(s.folder_state("sample", &archive).messages.iter().any(|m| m.subject == "Open call: winter group show"));
    // No such step; none selected.
    assert!(s.run("quickStep.9", &json!({})).is_err());
    s.selection.clear();
    assert!(s.run("quickStep.1", &json!({})).is_err());
    // Checked as made and edited.
    for bad in [
        json!({"name": "", "actions": [{"do": "flag"}]}),
        json!({"name": "done", "actions": [{"do": "flag"}]}),
        json!({"name": "Nothing", "actions": []}),
        json!({"name": "No actions"}),
        json!({"name": "Two moves", "actions": [{"do": "delete"}, {"do": "moveToRole", "value": "archive"}]}),
        json!({"name": "Ghost", "actions": [{"do": "categorize", "value": "Nope"}]}),
        json!({"name": "Nowhere", "actions": [{"do": "moveTo", "value": "No/Such"}]}),
        json!({"name": "Odd", "actions": [{"do": "explode"}]}),
        json!({"name": "Many", "actions": vec![json!({"do": "flag"}); 11]}),
    ] {
        assert!(s.run("quickStep.create", &bad).is_err(), "{bad}");
    }
    s.run("quickStep.edit", &json!({"name": "Flag it", "newName": "Flag", "actions": [{"do": "flag"}]})).unwrap();
    assert!(s.run("quickStep.edit", &json!({"name": "Flag", "newName": "Done"})).is_err(), "taken");
    s.run("quickStep.delete", &json!({"name": "flag"})).unwrap();
    assert_eq!(s.prefs.quick_steps.len(), 1);
    let back: crate::session::Prefs = serde_json::from_value(serde_json::to_value(&s.prefs).unwrap()).unwrap();
    assert_eq!(back.quick_steps, s.prefs.quick_steps);
}
