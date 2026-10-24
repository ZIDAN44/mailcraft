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
            Dialog::SendCheck(vec![
                mailcraft_engine::SendCheck::Lookalike { address: "orders@printsh0p.example".into(), like: "printshop.example".into() },
                mailcraft_engine::SendCheck::Exposed { count: 12 },
            ]),
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
            Dialog::Certificate { account: "sample".into(), path: "riley.p12".into(), password: String::new() },
            Dialog::SmartCard { account: "sample".into(), module: "/usr/lib/opensc-pkcs11.so".into(), pin: "1234".into() },
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
                warning: Some(mailcraft_engine::model::lookalike::SenderWarning::Lookalike {
                    domain: "printsh0p.example".into(),
                    like: "printshop.example".into(),
                }),
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

/// Services whose clock the test moves, remembering the notifications shown.
struct Ticking(std::sync::Arc<std::sync::Mutex<i64>>, std::sync::Arc<std::sync::Mutex<Vec<(String, String)>>>);

impl crate::Services for Ticking {
    fn clock(&self) -> (i64, i32) {
        (*self.0.lock().unwrap(), 0)
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
        self.1.lock().unwrap().push((title.into(), body.into()));
    }
}

mod accounts;
mod organize;
mod pim;
mod reading;
mod windows;
mod writing;
