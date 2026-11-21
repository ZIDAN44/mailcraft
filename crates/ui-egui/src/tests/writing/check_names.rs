//! UI tests: Check Names (issue #211) in the message being written: Ctrl+K in a recipient box or
//! the ribbon's button; one person put in, several asked about, no one said.

use super::*;

/// A new message in the reading pane, `typed` in its To box, which has the focus.
fn typing_in_to(ctx: &egui::Context, a: &mut MailApp, typed: &str) {
    a.session.prefs.new_mail_window = false;
    frame(ctx, a, vec![]);
    a.run("mail.new", json!({}));
    frame(ctx, a, vec![]);
    frame(ctx, a, vec![]);
    frame(ctx, a, vec![Event::Text(typed.into())]);
    assert_eq!(a.session.compose.as_ref().map(|c| c.to.as_str()), Some(typed));
}

fn to(a: &MailApp) -> String {
    a.session.compose.as_ref().map(|c| c.to.clone()).unwrap_or_default()
}

#[test]
fn ctrl_k_in_to_puts_in_one_and_asks_about_several() {
    let (mut a, _) = app("check-names-keys");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    typing_in_to(&ctx, &mut a, "zoe; studio; nobody-by-that-name");
    frame(&ctx, &mut a, vec![key(Key::K, Modifiers::COMMAND)]);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(to(&a), "Zoe Park <zoe@studio.example>; studio; nobody-by-that-name; ");
    assert!(a.session.notices.iter().any(|n| n.text.contains("nobody-by-that-name")), "said");
    let Some(Dialog::CheckNames(f)) = &a.ui.dialog else { panic!("asked: {:?}", a.ui.dialog) };
    assert_eq!((f.asks.len(), f.asks[0].typed.as_str()), (1, "studio"));
    let (name, email) = f.asks[0].people[1].clone();
    // OK is off until someone's picked; then it puts them in, in place of what was typed.
    let names = named(&ctx, &mut a, windows::layout::SIZE.with(std::cell::Cell::get));
    assert!(names.iter().any(|n| n.label.as_deref() == Some("OK") && n.disabled), "OK off");
    click_named(&ctx, &mut a, &format!("{name}  <{email}>"));
    click_named(&ctx, &mut a, "OK");
    frame(&ctx, &mut a, vec![]);
    assert!(a.ui.dialog.is_none());
    assert_eq!(to(&a), format!("Zoe Park <zoe@studio.example>; {name} <{email}>; nobody-by-that-name; "));
}

/// The ribbon's Check Names does it too; Cancel leaves the name as typed.
#[test]
fn the_ribbon_checks_names_and_cancel_leaves_them() {
    let (mut a, _) = app("check-names-ribbon");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    typing_in_to(&ctx, &mut a, "studio");
    click_named(&ctx, &mut a, "Check Names");
    frame(&ctx, &mut a, vec![]);
    assert!(matches!(&a.ui.dialog, Some(Dialog::CheckNames(_))), "{:?}", a.ui.dialog);
    click_named(&ctx, &mut a, "Cancel");
    frame(&ctx, &mut a, vec![]);
    assert!(a.ui.dialog.is_none());
    assert_eq!(to(&a), "studio");
}

/// A directory still asked: said, and the names checked again once it answers.
#[test]
fn a_name_waiting_on_the_directory_is_put_in_when_it_answers() {
    let (mut a, _) = app("check-names-directory");
    let p = json!({"email": "riley@contoso.example", "password": "pw", "username": "CONTOSO\\riley", "ews": "mail.contoso.example"});
    let account = a.session.run("account.add", &p).unwrap()["account"].as_str().unwrap().to_string();
    let ctx = egui::Context::default();
    typing_in_to(&ctx, &mut a, "Bo");
    frame(&ctx, &mut a, vec![key(Key::K, Modifiers::COMMAND)]);
    assert!(a.session.notices.iter().any(|n| n.text.contains("“Bo”") && n.text.contains("directory")), "said");
    assert_eq!(to(&a), "Bo");
    let people = vec![mailcraft_engine::model::DirectoryPerson { name: "Bo Ek".into(), email: "bo.ek@contoso.example".into(), ..Default::default() }];
    a.session.apply(MailEvent::Directory { account, query: "bo".into(), people, error: None, consent: false });
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(to(&a), "Bo Ek <bo.ek@contoso.example>; ");
    assert!(a.ui.names_pending.is_none());
}
