//! Deleting for good asks first (File › Options › Mail › Ask before deleting for good), from a
//! row's own Delete button in Deleted Items too: it deleted the message unasked.

use super::*;
use egui::accesskit::Role;

const SIZE: egui::Vec2 = egui::vec2(1280.0, 800.0);

/// The first three Inbox messages moved to Deleted Items: its path.
fn three_deleted(a: &mut MailApp) -> String {
    for k in a.session.row_keys().into_iter().take(3) {
        a.run("list.select", json!({"key": k.to_id()}));
        a.run("mail.delete", json!({}));
    }
    a.session.account("sample").and_then(|acc| acc.folder(mailcraft_engine::model::FolderRole::Trash)).map(|f| f.path.clone()).expect("Deleted Items")
}

/// The first row's own Delete button, shown under the pointer.
fn first_rows_delete(ctx: &egui::Context, a: &mut MailApp) -> egui::Pos2 {
    let rows: Vec<egui::Rect> = named(ctx, a, SIZE).into_iter().filter(|n| n.role == Role::ListItem).filter_map(|n| n.rect).collect();
    let first = rows.iter().min_by(|x, y| x.min.y.total_cmp(&y.min.y)).copied().expect("a row");
    frame(ctx, a, vec![Event::PointerMoved(first.center())]);
    let at = named(ctx, a, SIZE)
        .iter()
        .filter(|n| n.role == Role::Button && n.label.as_deref() == Some("Delete"))
        .filter_map(Named::centre)
        .find(|c| first.contains(*c));
    at.expect("its Delete button")
}

#[test]
fn a_rows_delete_in_deleted_items_asks_first() {
    let (mut a, _) = app("delete-row-asks");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    frame(&ctx, &mut a, vec![]);
    let trash = three_deleted(&mut a);
    a.run("folder.open", json!({"account": "sample", "folder": trash}));
    frame(&ctx, &mut a, vec![]);
    let target = a.session.row_keys().first().cloned().expect("in Deleted Items");
    let at = first_rows_delete(&ctx, &mut a);
    click(&ctx, &mut a, at);
    assert!(matches!(&a.ui.dialog, Some(Dialog::ConfirmDelete { count: 1, one: Some(k) }) if *k == target), "asked: {:?}", a.ui.dialog);
    assert!(a.session.row_keys().contains(&target), "not deleted yet");
    // Its Delete: that message alone, for good.
    frame(&ctx, &mut a, vec![]);
    // The question's own (not the ribbon's or a row's).
    let id = egui::Id::unique(("dialog", std::mem::discriminant(a.ui.dialog.as_ref().unwrap())));
    let question = ctx.memory(|m| m.area_rect(id)).expect("the question");
    let ok = named(&ctx, &mut a, SIZE)
        .iter()
        .filter(|n| n.role == Role::Button && n.label.as_deref() == Some("Delete"))
        .filter_map(Named::centre)
        .find(|c| question.contains(*c));
    click(&ctx, &mut a, ok.expect("the question's Delete"));
    frame(&ctx, &mut a, vec![]);
    assert!(a.ui.dialog.is_none());
    assert!(!a.session.row_keys().contains(&target), "deleted");
    assert_eq!(a.session.row_keys().len(), 2, "the others stay");
    // Not asked once the setting is off.
    a.run("view.confirmDelete", json!({"on": false}));
    let at = first_rows_delete(&ctx, &mut a);
    click(&ctx, &mut a, at);
    frame(&ctx, &mut a, vec![]);
    assert!(a.ui.dialog.is_none() && a.session.row_keys().len() == 1, "deleted unasked, as set");
}
