//! UI tests: rules, junk lists, Search Folders and Quick Steps.

use super::*;

#[test]
fn organising_renders_and_rules_save() {
    let (mut a, dir) = app("organise");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let k = a.session.list[1].key.clone();
    a.run("list.select", json!({"key": k.to_id()}));
    a.run("mail.categorize", json!({"category": "Work"}));
    a.run("mail.pin", json!({}));
    a.run("view.focused", json!({"on": true}));
    a.run("list.open", json!({"key": k.to_id()}));
    frame(&ctx, &mut a, vec![]);
    a.run("list.focusTab", json!({"value": "other"}));
    frame(&ctx, &mut a, vec![]);
    // The rule editor saves a rule; the Rules dialog lists it.
    a.ui.dialog = Some(Dialog::rule_from("zoe@studio.example", ""));
    frame(&ctx, &mut a, vec![]);
    if let Some(Dialog::EditRule(f)) = a.ui.dialog.as_mut() {
        f.action = crate::dialogs::RuleDo::Categorize;
        f.target = "Work".into();
    }
    a.act(crate::Action::Submit(
        "rules.set",
        json!({"rules": [{"name": "Zoe", "conditions": [{"if": "from", "value": "zoe@"}], "actions": [{"do": "categorize", "value": "Work"}]}]}),
    ));
    assert_eq!(a.session.rules.len(), 1);
    a.ui.dialog = Some(Dialog::Rules);
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    let _ = std::fs::remove_dir_all(dir);
}

/// Junk Email: the menu blocks the sender, and the lists in Options draw, long ones in part.
#[test]
fn junk_lists_block_and_draw() {
    let (mut a, dir) = app("junk-lists");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("list.open", json!({"key": "sample:1:1:INBOX"}));
    a.act(crate::Action::Run("junk.blockSender", json!({})));
    assert!(a.session.prefs.blocked_senders.contains("zoe@studio.example"));
    for n in 0..1500 {
        a.run("junk.edit", json!({"list": if n % 2 == 0 { "blocked" } else { "safe" }, "entry": format!("@d{n}.example")}));
    }
    assert_eq!(a.session.prefs.blocked_senders.len() + a.session.prefs.safe_senders.len(), 1501);
    a.act(crate::Action::Backstage(crate::backstage::Page::Options));
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    assert!(a.ui.backstage.is_some());
    let _ = std::fs::remove_dir_all(dir);
}

/// Search Folders: made from the dialog (a kind, or people), opened from the pane, the heading
/// its name and the search box left empty; the dialog shows the reason when refused.
#[test]
fn search_folders_are_made_from_the_dialog_and_open() {
    let (mut a, dir) = app("search-folders");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let mut f = crate::dialogs::SearchFolderForm::new();
    assert_eq!(f.final_query(), "is:unread");
    let before = f.preset;
    f.preset = 7;
    f.apply_preset(before);
    assert_eq!(f.name, "Mail from specific people", "the name follows the kind until one is typed");
    f.people = "zoe@studio.example; Ana Lima, , \"x".into();
    assert_eq!(f.final_query(), "from:zoe@studio.example,\"Ana Lima\",x");
    f.name = "Zoe and Ana".into();
    a.ui.dialog = Some(Dialog::SearchFolder(Box::new(f.clone())));
    frame(&ctx, &mut a, vec![]);
    a.ui.search_text = "typed before".into();
    a.act(crate::Action::Submit("searchFolder.create", json!({"name": f.name, "query": f.final_query()})));
    assert!(a.ui.dialog.is_none());
    assert_eq!(crate::view_title(&a.session), "Zoe and Ana");
    assert!(a.ui.search_text.is_empty());
    frame(&ctx, &mut a, vec![]);
    assert!(a.ui.search_text.is_empty(), "the box isn't filled with its query");
    assert!(!a.session.list.is_empty() && a.session.list.iter().all(|m| m.from.email.starts_with("zoe") || m.from.name.contains("Ana")));
    // The same name again: the dialog stays, with why.
    a.ui.dialog = Some(Dialog::SearchFolder(Box::new(f.clone())));
    a.act(crate::Action::Submit("searchFolder.create", json!({"name": "zoe AND ana", "query": "is:unread"})));
    assert!(matches!(&a.ui.dialog, Some(Dialog::SearchFolder(f)) if f.error.is_some()));
    // Edited: a preset's search shows as that kind, anything else as Custom.
    let edit = crate::dialogs::SearchFolderForm::edit(&a.session.prefs.search_folders[0]);
    assert_eq!((edit.preset, edit.query.as_str()), (8, "from:zoe@studio.example,\"Ana Lima\",x"));
    a.ui.dialog = Some(Dialog::SearchFolder(Box::new(edit)));
    frame(&ctx, &mut a, vec![]);
    // Many, with long names, draw.
    for n in 0..60 {
        a.run("searchFolder.create", json!({"name": format!("{} {n}", "Quite a long Search Folder name ".repeat(3)), "query": "is:flagged"}));
    }
    a.ui.dialog = None;
    frame(&ctx, &mut a, vec![]);
    a.run("search.run", json!({"query": "storyboard"}));
    assert!(crate::view_title(&a.session).contains("storyboard"));
    let _ = std::fs::remove_dir_all(dir);
}

/// Quick Steps: made from the dialog, done with Ctrl+Shift+1, changed and deleted from it.
#[test]
fn quick_steps_are_made_and_done_with_their_keys() {
    let (mut a, dir) = app("quick-steps");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let mut f = crate::dialogs::QuickStepForm::new(&a.session);
    assert_eq!(f.actions(), vec![json!({"do": "markRead"}), json!({"do": "moveToRole", "value": "archive"})], "Done, as Outlook's");
    f.name = "Done".into();
    f.flag = true;
    a.ui.dialog = Some(Dialog::QuickStep(Box::new(f.clone())));
    frame(&ctx, &mut a, vec![]);
    a.act(crate::Action::Submit("quickStep.create", json!({"name": f.name, "actions": f.actions()})));
    assert!(a.ui.dialog.is_none() && a.session.prefs.quick_steps.len() == 1);
    // Ctrl+Shift+1 on the first message: flagged, read, archived.
    frame(&ctx, &mut a, vec![key(Key::ArrowDown, Modifiers::NONE)]);
    let m = a.session.selected()[0].clone();
    // As Windows sends it: Shift makes the key `!`; where it is says 1.
    let shifted = Event::Key {
        key: Key::Exclamationmark,
        physical_key: Some(Key::Num1),
        pressed: true,
        repeat: false,
        modifiers: Modifiers::COMMAND | Modifiers::SHIFT,
    };
    frame(&ctx, &mut a, vec![shifted]);
    let archive = a.session.account("sample").and_then(|x| x.folder(mailcraft_engine::model::FolderRole::Archive)).map(|x| x.path.clone()).unwrap();
    let there = a.session.folder_state("sample", &archive).messages.iter().find(|x| x.message_id == m.message_id).cloned().unwrap();
    assert!(there.flags.seen && there.flags.flagged);
    // Opened again, it shows what it does; renamed to a taken name: the reason in the dialog.
    a.run("quickStep.create", json!({"name": "Later", "actions": [{"do": "pin"}]}));
    let mut edit = crate::dialogs::QuickStepForm::edit(&a.session, &a.session.prefs.quick_steps[0]);
    assert!(edit.read && edit.flag && edit.move_to == crate::dialogs::StepMove::Role(mailcraft_engine::model::FolderRole::Archive));
    edit.name = "later".into();
    a.ui.dialog = Some(Dialog::QuickStep(Box::new(edit.clone())));
    a.act(crate::Action::Submit("quickStep.edit", json!({"name": "Done", "newName": edit.name, "actions": edit.actions()})));
    assert!(matches!(&a.ui.dialog, Some(Dialog::QuickStep(f)) if f.error.is_some()));
    frame(&ctx, &mut a, vec![]);
    a.act(crate::Action::Submit("quickStep.delete", json!({"name": "Done"})));
    assert!(a.ui.dialog.is_none());
    assert_eq!(a.session.prefs.quick_steps.len(), 1);
    // Nothing to do: the dialog can't be saved.
    assert!(crate::dialogs::QuickStepForm { name: "x".into(), ..Default::default() }.actions().is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

/// Issue #4: the search box's recent searches (those with what's typed so far; none, closed)
/// and its advanced search (filled from the box, written back to it), drawn in a frame; Esc
/// closes them; where it looks named in every language's terms.
#[test]
fn the_search_box_drops_down_recent_and_advanced_search() {
    let (mut a, dir) = app("search-box");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.session.prefs.recent_searches = vec!["budget".into(), "storyboard".into()];
    a.ui.search_drop = crate::SearchDrop::Recent;
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.ui.search_drop, crate::SearchDrop::Recent);
    a.ui.search_text = "story".into();
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.ui.search_drop, crate::SearchDrop::Recent, "storyboard has it");
    a.ui.search_text = "zzz".into();
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.ui.search_drop, crate::SearchDrop::None, "nothing to show: closed");

    a.ui.search_text = "budget from:ana".into();
    let form = mailcraft_engine::model::search::Advanced::from_query(&a.ui.search_text);
    assert_eq!((form.words.as_str(), form.from.as_str()), ("budget", "ana"));
    a.ui.search_drop = crate::SearchDrop::Advanced(form.clone());
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.ui.search_drop, crate::SearchDrop::Advanced(form), "stays open, the form as it was");
    frame(&ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)]);
    assert_eq!(a.ui.search_drop, crate::SearchDrop::None);
    for scope in mailcraft_engine::SearchScope::ALL {
        assert!(!crate::scope_label(scope).is_empty());
    }
    let _ = std::fs::remove_dir_all(dir);
}
