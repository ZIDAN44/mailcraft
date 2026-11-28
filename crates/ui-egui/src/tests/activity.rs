//! The Activity button, its panel and the bar over the panes, where the status bar was: work under
//! way and how far it has come, what needs you, and what MailCraft said lately.

use mailcraft_engine::model::Event as MailEvent;

use super::*;

const SIZE: egui::Vec2 = egui::vec2(1280.0, 800.0);

/// Frames enough for an Area (the panel) to show: it's invisible on its first.
fn settled(ctx: &egui::Context, a: &mut MailApp) -> Vec<Drawn> {
    for _ in 0..3 {
        drawn_text(ctx, a, SIZE);
    }
    drawn_text(ctx, a, SIZE)
}

fn words(d: &[Drawn]) -> Vec<String> {
    d.iter().map(|d| d.text.clone()).collect()
}

/// Where the Activity button is, if it shows.
fn button(ctx: &egui::Context, a: &mut MailApp) -> Option<egui::Pos2> {
    named(ctx, a, SIZE).into_iter().find(|n| n.label.as_deref() == Some("Activity")).and_then(|n| n.centre())
}

/// With nothing going on there's no button and no bar; with work under way the button shows,
/// and its panel opens on a click, lists the work with how far it has come, and Esc closes it.
#[test]
fn the_button_shows_while_theres_something_and_opens_the_panel() {
    let (mut a, _) = app("activity-button");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    a.session.prefs.work_offline = false;
    settled(&ctx, &mut a);
    assert!(button(&ctx, &mut a).is_none(), "nothing going on: no button");
    assert!(!words(&settled(&ctx, &mut a)).iter().any(|t| t == "Activity"));

    // An account's mail written to a data file.
    let id = a.session.accounts[0].config.id.clone();
    a.session.run("account.exportPst", &json!({"account": id, "path": "C:/Mail/Archive.pst"})).unwrap();
    a.session.apply(MailEvent::Exported {
        account: id.clone(),
        messages: 120,
        skipped: 0,
        already: 0,
        occurrences: 0,
        replaced: 0,
        folders_skipped: 0,
        done: false,
        error: None,
    });
    settled(&ctx, &mut a);
    let at = button(&ctx, &mut a).expect("the button, while there's work");
    click(&ctx, &mut a, at);
    assert!(a.ui.activity.open);
    let texts = words(&settled(&ctx, &mut a));
    assert!(texts.iter().any(|t| t == "In progress"), "{texts:?}");
    assert!(texts.iter().any(|t| t.starts_with("Exporting ") && t.ends_with(" to Archive.pst")), "{texts:?}");
    assert!(texts.iter().any(|t| t == "120 messages so far"), "how far, said in the panel: {texts:?}");
    assert!(!a.session.notices.iter().any(|n| n.text.contains("so far")), "not a notice every few seconds");
    frame(&ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)]);
    assert!(!a.ui.activity.open, "Esc puts it away");
}

/// What needs you shows as a bar over the panes with what helps; put away, it stays away until
/// it says something else; and it's in the panel all the while.
#[test]
fn what_needs_you_is_a_bar_until_put_away() {
    let (mut a, _) = app("activity-bar");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    a.session.prefs.work_offline = false;
    let c = mailcraft_engine::model::AccountConfig::guess("zoe@studio.example");
    let id = a.session.add_account(c, None).unwrap();
    a.session.apply(MailEvent::Status { account: id.clone(), status: SyncStatus::NeedsPassword { error: String::new() } });
    // The password asked for at once is put away: the bar is what stays.
    settled(&ctx, &mut a);
    a.ui.dialog = None;
    let drawn = settled(&ctx, &mut a);
    assert!(a.ui.dialog.is_none(), "asked once");
    let ribbon = a.ui.ribbon_rect.expect("the ribbon").bottom();
    let said = drawn.iter().find(|d| d.text == "Password needed").expect("said over the panes");
    assert!(said.rect.top() > ribbon && said.rect.top() < ribbon + 40.0, "under the ribbon: {:?}", said.rect);
    assert!(words(&drawn).iter().any(|t| t == "zoe@studio.example"));
    assert!(named(&ctx, &mut a, SIZE).iter().any(|n| n.label.as_deref() == Some("Sign in")), "what helps, beside it");
    // The button's dot says something needs you, and the panel lists it.
    assert!(button(&ctx, &mut a).is_some());
    // Put away: gone from over the panes.
    click_named(&ctx, &mut a, "Dismiss");
    let drawn = settled(&ctx, &mut a);
    assert!(!drawn.iter().any(|d| d.text == "Password needed" && d.rect.top() < ribbon + 40.0), "put away");
    a.ui.activity.open = true;
    assert!(words(&settled(&ctx, &mut a)).iter().any(|t| t == "Password needed"), "still in the panel");
    a.ui.activity.open = false;
    // Said otherwise (Microsoft's sign-in now asked for): over the panes again.
    use mailcraft_engine::model::{FailureKind, MICROSOFT_NO_PASSWORDS};
    let status = SyncStatus::Error { error: MICROSOFT_NO_PASSWORDS.into(), kind: FailureKind::MicrosoftNoPasswords };
    a.session.apply(MailEvent::Status { account: id, status });
    assert!(words(&settled(&ctx, &mut a)).iter().any(|t| t == "Sign in with Microsoft"), "said again");
}

/// What MailCraft said lately stays in the panel under Recent (a notice is gone in seconds), the
/// newest first, until it's cleared.
#[test]
fn recent_notices_stay_in_the_panel_until_cleared() {
    let (mut a, _) = app("activity-recent");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    a.session.prefs.work_offline = false;
    a.session.notice("Archive.pst: 120 messages exported", false);
    a.session.notice("riley.pst couldn't be read", true);
    a.ui.activity.open = true;
    let drawn = settled(&ctx, &mut a);
    let place = |w: &str| drawn.iter().filter(|d| d.text == w).map(|d| d.rect.top()).fold(f32::INFINITY, f32::min);
    assert!(place("Recent").is_finite(), "{:?}", words(&drawn));
    assert!(place("riley.pst couldn't be read") < place("Archive.pst: 120 messages exported"), "newest first");
    click_named(&ctx, &mut a, "Clear");
    assert!(!words(&settled(&ctx, &mut a)).iter().any(|t| t == "Recent"), "cleared");
}

/// How far all the work has come: the share of each that knows, and none (the ring turning)
/// while any doesn't.
#[test]
fn the_rings_progress_is_all_the_works() {
    use crate::app::activity::{Activity, Work};
    let work = |permille| Work {
        key: String::new(),
        icon: crate::icons::Icon::File,
        title: String::new(),
        detail: String::new(),
        permille,
        action: None,
        quiet: false,
    };
    let a = Activity { needs: Vec::new(), work: vec![work(Some(200)), work(Some(600))] };
    assert_eq!(a.overall(), Some(400));
    let b = Activity { needs: Vec::new(), work: vec![work(Some(200)), work(None)] };
    assert_eq!(b.overall(), None);
    assert_eq!(Activity::default().overall(), None);
}

/// An account updating says so where it is: a ring at the end of its name in the folder pane,
/// and its words for a screen reader with the name.
#[test]
fn an_account_updating_says_so_beside_its_name() {
    let (mut a, _) = app("activity-in-place");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let id = a.session.accounts[0].config.id.clone();
    a.session.apply(MailEvent::Status { account: id, status: SyncStatus::Syncing { folder: "INBOX".into() } });
    // Once it has lasted (a few seconds of frames).
    for _ in 0..150 {
        frame(&ctx, &mut a, vec![]);
    }
    let names: Vec<String> = named(&ctx, &mut a, SIZE).into_iter().filter_map(|n| n.label).collect();
    assert!(names.iter().any(|n| n.ends_with(", Updating Inbox…")), "{names:?}");
}

/// Work going on by itself for a moment (an account checked, an index pass for a few new
/// messages) never shows: the button and its ring came and went with each, a flicker too quick
/// to see what it was. Work that lasts shows, a short pause doesn't put it out, and work the user
/// started shows at once.
#[test]
fn a_moments_work_doesnt_flicker_the_button() {
    let (mut a, _) = app("activity-flicker");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    a.session.prefs.work_offline = false;
    let id = a.session.accounts[0].config.id.clone();
    settled(&ctx, &mut a);
    // Checked, three frames at a time, again and again.
    for _ in 0..10 {
        a.session.apply(MailEvent::Status { account: id.clone(), status: SyncStatus::Syncing { folder: "INBOX".into() } });
        for _ in 0..3 {
            frame(&ctx, &mut a, vec![]);
            assert!(button(&ctx, &mut a).is_none(), "a moment's work: no button");
        }
        a.session.apply(MailEvent::Status { account: id.clone(), status: SyncStatus::Online { at: NOW } });
        for _ in 0..60 {
            frame(&ctx, &mut a, vec![]);
        }
    }

    // The same, kept up: shown once it has lasted, a pause of a few frames not ending it; and
    // once it's done, there a moment more, then gone.
    use crate::app::activity::{Held, SHOW_AFTER};
    let step = 1.0 / 60.0;
    let mut held = None;
    let mut first = None;
    for n in 0..300 {
        let now = f64::from(n) * step;
        let going = n < 200 && !(150..155).contains(&n);
        held = Held::note(held, now, going, true);
        if held.is_some_and(|h| h.shown()) {
            first.get_or_insert(now);
        } else if first.is_some() && now < 200.0 * step {
            panic!("put out at {now} by a pause of a few frames");
        }
    }
    let first = first.expect("shown once it lasted");
    assert!((SHOW_AFTER..SHOW_AFTER + 0.05).contains(&first), "{first}");
    assert!(held.is_none(), "gone a moment after it's done");
    let done = Held::note(Held::note(None, 0.0, true, false), 0.5, false, false).expect("there a moment after");
    assert!(done.shown() && !done.going(0.5));
    // Asked for: at once.
    assert!(Held::note(None, 10.0, true, false).is_some_and(|h| h.shown()));
}

/// With the panel open, work going on by itself for a moment (an account checked, again and
/// again) never comes up in it: its line came for a second and went, the panel saying nothing
/// was going on between. Work that lasts comes up, and stays a moment once it's done.
#[test]
fn a_moments_work_doesnt_flicker_in_the_panel() {
    let (mut a, _) = app("activity-panel-flicker");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    a.session.prefs.work_offline = false;
    let id = a.session.accounts[0].config.id.clone();
    a.ui.activity.open = true;
    let check = |a: &mut MailApp, on: bool| {
        let status = if on { SyncStatus::Syncing { folder: "INBOX".into() } } else { SyncStatus::Online { at: NOW } };
        a.session.apply(MailEvent::Status { account: id.clone(), status });
    };
    let listed = |ctx: &egui::Context, a: &mut MailApp| words(&drawn_text(ctx, a, SIZE)).iter().any(|t| t == "In progress");
    settled(&ctx, &mut a);
    // Checked for a second at a time, half a second apart (a frame is a sixtieth).
    for _ in 0..4 {
        check(&mut a, true);
        for _ in 0..60 {
            assert!(!listed(&ctx, &mut a), "a moment's work: not in the panel");
        }
        check(&mut a, false);
        for _ in 0..30 {
            assert!(!listed(&ctx, &mut a));
        }
    }
    // Kept up: in the panel once it has lasted, and a moment after it's done.
    check(&mut a, true);
    let mut came = None;
    for n in 0..180 {
        if listed(&ctx, &mut a) {
            came.get_or_insert(n);
        }
    }
    assert!(came.is_some_and(|n| n >= 110), "once it lasted: {came:?}");
    check(&mut a, false);
    for _ in 0..30 {
        assert!(listed(&ctx, &mut a), "there a moment after it's done");
    }
    for _ in 0..60 {
        drawn_text(&ctx, &mut a, SIZE);
    }
    assert!(!listed(&ctx, &mut a), "then gone");
}

/// A turning ring moves with the time itself, so each frame it's drawn somewhere new (it moved
/// ten times a second, in steps, and jerked round), and comes round again.
#[test]
fn a_turning_ring_moves_smoothly() {
    use crate::progress::turn_at;
    let frames: Vec<f32> = (0..6).map(|n| turn_at(10.0 + f64::from(n) / 60.0)).collect();
    assert!(frames.windows(2).all(|w| w[1] > w[0]), "a little further each frame: {frames:?}");
    assert!((turn_at(1.2) - turn_at(0.0)).abs() < 1e-4 || turn_at(1.2) < 0.01, "round again");
    assert_eq!((turn_at(f64::NAN), turn_at(-1.0)), (0.0, 0.0));
}
