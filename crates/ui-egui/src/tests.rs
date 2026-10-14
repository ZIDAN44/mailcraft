//! Headless UI tests: real egui frames over the sample mailbox, in every pane layout and dialog,
//! with keyboard shortcuts sent as input events.

use egui::{Event, Key, Modifiers, RawInput};
use mailcraft_engine::model::dates::Clock;
use mailcraft_engine::model::{Event as MailEvent, MessageKey, Summary, SyncStatus};
use mailcraft_engine::store::{FolderState, Store};
use mailcraft_engine::{ReadingPane, Session, sample};
use serde_json::json;

use crate::{Dialog, MailApp, NullBackend, NullServices};

const NOW: i64 = 1_791_979_200;

fn app(name: &str) -> (MailApp, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("mailcraft-ui-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    sample::seed_if_empty(&store, NOW).unwrap();
    let s = Session::open(Some(store), Clock::new(NOW, 0));
    (MailApp::new(s, Box::new(NullBackend), Box::new(NullServices(NOW))), dir)
}

fn frame(ctx: &egui::Context, app: &mut MailApp, events: Vec<Event>) {
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), events, ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| app.ui(ui));
    // No renderer here: drop the texture uploads deliberately.
    out.textures_delta.clear();
}

fn key(k: Key, modifiers: Modifiers) -> Event {
    Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers }
}

#[test]
fn renders_every_state_without_panicking() {
    let (mut a, dir) = app("states");
    let ctx = egui::Context::default();
    for dark in [Some(false), Some(true)] {
        a.ui.dark = dark;
        frame(&ctx, &mut a, vec![]);
        let k = a.session.list.first().map(|m| m.key.clone()).unwrap();
        a.run("list.open", json!({"key": k.to_id()}));
        frame(&ctx, &mut a, vec![]);
        for pane in [ReadingPane::Bottom, ReadingPane::Off, ReadingPane::Right] {
            a.session.prefs.reading_pane = pane;
            frame(&ctx, &mut a, vec![]);
        }
        a.run("view.conversations", json!({}));
        a.run("view.compact", json!({}));
        frame(&ctx, &mut a, vec![]);
        a.run("view.compact", json!({}));
        a.run("mail.reply", json!({}));
        frame(&ctx, &mut a, vec![]);
        a.run("compose.attach", json!({"name": "a.txt", "data": [1, 2]}));
        a.session.prefs.reading_pane = ReadingPane::Off;
        frame(&ctx, &mut a, vec![]);
        a.session.prefs.reading_pane = ReadingPane::Right;
        a.run("compose.discard", json!({}));
        for d in [
            Dialog::add_account(),
            Dialog::move_to(),
            Dialog::ConfirmDelete { count: 2 },
            Dialog::EmptySubject,
            Dialog::Settings,
            Dialog::RemoveAccount { account: "sample".into(), email: "x".into() },
            Dialog::Signature { account: "sample".into(), text: "sig".into() },
            Dialog::Password { account: "sample".into(), email: "x".into(), password: String::new() },
        ] {
            a.ui.dialog = Some(d);
            frame(&ctx, &mut a, vec![]);
            frame(&ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)]);
            assert!(a.ui.dialog.is_none(), "Escape closes dialogs");
        }
        a.ui.search_text = "storyboard".into();
        a.run("search.run", json!({"query": "storyboard"}));
        frame(&ctx, &mut a, vec![]);
        a.run("search.run", json!({"query": "zzzz-nothing"}));
        frame(&ctx, &mut a, vec![]);
        a.run("search.clear", json!({}));
        a.run("folder.open", json!({"account": "sample", "folder": "Sent"}));
        frame(&ctx, &mut a, vec![]);
        a.run("folder.open", json!({"account": "sample", "folder": "Trash"}));
        frame(&ctx, &mut a, vec![]);
        a.run("folder.open", json!({"account": "sample", "folder": "INBOX"}));
    }
    // Statuses and a notice toast.
    for st in [
        SyncStatus::Connecting,
        SyncStatus::NeedsPassword { error: "no".into() },
        SyncStatus::Error { error: "x".repeat(300) },
        SyncStatus::Online { at: NOW },
    ] {
        a.session.apply(MailEvent::Status { account: "sample".into(), status: st });
        frame(&ctx, &mut a, vec![]);
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn keyboard_shortcuts_run_commands() {
    let (mut a, dir) = app("keys");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![key(Key::ArrowDown, Modifiers::NONE)]);
    let first = a.session.focus.clone().unwrap();
    frame(&ctx, &mut a, vec![key(Key::ArrowDown, Modifiers::NONE)]);
    assert_ne!(a.session.focus.clone().unwrap(), first);
    let was = a.session.selected()[0].flags.flagged;
    frame(&ctx, &mut a, vec![key(Key::Insert, Modifiers::NONE)]);
    assert_eq!(a.session.selected()[0].flags.flagged, !was, "Insert toggles the flag");
    let before = a.session.list.len();
    frame(&ctx, &mut a, vec![key(Key::Delete, Modifiers::NONE)]);
    assert_eq!(a.session.list.len(), before - 1);
    frame(&ctx, &mut a, vec![key(Key::N, Modifiers::COMMAND)]);
    assert!(a.session.compose.is_some());
    // The new message's To box takes focus, so Delete edits text instead of deleting mail.
    frame(&ctx, &mut a, vec![]);
    let n = a.session.list.len();
    frame(&ctx, &mut a, vec![key(Key::Delete, Modifiers::NONE)]);
    assert_eq!(a.session.list.len(), n);
    frame(&ctx, &mut a, vec![key(Key::M, Modifiers::COMMAND | Modifiers::SHIFT)]);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn long_folders_render_quickly() {
    let (mut a, dir) = app("long");
    let store = a.session.store.clone().unwrap();
    let messages: Vec<Summary> = (1..=2000u32)
        .rev()
        .map(|uid| Summary {
            key: MessageKey { account: "sample".into(), folder: "Archive".into(), validity: 1, uid },
            subject: format!("Message {uid}"),
            date: NOW - i64::from(2000 - uid) * 3600,
            preview: "Lorem".into(),
            ..Default::default()
        })
        .collect();
    store.save_folder("sample", "Archive", &FolderState { validity: 1, uid_next: 2001, highest_modseq: 0, messages }).unwrap();
    a.session.apply(MailEvent::FolderChanged { account: "sample".into(), folder: "Archive".into() });
    a.run("folder.open", json!({"account": "sample", "folder": "Archive"}));
    assert_eq!(a.session.list.len(), 2000);
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let t = std::time::Instant::now();
    for _ in 0..10 {
        frame(&ctx, &mut a, vec![]);
    }
    let per = t.elapsed() / 10;
    // Virtualised: only visible rows are laid out (debug builds are slow; this is generous).
    assert!(per.as_millis() < 400, "frame took {per:?}");
    let _ = std::fs::remove_dir_all(dir);
}
