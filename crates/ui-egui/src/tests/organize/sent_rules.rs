//! Issue #202: a rule for mail the user sends, made in the rule editor (Applies to: Mail you
//! send) and said as such in the Rules dialog.

use mailcraft_engine::model::organize::{Condition, RuleAction};

use super::*;

fn editing(ctx: &egui::Context, a: &mut MailApp) -> Vec<String> {
    for _ in 0..3 {
        let _ = drawn_strings(ctx, a, egui::vec2(1280.0, 900.0));
    }
    drawn_strings(ctx, a, egui::vec2(1280.0, 900.0))
}

#[test]
fn a_rule_for_mail_sent_is_made_in_the_editor() {
    let (mut a, _) = app("sent-rule-editor");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    a.ui.dialog = Some(Dialog::rule_from("zoe@studio.example"));
    let shown = editing(&ctx, &mut a);
    assert!(has(&shown, "Applies to") && has(&shown, "Mail that arrives"), "{shown:?}");
    // Mail sent: list mail isn't a test it offers; one picked becomes To or Cc.
    if let Some(Dialog::EditRule(f)) = a.ui.dialog.as_mut() {
        f.test = crate::dialogs::RuleTest::Bulk;
        f.sent = true;
        f.name = "To Zoe".into();
        f.action = crate::dialogs::RuleDo::Categorize;
        f.target = "Work".into();
    }
    let shown = editing(&ctx, &mut a);
    assert!(has(&shown, "Mail you send") && has(&shown, "Run it now on the mail already in Sent"), "{shown:?}");
    let Some(Dialog::EditRule(f)) = a.ui.dialog.as_mut() else { panic!("the editor") };
    assert_eq!(f.test, crate::dialogs::RuleTest::To, "list mail isn't mail sent");
    f.text = "zoe@".into();
    f.run_now = false;
    click_named(&ctx, &mut a, "Save");
    assert!(a.ui.dialog.is_none(), "saved: {:?}", a.ui.dialog);
    let r = a.session.rules.last().cloned().expect("a rule");
    assert!(r.sent && r.conditions == [Condition::To("zoe@".into())] && r.actions == [RuleAction::Categorize("Work".into())], "{r:?}");
    // Opened again: still for mail sent; the Rules dialog says so.
    let n = a.session.rules.len() - 1;
    assert!(crate::dialogs::RuleForm::from_rule(n, &r).sent);
    a.ui.dialog = Some(Dialog::Rules);
    let shown = editing(&ctx, &mut a);
    assert!(shown.iter().any(|t| t.contains("If mail you send") && t.contains("zoe@")), "{shown:?}");
}
