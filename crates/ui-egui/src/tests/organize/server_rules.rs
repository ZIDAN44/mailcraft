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
        rules: Some(ServerRules { outlook_list: false, rules: vec![mine.clone(), theirs], ..Default::default() }),
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
        rules: Some(ServerRules { outlook_list: false, rules: vec![rule("R1", 1), rule("R2", 2), rule("R3", 3)], ..Default::default() }),
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

/// Issue #243: an IMAP account's rules on its server (Sieve): its section, with the user's other
/// filters named, and said when MailCraft's aren't running; none for an account whose server
/// keeps none.
#[test]
fn an_imap_accounts_rules_on_the_server_are_shown() {
    let (mut a, _) = app("rules-on-sieve");
    let ctx = egui::Context::default();
    let id = a.session.run("account.add", &json!({"email": "ann@st.test", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    a.session.take_jobs();
    let texts = drawn(&ctx, &mut a);
    assert!(texts.iter().any(|t| t == "On the server: ann@st.test"), "{texts:?}");
    let mine = ServerRule {
        id: "1-a".into(),
        name: "Invoices".into(),
        priority: 1,
        enabled: true,
        rule: Some(subject("invoice")),
        raw: "{}".into(),
        ..Default::default()
    };
    a.session.apply(MailEvent::ServerRules {
        account: id.clone(),
        rules: Some(ServerRules { rules: vec![mine], other_script: Some("roundcube".into()), not_running: true, ..Default::default() }),
        error: None,
    });
    let texts = drawn(&ctx, &mut a);
    assert!(texts.iter().any(|t| t.contains("(“roundcube”) run first")), "{texts:?}");
    assert!(texts.iter().any(|t| t.starts_with("These rules aren't running")), "{texts:?}");
    assert!(texts.iter().any(|t| t.starts_with("Invoices")), "{texts:?}");
    // A server without ManageSieve: no section.
    a.session.apply(MailEvent::ServerRules { account: id, rules: None, error: Some(mailcraft_engine::model::organize::NO_SERVER_RULES.into()) });
    let texts = drawn(&ctx, &mut a);
    assert!(!texts.iter().any(|t| t.starts_with("On the server")), "{texts:?}");
}

/// Issue #243, as the tester met it on Mailcow: the rules found from Home (it had no Rules) and
/// in File › Info, the server's beside MailCraft's; SOGo's filters running in their place said,
/// with a button that runs them again; and SOGo's automatic reply shown as SOGo's, not as off,
/// and not changed from here.
#[test]
fn rules_are_found_and_another_programs_filters_are_said() {
    let (mut a, _) = app("rules-found");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let id = a.session.run("account.add", &json!({"email": "ann@mc31.test", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    a.session.take_jobs();
    a.session.run("rules.add", &json!({"rule": subject("receipt")})).unwrap();
    for _ in 0..2 {
        frame(&ctx, &mut a, vec![]);
    }
    // Home › Rules › Manage rules.
    click_named(&ctx, &mut a, "Rules");
    frame(&ctx, &mut a, vec![]);
    click_named(&ctx, &mut a, "Manage rules");
    assert!(matches!(a.ui.dialog, Some(Dialog::Rules)), "{:?}", a.ui.dialog);
    a.session.take_jobs();
    let mine = ServerRule {
        id: "1-a".into(),
        name: "Archive invoices".into(),
        priority: 1,
        enabled: true,
        rule: Some(subject("invoice")),
        raw: "{}".into(),
        ..Default::default()
    };
    let rules = ServerRules { rules: vec![mine], other_script: Some("sogo".into()), not_running: true, ..Default::default() };
    a.session.apply(MailEvent::ServerRules { account: id.clone(), rules: Some(rules.clone()), error: None });
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    click_named(&ctx, &mut a, "Run my rules again");
    assert_eq!(*a.session.server_rules(&id), mailcraft_engine::ServerRulesState::Loading, "sent to the server");

    // File › Info: both lists, the warning, and SOGo's reply as SOGo's.
    a.ui.dialog = None;
    a.session.apply(MailEvent::ServerRules { account: id.clone(), rules: Some(rules), error: None });
    a.act(crate::Action::Backstage(crate::backstage::Page::Info));
    if let Some(b) = a.ui.backstage.as_mut() {
        b.account = Some(id.clone());
    }
    let sogo = mailcraft_engine::model::job::Vacation {
        enabled: true,
        subject: "Away".into(),
        text: "Back on Monday.".into(),
        sent_by: Some("sogo".into()),
        instead: Some("sogo".into()),
        ..Default::default()
    };
    a.session.apply(MailEvent::Vacation { account: id.clone(), vacation: Some(sogo.clone()), error: None });
    let texts = drawn_strings(&ctx, &mut a, egui::vec2(1280.0, 2400.0));
    for want in [
        "On, sent by your other filters on the server (“sogo”)",
        "Invoices",
        "On the server: ann@mc31.test",
        "Archive invoices",
        "Run my rules again",
    ] {
        assert!(texts.iter().any(|t| t == want), "{want}: {texts:?}");
    }
    // The replies' dialog: said whose they are, and not saved from here.
    a.ui.backstage = None;
    a.ui.dialog = Some(Dialog::vacation(&id, &sogo, false));
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    let texts = drawn_strings(&ctx, &mut a, egui::vec2(1280.0, 900.0));
    assert!(texts.iter().any(|t| t.starts_with("MailCraft's automatic replies and rules on the server aren't running")), "{texts:?}");
    assert!(texts.iter().any(|t| t.starts_with("These are the replies your other filters on the server (“sogo”) send")), "{texts:?}");
    let size = windows::layout::SIZE.with(std::cell::Cell::get);
    assert!(named(&ctx, &mut a, size).iter().any(|n| n.label.as_deref() == Some("Save") && n.disabled), "Save is off");
}
