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
    tour();
}

/// A newsletter with a picture behind its heading and a label placed over its poster draws in
/// both themes and in a narrow reading pane (where its columns stack).
#[test]
fn a_banner_and_a_placed_label_draw() {
    let (mut a, dir) = app("banner");
    let ctx = egui::Context::default();
    let k = a.session.list.iter().find(|m| m.subject.contains("winter group show")).map(|m| m.key.clone()).unwrap();
    a.run("list.open", json!({"key": k.to_id()}));
    let p = a.session.reading.as_ref().and_then(|r| r.parsed.clone()).unwrap();
    assert!(p.images.iter().any(|i| i == "cid:banner@gallery.example"), "the banner is one of its pictures: {:?}", p.images);
    for dark in [Some(false), Some(true)] {
        a.ui.dark = dark;
        for pane in [ReadingPane::Right, ReadingPane::Bottom] {
            a.session.prefs.reading_pane = pane;
            for _ in 0..3 {
                frame(&ctx, &mut a, vec![]);
            }
        }
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// People's list and A–Z index were a few pixels wider than the pane, so the pane (shared by
/// every module) grew each frame to its widest and Mail, Calendar and Notes stayed that wide.
#[test]
fn people_keep_the_pane_its_width() {
    let (mut a, dir) = app("pane-width");
    let ctx = egui::Context::default();
    // A panel's id is its parent's (the window's root) salted with its name.
    let mut root = egui::Id::NULL;
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| {
        root = ui.scope_id();
        a.ui(ui);
    });
    out.textures_delta.clear();
    let nav = root.with_salt(egui::IdSalt::new("nav"));
    let width = |ctx: &egui::Context| egui::containers::panel::PanelState::load(ctx, nav).map_or(0.0, |p| p.size().x);
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    let before = width(&ctx);
    a.run("view.people", json!({}));
    for _ in 0..60 {
        frame(&ctx, &mut a, vec![]);
    }
    a.run("view.mail", json!({}));
    frame(&ctx, &mut a, vec![]);
    let after = width(&ctx);
    assert!(before > 0.0 && after <= before + 1.0, "the pane went from {before} to {after}");
    let _ = std::fs::remove_dir_all(dir);
}

/// Dialogs kept the size of the last big one (their buttons' row took all the room left): Add
/// account stayed tall after Server settings closed, Remove account opened as tall as Options.
#[test]
fn dialogs_fit_what_they_show() {
    let (mut a, dir) = app("dialog-size");
    let ctx = egui::Context::default();
    let height = |ctx: &egui::Context, d: &Dialog| {
        // Its own window (or, before, the one all dialogs shared).
        let ids = [egui::Id::unique(("dialog", std::mem::discriminant(d))), egui::Id::unique("dialog")];
        ids.iter().find_map(|id| ctx.memory(|m| m.area_rect(*id))).map_or(0.0, |r| r.height())
    };
    let shown = |a: &mut MailApp, d: Dialog| {
        a.ui.dialog = Some(d);
        for _ in 0..4 {
            frame(&ctx, a, vec![]);
        }
        a.ui.dialog.clone().unwrap()
    };
    let mut d = Dialog::add_account_with("zoe@gmail.com", true);
    let open = height(&ctx, &shown(&mut a, d.clone()));
    if let Dialog::AddAccount(f) = &mut d {
        f.advanced = false;
    }
    let closed = height(&ctx, &shown(&mut a, d));
    assert!(open > 0.0 && closed < open - 60.0, "Server settings closed: {closed} (open: {open})");
    let edit = Dialog::edit_account(&a.session.accounts[0].config);
    shown(&mut a, edit);
    let remove = Dialog::RemoveAccount { account: "sample".into(), email: "x".into() };
    let small = height(&ctx, &shown(&mut a, remove));
    assert!(small > 0.0 && small < 220.0, "Remove account: {small}");
    let _ = std::fs::remove_dir_all(dir);
}

/// Every pane layout, state and dialog, in light and dark (also run in the pseudo-language by
/// the translation tests).
pub(crate) fn tour() {
    static RUN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = RUN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let (mut a, dir) = app(&format!("states-{n}"));
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
        a.run("message.popOut", json!({"key": k.to_id()}));
        frame(&ctx, &mut a, vec![]);
        a.run("message.closeWindow", json!({"key": k.to_id()}));
        backstage_tour(&mut a, &ctx);
        pim_tour(&mut a, &ctx);
        let projects = a.session.folder("sample", "Projects").cloned().unwrap();
        a.session.oauth_available = vec!["google".into()];
        for d in [
            Dialog::add_account(),
            Dialog::move_to(),
            Dialog::ConfirmDelete { count: 2 },
            Dialog::ConfirmDeleteContact { uid: "zoe".into(), name: "Zoe Park".into() },
            Dialog::EmptySubject,
            Dialog::edit_account(&a.session.accounts[0].config),
            Dialog::vacation(
                "sample",
                &mailcraft_engine::model::job::Vacation {
                    enabled: true,
                    subject: "Away".into(),
                    text: "Back soon".into(),
                    from: Some("2026-10-15".into()),
                    until: None,
                    ..Default::default()
                },
                false,
            ),
            Dialog::vacation(
                "sample",
                &mailcraft_engine::model::job::Vacation {
                    enabled: true,
                    text: "Back soon".into(),
                    external: Some("Out of office".into()),
                    outside: "known".into(),
                    ..Default::default()
                },
                true,
            ),
            Dialog::ConfirmEmpty { account: "sample".into(), folder: "Trash".into(), name: "Deleted Items".into(), for_good: true },
            Dialog::ConfirmEmpty { account: "sample".into(), folder: "INBOX".into(), name: "Inbox".into(), for_good: false },
            Dialog::FolderProperties {
                name: "Inbox".into(),
                account: "riley@studio.example".into(),
                location: "INBOX".into(),
                total: 7,
                unread: 3,
                size: 20_480,
            },
            Dialog::OpenSharedCalendar { email: "ana@contoso.example".into(), error: Some("no access".into()) },
            Dialog::AddSharedMailbox { account: "sample".into(), address: "sales@contoso.example".into(), error: Some("already open".into()) },
            Dialog::RemoveAccount { account: "sample".into(), email: "x".into() },
            Dialog::MailboxClosed {
                account: "sample".into(),
                email: "zoe@hotmail.com".into(),
                error: format!("{}. Turn on IMAP on Outlook.com.", mailcraft_engine::model::MICROSOFT_IMAP_CLOSED),
            },
            Dialog::add_account_with("zoe@hotmail.com", true),
            Dialog::Signature(Box::new(crate::dialogs::SignatureForm::new(&a.session, "sample", false))),
            Dialog::Signature(Box::new(crate::dialogs::SignatureForm::new(&a.session, "sample", true))),
            Dialog::Password { account: "sample".into(), email: "x".into(), password: String::new() },
            Dialog::new_folder("sample", Some("Projects")),
            Dialog::new_folder("sample", None),
            Dialog::rename_folder(&projects),
            Dialog::move_folder(&projects),
            Dialog::delete_folder(&a.session, &projects),
            Dialog::NewCategory { name: "Clients".into(), color: 3, error: Some("taken".into()) },
            Dialog::Rules,
            Dialog::rule_from("zoe@studio.example", "x"),
            Dialog::add_account_with("zoe@gmail.com", true),
            jmap_form(),
            graph_form(),
            pop_form(),
            Dialog::Preview {
                title: "notes.txt".into(),
                header: vec![("From".into(), "Zoe".into())],
                text: "line one
line two"
                    .into(),
            },
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
    // The Outbox: its folder row, list and reading pane.
    let store = a.session.store.clone().unwrap();
    let item = mailcraft_engine::store::OutboxItem {
        id: 5,
        from: "riley@studio.example".into(),
        recipients: vec!["ann@x.example".into()],
        queued: NOW,
        error: Some("network: refused".into()),
        ..Default::default()
    };
    store.outbox_add("sample", &item, b"From: riley@studio.example\r\nTo: ann@x.example\r\nSubject: Waiting\r\n\r\nLater.\r\n").unwrap();
    a.session.refresh_outbox();
    frame(&ctx, &mut a, vec![]);
    a.run("folder.outbox", json!({}));
    let k = a.session.list.first().map(|m| m.key.clone()).unwrap();
    a.run("list.open", json!({"key": k.to_id()}));
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.session.reading.as_ref().and_then(|r| r.parsed.as_ref()).map(|p| p.subject.as_str()), Some("Waiting"));
    a.run("folder.open", json!({"account": "sample", "folder": "INBOX"}));
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

fn jmap_form() -> Dialog {
    let mut d = Dialog::add_account_with("zoe@fastmail.example", true);
    if let Dialog::AddAccount(f) = &mut d {
        f.jmap = true;
        f.jmap_url = " https://api.fastmail.example/jmap/session".into();
        f.password = "pw".into();
    }
    d
}

fn pop_form() -> Dialog {
    let mut d = Dialog::add_account_with("ana@example.org", true);
    if let Dialog::AddAccount(f) = &mut d {
        f.imap_host = "imap.example.org".into();
        f.smtp_host = "smtp.example.org".into();
        f.pop = true;
        f.pop_host = "pop.example.org".into();
        f.pop_port = "995".into();
        f.pop_security = mailcraft_engine::model::Security::Tls;
        f.pst = "C:/Mail/old.pst".into();
        f.password = "pw".into();
    }
    d
}

fn graph_form() -> Dialog {
    let mut d = Dialog::add_account_with("ana@contoso.example", true);
    if let Dialog::AddAccount(f) = &mut d {
        f.graph = true;
    }
    d
}

#[test]
fn the_account_form_adds_exchange_accounts() {
    let (mut a, dir) = app("graph-form");
    let p = match graph_form() {
        Dialog::AddAccount(f) => f.params(),
        _ => serde_json::Value::Null,
    };
    assert_eq!((p["graph"].clone(), p.get("password")), (serde_json::json!(true), None), "Microsoft sign-in, no password");
    a.session.oauth_available = vec!["microsoft".into()];
    let mut p = p;
    p["oauth"] = serde_json::json!("microsoft");
    a.act(crate::Action::Submit("account.add", p));
    let added = a.session.accounts.iter().find(|x| x.config.email == "ana@contoso.example").unwrap();
    assert!(added.config.graph.is_some() && added.config.oauth.is_some());
    let _ = std::fs::remove_dir_all(dir);
}

/// The browser's services: a popped-out message goes to a browser window of its own.
struct PageServices(std::rc::Rc<std::cell::RefCell<Vec<String>>>);

impl crate::Services for PageServices {
    fn clock(&self) -> (i64, i32) {
        (NOW, 0)
    }
    fn pick_files(&mut self) -> Vec<std::path::PathBuf> {
        Vec::new()
    }
    fn save_path(&mut self, _: &str) -> Option<std::path::PathBuf> {
        None
    }
    fn open_path(&mut self, _: &std::path::Path) -> Result<(), String> {
        Err("no".into())
    }
    fn open_message_window(&mut self, key: &str) -> bool {
        self.0.borrow_mut().push(key.to_string());
        true
    }
    fn close_page(&mut self) {
        self.0.borrow_mut().push("closed".into());
    }
}

#[test]
fn in_the_browser_a_popped_out_message_opens_in_a_window_of_its_own() {
    let (a, dir) = app("tabs");
    let opened = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let mut a = MailApp::new(a.session, Box::new(NullBackend), Box::new(PageServices(opened.clone())));
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let k = a.session.list[0].key.clone();
    a.run("message.popOut", json!({"key": k.to_id()}));
    frame(&ctx, &mut a, vec![]);
    assert_eq!(*opened.borrow(), [k.to_id()], "a browser window, not one drawn in the page");
    assert!(a.session.windows.is_empty());
    // The new window shows only that message; done with, it closes.
    let mut tab = MailApp::new(a.session, Box::new(NullBackend), Box::new(PageServices(opened.clone())));
    tab.show_message_page(&k.to_id());
    frame(&ctx, &mut tab, vec![]);
    assert_eq!(tab.ui.page_message.as_ref(), Some(&k));
    assert_eq!(tab.session.windows.len(), 1, "shown in place, not opened again");
    tab.act(crate::Action::Run("message.closeWindow", json!({"key": k.to_id()})));
    frame(&ctx, &mut tab, vec![]);
    assert_eq!(opened.borrow().last().map(String::as_str), Some("closed"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn double_clicked_mail_opens_in_its_own_window() {
    let (mut a, dir) = app("windows");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let k = a.session.list[0].key.clone();
    a.run("message.popOut", json!({"key": k.to_id()}));
    // Without a native backend the window is drawn inside the main one.
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.session.windows.len(), 1);
    // Its toolbar acts on its own message: Reply opens the reply in the main window and closes it.
    a.act(crate::Action::Open(k.clone()));
    a.act(crate::Action::Run("mail.reply", json!({})));
    a.act(crate::Action::Run("message.closeWindow", json!({"key": k.to_id()})));
    assert!(a.session.compose.is_some() && a.session.windows.is_empty());
    // Escape closes a window.
    a.run("compose.discard", json!({}));
    a.run("message.popOut", json!({"key": k.to_id()}));
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)]);
    frame(&ctx, &mut a, vec![]);
    assert!(a.session.windows.is_empty(), "Escape closes the window");
    let _ = std::fs::remove_dir_all(dir);
}

/// The File page: every page, in the tour's languages and themes.
pub(crate) fn backstage_tour(a: &mut MailApp, ctx: &egui::Context) {
    for page in [crate::backstage::Page::Info, crate::backstage::Page::Open, crate::backstage::Page::Options, crate::backstage::Page::About] {
        a.act(crate::Action::Backstage(page));
        frame(ctx, a, vec![]);
    }
    a.ui.backstage = None;
}

/// Calendar (each view, an event open) and People (someone picked, the form open).
pub(crate) fn pim_tour(a: &mut MailApp, ctx: &egui::Context) {
    a.run("view.module", json!({"name": "calendar"}));
    for mode in ["day", "week", "month"] {
        a.run("calendar.view", json!({"mode": mode}));
        frame(ctx, a, vec![]);
    }
    let item =
        a.session.agenda(a.session.local_midnight(a.session.cal_days().0), a.session.local_midnight(a.session.cal_days().1)).into_iter().next();
    if let Some(it) = item {
        a.ui.dialog = Dialog::open_event(&a.session, &it.uid, it.occurrence);
        frame(ctx, a, vec![]);
    }
    a.ui.dialog = Some(Dialog::new_event(&a.session, None));
    frame(ctx, a, vec![]);
    a.ui.dialog = None;
    a.run("view.module", json!({"name": "people"}));
    frame(ctx, a, vec![]);
    a.ui.person = a.session.people("").first().map(|p| p.card.uid.clone());
    frame(ctx, a, vec![]);
    if let Some(p) = a.session.people("").first() {
        a.ui.dialog = Some(Dialog::edit_contact(&p.card));
        frame(ctx, a, vec![]);
    }
    a.ui.dialog = None;
    a.ui.people_query = "nobody-at-all".into();
    frame(ctx, a, vec![]);
    a.ui.people_query.clear();
    a.run("view.module", json!({"name": "mail"}));
    frame(ctx, a, vec![]);
}

#[test]
fn calendar_and_people_forms_save_through_commands() {
    let (mut a, dir) = app("pim-forms");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    pim_tour(&mut a, &ctx);
    // A new event from the form.
    let mut d = Dialog::new_event(&a.session, Some(NOW + 3 * 86_400));
    let params = match &mut d {
        Dialog::Event(f) => {
            f.summary = "Grade the trailer".into();
            f.location = "Suite B".into();
            f.repeat = "weekly".into();
            f.params()
        }
        _ => serde_json::Value::Null,
    };
    a.ui.dialog = Some(d);
    a.act(crate::Action::Submit("event.create", params));
    assert!(a.ui.dialog.is_none(), "saved and closed");
    let (from, to) = (NOW, NOW + 30 * 86_400);
    let made: Vec<_> = a.session.agenda(from, to).into_iter().filter(|i| i.summary == "Grade the trailer").collect();
    assert!(made.len() >= 4 && made.iter().all(|i| i.recurring && i.location == "Suite B"), "{made:?}");
    // Reopened, it shows what was saved; a bad time keeps the form open with the reason.
    let mut d = Dialog::open_event(&a.session, &made[0].uid, made[0].occurrence).unwrap();
    let params = match &mut d {
        Dialog::Event(f) => {
            assert_eq!((f.summary.as_str(), f.repeat.as_str(), f.recurring), ("Grade the trailer", "weekly", true));
            f.start = "25:00".into();
            f.params()
        }
        _ => serde_json::Value::Null,
    };
    a.ui.dialog = Some(d);
    a.act(crate::Action::Submit("event.update", params));
    assert!(matches!(&a.ui.dialog, Some(Dialog::Event(f)) if f.error.is_some()));
    // A person from the form is picked afterwards.
    a.ui.dialog = Some(Dialog::new_contact());
    let mut f = crate::pim_dialogs::ContactForm { name: "Mira Chen".into(), ..Default::default() };
    f.emails[0] = "mira@colour.example".into();
    f.phones[2] = "+32 490 12 34 56".into();
    f.addresses[1].city = "Gent".into();
    f.birthday = "1990-07-12".into();
    a.act(crate::Action::Submit("contact.create", f.params()));
    let picked = a.ui.person.clone().unwrap();
    let card = a.session.find_person(&picked).unwrap().card;
    assert_eq!(card.display(), "Mira Chen");
    // Issue #1: each number keeps its kind, the address and birthday their fields, and the form
    // shows them where they were.
    assert_eq!((card.phones[0].kind.as_str(), card.addresses[0].kind.as_str(), card.birthday.as_str()), ("cell", "home", "1990-07-12"));
    let back = crate::pim_dialogs::ContactForm::edit(&card);
    assert_eq!((back.phones[2].as_str(), back.addresses[1].city.as_str(), back.address_shown), ("+32 490 12 34 56", "Gent", 1));
    f.emails[0] = "not an address".into();
    a.ui.dialog = Some(Dialog::new_contact());
    a.act(crate::Action::Submit("contact.create", f.params()));
    assert!(matches!(&a.ui.dialog, Some(Dialog::Contact(f)) if f.error.is_some()));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_file_page_shows_accounts_replies_space_and_options() {
    let (mut a, dir) = app("backstage");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    backstage_tour(&mut a, &ctx);
    // A server account: its automatic replies are asked for once, and shown when they arrive.
    let id = a.session.run("account.add", &json!({"email": "zoe@example.org", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    a.session.take_jobs();
    a.act(crate::Action::Backstage(crate::backstage::Page::Info));
    if let Some(b) = a.ui.backstage.as_mut() {
        b.account = Some(id.clone());
    }
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    assert!(matches!(a.session.account(&id).unwrap().vacation, mailcraft_engine::VacationState::Loading), "asked the server");
    a.session.apply(mailcraft_engine::model::Event::Vacation {
        account: id.clone(),
        vacation: Some(mailcraft_engine::model::job::Vacation {
            enabled: true,
            text: "Away".into(),
            until: Some("2026-10-20".into()),
            ..Default::default()
        }),
        error: None,
    });
    a.session.apply(mailcraft_engine::model::Event::Quota { account: id.clone(), used: 3 << 30, limit: 15 << 30 });
    frame(&ctx, &mut a, vec![]);
    // The replies dialog saves through the command; bad dates are reported in it.
    let mut d = Dialog::vacation(&id, &Default::default(), false);
    if let Dialog::Vacation(f) = &mut d {
        f.enabled = true;
        f.text = "Back Monday".into();
        f.dated = true;
        f.until = "20 October".into();
    }
    a.ui.dialog = Some(d);
    let p = match &a.ui.dialog {
        Some(Dialog::Vacation(f)) => f.params(),
        _ => serde_json::Value::Null,
    };
    a.act(crate::Action::Submit("account.setVacation", p));
    assert!(matches!(&a.ui.dialog, Some(Dialog::Vacation(f)) if f.error.as_deref().is_some_and(|e| e.contains("not a date"))));
    frame(&ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)]);
    // Escape leaves the File page too.
    frame(&ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)]);
    assert!(a.ui.backstage.is_none() && a.ui.dialog.is_none());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_account_form_adds_jmap_accounts() {
    let (mut a, dir) = app("jmap-form");
    let p = match jmap_form() {
        Dialog::AddAccount(f) => f.params(),
        _ => serde_json::Value::Null,
    };
    assert_eq!(p["jmap"], "https://api.fastmail.example/jmap/session");
    assert!(p.get("imapHost").is_none(), "no IMAP servers for a JMAP account");
    a.act(crate::Action::Submit("account.add", p));
    let added = a.session.accounts.iter().find(|x| x.config.email == "zoe@fastmail.example").unwrap();
    assert!(added.config.jmap.is_some() && !added.is_local());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_account_form_adds_pop_accounts_with_their_old_data_file() {
    let (mut a, dir) = app("pop-form");
    let p = match pop_form() {
        Dialog::AddAccount(f) => f.params(),
        _ => serde_json::Value::Null,
    };
    assert_eq!((p["pop"].clone(), p["popHost"].clone(), p["popPort"].clone()), (json!(true), json!("pop.example.org"), json!(995)));
    assert_eq!((p["popDelete"].clone(), p["pst"].clone()), (json!(false), json!("C:/Mail/old.pst")), "a copy stays on the server");
    assert_eq!((p["popKeepDays"].clone(), p["popDeleteWithTrash"].clone()), (json!(14), json!(false)), "for 14 days, as Outlook's default");
    assert!(p.get("imapHost").is_none(), "no IMAP server for a POP account");
    a.act(crate::Action::Submit("account.add", p));
    let added = a.session.accounts.iter().find(|x| x.config.email == "ana@example.org").unwrap();
    assert!(added.config.pop.is_some() && !added.is_local());
    assert_eq!(added.config.pop_keep_days, Some(14));
    // Its server settings edit the POP server, and when the server's copy goes.
    let Dialog::EditAccount(mut e) = Dialog::edit_account(&added.config) else { panic!() };
    assert_eq!(e.params()["popHost"], "pop.example.org");
    (e.pop_keep.for_days, e.pop_keep.until_deleted) = (false, true);
    assert_eq!((e.params()["popKeepDays"].clone(), e.params()["popDeleteWithTrash"].clone()), (json!(0), json!(true)));
    e.pop_keep.keep = false;
    assert_eq!((e.params()["popDelete"].clone(), e.params()["popDeleteWithTrash"].clone()), (json!(true), json!(false)));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn folder_dialogs_create_rename_and_report_errors() {
    let (mut a, dir) = app("folders");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.ui.dialog = Some(Dialog::new_folder("sample", None));
    frame(&ctx, &mut a, vec![]);
    if let Some(Dialog::FolderName { name, .. }) = a.ui.dialog.as_mut() {
        *name = "Clients".into();
    }
    frame(&ctx, &mut a, vec![key(Key::Enter, Modifiers::NONE)]);
    assert!(a.session.folder("sample", "Clients").is_some());
    assert!(a.ui.dialog.is_none(), "the dialog closes once the folder exists");
    // A duplicate keeps the dialog open with the error.
    a.act(crate::Action::Dialog(Dialog::new_folder("sample", None)));
    a.act(crate::Action::Submit("folder.create", json!({"account": "sample", "name": "Clients"})));
    assert!(matches!(&a.ui.dialog, Some(Dialog::FolderName { error: Some(e), .. }) if e.contains("already")));
    frame(&ctx, &mut a, vec![]);
    // Favorites show the new folder once pinned.
    a.run("folder.favorite", json!({"account": "sample", "folder": "Clients"}));
    frame(&ctx, &mut a, vec![]);
    assert!(a.session.favorites().iter().any(|f| f.folder == "Clients"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn recipient_boxes_suggest_people() {
    let (mut a, dir) = app("suggest");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![Event::Text("zo".into())]);
    assert_eq!(a.session.compose.as_ref().unwrap().to, "zo");
    assert_eq!(a.ui.suggest_field, Some("to"));
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![key(Key::Enter, Modifiers::NONE)]);
    assert_eq!(a.session.compose.as_ref().unwrap().to, "Zoe Park <zoe@studio.example>, ");
    // Picked with the mouse: the box keeps the cursor after the click, so the next address is
    // typed straight in.
    frame(&ctx, &mut a, vec![Event::Text("sh".into())]);
    frame(&ctx, &mut a, vec![]);
    let list =
        ctx.memory(|m| m.areas().visible_layer_ids().into_iter().filter(|l| l.order == egui::Order::Foreground).find_map(|l| m.area_rect(l.id)));
    let row = list.map(|r| r.left_top() + egui::vec2(20.0, 16.0)).expect("the suggestions show");
    let button = |pressed| Event::PointerButton { pos: row, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    frame(&ctx, &mut a, vec![Event::PointerMoved(row)]);
    frame(&ctx, &mut a, vec![button(true)]);
    frame(&ctx, &mut a, vec![button(false)]);
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![Event::Text("x".into())]);
    let to = a.session.compose.as_ref().unwrap().to.clone();
    assert!(to.starts_with("Zoe Park <zoe@studio.example>, ") && to.ends_with(">, x") && to.matches('@').count() == 2, "{to}");
    a.session.compose.as_mut().unwrap().to = "Zoe Park <zoe@studio.example>, ".into();

    // The next address starts fresh; Escape hides the list.
    frame(&ctx, &mut a, vec![Event::Text("sh".into())]);
    frame(&ctx, &mut a, vec![key(Key::ArrowDown, Modifiers::NONE), key(Key::Escape, Modifiers::NONE)]);
    frame(&ctx, &mut a, vec![key(Key::Enter, Modifiers::NONE)]);
    assert!(a.session.compose.as_ref().unwrap().to.ends_with(", sh"), "{}", a.session.compose.as_ref().unwrap().to);

    let _ = std::fs::remove_dir_all(dir);
}

/// Services that remember the notifications shown.
struct Recorder(std::sync::Arc<std::sync::Mutex<Vec<(String, String)>>>);

impl crate::Services for Recorder {
    fn clock(&self) -> (i64, i32) {
        (NOW, 0)
    }
    fn pick_files(&mut self) -> Vec<std::path::PathBuf> {
        Vec::new()
    }
    fn save_path(&mut self, _: &str) -> Option<std::path::PathBuf> {
        None
    }
    fn open_path(&mut self, _: &std::path::Path) -> Result<(), String> {
        Ok(())
    }
    fn notify(&mut self, title: &str, body: &str) {
        self.0.lock().unwrap().push((title.into(), body.into()));
    }
    fn clipboard_picture(&mut self) -> Option<Vec<u8>> {
        Some(mailcraft_engine::sample::sample_png())
    }
    fn can_drag_out(&self) -> bool {
        true
    }
}

#[test]
fn attachments_drag_out_as_files() {
    let (mut a, dir) = app("drag-out");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let k = a.session.list.iter().find(|m| m.has_attachments).map(|m| m.key.clone()).unwrap();
    a.run("list.open", json!({"key": k.to_id()}));
    frame(&ctx, &mut a, vec![]);
    // Not offered where the system can't take the file.
    a.act(crate::Action::DragOut(0));
    assert!(a.take_drag_out().is_none());
    a.services = Box::new(Recorder(Default::default()));
    a.ui.can_drag_out = true;
    let index = a.session.reading.as_ref().and_then(|r| r.parsed.as_ref()).and_then(|p| p.attachments.first()).map(|x| x.index).unwrap();
    a.act(crate::Action::DragOut(index));
    let path = a.take_drag_out().expect("a file to drag");
    assert!(path.is_file() && path.metadata().unwrap().len() > 0, "{path:?}");
    let _ = std::fs::remove_dir_all(dir);
}

/// A message's own window: its attachment buttons work on its message, not the one in the
/// reading pane (they used to act on the reading pane's, and fail).
#[test]
fn a_message_windows_attachments_are_its_own() {
    let (mut a, dir) = app("window-attachments");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.services = Box::new(Recorder(Default::default()));
    a.ui.can_drag_out = true;
    // The reading pane shows a message without attachments; a window, one with.
    let plain = a.session.list.iter().find(|m| !m.has_attachments).map(|m| m.key.clone()).unwrap();
    let with = a.session.list.iter().find(|m| m.has_attachments).map(|m| m.key.clone()).unwrap();
    a.run("message.popOut", json!({"key": with.to_id()}));
    a.run("list.open", json!({"key": plain.to_id()}));
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.session.reading.as_ref().map(|r| r.key.clone()), Some(plain.clone()));
    let w = a.session.windows.iter().find(|w| w.key == with).cloned().expect("the window");
    let index = w.parsed.as_ref().and_then(|p| p.attachments.first()).map(|x| x.index).unwrap();
    a.act(crate::Action::InWindow(with.clone(), Box::new(crate::Action::DragOut(index))));
    let path = a.take_drag_out().expect("the window's attachment, as a file");
    let name = w.parsed.as_ref().and_then(|p| p.attachments.first()).map(|x| x.name.clone()).unwrap();
    assert!(path.is_file() && path.file_name().is_some_and(|n| n.to_string_lossy() == name), "{path:?}");
    assert_eq!(a.session.reading.as_ref().map(|r| r.key.clone()), Some(plain), "the reading pane is as it was");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_editor_formats_as_you_see_it_with_outlooks_keys() {
    use mailcraft_engine::mime::rich::{Align, List, PICTURE};
    let (mut a, dir) = app("editor");
    a.services = Box::new(Recorder(Default::default()));
    a.session.prefs.new_mail_window = false;
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    a.run("compose.update", json!({"to": "ann@x.example", "subject": "Plan", "body": ""}));
    frame(&ctx, &mut a, vec![]);
    let id = crate::rich_edit::body_id(a.session.compose.as_ref().unwrap());
    let compose_id = a.session.compose.as_ref().unwrap().id;
    ctx.memory_mut(|m| m.request_focus(id));
    frame(&ctx, &mut a, vec![]);
    let c = |a: &MailApp| a.session.compose.clone().unwrap();
    // Bold on, then typing: bold text, and no marks in it.
    frame(&ctx, &mut a, vec![key(Key::B, Modifiers::COMMAND)]);
    frame(&ctx, &mut a, vec![Event::Text("Hello".into())]);
    assert_eq!(c(&a).body, "Hello");
    assert!(c(&a).styles.style(0).bold && c(&a).styles.style(4).bold);
    // Off again: plain.
    frame(&ctx, &mut a, vec![key(Key::B, Modifiers::COMMAND)]);
    frame(&ctx, &mut a, vec![Event::Text(" world".into())]);
    assert_eq!(c(&a).body, "Hello world");
    assert!(!c(&a).styles.style(7).bold);
    // Select "Hello" and underline it.
    let mut state = egui::TextEdit::load_state(&ctx, id).unwrap_or_default();
    state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(5))));
    state.store(&ctx, id);
    frame(&ctx, &mut a, vec![key(Key::U, Modifiers::COMMAND)]);
    assert!(c(&a).styles.style(2).underline && !c(&a).styles.style(7).underline);
    // Bullets (Ctrl+Shift+L), then undo (Ctrl+Z) and redo (Ctrl+Y).
    frame(&ctx, &mut a, vec![key(Key::L, Modifiers::COMMAND | Modifiers::SHIFT)]);
    assert_eq!(c(&a).styles.para(0).list, List::Bullet);
    frame(&ctx, &mut a, vec![key(Key::Z, Modifiers::COMMAND)]);
    assert_eq!(c(&a).styles.para(0).list, List::None);
    frame(&ctx, &mut a, vec![key(Key::Y, Modifiers::COMMAND)]);
    assert_eq!(c(&a).styles.para(0).list, List::Bullet);
    // Ctrl+R aligns right while writing (it doesn't reply), Ctrl+E centres.
    frame(&ctx, &mut a, vec![key(Key::R, Modifiers::COMMAND)]);
    assert_eq!(c(&a).id, compose_id, "still the same message");
    assert_eq!(c(&a).styles.para(0).align, Align::Right);
    frame(&ctx, &mut a, vec![key(Key::E, Modifiers::COMMAND)]);
    assert_eq!(c(&a).styles.para(0).align, Align::Center);
    // Bigger (Ctrl+]), on "Hello" selected again (undo leaves the cursor collapsed).
    let mut state = egui::TextEdit::load_state(&ctx, id).unwrap_or_default();
    state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(5))));
    state.store(&ctx, id);
    frame(&ctx, &mut a, vec![key(Key::CloseBracket, Modifiers::COMMAND)]);
    assert_eq!(c(&a).styles.style(0).size(), 12);
    // Sent as HTML with what was seen.
    let (text, html, _) = c(&a).bodies();
    let html = html.unwrap();
    assert!(text.starts_with("• Hello world"), "{text}");
    assert!(html.contains("<ul>") && html.contains("<u>Hello</u>") && html.contains("text-align:center"), "{html}");
    // Ctrl+Shift+B: the address book.
    frame(&ctx, &mut a, vec![key(Key::B, Modifiers::COMMAND | Modifiers::SHIFT)]);
    frame(&ctx, &mut a, vec![]);
    assert!(matches!(a.ui.dialog, Some(Dialog::SelectNames(_))));
    a.ui.dialog = None;
    // A pasted picture goes in at the cursor.
    a.act(crate::Action::PastePicture);
    frame(&ctx, &mut a, vec![]);
    assert!(c(&a).body.contains(PICTURE) && c(&a).inline.len() == 1);
    a.run("compose.format", json!({"plain": true}));
    frame(&ctx, &mut a, vec![]);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_address_book_adds_people_to_to_cc_and_bcc() {
    let (mut a, dir) = app("names");
    let ctx = egui::Context::default();
    a.session.prefs.new_mail_window = false;
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    frame(&ctx, &mut a, vec![]);
    let c = a.session.compose.clone().unwrap();
    a.ui.dialog = Some(Dialog::select_names(&c, "to"));
    frame(&ctx, &mut a, vec![]);
    let Some(Dialog::SelectNames(f)) = a.ui.dialog.as_mut() else { panic!("no dialog") };
    assert!(f.list.iter().any(|x| x.email == "zoe@studio.example"), "everyone mail was exchanged with: {:?}", f.list);
    f.query = "zoe".into();
    f.stale = true;
    frame(&ctx, &mut a, vec![]);
    let Some(Dialog::SelectNames(f)) = a.ui.dialog.as_mut() else { panic!("no dialog") };
    assert!(!f.list.is_empty() && f.list.iter().all(|x| x.email.contains("zoe") || x.name.to_lowercase().contains("zoe")));
    f.to = "Zoe Park <zoe@studio.example>; ".into();
    f.cc = "ann@x.example".into();
    // OK: the fields go to the message.
    let p = json!({"to": f.to.trim(), "cc": f.cc.trim(), "bcc": ""});
    a.act(crate::Action::Submit("compose.update", p));
    let c = a.session.compose.clone().unwrap();
    assert!(c.to.contains("zoe@studio.example") && c.cc == "ann@x.example");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn new_mail_is_announced_and_counted_in_the_title() {
    let (mut a, dir) = app("announce");
    let shown = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    a.services = Box::new(Recorder(shown.clone()));
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(a.ui.title_shown, "(3) Inbox – MailCraft");
    assert!(shown.lock().unwrap().is_empty(), "the mail already there isn't news");
    let m = |uid: u32, who: &str, subject: &str| Summary {
        key: MessageKey { account: "sample".into(), folder: "INBOX".into(), validity: 1, uid },
        from: mailcraft_engine::model::EmailAddress::new(who, "x@y.example"),
        subject: subject.into(),
        date: NOW - 5,
        ..Default::default()
    };
    a.session.new_mail = vec![m(90, "Zoe Park", "Lunch?")];
    frame(&ctx, &mut a, vec![]);
    a.session.new_mail = vec![m(91, "Ann", "a"), m(92, "Bo", "b"), m(93, "Ann", "c")];
    frame(&ctx, &mut a, vec![]);
    let got = shown.lock().unwrap().clone();
    assert_eq!(got[0], ("Zoe Park".to_string(), "Lunch?".to_string()));
    assert_eq!(got[1], ("3 new messages".to_string(), "From Ann and Bo".to_string()));
    let _ = std::fs::remove_dir_all(dir);
}

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
    store.save_folder("sample", "Archive", &FolderState { validity: 1, uid_next: 2001, highest_modseq: 0, messages, window: 0 }).unwrap();
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

/// The browser's services: no files, so downloads and a picker that answers later.
#[derive(Default)]
struct WebLike {
    downloads: std::rc::Rc<std::cell::RefCell<Vec<(String, usize)>>>,
    picked: Vec<(String, String, Vec<u8>)>,
    asked: std::rc::Rc<std::cell::RefCell<Vec<String>>>,
}

impl crate::Services for WebLike {
    fn clock(&self) -> (i64, i32) {
        (NOW, 0)
    }
    fn pick_files(&mut self) -> Vec<std::path::PathBuf> {
        panic!("the browser can't pick files at once")
    }
    fn save_path(&mut self, _: &str) -> Option<std::path::PathBuf> {
        panic!("the browser has no paths")
    }
    fn open_path(&mut self, _: &std::path::Path) -> Result<(), String> {
        panic!("the browser has no paths")
    }
    fn downloads(&self) -> bool {
        true
    }
    fn download(&mut self, name: &str, bytes: &[u8]) {
        self.downloads.borrow_mut().push((name.to_string(), bytes.len()));
    }
    fn pick_later(&mut self, purpose: &str) -> bool {
        self.asked.borrow_mut().push(purpose.to_string());
        self.picked.push((purpose.to_string(), "notes.txt".into(), b"remember".to_vec()));
        true
    }
    fn take_picked(&mut self) -> Vec<(String, String, Vec<u8>)> {
        std::mem::take(&mut self.picked)
    }
}

#[test]
fn in_the_browser_files_download_and_attachments_come_from_the_picker() {
    let store = Store::memory();
    sample::seed_if_empty(&store, NOW).unwrap();
    let s = Session::open(Some(store), Clock::new(NOW, 0));
    let web = WebLike::default();
    let (downloads, asked) = (web.downloads.clone(), web.asked.clone());
    let mut a = MailApp::new(s, Box::new(NullBackend), Box::new(web));
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let key = a.session.list.iter().find(|m| m.has_attachments).map(|m| m.key.clone()).unwrap();
    a.session.open_message(&key);
    frame(&ctx, &mut a, vec![]);
    let first =
        a.session.reading.as_ref().and_then(|r| r.parsed.as_ref()).and_then(|p| p.attachments.iter().find(|x| !x.inline)).map(|x| x.index).unwrap();
    for act in [crate::Action::SaveMessage, crate::Action::ExportPdf, crate::Action::SaveAttachment(first), crate::Action::SaveAll] {
        a.act(act);
    }
    let got = downloads.borrow().clone();
    assert!(got.len() >= 4, "{got:?}");
    assert!(got[0].0.ends_with(".eml") && got[1].0.ends_with(".pdf") && got.iter().all(|(_, n)| *n > 0), "{got:?}");

    // Only JMAP accounts: the form asks for the session address, and nothing else is accepted.
    a.session.jmap_only = true;
    a.ui.dialog = Some(Dialog::add_account_with("zoe@fastmail.example", false));
    frame(&ctx, &mut a, vec![]);
    assert!(matches!(&a.ui.dialog, Some(Dialog::AddAccount(f)) if f.jmap && f.advanced));
    a.ui.dialog = None;
    assert!(a.session.run("account.add", &json!({"email": "zoe@example.com", "password": "pw", "imapHost": "imap.example.com"})).is_err());

    a.run("mail.new", json!({}));
    a.act(crate::Action::AttachFiles);
    frame(&ctx, &mut a, vec![]);
    assert_eq!(asked.borrow().as_slice(), ["attach"]);
    assert!(a.session.compose.as_ref().is_some_and(|c| c.attachments.iter().any(|x| x.name == "notes.txt")));
}

#[test]
fn new_mail_writes_in_its_own_window_and_replies_pop_out() {
    let (mut a, dir) = app("compose-window");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("mail.new", json!({}));
    frame(&ctx, &mut a, vec![]);
    assert!(a.session.compose.as_ref().is_some_and(|c| c.window));
    a.run("compose.update", json!({"to": "zoe@studio.example", "subject": "Poster sizes"}));
    frame(&ctx, &mut a, vec![]);
    // Back into the reading pane, out again, then the window closes into Drafts.
    a.act(crate::Action::Run("compose.popIn", json!({})));
    frame(&ctx, &mut a, vec![]);
    a.act(crate::Action::Run("compose.popOut", json!({})));
    frame(&ctx, &mut a, vec![]);
    a.act(crate::Action::Run("compose.close", json!({})));
    frame(&ctx, &mut a, vec![]);
    assert!(a.session.compose.is_none());
    a.run("folder.open", json!({"account": "sample", "folder": "Drafts"}));
    assert!(a.session.list.iter().any(|m| m.subject == "Poster sizes"));
    // A reply starts inline, with the original under a rule.
    a.run("folder.open", json!({"account": "sample", "folder": "INBOX"}));
    let k = a.session.list.first().map(|m| m.key.clone()).unwrap();
    a.run("list.open", json!({"key": k.to_id()}));
    a.run("mail.reply", json!({}));
    frame(&ctx, &mut a, vec![]);
    let c = a.session.compose.as_ref().unwrap();
    assert!(!c.window && c.body.contains("\nSent: "));
    let _ = std::fs::remove_dir_all(dir);
}

/// Fonts built in from craft-fonts (`CRAFT_FONTS_DIR`) are the last fallbacks, with their
/// licences; without them nothing changes.
#[test]
fn craft_fonts_are_the_last_fallbacks() {
    let ctx = egui::Context::default();
    // (Not install_fonts: that notes the main font's coverage, which other tests share.)
    let mut defs = egui::FontDefinitions::default();
    crate::theme::add_craft_fallbacks(&mut defs);
    ctx.set_fonts(defs);
    ctx.run_ui(egui::RawInput::default(), |_| {}).textures_delta.clear();
    let fonts = crate::craft_fonts::CRAFT_FONTS;
    for f in fonts {
        assert!(!f.bytes.is_empty() && f.licence_text.len() > 100, "{} comes with its licence", f.family);
    }
    // Arabic is drawn (from Noto Sans Arabic) when the build has it; nothing breaks when it hasn't.
    let arabic = ctx.fonts_mut(|f| f.has_glyphs(&egui::FontId::proportional(14.0), "مرحبا"));
    assert_eq!(arabic, crate::craft_fonts::has_script("Arab"));
}

/// A remembered window place is used only when it's sensible: a real size, roughly on a screen.
#[test]
fn window_places_open_only_where_they_make_sense() {
    use crate::WindowPlace;
    let p = |x: f32, y: f32, w: f32, h: f32| WindowPlace { x, y, w, h, maximized: false, monitor: None };
    assert!(p(213.0, 157.0, 1011.0, 707.0).usable().is_some());
    assert!(p(-1920.0, 40.0, 900.0, 700.0).usable().is_some(), "a screen to the left");
    assert!(p(100.0, 100.0, 50.0, 40.0).usable().is_none(), "too small to use");
    assert!(p(-32000.0, -32000.0, 900.0, 700.0).usable().is_none(), "Windows' minimized spot");
    assert!(p(f32::NAN, 0.0, 900.0, 700.0).usable().is_none());
    let v: WindowPlace = serde_json::from_value(json!({"x": 1.0, "y": 2.0, "w": 800.0, "h": 600.0})).unwrap();
    assert!(!v.maximized, "saved before maximized was kept");
}

#[test]
fn delay_delivery_and_replies_to_draw_while_writing() {
    let (mut a, dir) = app("compose-delay");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    // Delay Delivery shows for a server account (the Outbox sends it later), with its time.
    let id = a.session.run("account.add", &json!({"email": "zoe@example.org", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    a.session.prefs.new_mail_window = false;
    a.run("mail.new", json!({}));
    a.run(
        "compose.update",
        json!({"account": id, "to": "ann@example.org", "subject": "Later", "replyTo": "desk@example.org", "sendAfter": NOW + 3600}),
    );
    for _ in 0..3 {
        frame(&ctx, &mut a, vec![]);
    }
    assert!(a.session.compose.as_ref().is_some_and(|c| c.send_after == Some(NOW + 3600) && c.reply_to == "desk@example.org"));
    assert_eq!(super::reading::parse_local("2026-10-16", "08:30"), Some((20_742, 510)));
    assert_eq!(super::reading::parse_local("2026-02-30", "08:30"), None);
    assert_eq!(super::reading::parse_local("2026-10-16", "24:00"), None);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn every_kind_of_attachment_previews_in_the_reading_pane() {
    let (mut a, dir) = app("attachment-preview");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let k = a.session.list.iter().find(|m| m.subject.starts_with("Your proof")).map(|m| m.key.clone()).unwrap();
    a.run("list.open", json!({"key": k.to_id()}));
    frame(&ctx, &mut a, vec![]);
    let n = a.session.reading.as_ref().and_then(|r| r.parsed.as_ref()).map_or(0, |p| p.attachments.len());
    assert_eq!(n, 4, "notes, a PDF, a document and a workbook");
    for i in 0..n {
        a.ui.attachment_view = Some((k.clone(), i));
        // A PDF draws a page a frame.
        for _ in 0..3 {
            frame(&ctx, &mut a, vec![]);
        }
        assert_eq!(a.ui.attachment_view, Some((k.clone(), i)));
    }
    // Escape: back to the message.
    frame(&ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)]);
    assert!(a.ui.attachment_view.is_none());
    // One that isn't there any more (another message's index) is just the message.
    a.ui.attachment_view = Some((k.clone(), 99));
    frame(&ctx, &mut a, vec![]);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn people_lay_out_as_a_list_cards_or_a_phone_list() {
    let (mut a, dir) = app("people-views");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("view.module", json!({"name": "people"}));
    for v in crate::people::PeopleView::ALL {
        a.ui.people_view = v;
        frame(&ctx, &mut a, vec![]);
        frame(&ctx, &mut a, vec![]);
    }
    // The layout is remembered with the window's other settings.
    // The list's columns and sort are kept too.
    a.ui.people_list.columns = vec![crate::people::Column::Mobile, crate::people::Column::Book];
    a.ui.people_list.sort = Some(crate::people::Column::Book);
    frame(&ctx, &mut a, vec![]);
    let saved = a.saved_state();
    assert_eq!(saved["peopleView"], "phone");
    a.ui.people_view = crate::people::PeopleView::People;
    a.ui.people_list = Default::default();
    a.restore_state(&saved);
    assert_eq!(a.ui.people_view, crate::people::PeopleView::Phone);
    assert_eq!(a.ui.people_list.columns, [crate::people::Column::Mobile, crate::people::Column::Book]);
    assert_eq!(a.ui.people_list.sort, Some(crate::people::Column::Book));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn notes_are_written_on_and_kept() {
    let (mut a, dir) = app("notes-ui");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("view.module", json!({"name": "notes"}));
    frame(&ctx, &mut a, vec![]);
    // New note: made, and written on at once.
    a.act(crate::Action::NewNote);
    let uid = a.ui.note_edit.as_ref().map(|(u, _)| u.clone()).unwrap();
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![Event::Text("Water the plants".into())]);
    assert_eq!(a.ui.note_edit.as_ref().map(|(_, t)| t.as_str()), Some("Water the plants"));
    // Escape leaves it: the text is kept.
    frame(&ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)]);
    frame(&ctx, &mut a, vec![]);
    assert!(a.ui.note_edit.is_none());
    assert!(a.session.notes("").iter().any(|n| n.note.uid == uid && n.note.text == "Water the plants"));
    // Ctrl+1 goes back to Mail.
    frame(&ctx, &mut a, vec![key(Key::Num1, Modifiers::COMMAND)]);
    assert_eq!(a.session.module, mailcraft_engine::Module::Mail);
    let _ = std::fs::remove_dir_all(dir);
}

/// Ctrl+A goes to what was last clicked: the attachments when they were, else the mail list.
#[test]
fn ctrl_a_selects_attachments_when_they_have_the_keyboard() {
    let (mut a, dir) = app("attach-all");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let k = a.session.list.iter().find(|m| m.subject.contains("festival poster")).map(|m| m.key.clone()).unwrap();
    a.run("list.open", json!({"key": k.to_id()}));
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    a.ui.attachments_active = true;
    frame(&ctx, &mut a, vec![key(Key::A, Modifiers::COMMAND)]);
    let picked = a.ui.attachment_selected.as_ref().map(|(_, v)| v.len());
    assert_eq!(picked, Some(4), "all four attachments");
    assert_eq!(a.session.selection.len(), 1, "the mail selection is left alone");
    // Escape lets go of them.
    frame(&ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)]);
    assert!(a.ui.attachment_selected.is_none() && !a.ui.attachments_active);
    a.ui.attachments_active = true;
    frame(&ctx, &mut a, vec![key(Key::A, Modifiers::COMMAND)]);
    assert_eq!(a.ui.attachment_selected.as_ref().map(|(_, v)| v.len()), Some(4));
    // Elsewhere (a press off the attachments), Ctrl+A selects the mail again.
    let off = egui::pos2(5.0, 400.0);
    frame(&ctx, &mut a, vec![Event::PointerMoved(off)]);
    frame(&ctx, &mut a, vec![Event::PointerButton { pos: off, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE }]);
    frame(&ctx, &mut a, vec![Event::PointerButton { pos: off, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE }]);
    assert!(!a.ui.attachments_active);
    assert!(a.ui.attachment_selected.is_none(), "a click elsewhere lets go of them");
    frame(&ctx, &mut a, vec![key(Key::A, Modifiers::COMMAND)]);
    assert_eq!(a.session.selection.len(), a.session.list.len());
    let _ = std::fs::remove_dir_all(dir);
}

/// Back from a previewed attachment, its chip isn't left selected.
#[test]
fn leaving_an_attachment_preview_lets_go_of_its_chip() {
    let (mut a, dir) = app("attach-back");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let k = a.session.list.iter().find(|m| m.subject.contains("festival poster")).map(|m| m.key.clone()).unwrap();
    a.run("list.open", json!({"key": k.to_id()}));
    frame(&ctx, &mut a, vec![]);
    let index = a.session.reading.as_ref().and_then(|r| r.parsed.as_ref()).map(|p| p.attachments[0].index).unwrap();
    a.ui.attachment_view = Some((k.clone(), index));
    a.ui.attachment_selected = Some((k.clone(), vec![index]));
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![key(Key::Escape, Modifiers::NONE)]);
    assert!(a.ui.attachment_view.is_none() && a.ui.attachment_selected.is_none());
    let _ = std::fs::remove_dir_all(dir);
}

/// Enter in a dialog's one field submits it: the dialog gave the field its focus back before
/// looking for the Enter that had just ended input, so only the button worked.
#[test]
fn enter_submits_a_one_field_dialog() {
    let (mut a, dir) = app("enter-submits");
    let ctx = egui::Context::default();
    // The sample account isn't an Exchange one, so the submit comes back as the dialog's error.
    a.ui.dialog = Some(Dialog::AddSharedMailbox { account: "sample".into(), address: "sales@contoso.example".into(), error: None });
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![]);
    frame(&ctx, &mut a, vec![key(Key::Enter, Modifiers::NONE)]);
    frame(&ctx, &mut a, vec![]);
    assert!(matches!(&a.ui.dialog, Some(Dialog::AddSharedMailbox { error: Some(_), .. })), "submitted: {:?}", a.ui.dialog.is_some());
    let _ = std::fs::remove_dir_all(dir);
}

/// Issue #1: a shared mailbox is a mailbox of its own in the folder pane, after the account's
/// folders, open or collapsed (its unread mail then beside its name); a special folder under the
/// server's own name draws as it is.
#[test]
fn a_shared_mailbox_draws_as_a_mailbox_of_its_own() {
    use mailcraft_engine::model::{Folder, FolderRole};
    let (mut a, dir) = app("shared-mailbox");
    let mut folders = a.session.account("sample").unwrap().folders.clone();
    if let Some(inbox) = folders.iter_mut().find(|f| f.role == FolderRole::Inbox) {
        inbox.name = Some("Postvak IN".into());
    }
    let shared = |path: &str, role: FolderRole, unread: u32| Folder {
        account: "sample".into(),
        path: path.into(),
        delimiter: Some('/'),
        selectable: true,
        unread,
        mailbox: Some("test@studio.example".into()),
        mailbox_role: role,
        ..Default::default()
    };
    folders.push(shared("Test Grommunio/Postvak IN", FolderRole::Inbox, 3));
    folders.push(shared("Test Grommunio", FolderRole::Other, 0));
    folders.push(shared("Test Grommunio/Postvak IN/Orders", FolderRole::Other, 0));
    a.session.apply(MailEvent::Folders { account: "sample".into(), folders });
    let paths: Vec<String> = a.session.account("sample").unwrap().folders.iter().map(|f| f.path.clone()).collect();
    assert_eq!(&paths[paths.len() - 3..], ["Test Grommunio", "Test Grommunio/Postvak IN", "Test Grommunio/Postvak IN/Orders"]);
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("folder.open", json!({"account": "sample", "folder": "Test Grommunio/Postvak IN"}));
    frame(&ctx, &mut a, vec![]);
    // In Favorites it says whose Inbox it is.
    a.run("folder.favorite", json!({"account": "sample", "folder": "Test Grommunio/Postvak IN", "on": true}));
    a.run("view.collapse", json!({"key": "folder:sample/Test Grommunio"}));
    frame(&ctx, &mut a, vec![]);
    let e = a.session.run("folder.rename", &json!({"account": "sample", "folder": "Test Grommunio/Postvak IN", "name": "X"})).unwrap_err();
    assert!(e.to_string().contains("can't be renamed"), "{e}");
    let _ = std::fs::remove_dir_all(dir);
}

/// Issue #3: Server settings show the outgoing server's own sign-in; left alone, its kept
/// password stays (none is sent), and turned off it goes back to the incoming server's.
#[test]
fn server_settings_keep_the_outgoing_servers_own_sign_in() {
    let mut c = mailcraft_engine::model::AccountConfig::guess("zoe@example.org");
    c.id = "a1".into();
    c.smtp_username = Some("relay-user".into());
    let Dialog::EditAccount(mut f) = Dialog::edit_account(&c) else { panic!("the server settings") };
    assert!(f.smtp_sign_in.own && f.smtp_sign_in.kept);
    let p = f.params();
    assert_eq!(p["smtpUsername"], "relay-user");
    assert!(p.get("smtpPassword").is_none(), "the kept password stays");
    f.smtp_sign_in.password = "new".into();
    assert_eq!(f.params()["smtpPassword"], "new");
    f.smtp_sign_in.own = false;
    assert!(f.params()["smtpUsername"].is_null());
}

/// A window restored onto a monitor that went away is brought back, centred on the primary one;
/// one on the monitor it was saved on stays put; a maximized one fills whichever monitor it gets.
#[test]
fn a_window_whose_monitor_went_comes_back_on_screen() {
    use crate::WindowPlace;
    let saved = WindowPlace { x: 2100.0, y: 50.0, w: 1200.0, h: 800.0, maximized: false, monitor: Some([1920.0, 1080.0]) };
    let info = |monitor: egui::Vec2| egui::ViewportInfo { monitor_size: Some(monitor), ..Default::default() };
    assert_eq!(saved.rescue(&info(egui::vec2(1600.0, 900.0))), Some(egui::pos2(200.0, 50.0)));
    assert_eq!(saved.rescue(&info(egui::vec2(1920.0, 1080.0))), None, "the same monitor");
    assert_eq!(saved.rescue(&egui::ViewportInfo::default()), None, "not known yet");
    assert_eq!(WindowPlace { maximized: true, ..saved }.rescue(&info(egui::vec2(1600.0, 900.0))), None);
    assert_eq!(WindowPlace { monitor: None, ..saved }.rescue(&info(egui::vec2(1600.0, 900.0))), None, "saved before monitors were kept");
}

/// A window too big for its screen, frame included, shrinks to fit and is centred; one that fits
/// is left alone; the system's own placement counts its position, a restored one only its size.
#[test]
fn a_window_too_big_for_its_screen_shrinks_to_fit() {
    use crate::WindowPlace;
    let rect = |x: f32, y: f32, w: f32, h: f32| egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(w, h));
    let info = |outer: egui::Rect, inner: egui::Rect, monitor: egui::Vec2| egui::ViewportInfo {
        outer_rect: Some(outer),
        inner_rect: Some(inner),
        monitor_size: Some(monitor),
        ..Default::default()
    };
    let laptop = egui::vec2(1366.0, 768.0);
    // 1320 x 820 with a 31-point title bar on a 1366 x 768 screen.
    let big = info(rect(26.0, 26.0, 1336.0, 859.0), rect(34.0, 57.0, 1320.0, 820.0), laptop);
    let (pos, size) = WindowPlace::fit(&big, true).unwrap();
    assert!(size.x + 16.0 <= laptop.x - 96.0 + 0.5 && size.y + 39.0 <= laptop.y - 96.0 + 0.5, "{size:?}");
    assert!(pos.x >= 0.0 && pos.y >= 0.0);
    // Fits: nothing to do.
    let fits = info(rect(100.0, 100.0, 1016.0, 739.0), rect(108.0, 131.0, 1000.0, 700.0), egui::vec2(1920.0, 1080.0));
    assert_eq!(WindowPlace::fit(&fits, true), None);
    // Placed by the system past the screen's edge (not too big): moved; restored there: left.
    let past = info(rect(900.0, 300.0, 1016.0, 600.0), rect(908.0, 331.0, 1000.0, 561.0), egui::vec2(1920.0, 1080.0));
    assert!(WindowPlace::fit(&past, false).is_some());
    assert_eq!(WindowPlace::fit(&past, true), None);
    // Maximized: fills its monitor already.
    let max = egui::ViewportInfo { maximized: Some(true), ..big };
    assert_eq!(WindowPlace::fit(&max, true), None);
}
