//! The ribbon's tabs in Calendar, People, Tasks and Notes.

use super::*;

const SIZE: egui::Vec2 = egui::vec2(1280.0, 800.0);

/// Issue #142: in Calendar, People, Tasks and Notes, Send / Receive and View changed nothing, the
/// module's Home buttons always shown. Each tab now shows its own, as Outlook's: Home the
/// module's, Send / Receive the same as in Mail, View how the module shows what it holds, with
/// the folder pane.
#[test]
fn each_modules_tabs_show_their_own_buttons() {
    let (mut a, _) = app("module-ribbon");
    let ctx = egui::Context::default();
    // The words drawn on the ribbon (its buttons, under the tabs).
    let shown = |a: &mut MailApp, tab| {
        a.ui.tab = tab;
        for _ in 0..3 {
            frame(&ctx, a, vec![]);
        }
        let texts = drawn_text(&ctx, a, SIZE);
        let ribbon = a.ui.ribbon_rect.expect("the ribbon drawn");
        texts.into_iter().filter(|d| ribbon.contains(d.rect.center())).map(|d| d.text).collect::<Vec<String>>()
    };
    let is = |texts: &[String], what: &str| texts.iter().any(|t| t == what);
    for (module, home, view) in [
        ("calendar", "New event", "Month"),
        ("tasks", "New task", "Hide completed"),
        ("notes", "New note", "Arrange by"),
        ("people", "New contact", "Cards"),
    ] {
        a.run("view.module", json!({"name": module}));
        let h = shown(&mut a, crate::RibbonTab::Home);
        assert!(is(&h, home) && !is(&h, "Send / Receive all folders"), "{module} Home: {h:?}");
        let v = shown(&mut a, crate::RibbonTab::View);
        assert!(is(&v, view) && is(&v, "Folder pane") && !is(&v, home), "{module} View: {v:?}");
        let sr = shown(&mut a, crate::RibbonTab::SendReceive);
        assert!(is(&sr, "Send / Receive all folders") && is(&sr, "Update folder") && !is(&sr, home), "{module} Send / Receive: {sr:?}");
    }
}
