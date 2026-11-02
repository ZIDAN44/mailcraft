//! UI tests: a message written from an account that can't send (an Outlook data file, with no
//! account set up) says so, Send says why instead of "Message sent", and the message stays open.

use egui::accesskit::Role;

use super::*;

/// What the frame shows, as a screen reader is told it: each node's role, name and place.
fn shown(ctx: &egui::Context, a: &mut MailApp) -> Vec<(Role, String, egui::Rect, bool)> {
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
    let tree = out.platform_output.accesskit_update.expect("AccessKit is on");
    tree.nodes
        .iter()
        .filter_map(|(_, n)| {
            let name = n.label().or(n.value())?.to_string();
            let b = n.bounds()?;
            let rect = egui::Rect::from_min_max(egui::pos2(b.x0 as f32, b.y0 as f32), egui::pos2(b.x1 as f32, b.y1 as f32));
            Some((n.role(), name, rect, n.is_disabled()))
        })
        .collect()
}

fn click(ctx: &egui::Context, a: &mut MailApp, at: egui::Pos2) {
    let button = |pressed| Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    frame(ctx, a, vec![Event::PointerMoved(at)]);
    frame(ctx, a, vec![button(true)]);
    frame(ctx, a, vec![button(false)]);
    frame(ctx, a, vec![]);
}

#[test]
fn send_from_a_data_file_says_to_add_an_account() {
    let (mut a, dir) = app("send-data-file");
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
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    let nodes = shown(&ctx, &mut a);
    assert!(nodes.iter().any(|(_, name, ..)| *name == why), "the message says why it can't be sent");
    let send = nodes.iter().find(|(role, name, ..)| *role == Role::Button && name == "Send").expect("Send");
    assert!(send.3, "Send is off");
    // Clicked anyway: why, over the message; not sent, and the message stays as it was.
    click(&ctx, &mut a, send.2.center());
    assert_eq!(a.ui.compose_alert.as_deref(), Some(why.as_str()));
    assert_eq!(a.session.compose.as_ref(), Some(&written));
    assert!(!a.session.notices.iter().any(|n| n.text.contains("sent")), "{:?}", a.session.notices);
    // Ctrl+Enter and Alt+S, the same.
    for (k, m) in [(Key::Enter, Modifiers::COMMAND), (Key::S, Modifiers::ALT)] {
        a.ui.compose_alert = None;
        frame(&ctx, &mut a, vec![Event::ModifiersChanged(m), key(k, m)]);
        frame(&ctx, &mut a, vec![Event::ModifiersChanged(Modifiers::NONE)]);
        assert_eq!(a.ui.compose_alert.as_deref(), Some(why.as_str()), "{k:?}");
        assert_eq!(a.session.compose.as_ref(), Some(&written));
    }
    // The alert offers to add an account.
    for _ in 0..2 {
        frame(&ctx, &mut a, vec![]);
    }
    let nodes = shown(&ctx, &mut a);
    let add = nodes.iter().filter(|(role, name, ..)| *role == Role::Button && name == "Add an account").count();
    assert_eq!(add, 2, "in the bar and in the alert");
    a.ui.compose_alert = None;
    frame(&ctx, &mut a, vec![]);
    let nodes = shown(&ctx, &mut a);
    let add = nodes.iter().find(|(role, name, ..)| *role == Role::Button && name == "Add an account").map(|n| n.2.center()).unwrap();
    click(&ctx, &mut a, add);
    assert!(matches!(a.ui.dialog, Some(Dialog::AddAccount(_))), "the account form opens");
    assert!(a.session.compose.is_some(), "still open");
    let _ = std::fs::remove_dir_all(dir);
}

/// From the sample mailbox, Send is on and nothing about adding an account shows.
#[test]
fn send_from_an_account_that_can_shows_nothing_more() {
    let (mut a, dir) = app("send-sample");
    a.session.prefs.new_mail_window = false;
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    a.run("mail.new", json!({}));
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    let nodes = shown(&ctx, &mut a);
    let send = nodes.iter().find(|(role, name, ..)| *role == Role::Button && name == "Send").expect("Send");
    assert!(!send.3);
    assert!(!nodes.iter().any(|(_, name, ..)| name.contains("Add an account")));
    let _ = std::fs::remove_dir_all(dir);
}
