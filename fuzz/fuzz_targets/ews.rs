//! Exchange Web Services answers: the input is an answer to `GetInboxRules` (issue #139), read as
//! MailCraft reads an Exchange mailbox's rules on its server. Each rule MailCraft can say as its
//! own is written back as it would be sent, and that reads as the same rule again.
#![no_main]

use libfuzzer_sys::fuzz_target;
use mailcraft_net::ews;

/// What a rule does, in an order of its own (the server's order may be any).
fn doing(r: &mailcraft_model::organize::Rule) -> (Vec<String>, Vec<String>, bool) {
    let mut c: Vec<String> = r.conditions.iter().map(|c| format!("{c:?}")).collect();
    let mut a: Vec<String> = r.actions.iter().map(|a| format!("{a:?}")).collect();
    c.sort();
    c.dedup();
    a.sort();
    a.dedup();
    (c, a, r.stop)
}

fuzz_target!(|data: &[u8]| {
    let Ok(xml) = std::str::from_utf8(data) else { return };
    let Ok(rules) = ews::read_rules_answer(xml) else { return };
    for r in &rules.rules {
        let Some(rule) = &r.rule else { continue };
        let Ok(written) = ews::rule_written(rule) else { continue };
        let again = format!(
            "<GetInboxRulesResponse ResponseClass=\"Success\"><InboxRules><Rule><RuleId>x</RuleId><IsEnabled>true</IsEnabled>{written}</Rule></InboxRules></GetInboxRulesResponse>"
        );
        let back = ews::read_rules_answer(&again).expect("a rule written reads");
        let back = back.rules.first().and_then(|r| r.rule.as_ref()).expect("as MailCraft's own");
        assert_eq!(doing(back), doing(rule), "{written}");
    }
});
