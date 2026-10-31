//! UI tests: selecting several messages from the keyboard and with the mouse (issue #26).

use egui::accesskit::Role;

use super::*;

/// The rows of the message list as a screen reader is told of them: what each says, and where.
fn list_rows(ctx: &egui::Context, a: &mut MailApp) -> Vec<(String, egui::Rect)> {
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
    let tree = out.platform_output.accesskit_update.expect("AccessKit is on");
    let mut rows: Vec<(String, egui::Rect)> = tree
        .nodes
        .iter()
        .filter(|(_, n)| n.role() == Role::ListItem)
        .filter_map(|(_, n)| {
            let b = n.bounds()?;
            Some((
                n.label().unwrap_or_default().to_string(),
                egui::Rect::from_min_max(egui::pos2(b.x0 as f32, b.y0 as f32), egui::pos2(b.x1 as f32, b.y1 as f32)),
            ))
        })
        .collect();
    rows.sort_by(|x, y| x.1.min.y.total_cmp(&y.1.min.y));
    rows
}

/// Whether some text drawn says `what` (a label, as a screen reader reads it).
fn says(ctx: &egui::Context, a: &mut MailApp, what: &str) -> bool {
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
    let tree = out.platform_output.accesskit_update.expect("AccessKit is on");
    tree.nodes.iter().any(|(_, n)| n.label().or(n.value()).is_some_and(|l| l.contains(what)))
}

/// A click with keys held (held for the whole of it, as the input's modifiers say, not only on
/// the pointer's events).
fn click(ctx: &egui::Context, a: &mut MailApp, at: egui::Pos2, button: egui::PointerButton, modifiers: Modifiers) {
    frame(ctx, a, vec![Event::ModifiersChanged(modifiers), Event::PointerMoved(at)]);
    frame(ctx, a, vec![Event::PointerButton { pos: at, button, pressed: true, modifiers }]);
    frame(ctx, a, vec![Event::PointerButton { pos: at, button, pressed: false, modifiers }]);
    frame(ctx, a, vec![Event::ModifiersChanged(Modifiers::NONE)]);
}

fn subjects(a: &MailApp) -> Vec<String> {
    let rows = a.session.row_keys();
    let mut sel: Vec<&MessageKey> = a.session.selection.iter().collect();
    sel.sort_by_key(|k| rows.iter().position(|r| r == *k));
    sel.iter().filter_map(|k| a.session.summary(k)).map(|m| m.subject.clone()).collect()
}

/// Shift+Down and Shift+Up select from the keyboard, and the reading pane says how many are
/// selected; Shift+End to the last; Page Down goes as many rows as the list shows; in a text
/// box Shift+Down selects text, not mail.
#[test]
fn shift_arrows_select_from_the_keyboard() {
    let (mut a, dir) = app("shift-keys");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![key(Key::ArrowDown, Modifiers::NONE)]);
    let all: Vec<String> = a.session.row_keys().iter().filter_map(|k| a.session.summary(k)).map(|m| m.subject.clone()).collect();
    assert_eq!(subjects(&a), vec![all[0].clone()]);
    frame(&ctx, &mut a, vec![key(Key::ArrowDown, Modifiers::SHIFT)]);
    frame(&ctx, &mut a, vec![key(Key::ArrowDown, Modifiers::SHIFT)]);
    assert_eq!(subjects(&a), all[..3].to_vec());
    assert!(a.session.reading.is_none());
    assert!(says(&ctx, &mut a, "3 messages selected"), "the reading pane says how many");
    frame(&ctx, &mut a, vec![key(Key::ArrowUp, Modifiers::SHIFT)]);
    assert_eq!(subjects(&a), all[..2].to_vec());
    frame(&ctx, &mut a, vec![key(Key::End, Modifiers::SHIFT)]);
    assert_eq!(subjects(&a), all.clone());
    // Home alone: the first, by itself, read.
    frame(&ctx, &mut a, vec![key(Key::Home, Modifiers::NONE)]);
    assert_eq!(subjects(&a), vec![all[0].clone()]);
    assert!(a.session.reading.is_some());
    // A page: the rows the list shows at once (more than one at this window's size).
    assert!(a.ui.list_page > 1, "{}", a.ui.list_page);
    frame(&ctx, &mut a, vec![key(Key::PageDown, Modifiers::NONE)]);
    let page = usize::try_from(a.ui.list_page).unwrap().min(all.len() - 1);
    assert_eq!(subjects(&a), vec![all[page].clone()]);
    // Typing in the search box: Shift+Down is the text's.
    a.ui.focus_search = true;
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    let before = a.session.selection.clone();
    frame(&ctx, &mut a, vec![key(Key::ArrowDown, Modifiers::SHIFT)]);
    assert_eq!(a.session.selection, before);
    let _ = std::fs::remove_dir_all(dir);
}

/// Shift+click selects from the row clicked first to the one Shift+clicked; right-clicking one
/// of them keeps them all selected, so the menu acts on all of them (opening the one clicked
/// selected it alone: Delete took just that one).
#[test]
fn shift_click_and_a_right_click_on_the_selection() {
    let (mut a, dir) = app("shift-click-ui");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    let rows = list_rows(&ctx, &mut a);
    assert!(rows.len() >= 4, "{rows:?}");
    let all: Vec<String> = a.session.row_keys().iter().filter_map(|k| a.session.summary(k)).map(|m| m.subject.clone()).collect();
    click(&ctx, &mut a, rows[0].1.center(), egui::PointerButton::Primary, Modifiers::NONE);
    assert_eq!(subjects(&a), vec![all[0].clone()]);
    click(&ctx, &mut a, rows[2].1.center(), egui::PointerButton::Primary, Modifiers::SHIFT);
    assert_eq!(subjects(&a), all[..3].to_vec());
    // A right-click on the middle one: all three stay selected.
    click(&ctx, &mut a, rows[1].1.center(), egui::PointerButton::Secondary, Modifiers::NONE);
    assert_eq!(subjects(&a), all[..3].to_vec(), "the menu is for all of them");
    frame(&ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)]);
    // A right-click on one outside them: that one alone.
    click(&ctx, &mut a, rows[3].1.center(), egui::PointerButton::Secondary, Modifiers::NONE);
    assert_eq!(subjects(&a), vec![all[3].clone()]);
    let _ = std::fs::remove_dir_all(dir);
}
