//! Window tests: Send / Receive's progress, in the status bar and its window. Opened by Send /
//! Receive, the window closes once all is done well; a task that failed keeps it open on why.

use mailcraft_engine::model::job::{Transfer, TransferKind, TransferState};
use serde_json::json;

use super::*;

const SIZE: egui::Vec2 = egui::vec2(1400.0, 900.0);

fn said(a: &mut MailApp, account: &str, transfer: Transfer) {
    a.session.apply(mailcraft_engine::model::Event::Transfer { account: account.into(), transfer });
}

#[test]
fn send_receive_says_how_it_goes_in_the_status_bar_and_its_window() {
    let (mut a, _dir) = app("send-receive-window");
    let sent = Jobs::default();
    a.backend = Box::new(sent.clone());
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let id = a.session.run("account.add", &json!({"email": "zoe@pop.example", "password": "pw", "pop": true})).unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    a.session.take_jobs();
    frame(&ctx, &mut a, vec![]);

    // Send / Receive (F9): its window opens when asked to (not unless), both tasks waiting.
    a.session.run("mail.sync", &json!({})).unwrap();
    frame(&ctx, &mut a, vec![]);
    assert!(!has(&drawn_strings(&ctx, &mut a, SIZE), "Send / Receive progress"), "not unless asked");
    a.session.run("mail.openSyncProgress", &json!({"on": true})).unwrap();
    a.session.run("mail.sync", &json!({})).unwrap();
    // A window is drawn from its second frame on (the first measures it).
    frame(&ctx, &mut a, vec![]);
    let texts = drawn_strings(&ctx, &mut a, SIZE);
    assert!(has(&texts, "Send / Receive progress"), "{texts:?}");
    assert!(has(&texts, "zoe@pop.example") && has(&texts, "Sending") && has(&texts, "Receiving"));
    assert!(has(&texts, "0 of 2 done") && has(&texts, "Waiting"));

    // A download under way: the status bar says which message of how many, and their size.
    let going = Transfer { done: 9, total: 158, bytes_done: 27 << 20, bytes_total: 210 << 20, ..Transfer::working(TransferKind::Receiving) };
    said(&mut a, &id, going);
    let texts = drawn_strings(&ctx, &mut a, SIZE);
    assert!(has(&texts, "Downloading: 10 of 158 (27.0 MB of 210.0 MB)"), "{texts:?}");
    assert!(has(&texts, "9 of 158"), "{texts:?}");
    // Its line's own button stops it alone.
    sent.take();
    click_named(&ctx, &mut a, "Stop");
    let jobs = sent.take();
    assert!(jobs.contains(&mailcraft_engine::model::Job::CancelTransfers { account: id.clone() }), "{jobs:?}");

    // All done well: the window it opened by itself closes.
    said(&mut a, &id, Transfer { state: TransferState::Done, ..Transfer::working(TransferKind::Sending) });
    said(&mut a, &id, Transfer { state: TransferState::Done, done: 158, total: 158, ..Transfer::working(TransferKind::Receiving) });
    let texts = drawn_strings(&ctx, &mut a, SIZE);
    assert!(!has(&texts, "Send / Receive progress") && !has(&texts, "Downloading"), "{texts:?}");

    // Again, and the server refuses the password: kept open, on why.
    a.session.run("mail.sync", &json!({})).unwrap();
    frame(&ctx, &mut a, vec![]);
    let why = "-ERR [AUTH] Authentication failed.";
    said(&mut a, &id, Transfer { state: TransferState::Done, ..Transfer::working(TransferKind::Sending) });
    said(&mut a, &id, Transfer { state: TransferState::Failed, error: Some(why.into()), ..Transfer::working(TransferKind::Receiving) });
    frame(&ctx, &mut a, vec![]);
    let texts = drawn_strings(&ctx, &mut a, SIZE);
    assert!(has(&texts, "Send / Receive progress"), "{texts:?}");
    assert!(has(&texts, &format!("zoe@pop.example: receiving failed: {why}")), "under the tasks: {texts:?}");
    assert!(!a.session.send_receive.unseen_error, "shown, so no longer news");

    // Closed: the status bar says nothing more of it. One that fails while it's closed: said
    // there until it's shown.
    a.ui.send_receive.open = false;
    said(&mut a, &id, Transfer { state: TransferState::Failed, error: Some(why.into()), ..Transfer::working(TransferKind::Receiving) });
    let texts = drawn_strings(&ctx, &mut a, SIZE);
    assert!(has(&texts, "Send / Receive failed") && !has(&texts, "Send / Receive progress"), "{texts:?}");
    click_named(&ctx, &mut a, "Send / Receive failed");
    frame(&ctx, &mut a, vec![]);
    let texts = drawn_strings(&ctx, &mut a, SIZE);
    assert!(has(&texts, "Send / Receive progress") && has(&texts, "receiving failed"), "opened on why: {texts:?}");

    // Told not to open by itself: Send / Receive leaves it closed.
    click_named(&ctx, &mut a, "Open this window when Send / Receive starts");
    frame(&ctx, &mut a, vec![]);
    assert!(!a.session.prefs.open_send_receive);
    a.ui.send_receive.open = false;
    a.session.run("mail.sync", &json!({})).unwrap();
    let texts = drawn_strings(&ctx, &mut a, SIZE);
    assert!(!has(&texts, "Send / Receive progress"), "{texts:?}");
}
