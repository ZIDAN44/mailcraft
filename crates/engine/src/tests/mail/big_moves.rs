//! Engine tests: Undo of a big move, and a big move to another account's server, timed.

use mailcraft_model::{Folder, MessageKey, Summary};

use super::*;

/// An account (`local`: kept on this computer, else on a server) whose `folder` has `n`
/// messages (uids 1 to `n`, each with its own Message-ID) and whose other folders are empty.
/// Returns its id.
fn account_with(s: &mut Session, email: &str, local: bool, folder: &str, n: u32) -> String {
    let params = if local { json!({"email": email, "local": true}) } else { json!({"email": email, "password": "pw"}) };
    let id = s.run("account.add", &params).unwrap()["account"].as_str().unwrap().to_string();
    let f = |path: &str, role| Folder { account: id.clone(), path: path.into(), role, selectable: true, ..Default::default() };
    s.apply(Event::Folders { account: id.clone(), folders: vec![f("INBOX", FolderRole::Inbox), f("Archive", FolderRole::Archive)] });
    let store = s.store.clone().unwrap();
    for path in ["INBOX", "Archive"] {
        let messages = if path == folder {
            (1..=n)
                .map(|uid| {
                    let key = MessageKey { account: id.clone(), folder: path.into(), validity: 9, uid };
                    Summary {
                        key,
                        subject: format!("Note {uid}"),
                        message_id: format!("{email}.{uid}@x.example"),
                        date: NOW - 3600,
                        ..Default::default()
                    }
                    .into()
                })
                .collect()
        } else {
            Vec::new()
        };
        let st = mailcraft_store::FolderState { validity: 9, uid_next: n + 1, messages, ..Default::default() };
        store.save_folder(&id, path, &st).unwrap();
        s.apply(Event::FolderChanged { account: id.clone(), folder: path.into() });
    }
    s.take_jobs();
    id
}

/// 5,000 messages older than any there moved into an Archive of 50,000 kept on this computer
/// (so listed after them), then the move undone: the Undo timed. Run under the heavy lock:
/// `cargo test --release -p mailcraft-engine --lib measure_undo_big_move -- --ignored --nocapture`.
#[test]
#[ignore]
fn measure_undo_big_move() {
    let mut s = Session::open(Some(temp("measure-undo-move")), mailcraft_model::dates::Clock::new(NOW, 0));
    let id = account_with(&mut s, "ben@example.org", true, "Archive", 50_000);
    let store = s.store.clone().unwrap();
    let inbox: Vec<mailcraft_model::SharedSummary> = (1..=5000)
        .map(|uid| {
            let key = MessageKey { account: id.clone(), folder: "INBOX".into(), validity: 9, uid };
            Summary { key, subject: format!("Old {uid}"), message_id: format!("old{uid}@x.example"), date: NOW - 30 * 86_400, ..Default::default() }
                .into()
        })
        .collect();
    store.save_folder(&id, "INBOX", &mailcraft_store::FolderState { validity: 9, uid_next: 5001, messages: inbox, ..Default::default() }).unwrap();
    s.apply(Event::FolderChanged { account: id.clone(), folder: "INBOX".into() });
    s.run("folder.open", &json!({"account": id, "folder": "INBOX"})).unwrap();
    s.run("list.selectAll", &json!({})).unwrap();
    assert_eq!(s.run("mail.move", &json!({"folder": "Archive"})).unwrap()["moved"], 5000);
    let t = std::time::Instant::now();
    s.run("edit.undo", &json!({})).unwrap();
    println!("MEASURE undo a move of 5000 into 50000: {} ms", t.elapsed().as_millis());
    assert_eq!(store.load_folder(&id, "INBOX").unwrap().messages.len(), 5000);
    done(s);
}

/// 10,000 messages moved from an account kept on this computer to another's server, and the
/// server's answer (all of them there) taken in, timed. Run under the heavy lock:
/// `cargo test --release -p mailcraft-engine --lib measure_big_move_across -- --ignored --nocapture`.
#[test]
#[ignore]
fn measure_big_move_across() {
    let mut s = Session::open(Some(temp("measure-move-across")), mailcraft_model::dates::Clock::new(NOW, 0));
    let from = account_with(&mut s, "ben@example.org", true, "INBOX", 10_000);
    let to = account_with(&mut s, "zoe@example.org", false, "INBOX", 0);
    let store = s.store.clone().unwrap();
    for uid in 1..=10_000 {
        let key = MessageKey { account: from.clone(), folder: "INBOX".into(), validity: 9, uid };
        store.write_body(&key, format!("Subject: Note {uid}\r\nMessage-ID: <ben@example.org.{uid}@x.example>\r\n\r\nHi.\r\n").as_bytes()).unwrap();
    }
    s.run("folder.open", &json!({"account": from, "folder": "INBOX"})).unwrap();
    s.run("list.selectAll", &json!({})).unwrap();
    let r = s.run("mail.move", &json!({"account": to, "folder": "Archive"})).unwrap();
    assert_eq!(r["sending"], 10_000, "{r}");
    let filed: Vec<MessageKey> =
        s.take_jobs().into_iter().flat_map(|j| if let Job::FileCopies { mail, .. } = j { mail } else { Vec::new() }).map(|(k, _)| k).collect();
    assert_eq!(filed.len(), 10_000);
    let t = std::time::Instant::now();
    s.apply(Event::CopiesFiled { account: to.clone(), folder: "Archive".into(), done: filed, failed: Vec::new(), moved: true });
    println!("MEASURE a move of 10000 to a server answered: {} ms", t.elapsed().as_millis());
    assert!(s.transfers.is_empty());
    done(s);
}

/// 1,000 messages of an open Inbox of 100,000 on a server downloaded, each one's summary filled
/// from its source as it comes ([`Event::Filled`]), timed. Run under the heavy lock:
/// `cargo test --release -p mailcraft-engine --lib measure_filled_big_folder -- --ignored --nocapture`.
#[test]
#[ignore]
fn measure_filled_big_folder() {
    let mut s = Session::open(Some(temp("measure-filled")), mailcraft_model::dates::Clock::new(NOW, 0));
    let id = account_with(&mut s, "zoe@example.org", false, "INBOX", 100_000);
    s.run("folder.open", &json!({"account": id, "folder": "INBOX"})).unwrap();
    let t = std::time::Instant::now();
    for n in 1..=1000 {
        let key = MessageKey { account: id.clone(), folder: "INBOX".into(), validity: 9, uid: n * 100 };
        let summary = Summary { key, subject: format!("Note {}", n * 100), has_attachments: true, date: NOW - 3600, ..Default::default() };
        s.apply(Event::Filled { summary: Box::new(summary) });
    }
    println!("MEASURE 1000 filled in 100000: {} ms", t.elapsed().as_millis());
    done(s);
}
