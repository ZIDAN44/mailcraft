//! File › Options as it reads (audit F3 to F13): one kind of control for each kind of choice,
//! switches that say On or Off, numbers as the language writes them, names that say what a
//! setting does, descriptions without a sentence's stop.

use egui::accesskit::Role;

use super::*;

const TALL: egui::Vec2 = egui::vec2(1280.0, 3000.0);

fn options(name: &str, section: crate::backstage::Section) -> (MailApp, egui::Context) {
    let (mut a, _) = app(name);
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    frame(&ctx, &mut a, vec![]);
    a.act(crate::Action::Backstage(crate::backstage::Page::Options));
    options_part(&mut a, section);
    for _ in 0..3 {
        let _ = drawn_text(&ctx, &mut a, TALL);
    }
    (a, ctx)
}

/// F4, F11, F12, F13: Mail's "marked as read" is one drop-down (radio buttons and a box for
/// seconds, it was); the counts kept are named for what they count and written as the language
/// writes numbers; a switch says On or Off beside it; a one-sentence description has no stop.
#[test]
fn mails_options_say_what_they_do() {
    let (mut a, ctx) = options("options-mail", crate::backstage::Section::Mail);
    let texts = drawn_strings(&ctx, &mut a, TALL);
    assert!(has(&texts, "Marked as read") && has(&texts, "As soon as it's shown"), "{texts:?}");
    assert!(!named(&ctx, &mut a, TALL).iter().any(|n| n.role == Role::RadioButton && n.label.as_deref() == Some("When I choose another message")));
    assert!(has(&texts, "Messages kept from each folder") && texts.iter().any(|t| t == "1,000"), "{texts:?}");
    assert!(texts.iter().any(|t| t == "On") || texts.iter().any(|t| t == "Off"), "a switch says which");
    assert!(texts.iter().any(|t| t == "A message opened in its own window (double-click) is always marked as read"), "no stop: {texts:?}");
    a.ui.language = "de".into();
    crate::i18n::set_current(crate::i18n::Lang::from_code("de").unwrap());
    let _ = drawn_text(&ctx, &mut a, TALL);
    let german = drawn_strings(&ctx, &mut a, TALL);
    a.ui.language = "en".into();
    crate::i18n::set_current(crate::i18n::Lang::EN);
    assert!(german.iter().any(|t| t == "1.000") && !german.iter().any(|t| t == "1,000"), "{german:?}");
}

/// F4: the work week's days are joined toggles, each on or off (bare words, they were).
#[test]
fn work_days_are_toggles() {
    let (mut a, ctx) = options("options-days", crate::backstage::Section::Calendar);
    let days: Vec<Named> = named(&ctx, &mut a, TALL).into_iter().filter(|n| n.role == Role::CheckBox && n.label.as_deref() == Some("Mon")).collect();
    assert_eq!(days.len(), 1, "Monday a toggle");
    assert!(days[0].selected, "worked");
    click_named(&ctx, &mut a, "Sat");
    assert!(a.session.work_time().works_on(5), "Saturday on");
}
