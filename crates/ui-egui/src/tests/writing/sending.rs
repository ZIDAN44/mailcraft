//! UI tests: a message written from an account that can't send (an Outlook data file, with no
//! account set up) says so, Send says why instead of "Message sent", and the message stays open;
//! the check before sending says there's something to look at even when it can't say what.

use egui::accesskit::Role;

use super::*;

/// What the frame shows, as a screen reader is told it: the nodes on screen.
fn shown(ctx: &egui::Context, a: &mut MailApp) -> Vec<Named> {
    named(ctx, a, egui::vec2(1280.0, 800.0)).into_iter().filter(|n| n.rect.is_some()).collect()
}

/// The button a screen reader names `name`, of `nodes`.
fn button<'n>(nodes: &'n [Named], name: &str) -> Option<&'n Named> {
    nodes.iter().find(|n| n.role == Role::Button && n.name() == Some(name))
}

#[test]
fn send_from_a_data_file_says_to_add_an_account() {
    let (mut a, _) = app("send-data-file");
    a.session.prefs.new_mail_window = false;
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    // Only a data file to write from (the sample mailbox, which can send, set aside).
    let file = a.session.run("account.importPst", &json!({"path": r"C:\Mail\Archive.pst"})).unwrap()["account"].as_str().unwrap().to_string();
    a.session.accounts.retain(|x| x.config.id == file);
    a.run("mail.new", json!({"account": file}));
    a.run("compose.update", json!({"to": "ann@example.org", "subject": "Plans", "body": "See you"}));
    let written = a.session.compose.clone().expect("a message is being written");
    assert_eq!(written.account, file);
    let why = crate::reading::send_block_text(mailcraft_engine::SendBlock::DataFile);
    let blocked = crate::reading::ComposeAlert::Blocked(mailcraft_engine::SendBlock::DataFile);
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    let nodes = shown(&ctx, &mut a);
    assert!(nodes.iter().any(|n| n.name() == Some(why.as_str())), "the message says why it can't be sent");
    let send = button(&nodes, "Send").expect("Send");
    assert!(send.disabled, "Send is off");
    // Clicked anyway: why, over the message; not sent, and the message stays as it was.
    click(&ctx, &mut a, send.centre().expect("Send is on screen"));
    assert_eq!(a.ui.compose_alert, Some(blocked.clone()));
    assert_eq!(a.session.compose.as_ref(), Some(&written));
    assert!(!a.session.notices.iter().any(|n| n.text.contains("sent")), "{:?}", a.session.notices);
    // Ctrl+Enter and Alt+S, the same.
    for (k, m) in [(Key::Enter, Modifiers::COMMAND), (Key::S, Modifiers::ALT)] {
        a.ui.compose_alert = None;
        frame(&ctx, &mut a, vec![Event::ModifiersChanged(m), key(k, m)]);
        frame(&ctx, &mut a, vec![Event::ModifiersChanged(Modifiers::NONE)]);
        assert_eq!(a.ui.compose_alert, Some(blocked.clone()), "{k:?}");
        assert_eq!(a.session.compose.as_ref(), Some(&written));
    }
    // The alert offers to add an account.
    for _ in 0..2 {
        frame(&ctx, &mut a, vec![]);
    }
    let nodes = shown(&ctx, &mut a);
    let add = nodes.iter().filter(|n| n.role == Role::Button && n.name() == Some("Add an account")).count();
    assert_eq!(add, 2, "in the bar and in the alert");
    a.ui.compose_alert = None;
    frame(&ctx, &mut a, vec![]);
    let nodes = shown(&ctx, &mut a);
    let add = button(&nodes, "Add an account").and_then(Named::centre).unwrap();
    click(&ctx, &mut a, add);
    assert!(matches!(a.ui.dialog, Some(Dialog::AddAccount(_))), "the account form opens");
    assert!(a.session.compose.is_some(), "still open");
    // The account added (as the form adds it): the open message goes From it, the bar goes and
    // Send is on.
    a.ui.dialog = None;
    a.run("account.add", json!({"email": "zoe@example.org", "password": "pw"}));
    a.session.take_jobs();
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    let zoe = a.session.accounts.iter().find(|x| x.config.email == "zoe@example.org").map(|x| x.config.id.clone()).unwrap();
    assert_eq!(a.session.compose.as_ref().map(|c| c.account.as_str()), Some(zoe.as_str()));
    let nodes = shown(&ctx, &mut a);
    assert!(!nodes.iter().any(|n| n.name() == Some(why.as_str()) || n.name() == Some("Add an account")), "the bar is gone");
    let send = button(&nodes, "Send").expect("Send");
    assert!(!send.disabled, "Send is on");
}

/// From the sample mailbox, Send is on and nothing about adding an account shows.
#[test]
fn send_from_an_account_that_can_shows_nothing_more() {
    let (mut a, _) = app("send-sample");
    a.session.prefs.new_mail_window = false;
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    a.run("mail.new", json!({}));
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    let nodes = shown(&ctx, &mut a);
    let send = button(&nodes, "Send").expect("Send");
    assert!(!send.disabled);
    assert!(!nodes.iter().filter_map(Named::name).any(|name| name.contains("Add an account")));
}

/// Sent to no one: the alert says so in the window's words, whatever the engine's error says;
/// any other reason it gives is said as it gives it, as a sentence. Neither offers an account.
#[test]
fn send_to_no_one_says_to_add_a_name() {
    let (mut a, _) = app("send-no-one");
    a.session.prefs.new_mail_window = false;
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    a.run("mail.new", json!({}));
    a.run("compose.update", json!({"subject": "Plans", "body": "See you"}));
    a.run("compose.send", json!({}));
    assert_eq!(a.ui.compose_alert, Some(crate::reading::ComposeAlert::NoRecipient));
    for _ in 0..2 {
        frame(&ctx, &mut a, vec![]);
    }
    let nodes = shown(&ctx, &mut a);
    assert!(nodes.iter().any(|n| n.name() == Some("Add at least one name in To, Cc or Bcc before sending.")), "{nodes:?}");
    assert!(!nodes.iter().any(|n| n.name() == Some("Add an account")));
    assert!(a.session.compose.is_some(), "still open");
    // OK puts it away.
    let ok = button(&nodes, "OK").and_then(Named::centre).expect("OK");
    click(&ctx, &mut a, ok);
    assert_eq!(a.ui.compose_alert, None);
    // An address that isn't one: the engine's own words.
    a.run("compose.update", json!({"to": "ann@"}));
    a.run("compose.send", json!({}));
    for _ in 0..2 {
        frame(&ctx, &mut a, vec![]);
    }
    let nodes = shown(&ctx, &mut a);
    assert!(nodes.iter().any(|n| n.name() == Some("To: not an email address: ann@.")), "{nodes:?}");
    assert!(!nodes.iter().any(|n| n.name() == Some("Add an account")));
}

/// The window's GPU device goes (a driver reset after sleep): nothing can be drawn, so nobody can
/// answer "keep what you're writing?". Both messages being written are kept in Drafts, and the
/// window closes without asking (as PhotoCraft's).
#[test]
fn a_lost_gpu_keeps_whats_written_and_closes() {
    let (mut a, _) = app("gpu-lost");
    a.session.prefs.new_mail_window = false;
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    for subject in ["First thoughts", "Second thoughts"] {
        a.run("mail.new", json!({}));
        a.run("compose.update", json!({"subject": subject, "body": "Not sent yet."}));
        frame(&ctx, &mut a, vec![]);
    }
    assert_eq!(a.session.all_composes().count(), 2);
    a.graphics_lost(&ctx);
    a.graphics_lost(&ctx);
    assert!(a.ui.graphics_lost && a.ui.closing_anyway && a.ui.close_now && a.ui.dialog.is_none());
    a.run("folder.open", json!({"account": "sample", "folder": "Drafts"}));
    for subject in ["First thoughts", "Second thoughts"] {
        assert_eq!(a.session.list.iter().filter(|m| m.subject == subject).count(), 1, "{subject} in Drafts once");
    }
}

/// The check before sending, when what it found can't be read: it says there's something about
/// the recipients to look at, and Send anyway is still there.
#[test]
fn a_check_before_sending_that_cant_be_shown_still_says_so() {
    let (mut a, _) = app("send-check-unread");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.ui.dialog = Some(Dialog::SendCheck(Vec::new()));
    frame(&ctx, &mut a, vec![]);
    let texts = drawn_strings(&ctx, &mut a, egui::vec2(1280.0, 800.0));
    assert!(has(&texts, "Something about the recipients was found to look at, but it can't be shown."), "{texts:?}");
    assert!(has(&texts, "Send anyway"), "{texts:?}");
}
