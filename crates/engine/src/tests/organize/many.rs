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
}

/// Ignore Conversation on 300 messages selected in a folder of 5,000, timed (the folder is
/// threaded once, not once per message). Run under the heavy lock:
/// `cargo test -p mailcraft-engine --lib measure_ignore_many -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement"]
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
}

/// Past 5,000 ignored ids, the oldest go: never those of the conversation just ignored, which
/// stays ignored (it lost its ids when they sorted first). The order is kept across a restart.
#[test]
fn the_oldest_ignored_ids_go_first() {
    let mut s = sample_session("ignore-many");
    s.prefs.ignored.extend((0..5000).map(|n| format!("zz{n}@old.example")));
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    let k = key_of(&s, "Storyboard review on Thursday");
    s.run("list.select", &k).unwrap();
    let m = s.first_selected().cloned().unwrap();
    s.run("mail.ignore", &json!({})).unwrap();
    assert!(s.is_ignored(&m), "the conversation just ignored is");
    assert_eq!(s.prefs.ignored.len(), 5000);
    assert!(!s.prefs.ignored.contains("zz0@old.example") && s.prefs.ignored.contains("zz4999@old.example"), "the oldest went");
    let back: crate::session::Prefs = serde_json::from_value(serde_json::to_value(&s.prefs).unwrap()).unwrap();
    assert_eq!(back.ignored, s.prefs.ignored);
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
}

/// 5,000 messages of an Archive of 50,000 (every tenth) moved into the Inbox, timed. Run under the heavy lock:
/// `cargo test --release -p mailcraft-engine --lib measure_move_in_many -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement: run in release"]
fn measure_move_in_many() {
    let mut s = Session::open(Some(temp("measure-move-in")), mailcraft_model::dates::Clock::new(NOW, 0));
    let id = archive_of(&mut s, 50_000);
    s.run("folder.open", &json!({"account": id, "folder": "Archive"})).unwrap();
    s.selection =
        (1..=5000).map(|n| mailcraft_model::MessageKey { account: id.clone(), folder: "Archive".into(), validity: 9, uid: n * 10 }).collect();
    let t = std::time::Instant::now();
    s.run("mail.move", &json!({"folder": "INBOX"})).unwrap();
    println!("MEASURE move 5000 of 50000 into the Inbox: {} ms", t.elapsed().as_millis());
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

/// A server account whose Snoozed folder has `n` messages, and `stuck` snoozes due an hour ago
/// whose messages aren't in it (moved on another device, not synced here yet). Returns the
/// session and the account's id.
fn stuck_snoozes(name: &str, n: u32, stuck: usize) -> (Session, String) {
    let mut s = Session::open(Some(temp(name)), mailcraft_model::dates::Clock::new(NOW, 0));
    let id = s.run("account.add", &json!({"email": "riley@example.org", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    let folder = |path: &str, role| mailcraft_model::Folder { account: id.clone(), path: path.into(), role, selectable: true, ..Default::default() };
    s.apply(Event::Folders { account: id.clone(), folders: vec![folder("INBOX", FolderRole::Inbox), folder("Snoozed", FolderRole::Other)] });
    let messages = (1..=n)
        .map(|uid| {
            let key = mailcraft_model::MessageKey { account: id.clone(), folder: "Snoozed".into(), validity: 9, uid };
            mailcraft_model::Summary {
                key,
                subject: format!("Later {uid}"),
                message_id: format!("later{uid}@x.example"),
                date: NOW - 60,
                ..Default::default()
            }
            .into()
        })
        .collect();
    let store = s.store.clone().unwrap();
    store.save_folder(&id, "Snoozed", &mailcraft_store::FolderState { validity: 9, uid_next: n + 1, messages, ..Default::default() }).unwrap();
    s.apply(Event::FolderChanged { account: id.clone(), folder: "Snoozed".into() });
    for i in 0..stuck {
        s.snoozes.push(mailcraft_model::organize::Snooze {
            account: id.clone(),
            message_id: format!("elsewhere{i}@x.example"),
            folder: "Snoozed".into(),
            back_to: "INBOX".into(),
            until: NOW - 3600,
            subject: format!("Elsewhere {i}"),
        });
    }
    s.take_jobs();
    (s, id)
}

/// 20 snoozes stuck over a Snoozed folder of 10,000 messages, 1,000 frames timed. Run under the
/// heavy lock: `cargo test --release -p mailcraft-engine --lib measure_stuck_snoozes -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement: run in release"]
fn measure_stuck_snoozes() {
    let (mut s, _) = stuck_snoozes("measure-stuck-snoozes", 10_000, 20);
    let t = std::time::Instant::now();
    for _ in 0..1000 {
        s.tick();
    }
    println!("MEASURE 1000 frames, 20 stuck snoozes over 10000: {} ms", t.elapsed().as_millis());
}

/// A snooze due while its folder's list can't be read (its read marks are being saved, and it
/// was let go of from memory) is looked for again as soon as it can, not a minute on: the list
/// read as empty, the message taken for not there yet, and nothing said its folder changed.
#[test]
fn a_snooze_due_while_its_folder_cant_be_read_comes_back_once_it_can() {
    let mut s = sample_session("snooze-unread-folder");
    let store = s.store.clone().unwrap();
    let m = s.list.iter().find(|m| !m.flags.seen && !m.message_id.is_empty()).cloned().unwrap();
    assert_eq!(m.key.folder, "INBOX");
    s.run("folder.open", &json!({"account": "sample", "folder": "Sent"})).unwrap();
    let paused = store.pause_marks();
    s.apply_op("sample", Op::SetFlag { keys: vec![m.key.clone()], flag: Flag::Seen, on: true });
    s.cache.remove(&("sample".to_string(), "INBOX".to_string()));
    assert!(store.marks_waiting_for("sample", Some("INBOX")));
    s.snoozes.push(mailcraft_model::organize::Snooze {
        account: "sample".into(),
        message_id: m.message_id.clone(),
        folder: "INBOX".into(),
        back_to: "Archive".into(),
        until: NOW - 10,
        subject: m.subject.clone(),
    });
    s.tick();
    assert_eq!(s.snoozes.len(), 1, "not found in a list that can't be read");
    drop(paused);
    assert!(store.flush_marks());
    s.tick();
    assert!(s.snoozes.is_empty(), "back from snooze at the next look");
    assert!(store.load_folder("sample", "Archive").unwrap().messages.iter().any(|x| x.message_id == m.message_id));
}

/// Snoozes due whose messages aren't here yet are looked for again when the folder changes, or
/// after a while: not on every frame (each looked through the whole Snoozed folder). One that
/// comes in is brought back at once.
#[test]
fn stuck_snoozes_arent_looked_for_every_frame() {
    let (mut s, id) = stuck_snoozes("stuck-snoozes", 10, 3);
    let store = s.store.clone().unwrap();
    s.tick();
    let reads = s.snooze_looks;
    for _ in 0..100 {
        s.tick();
    }
    assert_eq!(s.snooze_looks, reads, "looked for again every frame");
    // A minute on: looked for again.
    s.clock.now += 61;
    s.tick();
    assert_eq!(s.snooze_looks, reads + 1);
    // One of them comes in: brought back at the next frame.
    let mut st = store.load_folder(&id, "Snoozed").unwrap();
    let key = mailcraft_model::MessageKey { account: id.clone(), folder: "Snoozed".into(), validity: 9, uid: 11 };
    st.messages.insert(
        0,
        mailcraft_model::Summary { key, subject: "Elsewhere 0".into(), message_id: "elsewhere0@x.example".into(), date: NOW, ..Default::default() }
            .into(),
    );
    store.save_folder(&id, "Snoozed", &st).unwrap();
    s.apply(Event::FolderChanged { account: id.clone(), folder: "Snoozed".into() });
    s.tick();
    assert_eq!(s.snoozes.len(), 2, "back from snooze");
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::Apply { op: Op::Move { to, .. }, .. } if to == "INBOX")));
}

/// Clean Up on a folder of 6,000 downloaded messages in 2,000 conversations of three (each
/// reply quoting the one before, in HTML and text, every fourth with a 200 KB attachment), with
/// `read` of each conversation's messages read (unread mail stays), timed. Run under the heavy
/// lock: `cargo test --release -p mailcraft-engine --lib measure_clean_up_folder -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement: run in release"]
fn measure_clean_up_folder() {
    for read in [3, 0] {
        let mut s = Session::open(Some(temp(&format!("measure-clean-up-{read}"))), mailcraft_model::dates::Clock::new(NOW, 0));
        let id = s.run("account.add", &json!({"email": "riley@example.org", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
        let folder =
            |path: &str, role| mailcraft_model::Folder { account: id.clone(), path: path.into(), role, selectable: true, ..Default::default() };
        s.apply(Event::Folders { account: id.clone(), folders: vec![folder("INBOX", FolderRole::Inbox), folder("Trash", FolderRole::Trash)] });
        let store = s.store.clone().unwrap();
        let file = "QUJD".repeat(50_000);
        let mut messages = Vec::new();
        for c in 0..2000u32 {
            let mut text = String::new();
            for n in 0..3u32 {
                let uid = c * 3 + n + 1;
                let line = format!("Message {n} of conversation {c}: the figures for the north office are in, please look them over.");
                text = if text.is_empty() {
                    line
                } else {
                    format!("{line}\r\n\r\n{}", text.lines().map(|l| format!("> {l}")).collect::<Vec<_>>().join("\r\n"))
                };
                let html = format!("<html><body><p>{}</p></body></html>", text.replace("\r\n", "<br>"));
                let attached = c % 4 == 0;
                let file_part = if attached {
                    format!(
                        "--b\r\nContent-Type: application/octet-stream; name=\"figures.bin\"\r\nContent-Transfer-Encoding: base64\r\n\r\n{file}\r\n"
                    )
                } else {
                    String::new()
                };
                let raw = format!(
                    "From: a@b.example\r\nSubject: Figures {c}\r\nMessage-ID: <c{c}-{n}@x.example>\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\nContent-Type: multipart/alternative; boundary=a\r\n\r\n--a\r\nContent-Type: text/plain\r\n\r\n{text}\r\n--a\r\nContent-Type: text/html\r\n\r\n{html}\r\n--a--\r\n{file_part}--b--\r\n"
                );
                let key = mailcraft_model::MessageKey { account: id.clone(), folder: "INBOX".into(), validity: 9, uid };
                store.write_body(&key, raw.as_bytes()).unwrap();
                let ids: Vec<String> = (0..n).map(|p| format!("c{c}-{p}@x.example")).collect();
                let m = mailcraft_model::Summary {
                    key,
                    subject: format!("Figures {c}"),
                    message_id: format!("c{c}-{n}@x.example"),
                    in_reply_to: ids.last().cloned().unwrap_or_default(),
                    references: ids,
                    date: NOW - 100_000 + i64::from(uid),
                    has_attachments: attached,
                    flags: mailcraft_model::Flags { seen: n < read, ..Default::default() },
                    ..Default::default()
                };
                messages.push(m.into());
            }
        }
        let st = mailcraft_store::FolderState { validity: 9, uid_next: 6001, messages, ..Default::default() };
        store.save_folder(&id, "INBOX", &st).unwrap();
        s.apply(Event::FolderChanged { account: id.clone(), folder: "INBOX".into() });
        let t = std::time::Instant::now();
        let r = s.run("folder.cleanUp", &json!({"account": id, "folder": "INBOX"})).unwrap();
        println!("MEASURE Clean Up of 6000 in 2000 conversations, {read} of 3 read: {} ms ({r})", t.elapsed().as_millis());
    }
}
