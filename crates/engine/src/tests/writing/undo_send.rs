//! Engine tests: Undo Send (issue #135). A message sent waits in the Outbox for the seconds set,
//! and can be taken back into the form as it was written; then it goes as Delay Delivery's do.

use super::*;

fn session(name: &str, wait: u64) -> Session {
    let mut s = Session::open(Some(temp(name)), mailcraft_model::dates::Clock::new(NOW, 0));
    s.run("account.add", &json!({"email": "zoe@example.org", "password": "pw"})).unwrap();
    s.run("options.undoSend", &json!({"seconds": wait})).unwrap();
    s.take_jobs();
    s
}

fn write(s: &mut Session, subject: &str) {
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "ann@example.org", "subject": subject, "body": "Hello Ann"})).unwrap();
}

#[test]
fn a_message_sent_waits_and_can_be_taken_back() {
    let mut s = session("undo-send", 10);
    write(&mut s, "Quote");
    let sent = s.run("compose.send", &json!({})).unwrap();
    assert_eq!(sent["undo"], true);
    assert!(s.compose.is_none(), "the form closes as it's sent");
    assert!(!s.take_jobs().iter().any(|j| matches!(j, Job::Send { .. })), "not sent yet");
    assert_eq!(s.outbox.first().map(|(_, i)| i.not_before), Some(NOW + 10), "waiting in the Outbox");
    assert_eq!(s.undo_holds().count(), 1);
    assert!(!s.notices.iter().any(|n| n.text.starts_with("It will be sent")), "said by the Undo bar, not a notice");

    // Undo: back in the form as it was written, out of the Outbox.
    s.run("outbox.undoSend", &json!({})).unwrap();
    let c = s.compose.as_ref().expect("open again");
    assert_eq!(c.subject, "Quote");
    assert!(c.to.contains("ann@example.org"), "{}", c.to);
    assert!(c.body.contains("Hello Ann") && !c.sending);
    assert!(s.outbox.is_empty() && s.undo_holds().count() == 0);
    assert!(s.run("outbox.undoSend", &json!({})).is_err(), "nothing left to undo");

    // Sent again and left alone: it goes once its time comes, and can't be undone then.
    s.run("compose.send", &json!({})).unwrap();
    s.set_clock(NOW + 5, 0);
    s.tick();
    assert!(s.take_jobs().is_empty(), "still waiting");
    s.set_clock(NOW + 11, 0);
    s.tick();
    assert_eq!(s.take_jobs().iter().filter(|j| matches!(j, Job::FlushOutbox { .. })).count(), 1);
    assert_eq!(s.undo_holds().count(), 0);
    assert!(s.run("outbox.undoSend", &json!({})).is_err(), "it has gone");
    assert_eq!(s.outbox.len(), 1, "left to go");
}

/// Off (as it starts), mail goes at once; a message given its own later time (Delay Delivery)
/// waits for that, with no Undo; the wait is 0 to 60 seconds.
#[test]
fn off_and_delay_delivery_are_as_they_were() {
    let mut s = session("undo-send-off", 0);
    write(&mut s, "Now");
    s.run("compose.send", &json!({})).unwrap();
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::Send { .. })), "sent at once");
    assert!(s.outbox.is_empty());

    s.run("options.undoSend", &json!({"seconds": 20})).unwrap();
    write(&mut s, "Tomorrow");
    s.run("compose.update", &json!({"sendAfter": NOW + 86_400})).unwrap();
    s.run("compose.send", &json!({})).unwrap();
    assert_eq!(s.outbox.first().map(|(_, i)| i.not_before), Some(NOW + 86_400));
    assert_eq!(s.undo_holds().count(), 0, "Delay Delivery's own time, no Undo");
    assert!(s.run("options.undoSend", &json!({"seconds": 61})).is_err());
    assert!(s.run("options.undoSend", &json!({"seconds": -1})).is_err());
    assert_eq!(s.prefs.undo_send_secs, 20);
}

/// Closing while a message waits to be undone: it goes now rather than at the next start.
#[test]
fn closing_lets_a_waiting_message_go() {
    let mut s = session("undo-send-close", 30);
    write(&mut s, "Before closing");
    s.run("compose.send", &json!({})).unwrap();
    s.take_jobs();
    assert!(s.release_undo_holds());
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::FlushOutbox { .. })), "asked to go");
    assert_eq!(s.outbox.first().map(|(_, i)| i.not_before), Some(NOW));
    assert_eq!(s.undo_holds().count(), 0);
    assert!(!s.release_undo_holds(), "nothing more to let go");
}
