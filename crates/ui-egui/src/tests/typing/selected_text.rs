//! A field given the keyboard with what it holds selected, so what's typed replaces it: the search
//! box asked for again (Ctrl+E), and a name offered to change (Rename folder). As PdfCraft's find
//! field and AniCraft's prompts do.

use super::*;

/// Ctrl+E while typing in the search box: its query selected, so typing starts a new one (it went
/// on the end of the old: the box had the keyboard already, so egui left its caret). After a term
/// put in the box (From, Subject), the caret is at its end, for the name.
#[test]
fn asking_for_the_search_box_again_selects_its_query() {
    let (mut a, _) = app("search-again");
    let ctx = egui::Context::default();
    for _ in 0..2 {
        frame(&ctx, &mut a, vec![]);
    }
    frame(&ctx, &mut a, vec![key(Key::E, Modifiers::COMMAND)]);
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![Event::Text("budget".into())]);
    assert_eq!(a.ui.search_text, "budget");
    frame(&ctx, &mut a, vec![key(Key::E, Modifiers::COMMAND)]);
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![Event::Text("q".into())]);
    assert_eq!(a.ui.search_text, "q");
    // A term put in by the Search tab: the caret after it, nothing selected.
    a.ui.search_text = "subject:".into();
    a.ui.focus_search = true;
    a.ui.search_cursor_end = true;
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![Event::Text("x".into())]);
    assert_eq!(a.ui.search_text, "subject:x");
}

/// Rename folder opens with the folder's name selected: typing replaces it (it went on the end).
#[test]
fn renaming_a_folder_starts_with_its_name_selected() {
    let (mut a, _) = app("rename-selected");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let account = a.session.accounts[0].config.id.clone();
    a.ui.dialog =
        Some(crate::dialogs::Dialog::FolderName { account, parent: None, rename: Some("Projects".into()), name: "Projects".into(), error: None });
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![Event::Text("Clients".into())]);
    let Some(crate::dialogs::Dialog::FolderName { name, .. }) = &a.ui.dialog else { panic!("the dialog stays open") };
    assert_eq!(name, "Clients");
}
