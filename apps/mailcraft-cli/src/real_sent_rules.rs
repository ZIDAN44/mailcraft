//! Issue #202 (rules for mail sent) through a real mail server on this computer (never a real
//! account), run by hand:
//!
//! ```text
//! MAILCRAFT_REAL_SENT_RULES="127.0.0.1:3143 127.0.0.1:3025" \
//!   cargo test -p mailcraft-cli real_sent_rules -- --ignored --nocapture
//! ```
//!
//! An IMAP and SMTP server that takes any password and makes mailboxes as they're used
//! (GreenMail). The whole app's way: a rule made, mail sent through the server, its copy filed in
//! Sent, the rule moving it there, and a second copy of MailCraft reading back where it is.

use super::*;

const RILEY: &str = "riley@studio.test";

fn servers() -> Option<((String, u16), (String, u16))> {
    let v = std::env::var("MAILCRAFT_REAL_SENT_RULES").ok()?;
    let p: Vec<&str> = v.split_whitespace().collect();
    let addr = |a: &str| a.rsplit_once(':').and_then(|(h, p)| Some((h.to_string(), p.parse().ok()?)));
    Some((addr(p.first()?)?, addr(p.get(1)?)?))
}

/// A copy of MailCraft with Riley's account on those servers, signed in.
fn open(name: &str, imap: &(String, u16), smtp: &(String, u16)) -> (Session, Pool, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("mailcraft-real-sent-rules-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    let mut s = Session::open(Some(store.clone()), clock());
    let p = json!({
        "email": RILEY, "deferSignIn": true, "username": RILEY,
        "imapHost": imap.0, "imapPort": imap.1, "imapSecurity": "none",
        "smtpHost": smtp.0, "smtpPort": smtp.1, "smtpSecurity": "none",
    });
    let id = s.run("account.add", &p).unwrap()["account"].as_str().unwrap().to_string();
    let config = s.account(&id).unwrap().config.clone();
    let mut pool = Pool::new(store, Arc::new(MemoryCredentials::default()));
    s.take_jobs();
    pool.submit(Job::Start { account: config, password: Some(Secret("pw".into())) });
    drive(&mut s, &mut pool, Duration::from_secs(60), |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }).then_some(Ok(())))
        .unwrap();
    (s, pool, dir)
}

fn subjects(s: &Session, folder: &str) -> Vec<String> {
    let id = s.accounts.first().map(|a| a.config.id.clone()).unwrap_or_default();
    s.store
        .as_ref()
        .and_then(|st| st.load_folder(&id, folder).ok())
        .map(|f| f.messages.iter().map(|m| m.subject.clone()).collect())
        .unwrap_or_default()
}

#[test]
#[ignore = "needs a real mail server in MAILCRAFT_REAL_SENT_RULES"]
fn real_sent_rules_file_the_sent_copy() {
    let Some((imap, smtp)) = servers() else {
        eprintln!("MAILCRAFT_REAL_SENT_RULES isn't set: skipped");
        return;
    };
    let (mut s, mut pool, dir) = open("writer", &imap, &smtp);
    let id = s.accounts[0].config.id.clone();
    let sent = s.accounts[0].folder(mailcraft_engine::model::FolderRole::Sent).map(|f| f.path.clone());
    eprintln!("folders: {:?}, Sent: {sent:?}", s.accounts[0].folders.iter().map(|f| f.path.as_str()).collect::<Vec<_>>());
    // Run again on the same mailbox, it has the folder (and Sent) already.
    if s.folder(&id, "Zoe's mail").is_none() {
        s.run("folder.create", &json!({"account": id, "name": "Zoe's mail"})).unwrap();
        drive(&mut s, &mut pool, Duration::from_secs(30), |e| matches!(e, Event::Folders { .. }).then_some(Ok(()))).unwrap();
    }
    let rule = json!({"rule": {"name": "Zoe's", "sent": true, "conditions": [{"if": "to", "value": "zoe@"}], "actions": [{"do": "moveTo", "value": "Zoe's mail"}]}});
    s.run("rules.add", &rule).unwrap();
    // Each run's own, on a mailbox kept from runs before.
    let run = std::process::id();
    let (zoe, sam) = (format!("Cut list for Zoe {run}"), format!("Lunch, Sam? {run}"));
    for (to, subject) in [("zoe@studio.test", &zoe), ("sam@studio.test", &sam)] {
        s.run("mail.new", &json!({})).unwrap();
        s.run("compose.update", &json!({"to": to, "subject": subject, "body": "Hello"})).unwrap();
        s.run("compose.send", &json!({"allowEmptySubject": true, "checked": true})).unwrap();
        drive(&mut s, &mut pool, Duration::from_secs(60), |e| match e {
            Event::Sent { .. } => Some(Ok(())),
            Event::SendFailed { error, .. } => Some(Err(error.clone())),
            _ => None,
        })
        .unwrap();
    }
    // The copy reaches Sent, the rule moves Zoe's: until this copy has it in the folder.
    let sent = s.accounts[0].folder(mailcraft_engine::model::FolderRole::Sent).map(|f| f.path.clone()).expect("a Sent folder");
    for _ in 0..30 {
        if subjects(&s, "Zoe's mail").contains(&zoe) {
            break;
        }
        let _ = drive(&mut s, &mut pool, Duration::from_secs(2), |_| None::<Result<(), String>>);
    }
    // Moved here at once: on the server once the folder is read from it again.
    let moved = |e: &Event| matches!(e, Event::FolderChanged { folder, .. } if folder == "Zoe's mail").then_some(Ok(()));
    let _ = drive(&mut s, &mut pool, Duration::from_secs(20), moved);
    eprintln!("here: Sent {:?}, Zoe's mail {:?}", subjects(&s, &sent), subjects(&s, "Zoe's mail"));
    drop(pool);
    // Read back from the server by a fresh copy.
    let (r, rpool, rdir) = open("reader", &imap, &smtp);
    let (in_sent, in_zoe) = (subjects(&r, &sent), subjects(&r, "Zoe's mail"));
    eprintln!("server: Sent {in_sent:?}, Zoe's mail {in_zoe:?}");
    assert!(in_zoe.contains(&zoe) && !in_sent.contains(&zoe), "moved by the rule");
    assert!(in_sent.contains(&sam) && !in_zoe.contains(&sam), "not to Zoe: stays");
    drop(rpool);
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&rdir);
}
