//! Issue #274: Archive among a row's buttons, as chosen in File › Options; Snooze beside a
//! message's Reply and Forward; and Snooze to a time of the user's own.

use super::*;
use egui::accesskit::Role;

/// The list's rows, top to bottom: what a screen reader calls each, and where.
fn rows(ctx: &egui::Context, a: &mut MailApp) -> Vec<(String, egui::Rect)> {
    let mut rows: Vec<(String, egui::Rect)> = named(ctx, a, egui::vec2(1280.0, 800.0))
        .into_iter()
        .filter(|n| n.role == Role::ListItem)
        .filter_map(|n| Some((n.label.unwrap_or_default(), n.rect?)))
        .collect();
    rows.sort_by(|x, y| x.1.min.y.total_cmp(&y.1.min.y));
    rows
}

/// The buttons the third row shows under the pointer, by name, and where.
fn third_rows_buttons(ctx: &egui::Context, a: &mut MailApp) -> Vec<(String, egui::Pos2)> {
    let third = rows(ctx, a)[2].1;
    frame(ctx, a, vec![Event::PointerMoved(third.center())]);
    let nodes = named(ctx, a, egui::vec2(1280.0, 800.0));
    nodes
        .iter()
        .filter(|n| n.role == Role::Button)
        .filter_map(|n| Some((n.label.clone()?, n.centre()?)))
        .filter(|(_, c)| third.contains(*c))
        .collect()
}

/// A row shows the buttons chosen, Delete, Flag and Mark read until then; Archive and Pin act on
/// that message alone, as the others do (issue #216).
#[test]
fn a_rows_buttons_are_the_ones_chosen() {
    let (mut a, _) = app("row-buttons-chosen");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![key(Key::Home, Modifiers::NONE)]);
    let selected = a.session.selection.clone();
    let names = |b: &[(String, egui::Pos2)]| b.iter().map(|(n, _)| n.clone()).collect::<Vec<_>>();
    let shown = names(&third_rows_buttons(&ctx, &mut a));
    for want in ["Delete", "Flag"] {
        assert!(shown.iter().any(|n| n == want), "{want} at first: {shown:?}");
    }
    assert!(!shown.iter().any(|n| n == "Archive"), "{shown:?}");

    a.run("options.rowActions", json!({"actions": ["delete", "archive", "pin"]}));
    let target = a.session.row_keys()[2].clone();
    let buttons = third_rows_buttons(&ctx, &mut a);
    let shown = names(&buttons);
    for want in ["Delete", "Archive", "Pin"] {
        assert!(shown.iter().any(|n| n == want), "{want}: {shown:?}");
    }
    assert!(!shown.iter().any(|n| n == "Flag" || n.starts_with("Mark ")), "only those chosen: {shown:?}");
    let at = |b: &[(String, egui::Pos2)], name: &str| b.iter().find(|(n, _)| n == name).map(|(_, c)| *c).unwrap();
    click(&ctx, &mut a, at(&buttons, "Pin"));
    frame(&ctx, &mut a, vec![]);
    assert!(a.session.summary(&target).unwrap().flags.pinned(), "that message pinned");
    assert_eq!(a.session.selection, selected, "the selection stays");

    // Pinned, it's at the top: Archive the third row as it is now.
    let target = a.session.row_keys()[2].clone();
    let subject = a.session.summary(&target).unwrap().subject.clone();
    let buttons = third_rows_buttons(&ctx, &mut a);
    click(&ctx, &mut a, at(&buttons, "Archive"));
    frame(&ctx, &mut a, vec![]);
    assert!(a.session.summary(&target).is_none(), "out of the Inbox");
    let archive = a.session.account("sample").unwrap().folder(mailcraft_engine::model::FolderRole::Archive).unwrap().path.clone();
    assert!(a.session.folder_state("sample", &archive).messages.iter().any(|m| m.subject == subject), "into the Archive");
    assert_eq!(a.session.selection, selected, "the selection stays");
}

/// Snooze is among the message's own buttons; its menu's Choose a date… takes a day and time of
/// the user's own, and won't take one past.
#[test]
fn a_message_is_snoozed_from_beside_it_to_a_time_chosen() {
    let (mut a, _) = app("snooze-chosen");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let size = egui::vec2(1280.0, 800.0);
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![key(Key::Home, Modifiers::NONE)]);
    let open = a.session.reading.as_ref().map(|r| r.key.clone()).expect("a message open");
    // The ribbon has a Snooze too: the message's is the lowest.
    let nodes = named(&ctx, &mut a, size);
    let snooze = nodes.iter().filter(|n| n.label.as_deref() == Some("Snooze")).filter_map(Named::centre).max_by(|x, y| x.y.total_cmp(&y.y));
    let snooze = snooze.expect("Snooze beside the message");
    assert!(snooze.y > 150.0, "below the ribbon: {snooze:?}");
    click(&ctx, &mut a, snooze);
    frame(&ctx, &mut a, vec![]);
    click_named(&ctx, &mut a, "Choose a date…");
    frame(&ctx, &mut a, vec![]);
    let Some(Dialog::Snooze(f)) = &mut a.ui.dialog else { panic!("the form") };
    assert_eq!(f.subject, a.session.summary(&open).unwrap().subject);
    // Begun at the menu's Tomorrow, eight in the morning.
    let tomorrow = mailcraft_engine::model::dates::day_string(NOW.div_euclid(86_400) + 1);
    assert_eq!((f.date.as_str(), f.time.as_str()), (tomorrow.as_str(), "08:00"));
    f.date = mailcraft_engine::model::dates::day_string(NOW.div_euclid(86_400) - 1);
    frame(&ctx, &mut a, vec![]);
    let texts = drawn_strings(&ctx, &mut a, size);
    assert!(has(&texts, "Pick a time in the future."), "a day past isn't taken");
    let snooze_button = |a: &mut MailApp| drawn_text(&ctx, a, size).into_iter().rev().find(|d| d.text == "Snooze").map(|d| d.rect.center()).unwrap();
    let at = snooze_button(&mut a);
    click(&ctx, &mut a, at);
    frame(&ctx, &mut a, vec![]);
    assert!(a.session.snoozes.is_empty() && a.ui.dialog.is_some(), "nor snoozed to");

    let Some(Dialog::Snooze(f)) = &mut a.ui.dialog else { panic!("the form") };
    f.date = "2026-10-20".into();
    f.time = "17:45".into();
    frame(&ctx, &mut a, vec![]);
    let at = snooze_button(&mut a);
    click(&ctx, &mut a, at);
    frame(&ctx, &mut a, vec![]);
    assert!(a.ui.dialog.is_none(), "the form closed");
    let day = mailcraft_engine::model::dates::days_from_civil(2026, 10, 20);
    assert_eq!(a.session.snoozes.iter().map(|z| z.until).collect::<Vec<_>>(), [day * 86_400 + 17 * 3600 + 45 * 60]);
    assert!(a.session.summary(&open).is_none(), "out of the Inbox until then");
}
