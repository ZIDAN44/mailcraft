//! Engine tests: Send / Receive as Outlook's progress window shows it, each POP or IMAP account's
//! sending and receiving: asked for, under way, done, failed with why, cancelled.

use mailcraft_model::job::{Transfer, TransferKind, TransferState};

use super::*;

fn add(s: &mut Session, v: serde_json::Value) -> String {
    s.run("account.add", &v).unwrap()["account"].as_str().unwrap().to_string()
}

/// `account`'s task of `kind`, as the session has it.
fn task(s: &Session, account: &str, kind: TransferKind) -> Option<Transfer> {
    s.send_receive.rows.iter().find(|r| r.account == account && r.transfer.kind == kind).map(|r| r.transfer.clone())
}

#[test]
fn send_receive_follows_each_accounts_sending_and_receiving() {
    let mut s = Session::open(Some(temp("send-receive")), mailcraft_model::dates::Clock::new(NOW, 0));
    let pop = add(&mut s, json!({"email": "zoe@example.org", "password": "pw", "pop": true}));
    let imap = add(&mut s, json!({"email": "ana@example.org", "password": "pw"}));
    let jmap = add(&mut s, json!({"email": "bo@example.org", "password": "pw", "jmap": "https://jmap.example.org/session"}));
    s.take_jobs();

    // Send / Receive: each POP and IMAP account's two tasks wait, sending first; the JMAP one,
    // which syncs on its own all the time, has none. Its window isn't asked for unless the user
    // asked for that, as Outlook's.
    s.run("mail.sync", &json!({})).unwrap();
    assert!(!s.send_receive.show_asked);
    s.run("mail.openSyncProgress", &json!({"on": true})).unwrap();
    s.run("mail.sync", &json!({})).unwrap();
    let rows: Vec<(String, TransferKind, TransferState)> =
        s.send_receive.rows.iter().map(|r| (r.account.clone(), r.transfer.kind, r.transfer.state)).collect();
    let waiting = TransferState::Waiting;
    assert_eq!(
        rows,
        [
            (pop.clone(), TransferKind::Sending, waiting),
            (pop.clone(), TransferKind::Receiving, waiting),
            (imap.clone(), TransferKind::Sending, waiting),
            (imap.clone(), TransferKind::Receiving, waiting)
        ]
    );
    assert!(s.send_receive.show_asked);
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::SyncAll { account } if *account == jmap)), "it syncs all the same");

    // As the network side says it goes: under way, then done.
    let going = Transfer { done: 2, total: 158, bytes_done: 1 << 20, bytes_total: 210 << 20, ..Transfer::working(TransferKind::Receiving) };
    s.apply(Event::Transfer { account: pop.clone(), transfer: going.clone() });
    assert_eq!(task(&s, &pop, TransferKind::Receiving), Some(going.clone()));
    s.apply(Event::Transfer {
        account: pop.clone(),
        transfer: Transfer { kind: TransferKind::Sending, state: TransferState::Done, ..Transfer::default() },
    });
    assert_eq!(s.send_receive.completed(), (1, 4));
    // Said of the JMAP account: not a task.
    s.apply(Event::Transfer { account: jmap.clone(), transfer: going.clone() });
    assert!(task(&s, &jmap, TransferKind::Receiving).is_none());

    // The IMAP account's connection fails: what it had waiting fails with its words.
    assert!(!s.send_receive.unseen_error);
    s.apply(Event::Status { account: imap.clone(), status: SyncStatus::NeedsPassword { error: "-ERR [AUTH] Authentication failed.".into() } });
    for kind in [TransferKind::Sending, TransferKind::Receiving] {
        let t = task(&s, &imap, kind).unwrap();
        assert_eq!((t.state, t.error.as_deref()), (TransferState::Failed, Some("-ERR [AUTH] Authentication failed.")));
    }
    assert!(s.send_receive.unseen_error, "the status bar says so until the window shows it");
    assert_eq!(s.send_receive.errors().count(), 2);
    s.transfers_seen();
    assert!(!s.send_receive.unseen_error);
    // One the network side says failed with no words of its own takes the account's.
    s.apply(Event::Transfer {
        account: imap.clone(),
        transfer: Transfer { state: TransferState::Failed, ..Transfer::working(TransferKind::Receiving) },
    });
    assert_eq!(task(&s, &imap, TransferKind::Receiving).and_then(|t| t.error), Some("-ERR [AUTH] Authentication failed.".into()));

    // Cancel All: what hasn't begun is cancelled now; what's under way is asked to stop.
    s.run("mail.sync", &json!({"account": pop})).unwrap();
    s.apply(Event::Transfer { account: pop.clone(), transfer: going.clone() });
    s.take_jobs();
    let r = s.run("mail.cancelSync", &json!({})).unwrap();
    assert_eq!((r["cancelled"].as_u64(), r["stopping"].as_u64()), (Some(1), Some(1)));
    assert_eq!(task(&s, &pop, TransferKind::Sending).map(|t| t.state), Some(TransferState::Cancelled));
    assert_eq!(task(&s, &pop, TransferKind::Receiving).map(|t| t.state), Some(TransferState::Working), "until the network side says it stopped");
    assert_eq!(s.take_jobs(), [Job::CancelTransfers { account: String::new() }]);
    // One account's (Cancel Task).
    s.run("mail.cancelSync", &json!({"account": pop})).unwrap();
    assert_eq!(s.take_jobs(), [Job::CancelTransfers { account: pop.clone() }]);

    // Told not to show its window: Send / Receive doesn't ask for it.
    s.send_receive.show_asked = false;
    s.run("mail.openSyncProgress", &json!({"on": false})).unwrap();
    s.run("mail.sync", &json!({})).unwrap();
    assert!(!s.send_receive.show_asked);

    // A removed account's tasks go with it.
    s.run("account.remove", &json!({"account": imap})).unwrap();
    assert!(s.send_receive.rows.iter().all(|r| r.account != imap));
}

/// Issue #231: an Exchange (EWS) account has its sending and receiving in the window too, as
/// Outlook's has the Exchange account's: the window said nothing had been sent or received while
/// the status bar said its folders were being read. A Microsoft Graph one still has none.
#[test]
fn an_exchange_accounts_send_receive_is_shown() {
    let mut s = Session::open(Some(temp("send-receive-ews")), mailcraft_model::dates::Clock::new(NOW, 0));
    let ews = add(&mut s, json!({"email": "zoe@contoso.example", "password": "pw", "ews": "https://mail.contoso.example/EWS/Exchange.asmx"}));
    s.take_jobs();
    s.run("mail.sync", &json!({})).unwrap();
    assert_eq!(task(&s, &ews, TransferKind::Sending).map(|t| t.state), Some(TransferState::Waiting));
    assert_eq!(task(&s, &ews, TransferKind::Receiving).map(|t| t.state), Some(TransferState::Waiting));
    let going = Transfer { folders_done: 3, folders_total: 47, folder: "Projects".into(), ..Transfer::working(TransferKind::Receiving) };
    s.apply(Event::Transfer { account: ews.clone(), transfer: going.clone() });
    assert_eq!(task(&s, &ews, TransferKind::Receiving), Some(going));
    let graph = mailcraft_model::AccountConfig { graph: Some("https://graph.microsoft.com/v1.0".into()), ..Default::default() };
    assert!(!graph.sends_and_receives());
}
