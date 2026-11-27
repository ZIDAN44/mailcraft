//! The Rules list: what it's worked with, where; and what it says with no rules yet.

use super::*;

/// Audit D18: the Rules list has New rule, Run all, Import and Export over it and Close alone
/// under it (Close was on a row of its own under them); with no rules yet it says what a rule is
/// for where the list would be ("No rules yet." alone sat over an empty line).
#[test]
fn the_rules_list_is_worked_from_above_and_says_what_a_rule_is_for() {
    let (mut a, _) = app("rules-tidy");
    let ctx = egui::Context::default();
    let size = egui::vec2(1280.0, 800.0);
    assert!(a.session.rules.is_empty(), "the sample has none");
    a.ui.dialog = Some(Dialog::Rules);
    for _ in 0..4 {
        let _ = drawn_text(&ctx, &mut a, size);
    }
    let texts = drawn_text(&ctx, &mut a, size);
    let at = |what: &str| texts.iter().find(|d| d.text == what).map(|d| d.rect).unwrap_or_else(|| panic!("no {what}"));
    let (new, empty, close) = (at("New rule…"), at("No rules yet."), at("Close"));
    assert!(new.bottom() < empty.top(), "New rule over the list");
    for over in ["Run all", "Import…", "Export…"] {
        assert!((at(over).center().y - new.center().y).abs() < 1.0, "{over} beside New rule");
    }
    assert!(texts.iter().any(|d| d.text.starts_with("A rule files, flags or answers mail")), "what a rule is for");
    assert!(close.top() > empty.bottom(), "Close under it");
}
