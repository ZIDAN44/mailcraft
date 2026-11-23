//! UI tests: an organisation's profile in the window (issue #245). A locked setting is greyed
//! out and said to be the organisation's, and clicking it changes nothing; a locked theme and
//! language hold; File › Info says what the profile sets and takes the person's details for its
//! signature; File › About names the organisation beside MailCraft.

use mailcraft_engine::policy::Organisation;
use serde_json::Value;

use crate::Action;

use super::*;

fn org(v: Value) -> Organisation {
    Organisation::from_sources(vec![("C:\\ProgramData\\MailCraft\\policies.json".into(), v, true)])
}

fn page(ctx: &egui::Context, a: &mut MailApp, p: crate::backstage::Page) -> Vec<String> {
    a.ui.backstage = Some(crate::backstage::Backstage::new(p));
    for _ in 0..2 {
        frame(ctx, a, vec![]);
    }
    drawn_strings(ctx, a, egui::vec2(1280.0, 8000.0))
}

#[test]
fn a_locked_setting_is_greyed_out_and_holds() {
    let (mut a, _) = app("org-options");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let _tall = super::windows::layout::Dressed::new(egui::vec2(1280.0, 8000.0), "");
    a.set_organisation(
        org(json!({"settings": {"plainTopBar": {"value": true, "locked": true}, "accent": {"value": "#0055aa", "locked": true}},
                   "theme": {"value": "dark", "locked": true}, "updates": {"value": "off", "locked": true}})),
        None,
    );
    assert_eq!(a.ui.dark, Some(true));
    let texts = page(&ctx, &mut a, crate::backstage::Page::Options);
    let said = texts.iter().filter(|t| t.contains("Set by your organisation")).count();
    assert!(said >= 3, "plain top bar, accent, theme, updates: {texts:?}");
    // Its switch can't be clicked to change it.
    let nodes = super::named(&ctx, &mut a, egui::vec2(1280.0, 8000.0));
    let plain = nodes.iter().find(|n| n.label.as_deref() == Some("Plain top bar") && n.role == egui::accesskit::Role::CheckBox).expect("switch");
    assert!(plain.disabled, "{plain:?}");
    a.act(Action::SetDark(Some(false)));
    assert_eq!(a.ui.dark, Some(true), "the theme stays as locked");
    assert!(a.session.run("view.plainTopBar", &json!({"on": false})).is_err());
    assert!(a.session.prefs.plain_top_bar);
    assert!(a.session.run("app.checkUpdates", &json!({})).is_err(), "updates are the organisation's");
}

#[test]
fn file_info_says_what_the_profile_sets_and_takes_the_details() {
    let (mut a, _) = app("org-info");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let _tall = super::windows::layout::Dressed::new(egui::vec2(1280.0, 8000.0), "");
    a.set_organisation(
        org(json!({"organisation": "Contoso", "contact": "Help desk, extension 4000",
                   "settings": {"autoPictures": {"value": false, "locked": true}, "nonsense": 1},
                   "requireEncryption": true,
                   "signature": {"name": "Contoso", "text": "{name}\n{title}"}})),
        None,
    );
    let texts = page(&ctx, &mut a, crate::backstage::Page::Info);
    for want in ["Contoso", "Help desk, extension 4000", "Encrypted connections only", "Your details for its signature", "Job title"] {
        assert!(texts.iter().any(|t| t.contains(want)), "{want}: {texts:?}");
    }
    assert!(texts.iter().any(|t| t.contains("nonsense")), "what wasn't taken is said");
    // The details, typed and saved: the signature takes them.
    let nodes = super::named(&ctx, &mut a, egui::vec2(1280.0, 8000.0));
    let title = nodes
        .iter()
        .find(|n| n.label.as_deref() == Some("Job title") && n.role != egui::accesskit::Role::Label)
        .and_then(super::Named::centre)
        .unwrap();
    super::click(&ctx, &mut a, title);
    frame(&ctx, &mut a, vec![Event::Text("Producer".into())]);
    frame(&ctx, &mut a, vec![]);
    click_named(&ctx, &mut a, "Save");
    frame(&ctx, &mut a, vec![]);
    let c = &a.session.account("sample").unwrap().config;
    assert_eq!(c.details.get("title").map(String::as_str), Some("Producer"));
    assert!(c.signature_named("Contoso").is_some_and(|g| g.text.contains("Producer")), "{:?}", c.signatures);
    // About names the organisation beside MailCraft.
    let texts = page(&ctx, &mut a, crate::backstage::Page::About);
    assert!(texts.iter().any(|t| t == "Set up for Contoso") && texts.iter().any(|t| t.starts_with("MailCraft ")), "{texts:?}");
}

/// A domain's profile that locks the theme and language holds them as soon as an account of the
/// domain is added, not from the next start.
#[test]
fn a_domains_locked_look_holds_once_its_account_comes() {
    let (mut a, _) = app("org-domain-look");
    let ctx = egui::Context::default();
    a.ui.dark = Some(false);
    a.set_organisation(
        org(json!({"profiles": [{"domains": ["lab.example"], "theme": {"value": "dark", "locked": true}, "language": {"value": "de", "locked": true}}]})),
        None,
    );
    a.logic(&ctx);
    assert!(a.ui.dark == Some(false) && a.ui.language != "de", "no account of it yet");
    a.session.run("account.add", &json!({"email": "riley@lab.example", "password": "pw"})).unwrap();
    a.logic(&ctx);
    assert_eq!((a.ui.dark, a.ui.language.as_str()), (Some(true), "de"));
}

#[test]
fn a_profile_refused_is_said() {
    let (mut a, _) = app("org-refused");
    let ctx = egui::Context::default();
    let why = "C:\\ProgramData\\MailCraft\\policies.json: refused, as it can be changed by people who aren't administrators";
    a.set_organisation(Organisation { ignored: vec![why.into()], ..Default::default() }, None);
    let texts = page(&ctx, &mut a, crate::backstage::Page::Info);
    assert!(texts.iter().any(|t| t.contains(why)), "{texts:?}");
    assert!(!a.session.prefs.plain_top_bar && a.session.org.held.is_empty(), "nothing of it taken");
}

#[test]
fn without_a_profile_nothing_says_one() {
    let (mut a, _) = app("org-none");
    let ctx = egui::Context::default();
    a.set_organisation(Organisation::default(), None);
    let texts = page(&ctx, &mut a, crate::backstage::Page::Info);
    assert!(!texts.iter().any(|t| t.contains("Your organisation") || t.contains("Set by your organisation")), "{texts:?}");
    let texts = page(&ctx, &mut a, crate::backstage::Page::Options);
    assert!(!texts.iter().any(|t| t.contains("Set by your organisation")));
}
