//! Issue #243: MailCraft's rules run by a real Sieve server (Dovecot with Pigeonhole in Docker;
//! never a real account), run by hand:
//!
//! ```text
//! MAILCRAFT_REAL_SIEVE="127.0.0.1:31143 user password 34190" \
//!   cargo test -p mailcraft-net real_sieve -- --ignored --nocapture
//! ```
//!
//! The IMAP server, a user and password, and its ManageSieve port. The server checks each script
//! MailCraft writes (a script it can't compile is refused); the rules are read back as written.

use mailcraft_model::job::{Secret, Vacation};
use mailcraft_model::organize::{Condition, Rule, RuleAction, ServerRule, ServerRuleOp, ServerRules};
use mailcraft_model::{Event, Job, SyncStatus};

use super::real_servers::{at, imap_account};
use crate::worker::{MemoryCredentials, Pool};

/// The rules the server says it has, or why not.
fn rules(pool: &mut Pool, job: Job) -> (Option<ServerRules>, Option<String>) {
    pool.submit(job);
    let seen = crate::waiting::until(pool, "the rules", |e| matches!(e, Event::ServerRules { .. }));
    match seen.into_iter().rev().find(|e| matches!(e, Event::ServerRules { .. })) {
        Some(Event::ServerRules { rules, error, .. }) => (rules, error),
        _ => (None, None),
    }
}

#[test]
#[ignore = "needs a real IMAP server with ManageSieve in MAILCRAFT_REAL_SIEVE"]
fn real_sieve_rules() {
    let Ok(v) = std::env::var("MAILCRAFT_REAL_SIEVE") else {
        eprintln!("MAILCRAFT_REAL_SIEVE isn't set: skipped");
        return;
    };
    let p: Vec<&str> = v.split_whitespace().collect();
    let [imap, user, password, port] = p.as_slice() else { panic!("MAILCRAFT_REAL_SIEVE: imap user password sieve-port") };
    let store = mailcraft_store::Store::open(std::env::temp_dir().join(format!("mailcraft-real-sieve-{}", std::process::id()))).unwrap();
    let mut pool = Pool::new(store, std::sync::Arc::new(MemoryCredentials::default()));
    pool.sieve_port = port.parse().unwrap();
    pool.submit(Job::Start { account: imap_account("s1", user, &at(imap)), password: Some(Secret((*password).into())) });
    crate::waiting::until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    let set = |ops: Vec<ServerRuleOp>| Job::SetServerRules {
        account: "s1".into(),
        ops,
        drop_outlook_list: false,
        keywords: vec![("Work".into(), "$label2".into())],
    };
    let rule = |name: &str, conditions: Vec<Condition>, actions: Vec<RuleAction>| {
        let r = Rule { name: name.into(), enabled: true, account: None, conditions, match_all: false, actions, stop: true, sent: false };
        ServerRuleOp::Create { now: ServerRule { name: name.into(), priority: 1, enabled: true, rule: Some(r), ..Default::default() } }
    };
    // From none: those a run before left are deleted.
    let (before, _) = rules(&mut pool, Job::GetServerRules { account: "s1".into() });
    let old: Vec<ServerRuleOp> = before.map(|b| b.rules.into_iter().map(|was| ServerRuleOp::Delete { was }).collect()).unwrap_or_default();
    if !old.is_empty() {
        let (left, error) = rules(&mut pool, set(old));
        assert_eq!((left.map(|l| l.rules.len()), error), (Some(0), None));
    }
    // Every test and action MailCraft writes, which the server compiles.
    let shop = rule(
        "Shop",
        vec![Condition::From("shop.example".into()), Condition::Subject("Your order".into())],
        vec![RuleAction::MoveTo("Shop".into()), RuleAction::MarkRead, RuleAction::Flag, RuleAction::Pin, RuleAction::Categorize("Work".into())],
    );
    let lists =
        rule("Lists", vec![Condition::Bulk, Condition::AnyText("newsletter".into()), Condition::To("team@".into())], vec![RuleAction::Delete]);
    let (got, error) = rules(&mut pool, set(vec![shop, lists]));
    let got = got.unwrap_or_else(|| panic!("{error:?}"));
    assert_eq!(error, None);
    let names: Vec<&str> = got.rules.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["Shop", "Lists"]);
    assert!(!got.not_running);
    // The automatic replies on with them, then off: the rules stay.
    let away = Vacation { enabled: true, subject: "Away".into(), text: "Back soon".into(), ..Default::default() };
    pool.submit(Job::SetVacation { account: "s1".into(), vacation: away.clone() });
    crate::waiting::until(&pool, "replies on", |e| matches!(e, Event::Vacation { vacation: Some(v), error: None, .. } if *v == away));
    pool.submit(Job::SetVacation { account: "s1".into(), vacation: Vacation { enabled: false, ..away } });
    crate::waiting::until(&pool, "replies off", |e| matches!(e, Event::Vacation { vacation: Some(v), error: None, .. } if !v.enabled));
    let (got, _) = rules(&mut pool, Job::GetServerRules { account: "s1".into() });
    assert_eq!(got.map(|g| g.rules.len()), Some(2), "kept with the replies");
    eprintln!("two rules on the server, compiled by it; left there to deliver mail through");
}
