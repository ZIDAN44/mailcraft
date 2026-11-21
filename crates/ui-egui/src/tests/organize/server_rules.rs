//! The Rules dialog with an Exchange account (issue #139): its rules on the server, asked for as
//! it opens, shown under MailCraft's own, and changed from it.

use mailcraft_engine::model::organize::{Condition, Rule, RuleAction, ServerRule, ServerRules};
use mailcraft_engine::model::{Event as MailEvent, Job};

use super::*;

fn drawn(ctx: &egui::Context, a: &mut MailApp) -> Vec<String> {
    a.ui.dialog.get_or_insert(Dialog::Rules);
    for _ in 0..3 {
        let _ = drawn_strings(ctx, a, egui::vec2(1280.0, 900.0));
    }
    drawn_strings(ctx, a, egui::vec2(1280.0, 900.0))
}

fn subject(word: &str) -> Rule {
    Rule {
        name: "Invoices".into(),
        enabled: true,
        account: None,
        conditions: vec![Condition::Subject(word.into())],
        match_all: true,
        actions: vec![RuleAction::MarkRead],
        stop: false,
        sent: false,
    }
}

#[test]
fn an_exchange_accounts_rules_on_the_server_are_shown_and_changed() {
    let (mut a, _) = app("rules-on-server");
    let ctx = egui::Context::default();
    let id = a
        .session
        .run("account.add", &json!({"email": "zoe@contoso.example", "password": "pw", "ews": "https://mail.contoso.example/EWS/Exchange.asmx"}))
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    a.session.take_jobs();
    let texts = drawn(&ctx, &mut a);
    assert!(texts.iter().any(|t| t == "On the server: zoe@contoso.example"), "{texts:?}");
    assert_eq!(*a.session.server_rules(&id), mailcraft_engine::ServerRulesState::Loading, "asked for as the dialog opens");
    assert!(texts.iter().any(|t| t == "Reading the rules on the server…"), "{texts:?}");

    let mine = ServerRule {
        id: "R1".into(),
        name: "Invoices".into(),
        priority: 1,
        enabled: true,
        rule: Some(subject("invoice")),
        raw: "<a/>".into(),
        ..Default::default()
    };
    let theirs = ServerRule { id: "R2".into(), name: "To my phone".into(), priority: 2, enabled: true, raw: "<b/>".into(), ..Default::default() };
    a.session.apply(MailEvent::ServerRules {
        account: id.clone(),
        rules: Some(ServerRules { outlook_list: false, rules: vec![mine.clone(), theirs] }),
        error: None,
    });
    let texts = drawn(&ctx, &mut a);
    assert!(texts.iter().any(|t| t.starts_with("Invoices") && t.contains("invoice")), "{texts:?}");
    assert!(texts.iter().any(|t| t.starts_with("To my phone") && t.contains("Made in another program")), "{texts:?}");
    assert!(texts.iter().any(|t| t == "New rule on the server…"));

    // Edited as MailCraft can say it: what it looks for changed, saved on the server.
    let target = crate::dialogs::ServerTarget { account: id.clone(), id: Some("R1".into()), drop_outlook_list: false };
    let mut f = crate::dialogs::RuleForm::for_server(target, Some(&mine));
    assert_eq!(f.text, "invoice");
    f.text = "bill".into();
    let rule = f.to_rule().unwrap();
    a.session.take_jobs();
    a.act(crate::Action::Submit("rules.serverSave", json!({"account": id, "id": "R1", "rule": rule})));
    let jobs = a.session.take_jobs();
    let Some(Job::SetServerRules { ops, .. }) = jobs.first() else { panic!("{jobs:?}") };
    assert!(matches!(&ops[..], [mailcraft_engine::model::organize::ServerRuleOp::Set { now, .. }]
        if now.rule.as_ref().is_some_and(|r| r.conditions == [Condition::Subject("bill".into())])));
    // A new one on the server offers only what a server rule can do.
    let target = crate::dialogs::ServerTarget { account: id.clone(), id: None, drop_outlook_list: false };
    let f = crate::dialogs::RuleForm::for_server(target, None);
    assert_eq!(f.account.as_deref(), Some(id.as_str()));
    assert!(f.server.is_some() && f.original.is_none());
}

/// Issue #214: a rule made in another program, its note beside its name, is cut short before the
/// row's buttons (the whole of it on hover): the note ran under ↑, ↓ and Delete, and past the
/// dialog's edge.
#[test]
fn a_rules_note_stays_clear_of_its_buttons() {
    let (mut a, _) = app("rules-note-width");
    let ctx = egui::Context::default();
    let id = a
        .session
        .run("account.add", &json!({"email": "zoe@contoso.example", "password": "pw", "ews": "https://mail.contoso.example/EWS/Exchange.asmx"}))
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    a.session.take_jobs();
    let rule = |n: &str, p: u32| ServerRule {
        id: n.into(),
        name: format!("Forward the studio's invoices {n}"),
        priority: p,
        enabled: true,
        raw: "<b/>".into(),
        ..Default::default()
    };
    a.session.apply(MailEvent::ServerRules {
        account: id,
        rules: Some(ServerRules { outlook_list: false, rules: vec![rule("R1", 1), rule("R2", 2), rule("R3", 3)] }),
        error: None,
    });
    a.ui.dialog.get_or_insert(Dialog::Rules);
    let size = egui::vec2(1280.0, 900.0);
    let mut shown = Vec::new();
    for _ in 0..4 {
        shown = drawn_text(&ctx, &mut a, size);
    }
    let note = shown.iter().find(|d| d.text.starts_with("Forward the studio's invoices R2")).expect("the rule's row");
    let up = shown.iter().filter(|d| d.text == "↑").map(|d| d.rect).find(|r| (r.center().y - note.rect.center().y).abs() < 10.0).expect("its ↑");
    assert!(note.rect.right() <= up.left(), "the note {:?} runs under ↑ {:?}", note.rect, up);
}
