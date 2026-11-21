//! Issue #139: an Exchange mailbox's Inbox rules on its server, as the session asks for them,
//! shows them and turns what's done to them into changes for the server.

use mailcraft_model::organize::{Condition, Rule, RuleAction, ServerRule, ServerRuleOp, ServerRules};

use crate::ServerRulesState;

use super::*;

fn exchange(name: &str) -> (Session, String) {
    let mut s = Session::open(Some(temp(name)), mailcraft_model::dates::Clock::new(NOW, 0));
    let id = s
        .run("account.add", &json!({"email": "zoe@contoso.example", "password": "pw", "ews": "https://mail.contoso.example/EWS/Exchange.asmx"}))
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    s.take_jobs();
    (s, id)
}

fn rule(name: &str, word: &str) -> Rule {
    Rule {
        name: name.into(),
        enabled: true,
        account: None,
        conditions: vec![Condition::Subject(word.into())],
        match_all: true,
        actions: vec![RuleAction::MarkRead],
        stop: false,
        sent: false,
    }
}

/// Two rules on the server (one MailCraft can say, one it can't), and Outlook's own copy of them.
fn on_server() -> ServerRules {
    let mine = ServerRule {
        id: "R1".into(),
        name: "News".into(),
        priority: 1,
        enabled: true,
        rule: Some(rule("News", "news")),
        raw: "<a/>".into(),
        ..Default::default()
    };
    let forward =
        ServerRule { id: "R2".into(), name: "Forward".into(), priority: 2, enabled: true, rule: None, raw: "<b/>".into(), ..Default::default() };
    ServerRules { outlook_list: true, rules: vec![mine, forward] }
}

fn ops(s: &mut Session) -> (Vec<ServerRuleOp>, bool) {
    match s.take_jobs().as_slice() {
        [Job::SetServerRules { ops, drop_outlook_list, .. }] => (ops.clone(), *drop_outlook_list),
        other => panic!("{other:?}"),
    }
}

#[test]
fn the_rules_on_the_server_are_asked_for_shown_and_changed() {
    let (mut s, id) = exchange("server-rules");
    assert_eq!(s.server_rule_accounts(), std::slice::from_ref(&id));
    s.run("rules.server", &json!({"account": id})).unwrap();
    assert!(matches!(s.take_jobs().as_slice(), [Job::GetServerRules { .. }]));
    assert_eq!(*s.server_rules(&id), ServerRulesState::Loading);
    s.apply(Event::ServerRules { account: id.clone(), rules: Some(on_server()), error: None });
    let ServerRulesState::Loaded { rules, error: None } = s.server_rules(&id).clone() else { panic!("{:?}", s.server_rules(&id)) };
    assert_eq!(rules.rules[0].rule.as_ref().and_then(|r| r.account.clone()), Some(id.clone()), "a server rule is its account's alone");

    // Beside Outlook's own copy: not changed unless it goes.
    let e = s.run("rules.serverSwitch", &json!({"account": id, "id": "R2", "on": false})).unwrap_err();
    assert!(e.to_string().contains("Outlook"), "{e}");
    assert!(s.take_jobs().is_empty());
    s.run("rules.serverSwitch", &json!({"account": id, "id": "R2", "on": false, "dropOutlookList": true})).unwrap();
    let (ops1, dropped) = ops(&mut s);
    assert!(dropped);
    assert!(matches!(&ops1[..], [ServerRuleOp::Set { was, now }] if was.id == "R2" && !now.enabled && now.raw == "<b/>"));
    assert_eq!(*s.server_rules(&id), ServerRulesState::Loading);

    // Read again, the copy gone: a new rule goes last; one moved up changes places.
    let mut rules = on_server();
    rules.outlook_list = false;
    s.apply(Event::ServerRules { account: id.clone(), rules: Some(rules), error: None });
    s.run("rules.serverSave", &json!({"account": id, "rule": rule("Bills", "bill")})).unwrap();
    let (made, _) = ops(&mut s);
    assert!(matches!(&made[..], [ServerRuleOp::Create { now }] if now.priority == 3 && now.name == "Bills"));
    s.apply(Event::ServerRules { account: id.clone(), rules: Some(ServerRules { outlook_list: false, ..on_server() }), error: None });
    s.run("rules.serverMove", &json!({"account": id, "id": "R2", "up": true})).unwrap();
    let (moved, _) = ops(&mut s);
    let places: Vec<(String, u32)> = moved
        .iter()
        .filter_map(|o| match o {
            ServerRuleOp::Set { now, .. } => Some((now.id.clone(), now.priority)),
            _ => None,
        })
        .collect();
    assert_eq!(places, [("R2".to_string(), 1), ("R1".to_string(), 2)]);

    // A rule made elsewhere is turned on or off, moved or deleted here, not changed.
    s.apply(Event::ServerRules { account: id.clone(), rules: Some(ServerRules { outlook_list: false, ..on_server() }), error: None });
    assert!(s.run("rules.serverSave", &json!({"account": id, "id": "R2", "rule": rule("Forward", "x")})).is_err());
    s.run("rules.serverSave", &json!({"account": id, "id": "R1", "rule": rule("News", "newsletter")})).unwrap();
    let (changed, _) = ops(&mut s);
    assert!(
        matches!(&changed[..], [ServerRuleOp::Set { now, .. }] if now.rule.as_ref().is_some_and(|r| r.conditions == [Condition::Subject("newsletter".into())]))
    );

    // A change that failed: the rules as the server has them, with why.
    s.apply(Event::ServerRules { account: id.clone(), rules: Some(on_server()), error: Some("changed elsewhere".into()) });
    assert!(matches!(s.server_rules(&id), ServerRulesState::Loaded { error: Some(_), .. }));
    // An account with no rules on its server.
    s.apply(Event::ServerRules { account: id.clone(), rules: None, error: Some("offline".into()) });
    assert_eq!(*s.server_rules(&id), ServerRulesState::Failed("offline".into()));
    let (mut s2, _) = (Session::open(Some(temp("server-rules-imap")), mailcraft_model::dates::Clock::new(NOW, 0)), ());
    let imap = s2
        .run("account.add", &json!({"email": "a@b.example", "password": "pw", "imapHost": "imap.b.example", "smtpHost": "smtp.b.example"}))
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(s2.run("rules.server", &json!({"account": imap})).is_err(), "not an Exchange account");
}
