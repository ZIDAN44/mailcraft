//! Issue #139 against the fake EWS server: the mailbox's Inbox rules on the server, read through
//! the account's worker as MailCraft's own rules where they can be, and changed without losing
//! what MailCraft can't say of the others.

use mailcraft_model::organize::{Condition, Rule, RuleAction, ServerRule, ServerRuleOp, ServerRules};

use super::*;

/// A rule as Outlook could make it: a subject's word, moved to `folder`, stopping there.
fn to_folder(name: &str, word: &str, folder: &str) -> String {
    format!(
        "<t:Rule xmlns:t=\"t\"><t:DisplayName>{name}</t:DisplayName><t:Priority>1</t:Priority><t:IsEnabled>true</t:IsEnabled>\
         <t:Conditions><t:ContainsSubjectStrings><t:String>{word}</t:String></t:ContainsSubjectStrings></t:Conditions>\
         <t:Actions><t:MoveToFolder><t:FolderId Id=\"{folder}\" ChangeKey=\"ck\"/></t:MoveToFolder><t:StopProcessingRules>true</t:StopProcessingRules></t:Actions></t:Rule>"
    )
}

/// One MailCraft can't say: forwarding, with an exception.
const FORWARD: &str = "<t:Rule xmlns:t=\"t\"><t:DisplayName>To my phone</t:DisplayName><t:Priority>2</t:Priority><t:IsEnabled>true</t:IsEnabled>\
     <t:Conditions><t:ContainsSenderStrings><t:String>boss</t:String></t:ContainsSenderStrings></t:Conditions>\
     <t:Exceptions><t:ContainsSubjectStrings><t:String>lunch</t:String></t:ContainsSubjectStrings></t:Exceptions>\
     <t:Actions><t:ForwardToRecipients><t:Address><t:EmailAddress>me@phone.example</t:EmailAddress></t:Address></t:ForwardToRecipients></t:Actions></t:Rule>";

struct Mailbox {
    pool: crate::worker::Pool,
    data: Arc<Mutex<Data>>,
    _server: Server,
}

/// A mailbox with an Inbox and a Projects folder, a rule moving to it and one forwarding, its
/// account online.
fn mailbox(name: &str, outlook_list: bool) -> Mailbox {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let projects = d.folder("Projects", None, None, "IPF.Note");
    d.add_rule(&to_folder("Invoices", "invoice", &projects));
    d.add_rule(FORWARD);
    d.outlook_rules = outlook_list;
    let server = serve(d);
    let data = server.data.clone();
    let store = Store::open(crate::data_files::scratch::scratch(&format!("ews-rules-{name}"))).unwrap();
    let mut pool = crate::worker::Pool::new(store, Arc::new(crate::worker::MemoryCredentials::default()));
    pool.submit(Job::Start { account: account("a1", &server.url), password: Some(Secret(PASSWORD.into())) });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    Mailbox { pool, data, _server: server }
}

/// What the worker says of the rules after `job`: the rules, and why a change failed.
fn said(pool: &mut crate::worker::Pool, job: Job) -> (Option<ServerRules>, Option<String>) {
    pool.submit(job);
    let seen = until(pool, "the rules", |e| matches!(e, Event::ServerRules { .. }));
    match seen.into_iter().last() {
        Some(Event::ServerRules { rules, error, .. }) => (rules, error),
        other => panic!("{other:?}"),
    }
}

fn read(m: &mut Mailbox) -> ServerRules {
    let (rules, error) = said(&mut m.pool, Job::GetServerRules { account: "a1".into() });
    assert_eq!(error, None);
    rules.expect("the rules")
}

fn set(m: &mut Mailbox, ops: Vec<ServerRuleOp>, drop_outlook_list: bool) -> (Option<ServerRules>, Option<String>) {
    said(&mut m.pool, Job::SetServerRules { account: "a1".into(), ops, drop_outlook_list })
}

/// The rules come in order, one MailCraft can say as its own (its folder by its path here), and
/// one it can't (forwarding, an exception), kept as it is.
#[test]
fn the_rules_on_the_server_are_read_as_mailcraft_can_say_them() {
    let mut m = mailbox("read", false);
    let rules = read(&mut m);
    assert!(!rules.outlook_list);
    let [invoices, forward] = rules.rules.as_slice() else { panic!("{rules:?}") };
    assert_eq!((invoices.name.as_str(), invoices.priority, invoices.enabled), ("Invoices", 1, true));
    let rule = invoices.rule.clone().expect("said as MailCraft's own");
    assert_eq!(rule.conditions, [Condition::Subject("invoice".into())]);
    assert_eq!(rule.actions, [RuleAction::MoveTo("Projects".into())]);
    assert!(rule.stop);
    assert_eq!(forward.name, "To my phone");
    assert_eq!(forward.rule, None, "forwarding and an exception: kept as it is");
}

/// A new rule made here goes to the server with its folder's id; one changed here is changed
/// there; one MailCraft can't say, turned off and renamed, keeps all it does byte for byte;
/// one deleted goes.
#[test]
fn rules_made_and_changed_here_go_to_the_server() {
    let mut m = mailbox("change", false);
    let rules = read(&mut m);
    let projects = m.data.lock().unwrap().by_name("Projects").map(|f| f.id.clone()).unwrap();
    let new = Rule {
        name: "Receipts".into(),
        enabled: true,
        account: None,
        conditions: vec![Condition::From("shop".into()), Condition::HasAttachment],
        match_all: true,
        actions: vec![RuleAction::MarkRead, RuleAction::MoveTo("Projects".into())],
        stop: false,
        sent: false,
    };
    let now = ServerRule { name: "Receipts".into(), priority: 3, enabled: true, rule: Some(new.clone()), ..Default::default() };
    let (after, error) = set(&mut m, vec![ServerRuleOp::Create { now }], false);
    assert_eq!(error, None);
    let after = after.unwrap();
    let made = after.rules.iter().find(|r| r.name == "Receipts").expect("made");
    assert_eq!(made.rule.as_ref().map(|r| (&r.conditions, &r.actions)), Some((&new.conditions, &new.actions)));
    assert!(m.data.lock().unwrap().rules.iter().any(|r| r.write("t").contains(&format!("Id=\"{projects}\""))), "moved by its folder's id");

    // Changed here: what it looks for.
    let was = after.rules.iter().find(|r| r.name == "Invoices").cloned().unwrap();
    let mut changed = was.clone();
    if let Some(r) = changed.rule.as_mut() {
        r.conditions = vec![Condition::Subject("bill".into())];
    }
    let (after, error) = set(&mut m, vec![ServerRuleOp::Set { was, now: changed }], false);
    assert_eq!(error, None);
    let after = after.unwrap();
    let invoices = after.rules.iter().find(|r| r.name == "Invoices").unwrap();
    assert_eq!(invoices.rule.as_ref().map(|r| r.conditions.clone()), Some(vec![Condition::Subject("bill".into())]));

    // One MailCraft can't say: off and renamed, all it does kept as it came.
    let was = rules.rules.iter().find(|r| r.name == "To my phone").cloned().unwrap();
    let actions_before =
        m.data.lock().unwrap().rules.iter().find(|r| r.text_of("DisplayName") == Some("To my phone")).unwrap().child("Actions").unwrap().write("t");
    let now = ServerRule { name: "To my old phone".into(), enabled: false, ..was.clone() };
    let (after, error) = set(&mut m, vec![ServerRuleOp::Set { was, now }], false);
    assert_eq!(error, None);
    let kept = after.unwrap().rules.into_iter().find(|r| r.name == "To my old phone").expect("renamed");
    assert!(!kept.enabled);
    let d = m.data.lock().unwrap();
    let on_server = d.rules.iter().find(|r| r.text_of("DisplayName") == Some("To my old phone")).unwrap();
    assert_eq!(on_server.child("Actions").unwrap().write("t"), actions_before, "forwarding kept as it was");
    assert!(on_server.child("Exceptions").is_some_and(|e| !e.kids.is_empty()), "the exception kept");
    drop(d);

    let (after, error) = set(&mut m, vec![ServerRuleOp::Delete { was: kept }], false);
    assert_eq!(error, None);
    assert!(after.unwrap().rules.iter().all(|r| r.name != "To my old phone"));
}

/// Outlook's own copy of the rules beside the server's: a change isn't made until it's asked to
/// remove it, and the rules as they are come back with why; then it's made.
#[test]
fn outlooks_own_list_of_rules_is_removed_only_when_asked() {
    let mut m = mailbox("outlook", true);
    let rules = read(&mut m);
    assert!(rules.outlook_list);
    let was = rules.rules[0].clone();
    let now = ServerRule { enabled: false, ..was.clone() };
    let (back, error) = set(&mut m, vec![ServerRuleOp::Set { was: was.clone(), now: now.clone() }], false);
    assert!(error.is_some_and(|e| e.contains("Outlook")), "refused");
    assert!(back.is_some_and(|b| b.rules[0].enabled), "the rules as they are, unchanged");
    let (back, error) = set(&mut m, vec![ServerRuleOp::Set { was, now }], true);
    assert_eq!(error, None);
    let back = back.unwrap();
    assert!(!back.outlook_list && !back.rules[0].enabled);
}

/// Rules saved again by another program (Outlook gives each a new id) since they were read: a
/// change made with the old ones isn't made, and the rules come back as they are now.
#[test]
fn a_change_to_rules_changed_elsewhere_is_not_made() {
    let mut m = mailbox("elsewhere", false);
    let rules = read(&mut m);
    m.data.lock().unwrap().rules_saved_elsewhere();
    let was = rules.rules[0].clone();
    let (back, error) = set(&mut m, vec![ServerRuleOp::Delete { was }], false);
    assert!(error.is_some_and(|e| e.contains("another program")), "refused");
    let back = back.unwrap();
    assert_eq!(back.rules.len(), 2, "nothing deleted");
    assert!(back.rules.iter().all(|r| !rules.rules.iter().any(|old| old.id == r.id)), "read again, with the new ids");
    assert_eq!(m.data.lock().unwrap().rule_updates, 0);
}

/// A rule MailCraft can't say on a server can't be made from one that asks for it: refused
/// before anything is sent.
#[test]
fn a_rule_the_server_cant_hold_is_refused() {
    let mut m = mailbox("refused", false);
    let bulk = Rule {
        name: "Lists".into(),
        enabled: true,
        account: None,
        conditions: vec![Condition::Bulk],
        match_all: true,
        actions: vec![RuleAction::Flag],
        stop: false,
        sent: false,
    };
    let now = ServerRule { name: "Lists".into(), priority: 3, enabled: true, rule: Some(bulk), ..Default::default() };
    let (back, error) = set(&mut m, vec![ServerRuleOp::Create { now }], false);
    assert!(error.is_some_and(|e| e.contains("can't")), "refused");
    assert_eq!(back.map(|b| b.rules.len()), Some(2));
    assert_eq!(m.data.lock().unwrap().rule_updates, 0);
}

/// The fuzz target's seed: its rules read (one said as MailCraft's own, one forwarding and with
/// an exception kept as it is), and the one MailCraft can say, written as it would be sent and
/// read back, is the same rule.
#[test]
fn a_rule_written_reads_back_the_same() {
    let seed = include_str!("../../../../../../fuzz/seeds/ews/rules.xml");
    let rules = crate::ews::read_rules_answer(seed).unwrap();
    assert!(rules.outlook_list);
    let [mine, theirs] = rules.rules.as_slice() else { panic!("{rules:?}") };
    assert!(theirs.rule.is_none() && theirs.in_error && !theirs.enabled);
    let rule = mine.rule.clone().expect("said as MailCraft's own");
    assert_eq!(rule.actions, [RuleAction::Categorize("Bills".into()), RuleAction::MarkRead, RuleAction::MoveTo("AAMk".into())]);
    let written = crate::ews::rule_written(&rule).unwrap();
    let again = format!(
        "<GetInboxRulesResponse ResponseClass=\"Success\"><InboxRules><Rule><RuleId>x</RuleId><IsEnabled>true</IsEnabled>{written}</Rule></InboxRules></GetInboxRulesResponse>"
    );
    let back = crate::ews::read_rules_answer(&again).unwrap();
    let back = back.rules[0].rule.clone().unwrap();
    assert_eq!((&back.conditions, &back.actions, back.stop), (&rule.conditions, &rule.actions, rule.stop));
    // Two folders or two categories aren't one rule on a server: refused, not one of them lost.
    let two = Rule { actions: vec![RuleAction::MoveTo("a".into()), RuleAction::MoveTo("b".into())], ..rule };
    assert!(crate::ews::rule_written(&two).is_err());
}
