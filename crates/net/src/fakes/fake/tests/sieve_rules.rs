//! Issue #243: MailCraft's rules run by an IMAP server through Sieve, against a ManageSieve server
//! here: kept in MailCraft's script beside its automatic replies, the user's own filters
//! included and left as they are, and run only while a rule or the replies are on.

use mailcraft_model::organize::{Condition, Rule, RuleAction, ServerRule, ServerRuleOp, ServerRules};

use super::*;

/// The rules the server says it has, or why not.
fn rules(pool: &mut crate::worker::Pool, job: Job) -> (Option<ServerRules>, Option<String>) {
    pool.submit(job);
    let seen = until(pool, "the rules", |e| matches!(e, Event::ServerRules { .. }));
    match seen.into_iter().rev().find(|e| matches!(e, Event::ServerRules { .. })) {
        Some(Event::ServerRules { rules, error, .. }) => (rules, error),
        _ => (None, None),
    }
}

fn set(ops: Vec<ServerRuleOp>) -> Job {
    Job::SetServerRules { account: "a1".into(), ops, drop_outlook_list: false, keywords: vec![("Clients".into(), "$label2".into())] }
}

fn new_rule(name: &str, conditions: Vec<Condition>, actions: Vec<RuleAction>) -> ServerRuleOp {
    let rule = Rule { name: name.into(), enabled: true, account: Some("a1".into()), conditions, match_all: true, actions, stop: false, sent: false };
    ServerRuleOp::Create { now: ServerRule { name: name.into(), priority: 1, enabled: true, rule: Some(rule), ..Default::default() } }
}

#[test]
fn rules_run_on_an_imap_server_through_sieve() {
    let st = State::new();
    let imap_port = serve_tcp(&st);
    // The user's own filters, as a webmail keeps them, already running.
    let (sieve_port, sieve) = crate::sieve::tests::serve(crate::sieve::tests::Fake {
        scripts: vec![("roundcube".into(), "require \"fileinto\";\r\n".into())],
        active: Some("roundcube".into()),
        sieve: "fileinto imap4flags vacation include",
        log: Vec::new(),
    });
    let store = store("sieve-rules");
    crate::fakes::fake_dav::looked_for_today(&store, "a1");
    let mut pool = crate::worker::Pool::new(store.clone(), std::sync::Arc::new(crate::worker::MemoryCredentials::default()));
    pool.sieve_port = sieve_port;
    pool.submit(Job::Start { account: imap_account(imap_port, 1), password: Some(Secret("pw".into())) });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    let ours = |what: &str| -> String {
        sieve
            .lock()
            .unwrap()
            .scripts
            .iter()
            .find(|(n, _)| n == crate::sieve::SCRIPT)
            .map(|(_, s)| s.clone())
            .unwrap_or_else(|| panic!("no script: {what}"))
    };
    let active = || sieve.lock().unwrap().active.clone();

    // None yet: the user's filters named, theirs still running.
    let (got, error) = rules(&mut pool, Job::GetServerRules { account: "a1".into() });
    let got = got.unwrap_or_else(|| panic!("{error:?}"));
    assert!(got.rules.is_empty() && !got.not_running);
    assert_eq!(got.other_script.as_deref(), Some("roundcube"));

    // A rule made: written as Sieve, theirs included first, ours running.
    let op = new_rule(
        "Shop",
        vec![Condition::From("orders@shop.example".into())],
        vec![RuleAction::MoveTo("Sent".into()), RuleAction::MarkRead, RuleAction::Categorize("Clients".into())],
    );
    let (got, error) = rules(&mut pool, set(vec![op]));
    assert_eq!(error, None);
    let got = got.unwrap();
    assert_eq!(got.rules.len(), 1);
    assert_eq!(got.rules[0].rule.as_ref().map(|r| r.actions.len()), Some(3));
    let script = ours("a rule made");
    assert!(script.contains("include :personal \"roundcube\";"), "{script}");
    assert!(script.contains("if header :contains \"from\" \"orders@shop.example\" {"), "{script}");
    assert!(script.contains("addflag [\"\\\\Seen\", \"$label2\"];") && script.contains("fileinto \"Sent\";"), "{script}");
    assert_eq!(active().as_deref(), Some(crate::sieve::SCRIPT));
    assert_eq!(got.other_script.as_deref(), Some("roundcube"), "theirs still named");

    // The automatic replies turned on and off: the rule stays.
    let away = mailcraft_model::job::Vacation { enabled: true, subject: "Away".into(), text: "Back soon".into(), ..Default::default() };
    pool.submit(Job::SetVacation { account: "a1".into(), vacation: away.clone() });
    until(&pool, "replies on", |e| matches!(e, Event::Vacation { vacation: Some(v), error: None, .. } if *v == away));
    let script = ours("replies on");
    assert!(script.contains("vacation :days 1") && script.contains("orders@shop.example"), "{script}");
    pool.submit(Job::SetVacation { account: "a1".into(), vacation: mailcraft_model::job::Vacation { enabled: false, ..away } });
    until(&pool, "replies off", |e| matches!(e, Event::Vacation { vacation: Some(v), error: None, .. } if !v.enabled));
    assert!(ours("replies off").contains("orders@shop.example"));
    assert_eq!(active().as_deref(), Some(crate::sieve::SCRIPT), "the rule still runs");

    // What Sieve can't say here (attachments, with no `mime`) is refused, the rules unchanged.
    let (got, error) = rules(&mut pool, set(vec![new_rule("Files", vec![Condition::HasAttachment], vec![RuleAction::Flag])]));
    assert!(error.as_deref().is_some_and(|e| e.contains("attachments")), "{error:?}");
    assert_eq!(got.map(|g| g.rules.len()), Some(1));

    // Turned off: written so, and with nothing on, the user's own filters run again.
    let (got, _) = rules(&mut pool, Job::GetServerRules { account: "a1".into() });
    let was = got.unwrap().rules.remove(0);
    let now = ServerRule { enabled: false, rule: was.rule.clone().map(|r| Rule { enabled: false, ..r }), ..was.clone() };
    let (got, error) = rules(&mut pool, set(vec![ServerRuleOp::Set { was: was.clone(), now }]));
    assert_eq!(error, None);
    assert!(!got.unwrap().rules[0].enabled);
    assert!(!ours("off").contains("if header"), "a rule turned off isn't run");
    assert_eq!(active().as_deref(), Some("roundcube"));

    // A change to the rule as it was before (changed elsewhere since): refused.
    let (_, error) = rules(&mut pool, set(vec![ServerRuleOp::Delete { was }]));
    assert!(error.as_deref().is_some_and(|e| e.contains("changed meanwhile")), "{error:?}");

    // Another program made its own script run: MailCraft's rule, turned on, isn't running.
    let (got, _) = rules(&mut pool, Job::GetServerRules { account: "a1".into() });
    let was = got.unwrap().rules.remove(0);
    let now = ServerRule { enabled: true, rule: was.rule.clone().map(|r| Rule { enabled: true, ..r }), ..was.clone() };
    rules(&mut pool, set(vec![ServerRuleOp::Set { was, now }]));
    sieve.lock().unwrap().active = Some("roundcube".into());
    let (got, _) = rules(&mut pool, Job::GetServerRules { account: "a1".into() });
    let got = got.unwrap();
    assert!(got.not_running);
    assert_eq!(got.other_script.as_deref(), Some("roundcube"));
    pool.submit(Job::Remove { account: "a1".into() });
}
