//! UI tests: rules, junk lists, Search Folders and Quick Steps.

use super::*;

mod categories;
mod folder_pane;
mod server_rules;

#[test]
fn organising_renders_and_rules_save() {
    let (mut a, _) = app("organise");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
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
    // The rule editor saves the rule its form says; the Rules dialog lists it.
    a.ui.dialog = Some(Dialog::rule_from("zoe@studio.example"));
    frame(&ctx, &mut a, vec![]);
    if let Some(Dialog::EditRule(f)) = a.ui.dialog.as_mut() {
        f.action = crate::dialogs::RuleDo::Categorize;
        f.target = "Work".into();
    }
    // A frame for the dialog to settle, taller with its Category row.
    frame(&ctx, &mut a, vec![]);
    click_named(&ctx, &mut a, "Save");
    assert!(a.ui.dialog.is_none(), "saved and closed: {:?}", a.ui.dialog);
    use mailcraft_engine::model::organize::{Condition, RuleAction};
    let rules: Vec<_> = a.session.rules.iter().map(|r| (r.conditions.clone(), r.actions.clone())).collect();
    assert_eq!(rules, [(vec![Condition::From("zoe@studio.example".into())], vec![RuleAction::Categorize("Work".into())])]);
    a.ui.dialog = Some(Dialog::Rules);
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
}

/// Junk Email: the menu blocks the sender, and the lists in Options draw, long ones in part.
#[test]
fn junk_lists_block_and_draw() {
    let (mut a, _) = app("junk-lists");
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
}

/// Each Search Folder offered is known by what it searches for, not by its place in the list:
/// its name from its kind, one that asks for people and one for a search, and each fixed one
/// opened again as itself.
#[test]
fn search_folder_presets_are_known_by_kind() {
    use crate::dialogs::{PresetKind, SEARCH_PRESETS, SearchFolderForm};
    for (label, kind) in SEARCH_PRESETS {
        assert_eq!(kind.label(), label);
        if let PresetKind::Fixed(query) = kind {
            let f = SearchFolderForm::edit(&mailcraft_engine::SearchFolder { name: label.into(), query: query.into() });
            assert_eq!((f.preset, f.final_query()), (kind, query.to_string()));
        }
    }
    for asks in [PresetKind::People, PresetKind::Custom] {
        assert_eq!(SEARCH_PRESETS.iter().filter(|p| p.1 == asks).count(), 1, "{asks:?}");
    }
    assert_eq!(SearchFolderForm::new().preset, SEARCH_PRESETS[0].1);
    // Custom starts from the search of the one it was.
    let mut f = SearchFolderForm::new();
    f.preset = PresetKind::Custom;
    f.apply_preset(PresetKind::Fixed("is:flagged"));
    assert_eq!((f.query.as_str(), f.name.as_str()), ("is:flagged", "Unread mail"), "a name typed (here, the first's) stays");
}

/// Search Folders: made from the dialog (a kind, or people), opened from the pane, the heading
/// its name and the search box left empty; the dialog shows the reason when refused.
#[test]
fn search_folders_are_made_from_the_dialog_and_open() {
    let (mut a, _) = app("search-folders");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let mut f = crate::dialogs::SearchFolderForm::new();
    assert_eq!(f.final_query(), "is:unread");
    let before = f.preset;
    f.preset = crate::dialogs::PresetKind::People;
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
    assert_eq!((edit.preset, edit.query.as_str()), (crate::dialogs::PresetKind::Custom, "from:zoe@studio.example,\"Ana Lima\",x"));
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
}

/// Quick Steps: made from the dialog, done with Ctrl+Shift+1, changed and deleted from it.
#[test]
fn quick_steps_are_made_and_done_with_their_keys() {
    let (mut a, _) = app("quick-steps");
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
}

/// Issue #4: the search box's recent searches (those with what's typed so far; none, closed)
/// and its advanced search (filled from the box, written back to it), drawn in a frame; Esc
/// closes them; where it looks named in every language's terms.
#[test]
fn the_search_box_drops_down_recent_and_advanced_search() {
    let (mut a, _) = app("search-box");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.session.prefs.recent_searches = vec!["budget".into(), "storyboard".into()];
    // In the box, as typing is: words outside it with no search behind them go (issue #187).
    a.ui.focus_search = true;
    frame(&ctx, &mut a, vec![]);
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
}

/// Issue #187: a search ended by opening a folder, or by going to another module, takes its
/// words out of the box; they're among the recent searches. They stayed, as if it were still on.
#[test]
fn a_search_left_empties_the_box() {
    let (mut a, _) = app("search-left");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("search.run", json!({"query": "storyboard", "remember": true}));
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.ui.search_text, "storyboard", "the search's words in the box");
    a.run("folder.open", json!({"account": "sample", "folder": "Projects"}));
    frame(&ctx, &mut a, vec![]);
    assert!(a.session.search.is_none() && a.ui.search_text.is_empty(), "{:?}", a.ui.search_text);
    assert!(a.session.recent_searches().iter().any(|r| r == "storyboard"), "kept among the recent searches");
    // Searched again, then to the Calendar: the search ends with the box.
    a.run("search.run", json!({"query": "budget"}));
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.ui.search_text, "budget");
    a.run("view.calendar", json!({}));
    frame(&ctx, &mut a, vec![]);
    assert!(a.session.search.is_none() && a.ui.search_text.is_empty(), "{:?}", a.ui.search_text);
    a.run("view.mail", json!({}));
    frame(&ctx, &mut a, vec![]);
    assert!(a.ui.search_text.is_empty(), "nothing back in the box on the way back");
}

/// Issue #11: Favorites can be hidden from the top of the folder pane (View › Favorites), not
/// only collapsed, and shown again.
#[test]
fn favorites_can_be_hidden() {
    let (mut a, _) = app("favorites-hidden");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let headings = |a: &mut MailApp| {
        let nodes = named(&ctx, a, egui::vec2(1280.0, 800.0));
        nodes.iter().filter(|n| n.label.as_ref().is_some_and(|l| l.contains("Favorites"))).count()
    };
    headings(&mut a);
    assert!(headings(&mut a) > 0, "shown at first");
    a.run("view.favorites", json!({}));
    headings(&mut a);
    assert_eq!(headings(&mut a), 0, "hidden");
    a.run("view.favorites", json!({}));
    headings(&mut a);
    assert!(headings(&mut a) > 0, "shown again");
}

/// Issue #28: a keyword another program set shows on the message (in the list, the reading pane
/// and the Categorize menu's list) and in the Categories dialog; added from there it keeps its
/// keyword; a keyword another category has is refused in the dialog.
#[test]
fn categories_from_other_programs_render_and_join_the_list() {
    let (mut a, _) = app("categories-28");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    frame(&ctx, &mut a, vec![]);
    let k = a.session.list[0].key.clone();
    a.run("list.select", json!({"key": k.to_id()}));
    a.run("mail.categorize", json!({"keyword": "r=c3=bcckfrage"}));
    a.run("mail.categorize", json!({"category": "Work"}));
    a.run("list.open", json!({"key": k.to_id()}));
    frame(&ctx, &mut a, vec![]);
    let names: Vec<String> = a.session.categories_of(&a.session.selected()[0]).into_iter().map(|c| c.name).collect();
    assert_eq!(names, ["Work", "rückfrage"]);
    a.ui.dialog = Some(Dialog::Categories);
    frame(&ctx, &mut a, vec![]);
    a.ui.dialog = Some(Dialog::name_keyword("r=c3=bcckfrage"));
    frame(&ctx, &mut a, vec![]);
    if let Some(Dialog::NewCategory { name, .. }) = a.ui.dialog.as_mut() {
        *name = "Rückfrage".into();
    }
    click_named(&ctx, &mut a, "Create");
    assert!(a.ui.dialog.is_none(), "{:?}", a.ui.dialog);
    assert_eq!(a.session.category("rückfrage").map(|c| c.keyword.as_str()), Some("r=c3=bcckfrage"));
    // Taken: the dialog stays, saying why.
    a.ui.dialog = Some(Dialog::new_category(&a.session));
    a.act(crate::Action::Submit("category.create", json!({"name": "Other", "keyword": "R=C3=BCCKFRAGE"})));
    assert!(matches!(&a.ui.dialog, Some(Dialog::NewCategory { error: Some(e), .. }) if e.contains("Rückfrage")), "{:?}", a.ui.dialog);
    // Edited: renamed, keyword kept.
    let c = a.session.category("Rückfrage").cloned().unwrap();
    a.ui.dialog = Some(Dialog::edit_category(&c));
    frame(&ctx, &mut a, vec![]);
    a.act(crate::Action::Submit("category.edit", json!({"name": "Rückfrage", "newName": "Nachfrage", "color": 2})));
    assert_eq!(a.session.category("Nachfrage").map(|c| c.keyword.as_str()), Some("r=c3=bcckfrage"));
    frame(&ctx, &mut a, vec![]);
}

/// A follow-up marked complete shows as done: the list row says so (and draws its tick where the
/// flag was), the reading pane says on which day, and Clear Flag takes it away.
#[test]
fn a_completed_follow_up_shows_as_done() {
    let (mut a, _) = app("follow-up-complete");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let labels = |ctx: &egui::Context, a: &mut MailApp| -> Vec<String> {
        named(ctx, a, egui::vec2(1280.0, 800.0)).iter().filter_map(Named::name).map(str::to_string).collect()
    };
    frame(&ctx, &mut a, vec![]);
    let m = a.session.list[1].clone();
    a.run("list.open", json!({"key": m.key.to_id()}));
    a.run("mail.flag", json!({"due": "tomorrow"}));
    a.run("mail.flag", json!({"complete": true}));
    for dark in [Some(false), Some(true)] {
        a.ui.dark = dark;
        frame(&ctx, &mut a, vec![]);
    }
    let shown = labels(&ctx, &mut a);
    let spoken = format!("{}, {}, ", m.from.display(), m.subject);
    let row = shown.iter().find(|l| l.starts_with(&spoken)).cloned().unwrap_or_default();
    assert!(row.contains("Completed") && !row.contains("Flagged"), "{row}");
    let today = mailcraft_engine::model::dates::day_label(mailcraft_engine::model::dates::today(&a.session.clock));
    assert!(shown.iter().any(|l| l.starts_with("Completed on ") && l.contains(&today)), "{shown:?}");
    // Arranged by the flag, under its own heading.
    a.run("view.arrange", json!({"by": "flag"}));
    assert!(a.session.rows().contains(&mailcraft_engine::Row::Header("Completed")));
    frame(&ctx, &mut a, vec![]);
    a.run("view.arrange", json!({"by": "date"}));
    // Cleared: neither.
    a.run("mail.flag", json!({"on": false}));
    let shown = labels(&ctx, &mut a);
    assert!(!shown.iter().any(|l| l.contains("Completed")), "{shown:?}");
}

/// A rule with more than the editor shows (two conditions, any of which will do, two actions,
/// the one account it's for) keeps all of that when it's opened and saved with only its name
/// changed, and keeps the rest when what the editor shows is changed. It was saved as the
/// editor's one condition and one action, for every account, and its move to Junk came back as
/// a move to Archive.
#[test]
fn a_rule_shown_in_part_keeps_the_rest() {
    use crate::dialogs::{RuleDo, RuleForm, RuleTest};
    use mailcraft_engine::model::FolderRole;
    use mailcraft_engine::model::organize::{Condition, Rule, RuleAction};
    let rule = Rule {
        name: "Studio".into(),
        enabled: true,
        account: Some("sample".into()),
        conditions: vec![Condition::From("zoe@".into()), Condition::Subject("cut".into())],
        match_all: false,
        actions: vec![RuleAction::Flag, RuleAction::MoveToRole(FolderRole::Junk)],
        stop: true,
    };
    let mut f = RuleForm::from_rule(0, &rule);
    f.name = "Studio mail".into();
    assert_eq!(f.to_rule().unwrap(), Rule { name: "Studio mail".into(), ..rule.clone() });
    // What it shows, changed: that changes, the rest stays.
    let mut f = RuleForm::from_rule(0, &rule);
    (f.test, f.text) = (RuleTest::To, "ana@".into());
    (f.action, f.target) = (RuleDo::Categorize, "Work".into());
    let saved = f.to_rule().unwrap();
    assert_eq!(saved.conditions, [Condition::To("ana@".into()), Condition::Subject("cut".into())]);
    assert_eq!(saved.actions, [RuleAction::Categorize("Work".into()), RuleAction::MoveToRole(FolderRole::Junk)]);
    assert_eq!((saved.match_all, saved.account.as_deref(), saved.stop), (false, Some("sample"), true));
    // One with nothing to show (no conditions, no actions) gets none it didn't have.
    let bare = Rule { conditions: Vec::new(), actions: Vec::new(), ..rule.clone() };
    let mut f = RuleForm::from_rule(0, &bare);
    f.name = "Bare".into();
    assert_eq!(f.to_rule().unwrap(), Rule { name: "Bare".into(), ..bare });
}

/// A rule that moved mail to a folder of one account, changed to do something else, is for every
/// account, as a new rule of that kind is: it kept the move's account, which the editor shows for
/// no other action. One that still moves mail to a folder (an action the editor doesn't show)
/// keeps it, and so does one whose move goes to another folder.
#[test]
fn a_rule_that_no_longer_moves_is_for_every_account() {
    use crate::dialogs::{RuleDo, RuleForm};
    use mailcraft_engine::model::organize::{Condition, Rule, RuleAction};
    let rule = Rule {
        name: "Studio".into(),
        enabled: true,
        account: Some("sample".into()),
        conditions: vec![Condition::From("zoe@".into())],
        match_all: true,
        actions: vec![RuleAction::MoveTo("Projects".into()), RuleAction::MarkRead],
        stop: false,
    };
    let mut f = RuleForm::from_rule(0, &rule);
    (f.action, f.target) = (RuleDo::Flag, String::new());
    let saved = f.to_rule().unwrap();
    assert_eq!((saved.actions.as_slice(), saved.account), ([RuleAction::Flag, RuleAction::MarkRead].as_slice(), None));
    // As the same rule made new.
    let new = RuleForm { index: None, original: None, ..f }.to_rule().unwrap();
    assert_eq!(new.account, None);
    // A second move to a folder still needs the account its folder is in.
    let two = Rule { actions: vec![RuleAction::MoveTo("Projects".into()), RuleAction::MoveTo("Clients".into())], ..rule.clone() };
    let mut f = RuleForm::from_rule(0, &two);
    (f.action, f.target) = (RuleDo::Flag, String::new());
    let saved = f.to_rule().unwrap();
    assert_eq!(
        (saved.actions.as_slice(), saved.account.as_deref()),
        ([RuleAction::Flag, RuleAction::MoveTo("Clients".into())].as_slice(), Some("sample"))
    );
    // The move sent elsewhere keeps it.
    let mut f = RuleForm::from_rule(0, &rule);
    f.target = "Clients".into();
    assert_eq!(f.to_rule().unwrap().account.as_deref(), Some("sample"));
}

/// The folder search in Move to and Move folder finds a folder by the name it shows
/// ("Posteingang" in German), as well as by its path on the server ("INBOX").
#[test]
fn folder_search_finds_the_name_shown() {
    let (mut a, _) = app("move-search");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let k = a.session.list[1].key.clone();
    a.run("list.select", json!({"key": k.to_id()}));
    // The folders the dialog lists: what it draws after the search box's text, before its buttons.
    let rows = |a: &mut MailApp, dialog: Dialog, typed: &str| {
        a.ui.dialog = Some(dialog);
        frame(&ctx, a, vec![]);
        let texts: Vec<String> = drawn_strings(&ctx, a, egui::vec2(1280.0, 800.0)).iter().map(|t| t.trim().to_string()).collect();
        let from = texts.iter().rposition(|t| t == typed).map_or(texts.len(), |i| i + 1);
        let buttons = texts.len().saturating_sub(2).max(from);
        texts[from..buttons].to_vec()
    };
    a.ui.language = "de".into();
    for typed in ["posteingang", "inbox"] {
        assert_eq!(rows(&mut a, Dialog::MoveTo { filter: typed.into(), copy: false }, typed), ["Posteingang"], "Move to, {typed:?}");
    }
    assert_eq!(rows(&mut a, Dialog::MoveTo { filter: "entw".into(), copy: false }, "entw"), ["Entwürfe"]);
    let moving =
        |filter: &str| Dialog::MoveFolder { account: "sample".into(), folder: "Projects".into(), filter: filter.into(), chosen: None, error: None };
    for typed in ["posteingang", "inbox"] {
        assert_eq!(rows(&mut a, moving(typed), typed), ["Posteingang"], "Move folder, {typed:?}");
    }
    assert_eq!(rows(&mut a, moving("gel"), "gel"), ["Gelöschte Elemente"]);
    crate::i18n::set_current(crate::i18n::Lang::EN);
}

/// Issue #182: in Move folder a click picks where the folder goes, Move turns on and moves it
/// there. A click moved it at once, and Move never turned on.
#[test]
fn move_folder_picks_then_moves() {
    let (mut a, _) = app("move-folder-pick");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let projects = a.session.folder("sample", "Projects/Short film").cloned().unwrap();
    a.ui.dialog = Some(Dialog::move_folder(&projects));
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    let move_button = |ctx: &egui::Context, a: &mut MailApp| {
        named(ctx, a, egui::vec2(1280.0, 800.0)).into_iter().find(|n| n.label.as_deref() == Some("Move")).map(|n| n.disabled)
    };
    assert_eq!(move_button(&ctx, &mut a), Some(true), "nothing picked yet");
    // Each row across the list, a short name's as a long one's.
    let width = |ctx: &egui::Context, a: &mut MailApp, name: &str| {
        let all = named(ctx, a, egui::vec2(1280.0, 800.0));
        all.into_iter().find(|n| n.label.as_deref() == Some(name)).and_then(|n| n.rect).map(|r| r.width()).unwrap()
    };
    let (inbox, top) = (width(&ctx, &mut a, "Inbox"), width(&ctx, &mut a, "(Top level)"));
    assert!(inbox > 200.0 && (inbox - top).abs() < 1.0, "rows as wide as the list: {inbox} {top}");
    click_named(&ctx, &mut a, "Inbox");
    frame(&ctx, &mut a, vec![]);
    assert!(a.session.folder("sample", "Projects/Short film").is_some(), "a click only picks");
    assert!(matches!(&a.ui.dialog, Some(Dialog::MoveFolder { chosen: Some(Some(p)), .. }) if p == "INBOX"), "{:?}", a.ui.dialog);
    assert_eq!(move_button(&ctx, &mut a), Some(false), "Move turns on");
    click_named(&ctx, &mut a, "Move");
    frame(&ctx, &mut a, vec![]);
    assert!(a.session.folder("sample", "INBOX/Short film").is_some(), "moved there");
    assert!(a.ui.dialog.is_none());
}

/// `n` rules, the fourth moving mail to "Projects/Short film", the others flagging it.
fn many_rules(n: usize) -> Vec<mailcraft_engine::model::organize::Rule> {
    use mailcraft_engine::model::organize::{Condition, Rule, RuleAction};
    (0..n)
        .map(|i| Rule {
            name: format!("Rule {i}"),
            enabled: i % 3 != 2,
            account: Some("sample".into()),
            conditions: vec![Condition::From(format!("s{i}@")), Condition::Subject("report".into())],
            match_all: i % 2 == 0,
            actions: vec![
                if i == 3 { RuleAction::MoveTo("Projects/Short film".into()) } else { RuleAction::MoveTo(format!("Projects/p{i}")) },
                RuleAction::Flag,
            ],
            stop: false,
        })
        .collect()
}

/// The Rules dialog's frame: each text drawn that can be seen (not wholly outside its clip), where.
fn rules_drawn(ctx: &egui::Context, a: &mut MailApp, find: &str) -> Vec<(String, egui::Pos2)> {
    ctx.data_mut(|d| d.insert_temp(egui::Id::unique("rules-find"), find.to_string()));
    a.ui.dialog = Some(Dialog::Rules);
    frame(ctx, a, vec![]);
    drawn_text(ctx, a, egui::vec2(1280.0, 800.0)).into_iter().filter(Drawn::seen).map(|d| (d.text, d.rect.min)).collect()
}

/// The Rules dialog: each rule's line says what it does (a folder by the names the folder pane
/// shows), the find box keeps the rules it names, and a rule's switch turns that rule alone off.
#[test]
fn the_rules_dialog_describes_finds_and_switches() {
    let (mut a, _) = app("rules-dialog");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    frame(&ctx, &mut a, vec![]);
    a.session.rules = many_rules(8);
    let texts: Vec<String> = rules_drawn(&ctx, &mut a, "").into_iter().map(|(t, _)| t).collect();
    let rows: Vec<&String> = texts.iter().filter(|t| t.starts_with("Rule ")).collect();
    assert_eq!(rows.len(), 8, "{texts:?}");
    assert_eq!(rows[3], "Rule 3If it from “s3@” or subject has “report”: move to Projects › Short film, flag");
    assert_eq!(rows[0], "Rule 0If it from “s0@” and subject has “report”: move to Projects › p0, flag");
    let texts: Vec<String> = rules_drawn(&ctx, &mut a, "SHORT film").into_iter().map(|(t, _)| t).collect();
    assert_eq!(
        texts.iter().filter(|t| t.starts_with("Rule ")).collect::<Vec<_>>(),
        ["Rule 3If it from “s3@” or subject has “report”: move to Projects › Short film, flag"]
    );
    let texts: Vec<String> = rules_drawn(&ctx, &mut a, "rule 7").into_iter().map(|(t, _)| t).collect();
    assert_eq!(texts.iter().filter(|t| t.starts_with("Rule ")).count(), 1);
    let texts: Vec<String> = rules_drawn(&ctx, &mut a, "nothing like it").into_iter().map(|(t, _)| t).collect();
    assert!(texts.iter().any(|t| t == "No rule matches."), "{texts:?}");
    // The second rule's switch, clicked: it alone goes off.
    let _ = rules_drawn(&ctx, &mut a, "");
    let nodes = named(&ctx, &mut a, egui::vec2(1280.0, 800.0));
    let mut boxes: Vec<egui::Pos2> = nodes.iter().filter(|n| n.role == egui::accesskit::Role::CheckBox).filter_map(Named::centre).collect();
    boxes.sort_by(|p, q| p.y.total_cmp(&q.y));
    click(&ctx, &mut a, boxes[1]);
    let mut expected = many_rules(8);
    expected[1].enabled = false;
    assert_eq!(a.session.rules, expected);
}

/// The Rules dialog's lines follow the language as it's switched, between languages that word
/// "If it …: …" alike too (Italian and Brazilian Portuguese, the two Chineses): the lines kept
/// from the frame before stayed in the language they were made in.
#[test]
fn the_rules_dialog_follows_the_language() {
    let (mut a, _) = app("rules-language");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.session.rules = many_rules(2);
    for (code, from) in
        [("it", "arriva da «s0@»"), ("pt-br", "vier de “s0@”"), ("zh-hans", "来自“s0@”"), ("zh-hant", "來自「s0@」"), ("it", "arriva da «s0@»")]
    {
        a.ui.language = code.into();
        let texts: Vec<String> = rules_drawn(&ctx, &mut a, "").into_iter().map(|(t, _)| t).collect();
        assert!(texts.iter().any(|t| t.starts_with("Rule 0") && t.contains(from)), "{code}: {texts:?}");
    }
    crate::i18n::set_current(crate::i18n::Lang::EN);
}

/// A frame of the Rules dialog with 300 rules moving mail into folders of an account of 3,000,
/// every rule shown, eleven found and none, and a fingerprint of what it draws (the same before and
/// after a change that only makes it faster). Run in release under the machine's lock:
/// `cargo test --release -p mailcraft-ui-egui --lib measure_rules_dialog -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement: run in release"]
fn measure_rules_dialog() {
    use std::hash::{Hash, Hasher};
    let (mut a, _) = app("measure-rules");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let base = a.session.accounts[0].folders[0].clone();
    for n in 0..3000 {
        let f = mailcraft_engine::model::Folder { path: format!("Projects/p{n}"), role: mailcraft_engine::model::FolderRole::Other, ..base.clone() };
        a.session.accounts[0].folders.push(f);
    }
    a.session.rules = many_rules(300);
    // The folder pane, which would draw the 3,000 folders behind the dialog, kept small.
    a.run("view.folderPane", json!({"minimized": true}));
    let timed = |a: &mut MailApp| {
        let t = std::time::Instant::now();
        for _ in 0..50 {
            frame(&ctx, a, vec![]);
        }
        t.elapsed().as_secs_f64() * 1000.0 / 50.0
    };
    // The window without the dialog, to take away.
    frame(&ctx, &mut a, vec![]);
    let window = timed(&mut a);
    println!("MEASURE the window alone (3000 folders): {window:.2} ms a frame");
    for find in ["", "rule 12", "no such rule"] {
        let drawn = rules_drawn(&ctx, &mut a, find);
        let mut h = std::collections::hash_map::DefaultHasher::new();
        for (t, p) in &drawn {
            (t, p.x.to_bits(), p.y.to_bits()).hash(&mut h);
        }
        let each = timed(&mut a);
        println!(
            "MEASURE rules dialog, 300 rules, 3000 folders, find {find:?}: {each:.2} ms a frame, the dialog {:.2} ms; drawn {} texts, {:016x}",
            each - window,
            drawn.len(),
            h.finish()
        );
    }
}

/// Issue #199: File › Options says new mail in Other isn't announced while the Focused Inbox is
/// on, with a switch to have it announced: shown with the Focused Inbox and notifications both
/// on, gone with either off.
#[test]
fn options_offer_notifications_for_other_with_the_focused_inbox() {
    let _tall = crate::tests::windows::layout::Dressed::new(egui::vec2(1280.0, 2400.0), "");
    let (mut a, _) = app("notify-other");
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let shown = |ctx: &egui::Context, a: &mut MailApp| {
        frame(ctx, a, vec![]);
        drawn_text(ctx, a, egui::vec2(1280.0, 2400.0)).iter().any(|d| d.text == "Notify of mail in Other too")
    };
    a.ui.backstage = Some(crate::backstage::Backstage::new(crate::backstage::Page::Options));
    assert!(!shown(&ctx, &mut a), "the Focused Inbox off: nothing to say");
    a.run("view.focused", json!({"on": true}));
    assert!(shown(&ctx, &mut a), "with the Focused Inbox on");
    click_named(&ctx, &mut a, "Notify of mail in Other too");
    assert!(a.session.prefs.notify_other, "turned on");
    a.run("view.notifications", json!({"on": false}));
    assert!(!shown(&ctx, &mut a), "no notifications: nothing to say");
}
