//! Engine tests: rules, junk lists and ignored conversations on a lot of mail at once.

use super::*;

/// A server account whose Inbox has one message, then `n` more that arrive, from Zoe (odd
/// numbers) and Sam (even), with an Archive, Junk and Deleted Items beside it. Returns the
/// account's id; the new mail is seen arriving at the next `FolderChanged`.
fn many_arrive(s: &mut Session, n: u32) -> String {
    let r = s.run("account.add", &json!({"email": "riley@example.org", "password": "pw"})).unwrap();
    let id = r["account"].as_str().unwrap().to_string();
    let folder = |path: &str, role| mailcraft_model::Folder { account: id.clone(), path: path.into(), role, selectable: true, ..Default::default() };
    let folders = vec![
        folder("INBOX", FolderRole::Inbox),
        folder("Archive", FolderRole::Archive),
        folder("Junk", FolderRole::Junk),
        folder("Trash", FolderRole::Trash),
    ];
    s.apply(Event::Folders { account: id.clone(), folders });
    let store = s.store.clone().unwrap();
    let st = |last: u32| mailcraft_store::FolderState {
        validity: 9,
        uid_next: last + 1,
        highest_modseq: 0,
        messages: (1..=last)
            .map(|uid| {
                let who = if uid % 2 == 1 { "zoe" } else { "sam" };
                mailcraft_model::Summary {
                    key: mailcraft_model::MessageKey { account: id.clone(), folder: "INBOX".into(), validity: 9, uid },
                    subject: format!("Note {uid}"),
                    from: mailcraft_model::EmailAddress::new(who, format!("{who}@studio.example")),
                    message_id: format!("note{uid}@studio.example"),
                    date: NOW - 60,
                    ..Default::default()
                }
                .into()
            })
            .collect(),
        window: 0,
    };
    // The first look only sets the mark: what comes after it arrived.
    store.save_folder(&id, "INBOX", &st(1)).unwrap();
    s.apply(Event::FolderChanged { account: id.clone(), folder: "INBOX".into() });
    store.save_folder(&id, "INBOX", &st(n + 1)).unwrap();
    s.take_jobs();
    id
}

/// The changes the session asked the server for: how many, and how many messages they name.
fn applied(s: &mut Session) -> (usize, usize) {
    let ops: Vec<Op> = s.take_jobs().into_iter().filter_map(|j| if let Job::Apply { op, .. } = j { Some(op) } else { None }).collect();
    (ops.len(), ops.iter().map(|o| o.keys().len()).sum())
}

/// Rules run on a lot of mail ask the server once per change, not once per message: 3,000
/// messages a rule categorizes and files are two changes (the category, then the move), as
/// dragging them would be, where they were 6,000, each rebuilding the list.
#[test]
fn rules_on_many_messages_are_one_change_each() {
    let mut s = Session::open(Some(temp("many-rules")), mailcraft_model::dates::Clock::new(NOW, 0));
    let id = many_arrive(&mut s, 3000);
    let rule = json!({"name": "Studio", "conditions": [{"if": "from", "value": "@studio.example"}], "actions": [{"do": "categorize", "value": "Work"}, {"do": "moveToRole", "value": "archive"}]});
    s.run("rules.add", &json!({ "rule": rule })).unwrap();
    // As it arrives.
    s.apply(Event::FolderChanged { account: id.clone(), folder: "INBOX".into() });
    let jobs = s.take_jobs();
    let ops: Vec<&Op> = jobs.iter().filter_map(|j| if let Job::Apply { op, .. } = j { Some(op) } else { None }).collect();
    match ops.as_slice() {
        [Op::SetKeyword { keys: tagged, .. }, Op::Move { keys: moved, to }] => {
            assert_eq!((tagged.len(), moved.len(), to.as_str()), (3000, 3000, "Archive"));
        }
        other => panic!("{} changes: {:?}", other.len(), other.iter().take(3).collect::<Vec<_>>()),
    }
    assert_eq!(s.folder_state(&id, "INBOX").messages.len(), 1, "only the one from before is left");
    // Run by hand on what's in the Inbox: the one that was there before.
    assert_eq!(s.run("rules.run", &json!({"inbox": true})).unwrap()["matched"], 1);
    assert_eq!(applied(&mut s), (2, 2));
    done(s);
}

/// A blocked sender's mail goes to Junk, and replies in ignored conversations to Deleted Items,
/// one move for each, however many arrive.
#[test]
fn junk_and_ignored_mail_move_together() {
    let mut s = Session::open(Some(temp("many-junk")), mailcraft_model::dates::Clock::new(NOW, 0));
    let id = many_arrive(&mut s, 2000);
    s.prefs.blocked_senders.insert("sam@studio.example".into());
    s.prefs.ignored.extend((1..=2001).filter(|n| n % 2 == 1).map(|n| format!("note{n}@studio.example")));
    s.apply(Event::FolderChanged { account: id.clone(), folder: "INBOX".into() });
    let jobs = s.take_jobs();
    let moves: Vec<(&str, usize)> = jobs
        .iter()
        .filter_map(|j| if let Job::Apply { op: Op::Move { keys, to }, .. } = j { Some((to.as_str(), keys.len())) } else { None })
        .collect();
    assert_eq!(moves.len(), 2, "{} moves", moves.len());
    assert_eq!(moves, [("Junk", 1000), ("Trash", 1000)]);
    assert!(s.take_new_mail().is_empty(), "neither is announced");
    done(s);
}

/// Ignore Conversation on 300 messages selected in a folder of 5,000, timed (the folder is
/// threaded once, not once per message). Run under the heavy lock:
/// `cargo test -p mailcraft-engine --lib measure_ignore_many -- --ignored --nocapture`.
#[test]
#[ignore]
fn measure_ignore_many() {
    let mut s = Session::open(Some(temp("measure-ignore")), mailcraft_model::dates::Clock::new(NOW, 0));
    let id = many_arrive(&mut s, 5000);
    s.apply(Event::FolderChanged { account: id.clone(), folder: "INBOX".into() });
    s.run("folder.open", &json!({"account": id, "folder": "INBOX"})).unwrap();
    let keys: Vec<String> = s.list.iter().take(300).map(|m| m.key.to_id()).collect();
    for k in &keys {
        s.run("list.select", &json!({"key": k, "add": true})).unwrap();
    }
    assert_eq!(s.selection.len(), 300);
    let t = std::time::Instant::now();
    let r = s.run("mail.ignore", &json!({})).unwrap();
    println!("MEASURE ignore 300 of 5000: {} ms ({r})", t.elapsed().as_millis());
    assert_eq!(s.prefs.ignored.len(), 300);
    done(s);
}

/// A server account with an empty Inbox and an Archive of `n` messages (uids 1 to `n`, each its
/// own Message-ID), both read. Returns the account's id.
fn archive_of(s: &mut Session, n: u32) -> String {
    let id = s.run("account.add", &json!({"email": "riley@example.org", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    let folder = |path: &str, role| mailcraft_model::Folder { account: id.clone(), path: path.into(), role, selectable: true, ..Default::default() };
    s.apply(Event::Folders { account: id.clone(), folders: vec![folder("INBOX", FolderRole::Inbox), folder("Archive", FolderRole::Archive)] });
    let store = s.store.clone().unwrap();
    let messages = (1..=n)
        .map(|uid| {
            mailcraft_model::Summary {
                key: mailcraft_model::MessageKey { account: id.clone(), folder: "Archive".into(), validity: 9, uid },
                subject: format!("Filed {uid}"),
                message_id: format!("filed{uid}@x.example"),
                date: NOW - 3600,
                ..Default::default()
            }
            .into()
        })
        .collect();
    store.save_folder(&id, "Archive", &mailcraft_store::FolderState { validity: 9, uid_next: n + 1, messages, ..Default::default() }).unwrap();
    store.save_folder(&id, "INBOX", &mailcraft_store::FolderState { validity: 9, uid_next: 1, ..Default::default() }).unwrap();
    for f in ["INBOX", "Archive"] {
        s.apply(Event::FolderChanged { account: id.clone(), folder: f.into() });
    }
    s.take_jobs();
    id
}

/// Move the Archive's messages `uids` to the Inbox.
fn move_in(s: &mut Session, id: &str, uids: std::ops::RangeInclusive<u32>) {
    s.run("folder.open", &json!({"account": id, "folder": "Archive"})).unwrap();
    s.selection = uids.map(|uid| mailcraft_model::MessageKey { account: id.to_string(), folder: "Archive".into(), validity: 9, uid }).collect();
    s.run("mail.move", &json!({"folder": "INBOX"})).unwrap();
    s.take_jobs();
}

/// 2,100 messages moved into the Inbox, then one more: none of them is mail arriving when the
/// server shows them there. Past 2,000 remembered, the next move forgot all of them, and the
/// 2,100 were announced as new.
#[test]
fn a_big_move_into_the_inbox_is_remembered_whole() {
    let mut s = Session::open(Some(temp("moved-in-many")), mailcraft_model::dates::Clock::new(NOW, 0));
    let id = archive_of(&mut s, 2200);
    move_in(&mut s, &id, 1..=2100);
    move_in(&mut s, &id, 2101..=2101);
    // The server files them in the Inbox under new numbers.
    let store = s.store.clone().unwrap();
    let mut archive = store.load_folder(&id, "Archive").unwrap();
    let moved: Vec<mailcraft_model::SharedSummary> = archive
        .messages
        .iter()
        .filter(|m| m.key.uid <= 2101)
        .map(|m| {
            let mut m = mailcraft_model::Summary::clone(m);
            m.key = mailcraft_model::MessageKey { account: id.clone(), folder: "INBOX".into(), validity: 9, uid: m.key.uid };
            m.into()
        })
        .collect();
    archive.messages.retain(|m| m.key.uid > 2101);
    store.save_folder(&id, "Archive", &archive).unwrap();
    store.save_folder(&id, "INBOX", &mailcraft_store::FolderState { validity: 9, uid_next: 2102, messages: moved, ..Default::default() }).unwrap();
    for f in ["Archive", "INBOX"] {
        s.apply(Event::FolderChanged { account: id.clone(), folder: f.into() });
    }
    assert_eq!(s.take_new_mail().len(), 0, "announced as new");
    done(s);
}

/// 5,000 messages of an Archive of 50,000 (every tenth) moved into the Inbox, timed. Run under the heavy lock:
/// `cargo test --release -p mailcraft-engine --lib measure_move_in_many -- --ignored --nocapture`.
#[test]
#[ignore]
fn measure_move_in_many() {
    let mut s = Session::open(Some(temp("measure-move-in")), mailcraft_model::dates::Clock::new(NOW, 0));
    let id = archive_of(&mut s, 50_000);
    s.run("folder.open", &json!({"account": id, "folder": "Archive"})).unwrap();
    s.selection =
        (1..=5000).map(|n| mailcraft_model::MessageKey { account: id.clone(), folder: "Archive".into(), validity: 9, uid: n * 10 }).collect();
    let t = std::time::Instant::now();
    s.run("mail.move", &json!({"folder": "INBOX"})).unwrap();
    println!("MEASURE move 5000 of 50000 into the Inbox: {} ms", t.elapsed().as_millis());
    done(s);
}

/// Past the most remembered, the oldest is forgotten first, not all of them; one remembered
/// again counts from then; one taken out leaves room.
#[test]
fn moves_into_the_inbox_are_forgotten_oldest_first() {
    use crate::session::{MOVED_IN_MAX, MovedIn};
    let k = |n: usize| ("a1".to_string(), "INBOX".to_string(), format!("m{n}@x"));
    let mut m = MovedIn::default();
    for n in 0..MOVED_IN_MAX {
        m.insert(k(n));
    }
    m.insert(k(0));
    m.insert(k(MOVED_IN_MAX));
    assert!(m.remove(&k(0)), "remembered again, so not the oldest");
    assert!(!m.remove(&k(1)), "the oldest went");
    assert!(m.remove(&k(2)) && m.remove(&k(MOVED_IN_MAX)));
    m.insert(k(MOVED_IN_MAX + 1));
    assert!(m.remove(&k(3)), "taken out ones left room");
}
