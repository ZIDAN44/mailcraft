//! Issue #202: rules for mail the user sends, as Outlook's rules can be for messages sent: run
//! as each sent message's copy reaches Sent, never on mail arriving; and the Inbox's rules never
//! on mail sent.

use super::*;

/// A message the user sent, its copy new in Sent.
fn sent_copy(s: &mut Session, uid: u32, to: (&str, &str), subject: &str) {
    let mut m = mailcraft_model::Summary::clone(&s.store.as_ref().unwrap().load_folder("sample", "INBOX").unwrap().messages[0]);
    m.key.folder = "Sent".into();
    m.key.uid = uid;
    m.subject = subject.into();
    m.message_id = format!("<{uid}@sent.example>");
    m.from = mailcraft_model::EmailAddress::new("Riley Moreno", "riley@studio.example");
    m.to = vec![mailcraft_model::EmailAddress::new(to.0, to.1)];
    m.cc.clear();
    m.flags.seen = true;
    m.date = NOW;
    file_in(s, m, None);
}

fn subjects(s: &mut Session, folder: &str) -> Vec<String> {
    s.folder_state("sample", folder).messages.iter().map(|m| m.subject.clone()).collect()
}

#[test]
fn a_rule_for_mail_sent_files_the_sent_copy() {
    let mut s = sample_session("sent-rules");
    // Sent looked at once: what's there already isn't new.
    s.apply(Event::FolderChanged { account: "sample".into(), folder: "Sent".into() });
    let sent = json!({"rule": {"name": "Zoe's", "sent": true, "conditions": [{"if": "to", "value": "zoe@"}],
        "actions": [{"do": "categorize", "value": "Work"}, {"do": "moveTo", "value": "Projects"}]}});
    s.run("rules.add", &sent).unwrap();
    // An Inbox rule from Zoe: never on mail sent to her.
    s.run("rules.add", &json!({"rule": {"name": "From Zoe", "conditions": [{"if": "anyText", "value": "zoe@"}], "actions": [{"do": "flag"}]}}))
        .unwrap();
    s.take_new_mail();
    sent_copy(&mut s, 900, ("Zoe Park", "zoe@studio.example"), "Cut list for Zoe");
    sent_copy(&mut s, 901, ("Sam Okafor", "sam@studio.example"), "Lunch, Sam?");
    assert!(!subjects(&mut s, "Sent").contains(&"Cut list for Zoe".to_string()), "filed away");
    assert!(subjects(&mut s, "Sent").contains(&"Lunch, Sam?".to_string()), "not to Zoe: stays");
    let filed = s.folder_state("sample", "Projects").messages.iter().find(|m| m.subject == "Cut list for Zoe").cloned().expect("in Projects");
    assert!(filed.flags.has("$label2") && !filed.flags.flagged, "the sent rule's category, not the Inbox rule's flag");
    assert!(s.take_new_mail().is_empty(), "mail sent isn't announced");

    // Mail from Zoe arriving: the Inbox's rule, not the one for mail sent.
    let mut m = mailcraft_model::Summary::clone(&s.store.as_ref().unwrap().load_folder("sample", "INBOX").unwrap().messages[0]);
    m.key.uid = 950;
    m.subject = "Notes from Zoe".into();
    m.from = mailcraft_model::EmailAddress::new("Zoe Park", "zoe@studio.example");
    m.to = vec![
        mailcraft_model::EmailAddress::new("Riley Moreno", "riley@studio.example"),
        mailcraft_model::EmailAddress::new("Zoe", "zoe@studio.example"),
    ];
    m.date = NOW;
    file_in(&mut s, m, None);
    let notes = s.folder_state("sample", "INBOX").messages.iter().find(|m| m.subject == "Notes from Zoe").cloned().expect("stays in the Inbox");
    assert!(notes.flags.flagged && !notes.flags.has("$label2"));
}

/// Mail put in Sent by hand wasn't sent: its rules leave it alone when the server shows it there.
/// Run now, a rule for mail sent runs on what's in Sent already.
#[test]
fn mail_moved_into_sent_is_left_and_run_now_looks_in_sent() {
    let mut s = Session::open(Some(temp("sent-rules-moved")), mailcraft_model::dates::Clock::new(NOW, 0));
    let id = s.run("account.add", &json!({"email": "riley@example.org", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    let folder = |path: &str, role| mailcraft_model::Folder { account: id.clone(), path: path.into(), role, selectable: true, ..Default::default() };
    s.apply(Event::Folders {
        account: id.clone(),
        folders: vec![folder("INBOX", FolderRole::Inbox), folder("Archive", FolderRole::Archive), folder("Sent", FolderRole::Sent)],
    });
    let store = s.store.clone().unwrap();
    let mail = |folder: &str, uid: u32| -> mailcraft_model::SharedSummary {
        mailcraft_model::Summary {
            key: mailcraft_model::MessageKey { account: id.clone(), folder: folder.into(), validity: 9, uid },
            subject: "Budget, dragged in".into(),
            message_id: "dragged@x.example".into(),
            to: vec![mailcraft_model::EmailAddress::new("Zoe", "zoe@x.example")],
            date: NOW - 3600,
            ..Default::default()
        }
        .into()
    };
    let save = |folder: &str, messages: Vec<mailcraft_model::SharedSummary>| {
        store.save_folder(&id, folder, &mailcraft_store::FolderState { validity: 9, uid_next: 10, messages, ..Default::default() }).unwrap();
    };
    save("Archive", vec![mail("Archive", 1)]);
    save("Sent", Vec::new());
    for f in ["INBOX", "Archive", "Sent"] {
        s.apply(Event::FolderChanged { account: id.clone(), folder: f.into() });
    }
    let rule = json!({"rule": {"name": "Away", "sent": true, "conditions": [{"if": "subject", "value": "budget"}], "actions": [{"do": "moveToRole", "value": "archive"}]}});
    s.run("rules.add", &rule).unwrap();
    s.run("folder.open", &json!({"account": id, "folder": "Archive"})).unwrap();
    s.selection = vec![mailcraft_model::MessageKey { account: id.clone(), folder: "Archive".into(), validity: 9, uid: 1 }];
    s.run("mail.move", &json!({"folder": "Sent"})).unwrap();
    s.take_jobs();
    // The server shows it in Sent under a number of its own.
    save("Archive", Vec::new());
    save("Sent", vec![mail("Sent", 7)]);
    for f in ["Archive", "Sent"] {
        s.apply(Event::FolderChanged { account: id.clone(), folder: f.into() });
    }
    let moves = |s: &mut Session| s.take_jobs().into_iter().filter(|j| matches!(j, Job::Apply { op: Op::Move { .. }, .. })).count();
    assert_eq!(moves(&mut s), 0, "not taken for mail sent");
    // Asked to run now: it does, in Sent.
    let r = s.run("rules.run", &json!({"index": 0, "inbox": true})).unwrap();
    assert_eq!(r["matched"], 1, "{r}");
    assert_eq!(moves(&mut s), 1);
}

/// A new mailbox has no Sent until its first copy makes one: what's in a folder at its first
/// look isn't new, yet the copy of a message sent from here is, and its rules run (GreenMail
/// showed both copies left in Sent).
#[test]
fn the_first_copy_in_a_new_sent_folder_has_its_rules() {
    let mut s = Session::open(Some(temp("sent-rules-first")), mailcraft_model::dates::Clock::new(NOW, 0));
    let id = s.run("account.add", &json!({"email": "riley@example.org", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    let folder = |path: &str, role| mailcraft_model::Folder { account: id.clone(), path: path.into(), role, selectable: true, ..Default::default() };
    s.apply(Event::Folders { account: id.clone(), folders: vec![folder("INBOX", FolderRole::Inbox), folder("Archive", FolderRole::Archive)] });
    let rule = json!({"rule": {"name": "Zoe's", "sent": true, "conditions": [{"if": "to", "value": "zoe@"}], "actions": [{"do": "moveToRole", "value": "archive"}]}});
    s.run("rules.add", &rule).unwrap();
    s.take_jobs();
    let mut sent_ids = Vec::new();
    for to in ["zoe@x.example", "sam@x.example"] {
        s.run("mail.new", &json!({})).unwrap();
        s.run("compose.update", &json!({"to": to, "subject": "Cut list", "body": "Hi"})).unwrap();
        s.run("compose.send", &json!({"allowEmptySubject": true, "checked": true})).unwrap();
        let raw = s.take_jobs().into_iter().find_map(|j| if let Job::Send { raw, .. } = j { Some(raw) } else { None }).expect("sent");
        sent_ids.push((to, mailcraft_mime::written_message_id(&raw)));
    }
    // Sent made by the first copy, both copies there at its first look.
    let copies: Vec<mailcraft_model::SharedSummary> = sent_ids
        .iter()
        .zip(1u32..)
        .map(|((to, mid), uid)| {
            mailcraft_model::Summary {
                key: mailcraft_model::MessageKey { account: id.clone(), folder: "Sent".into(), validity: 9, uid },
                subject: "Cut list".into(),
                message_id: mid.clone(),
                to: vec![mailcraft_model::EmailAddress::new("", *to)],
                date: NOW,
                ..Default::default()
            }
            .into()
        })
        .collect();
    s.store
        .as_ref()
        .unwrap()
        .save_folder(&id, "Sent", &mailcraft_store::FolderState { validity: 9, uid_next: 3, messages: copies, ..Default::default() })
        .unwrap();
    s.apply(Event::Folders {
        account: id.clone(),
        folders: vec![folder("INBOX", FolderRole::Inbox), folder("Archive", FolderRole::Archive), folder("Sent", FolderRole::Sent)],
    });
    s.apply(Event::FolderChanged { account: id.clone(), folder: "Sent".into() });
    let moved: Vec<Vec<u32>> = s
        .take_jobs()
        .into_iter()
        .filter_map(|j| if let Job::Apply { op: Op::Move { keys, .. }, .. } = j { Some(keys.iter().map(|k| k.uid).collect()) } else { None })
        .collect();
    assert_eq!(moved, [vec![1]], "Zoe's copy moved, Sam's left");
    // Looked at again: not moved twice.
    s.apply(Event::FolderChanged { account: id.clone(), folder: "Sent".into() });
    assert!(!s.take_jobs().iter().any(|j| matches!(j, Job::Apply { op: Op::Move { .. }, .. })));
}
