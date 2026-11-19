//! Gap 9: flagged mail's reminders, in the Reminders window with events' and tasks'. A flag given
//! a reminder (Follow Up › Add Reminder…) reminds when its time comes; snoozed, it reminds again
//! then; dismissed, it reminds no more and stays flagged; Clear Flag and Mark Complete take it
//! off, and their Undo puts it back. An IMAP account's are kept here; an Exchange account's are
//! the server's, as Outlook keeps them on the message.

use mailcraft_model::{Flags, MessageKey, Summary};

use super::*;

/// Every folder's flagged mail read, as the To-Do Bar (or the reminders) read it.
fn read_all(s: &mut Session) {
    for _ in 0..500 {
        if !s.todo_step() {
            return;
        }
    }
    panic!("the folders were never all read");
}

/// The flagged messages' reminders due now: (subject, when it reminds, its key).
fn due_mail(s: &Session) -> Vec<(String, i64, String)> {
    s.due_reminders(s.clock.now).into_iter().filter(|r| r.message.is_some()).map(|r| (r.summary, r.due, r.key)).collect()
}

#[test]
fn flagged_mail_reminds_and_is_answered_here() {
    let mut s = sample_session("flag-reminders");
    read_all(&mut s);
    assert!(due_mail(&s).is_empty());
    let k = key_of(&s, "Character turnarounds");
    let today = mailcraft_model::dates::today(&s.clock);
    let tomorrow = mailcraft_model::dates::day_string(today + 1);
    s.run("list.select", &k).unwrap();
    // A day alone: nine in its morning, as a task's.
    let r = s.run("mail.flag", &json!({"reminder": tomorrow})).unwrap();
    let at = s.clock.utc((today + 1) * 86_400 + 9 * 3600);
    assert_eq!(r["reminder"], at);
    let m = s.list.iter().find(|m| m.subject == "Character turnarounds").cloned().unwrap();
    assert!(m.flags.flagged, "Add Reminder flags it");
    assert_eq!(s.message_reminder(&m), Some(at));
    assert!(s.mail_reminders_wanted());
    assert!(due_mail(&s).is_empty(), "not yet");

    // Its time comes: listed, with the message.
    s.clock.now = at + 60;
    let due = s.due_reminders(s.clock.now);
    let r = due.iter().find(|r| r.message.is_some()).cloned().unwrap();
    assert_eq!((r.summary.as_str(), r.due, r.message.as_ref()), ("Character turnarounds", at, Some(&m.key)));
    // Snoozed ten minutes: gone, then back.
    s.run("reminders.snooze", &json!({"key": r.key, "minutes": 10})).unwrap();
    assert!(due_mail(&s).is_empty());
    assert_eq!(s.prefs.mail_reminders.get(&m.follow_up_id()), Some(&(at + 60 + 600)));
    s.clock.now += 600;
    let again = due_mail(&s);
    assert_eq!(again.len(), 1);
    // Dismissed: gone for good, still flagged.
    s.run("reminders.dismiss", &json!({"key": again[0].2})).unwrap();
    assert!(due_mail(&s).is_empty() && s.prefs.mail_reminders.is_empty());
    assert!(s.list.iter().any(|m| m.subject == "Character turnarounds" && m.flags.flagged));

    // Given one again (a time of its own); Clear Flag takes it off, Undo puts it back.
    s.run("list.select", &k).unwrap();
    let when = format!("{}T08:15", mailcraft_model::dates::day_string(today));
    s.run("mail.flag", &json!({"reminder": when})).unwrap();
    let at = s.clock.utc(today * 86_400 + 8 * 3600 + 15 * 60);
    assert_eq!(due_mail(&s).first().map(|d| d.1), Some(at));
    for p in [json!({"on": false}), json!({"complete": true})] {
        s.run("list.select", &k).unwrap();
        s.run("mail.flag", &p).unwrap();
        assert!(due_mail(&s).is_empty() && s.prefs.mail_reminders.is_empty(), "{p}");
        s.run("edit.undo", &json!({})).unwrap();
        read_all(&mut s);
        assert_eq!(due_mail(&s).first().map(|d| d.1), Some(at), "{p}: back");
    }
    // Taken off (`none`), and that undone; given one, and that undone.
    s.run("list.select", &k).unwrap();
    s.run("mail.flag", &json!({"reminder": "none"})).unwrap();
    assert!(due_mail(&s).is_empty());
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(due_mail(&s).len(), 1);
    s.run("list.select", &k).unwrap();
    s.run("mail.flag", &json!({"reminder": "none"})).unwrap();
    s.run("mail.flag", &json!({"due": "tomorrow", "reminder": tomorrow})).unwrap();
    s.run("edit.undo", &json!({})).unwrap();
    let m = s.list.iter().find(|m| m.subject == "Character turnarounds").cloned().unwrap();
    assert_eq!((s.follow_up_due(&m), s.message_reminder(&m)), (None, None), "one Undo takes back the day and the reminder");
    for bad in [json!({"reminder": "someday"}), json!({"reminder": 5})] {
        assert!(s.run("mail.flag", &bad).is_err(), "{bad}");
    }
}

/// The reminders kept in the preferences come from disk: what isn't a time a follow-up may
/// have (out of range, as a damaged or hostile file has it), or by a key too long or empty, is
/// left out as they're read.
#[test]
fn the_preferences_flag_reminders_read_from_disk_are_times() {
    let long = "x".repeat(mailcraft_model::organize::MAX_FOLLOW_UP_ID + 1);
    let read = json!({"mailReminders": {"ok@x": 1_795_000_000, "far@x": i64::MAX, "early@x": i64::MIN, (long): 1_795_000_000, "": 1}});
    let p: crate::Prefs = serde_json::from_value(read).unwrap();
    assert_eq!(p.mail_reminders, std::collections::BTreeMap::from([("ok@x".to_string(), 1_795_000_000)]));
}

/// An Exchange account's flag reminders are the server's: one Outlook set reminds here; dismissed
/// or snoozed here, the server's is changed (so Outlook doesn't remind of it either), and it's
/// gone at once; set here, it's sent, and Undo sends back what it was; Clear Flag and Mark
/// Complete take it off there. Nothing is kept here for it.
#[test]
fn an_exchange_flags_reminder_is_the_servers() {
    let mut s = Session::open(Some(temp("flag-reminders-ews")), mailcraft_model::dates::Clock::new(NOW, 3600));
    let id = s
        .run("account.add", &json!({"email": "zoe@contoso.example", "password": "pw", "ews": "https://mail.contoso.example/EWS/Exchange.asmx"}))
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    let inbox =
        mailcraft_model::Folder { account: id.clone(), path: "INBOX".into(), role: FolderRole::Inbox, selectable: true, ..Default::default() };
    s.apply(Event::Folders { account: id.clone(), folders: vec![inbox] });
    let key = MessageKey { account: id.clone(), folder: "INBOX".into(), validity: 1, uid: 1 };
    let flags = Flags { flagged: true, reminder: Some(NOW - 60), ..Default::default() };
    file_in(
        &mut s,
        Summary {
            key: key.clone(),
            message_id: "budget@contoso.example".into(),
            subject: "RE: Budget".into(),
            date: NOW,
            flags,
            ..Default::default()
        },
        None,
    );
    s.run("folder.open", &json!({"account": id, "folder": "INBOX"})).unwrap();
    assert!(s.mail_reminders_wanted(), "an Exchange account's may have some");
    read_all(&mut s);
    s.take_jobs();
    let sent = |s: &mut Session| -> Vec<Op> {
        s.take_jobs().into_iter().filter_map(|j| if let Job::Apply { op, .. } = j { Some(op) } else { None }).collect()
    };
    let due = due_mail(&s);
    assert_eq!(due.iter().map(|d| (d.0.as_str(), d.1)).collect::<Vec<_>>(), [("Budget", NOW - 60)], "as Outlook set it, its RE: off");

    // Snoozed: the server's moved on; dismissed: no longer set there. Each gone at once.
    s.run("reminders.snooze", &json!({"key": due[0].2, "minutes": 5})).unwrap();
    assert_eq!(sent(&mut s), [Op::SetReminder { keys: vec![key.clone()], at: Some(NOW + 300) }]);
    assert!(due_mail(&s).is_empty());
    s.clock.now = NOW + 300;
    let due = due_mail(&s);
    assert_eq!(due.len(), 1);
    s.run("reminders.dismiss", &json!({"key": due[0].2})).unwrap();
    assert_eq!(sent(&mut s), [Op::SetReminder { keys: vec![key.clone()], at: None }]);
    assert!(due_mail(&s).is_empty());
    assert!(s.prefs.mail_reminders.is_empty() && s.prefs.reminders.is_empty(), "nothing kept here");

    // Set here: sent; Undo sends back none.
    s.selection = vec![key.clone()];
    s.run("mail.flag", &json!({"reminder": "2026-12-01T10:30"})).unwrap();
    let at = s.clock.utc(mailcraft_model::dates::days_from_civil(2026, 12, 1) * 86_400 + 10 * 3600 + 30 * 60);
    assert!(sent(&mut s).contains(&Op::SetReminder { keys: vec![key.clone()], at: Some(at) }));
    assert_eq!(s.summary(&key).and_then(|m| s.message_reminder(m)), Some(at));
    s.run("edit.undo", &json!({})).unwrap();
    assert!(sent(&mut s).contains(&Op::SetReminder { keys: vec![key.clone()], at: None }));
    // Clear Flag and Mark Complete take it off there; their Undo sends it back.
    for p in [json!({"on": false}), json!({"complete": true})] {
        s.selection = vec![key.clone()];
        s.run("mail.flag", &json!({"reminder": "2026-12-01T10:30"})).unwrap();
        sent(&mut s);
        s.run("mail.flag", &p).unwrap();
        assert!(sent(&mut s).contains(&Op::SetReminder { keys: vec![key.clone()], at: None }), "{p}");
        s.run("edit.undo", &json!({})).unwrap();
        assert!(sent(&mut s).contains(&Op::SetReminder { keys: vec![key.clone()], at: Some(at) }), "{p}: back");
    }
}

/// A reminder already due when its folder's flagged mail is first read (MailCraft started, or
/// Add Reminder… before the folders were read): the Reminders window is told at once, not at its
/// next look a quarter of a minute on.
#[test]
fn a_flag_reminder_found_by_reading_its_folder_is_told_at_once() {
    let mut s = sample_session("flag-reminders-found");
    let m = s.list.iter().find(|m| m.flags.flagged).cloned().expect("a flagged message in the sample");
    s.prefs.mail_reminders.insert(m.follow_up_id(), NOW - 60);
    let revision = s.pim_revision;
    read_all(&mut s);
    assert_ne!(s.pim_revision, revision, "the window looks again");
    assert_eq!(due_mail(&s).len(), 1);
}
