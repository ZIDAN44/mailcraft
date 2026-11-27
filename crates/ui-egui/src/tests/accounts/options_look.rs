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

/// F7: Info has the everyday cards first, S/MIME and OpenPGP in one Security part that opens.
#[test]
fn info_puts_the_everyday_first() {
    let (mut a, _) = app("info-order");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    frame(&ctx, &mut a, vec![]);
    a.act(crate::Action::Backstage(crate::backstage::Page::Info));
    for _ in 0..3 {
        let _ = drawn_text(&ctx, &mut a, TALL);
    }
    let texts = drawn_text(&ctx, &mut a, TALL);
    let y = |w: &str| texts.iter().find(|d| d.text == w).map(|d| d.rect.top());
    assert!(y("Automatic replies").zip(y("Security")).is_some_and(|(r, s)| r < s), "replies over security");
    assert!(y("Import certificate…").is_none(), "closed");
    // Clicked in a window this tall (it's under the usual one's edge).
    let at = named(&ctx, &mut a, TALL).into_iter().find(|n| n.label.as_deref() == Some("Security")).and_then(|n| n.centre()).expect("Security");
    let press = |pressed| Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    for events in [vec![Event::PointerMoved(at)], vec![press(true)], vec![press(false)]] {
        let _ = sized_frame(&ctx, &mut a, TALL, events);
    }
    let _ = drawn_text(&ctx, &mut a, TALL);
    let after = drawn_strings(&ctx, &mut a, TALL);
    assert!(after.iter().any(|t| t == "Import certificate…"), "opened: {after:?}");
}

/// F8 and F9: Open & Export's buttons say what they open, a greyed Export says why; a blocked
/// sender mistyped is said under its box, not added.
#[test]
fn file_pages_say_what_and_why() {
    let (mut a, ctx) = options("file-pages", crate::backstage::Section::Safety);
    let before = a.session.prefs.blocked_senders.len();
    // What's typed in Blocked senders' box.
    ctx.data_mut(|d| d.insert_temp(egui::Id::unique(("junk-add", "blocked")), "not an address".to_string()));
    let _ = drawn_text(&ctx, &mut a, TALL);
    let texts = drawn_strings(&ctx, &mut a, TALL);
    assert!(has(&texts, "Type an address (name@example.com) or a domain (@example.com)."), "said under it");
    assert_eq!(a.session.prefs.blocked_senders.len(), before, "not added");
    a.act(crate::Action::Backstage(crate::backstage::Page::Open));
    let _ = drawn_text(&ctx, &mut a, TALL);
    let texts = drawn_strings(&ctx, &mut a, TALL);
    assert!(has(&texts, "Open data file…") && has(&texts, "Open message file…"), "{texts:?}");
    assert!(has(&texts, "No rules yet: make one in Rules, on the Home tab"), "why Export is off");
}
