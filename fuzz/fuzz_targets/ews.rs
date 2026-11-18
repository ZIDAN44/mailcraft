//! Exchange Web Services answers: the input is an answer to `GetInboxRules` (issue #139), read as
//! MailCraft reads an Exchange mailbox's rules on its server. Each rule MailCraft can say as its
//! own is written back as it would be sent, and that reads as the same rule again. The same input
//! is read as an answer to `GetUserAvailability` too (issue #140): an answer for each person asked
//! about, in order, each time taken ending no sooner than it starts; and as an answer to
//! `GetEvents` (issue #145): the folders it names, each once, never empty, no more than are kept.
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
    let people = ["a@x.example".to_string(), "b@x.example".to_string(), "c@x.example".to_string()];
    let answers = ews::availability_answers(&ews::parse(xml), &people);
    assert_eq!(answers.len(), people.len());
    for (a, p) in answers.iter().zip(&people) {
        assert_eq!(&a.email, p);
        assert!(a.busy.is_empty() || a.error.is_none());
        assert!(a.busy.len() <= 5000);
        for b in &a.busy {
            assert!(b.end >= b.start);
            assert!(b.subject.chars().count() <= 256 && b.location.chars().count() <= 256);
        }
    }
    if let Ok(got) = ews::watch_events(&ews::parse(xml)) {
        assert!(got.folders.len() <= ews::WATCH_CHANGED_MAX);
        for (i, f) in got.folders.iter().enumerate() {
            assert!(!f.is_empty() && f.len() <= 1024 && !got.folders[..i].contains(f), "{f:?}");
        }
    }
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
