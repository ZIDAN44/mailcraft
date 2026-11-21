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

/// The methods [`crate::Services`] requires, as a test's services that don't vary them have
/// them: the clock at [`NOW`], no files picked or saved to, and a file opened without complaint.
/// `stub_services!()` writes all four; `stub_services!(clock, save_path)` only those named.
macro_rules! stub_services {
    () => {
        stub_services!(clock, pick_files, save_path, open_path);
    };
    ($($method:ident),+) => {
        $(stub_services!(@ $method);)+
    };
    (@ clock) => {
        fn clock(&self) -> (i64, i32) {
            (crate::tests::NOW, 0)
        }
    };
    (@ pick_files) => {
        fn pick_files(&mut self) -> Vec<std::path::PathBuf> {
            Vec::new()
        }
    };
    (@ save_path) => {
        fn save_path(&mut self, _: &str) -> Option<std::path::PathBuf> {
            None
        }
    };
    (@ open_path) => {
        fn open_path(&mut self, _: &std::path::Path) -> Result<(), String> {
            Ok(())
        }
    };
}

/// A backend that keeps the jobs it's given (the tests look at them).
#[derive(Clone, Default)]
struct Jobs(std::sync::Arc<std::sync::Mutex<Vec<mailcraft_engine::model::Job>>>);

impl Jobs {
    fn take(&self) -> Vec<mailcraft_engine::model::Job> {
        self.0.lock().map(|mut j| std::mem::take(&mut *j)).unwrap_or_default()
    }
}

impl crate::Backend for Jobs {
    fn submit(&mut self, job: mailcraft_engine::model::Job) {
        if let Ok(mut j) = self.0.lock() {
            j.push(job);
        }
    }
    fn poll(&mut self) -> Vec<MailEvent> {
        Vec::new()
    }
}

/// What a test made in the temporary folder, removed when the test's thread ends, passed or not.
#[derive(Default)]
struct Scratch {
    folders: Vec<std::path::PathBuf>,
    /// The caches made in them: their read marks are written first (written after, on their
    /// own thread, they'd make the folder again).
    stores: Vec<Store>,
    /// Where the window puts files for other apps in this test ([`own_scratch_dir`]).
    handed: Option<std::path::PathBuf>,
}

impl Drop for Scratch {
    fn drop(&mut self) {
        for store in self.stores.drain(..) {
            store.flush_marks();
        }
        for folder in self.folders.iter().chain(&self.handed) {
            // Leftovers in the temporary folder are harmless; a failed clean-up isn't a failed test.
            let _ = std::fs::remove_dir_all(folder);
        }
    }
}

thread_local!(static SCRATCH: std::cell::RefCell<Scratch> = std::cell::RefCell::default());

/// A scratch folder for the test, `mailcraft-ui-<name>-<pid>` in the temporary folder: what a
/// run before left there is removed now, and the folder with all in it once the test is over.
pub(crate) fn scratch(name: &str) -> std::path::PathBuf {
    let folder = std::env::temp_dir().join(format!("mailcraft-ui-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    SCRATCH.with(|s| s.borrow_mut().folders.push(folder.clone()));
    folder
}

/// The folder `name` the window puts files for other apps in ([`crate::scratch_dir`]), for a
/// test that has a [`scratch`] folder: `<that folder>-handed/<name>`, gone with it. `None` on a
/// thread with none (work a test sent to a thread of its own), which takes the user's.
pub(crate) fn own_scratch_dir(name: &str) -> Option<std::path::PathBuf> {
    let root = SCRATCH.with(|s| {
        let mut s = s.borrow_mut();
        if s.handed.is_none() {
            let first = s.folders.first()?;
            let handed = first.with_file_name(format!("{}-handed", first.file_name()?.to_string_lossy()));
            let _ = std::fs::remove_dir_all(&handed);
            s.handed = Some(handed);
        }
        s.handed.clone()
    })?;
    let dir = root.join(name);
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

/// A cache made in a [`scratch`] folder, its marks written before the folder goes.
pub(crate) fn scratch_store(store: Store) -> Store {
    SCRATCH.with(|s| s.borrow_mut().stores.push(store.clone()));
    store
}

/// The window over the sample mailbox, in a [`scratch`] folder (also given, for the test's files).
pub(crate) fn app(name: &str) -> (MailApp, std::path::PathBuf) {
    let dir = scratch(name);
    let store = scratch_store(Store::open(&dir).unwrap());
    sample::seed_if_empty(&store, NOW).unwrap();
    let s = Session::open(Some(store), Clock::new(NOW, 0));
    (MailApp::new(s, Box::new(NullBackend), Box::new(NullServices(NOW))), dir)
}

pub(crate) fn frame(ctx: &egui::Context, app: &mut MailApp, events: Vec<Event>) {
    let size = windows::layout::SIZE.with(std::cell::Cell::get);
    windows::layout::dress(app);
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)), events, ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| app.ui(ui));
    windows::layout::look(app, &out.shapes);
    // No renderer here: drop the texture uploads deliberately.
    out.textures_delta.clear();
}

pub(crate) fn key(k: Key, modifiers: Modifiers) -> Event {
    Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers }
}

/// The input of a frame in which the window is asked to close, as its close button does.
pub(crate) fn close_input() -> RawInput {
    let asked = egui::ViewportInfo { events: vec![egui::ViewportEvent::Close], ..Default::default() };
    RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))),
        viewports: std::iter::once((egui::ViewportId::ROOT, asked)).collect(),
        ..Default::default()
    }
}

/// A frame in which the window is asked to close: what it put out.
pub(crate) fn close(ctx: &egui::Context, a: &mut MailApp) -> egui::FullOutput {
    let mut out = ctx.run_ui(close_input(), |ui| a.ui(ui));
    // No renderer here: drop the texture uploads deliberately.
    out.textures_delta.clear();
    out
}

/// A frame in a window of `size` given `events` (not dressed or looked at, as [`frame`]'s are):
/// what it put out.
pub(crate) fn sized_frame(ctx: &egui::Context, a: &mut MailApp, size: egui::Vec2, events: Vec<Event>) -> egui::FullOutput {
    let input = RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)), events, ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| a.ui(ui));
    out.textures_delta.clear();
    out
}

/// A text a frame draws.
#[derive(Clone, Debug)]
pub(crate) struct Drawn {
    pub(crate) text: String,
    /// Where it's laid out: its place, as wide and tall as its lines.
    pub(crate) rect: egui::Rect,
    /// Where its letters are, left to right on the screen, as tall as its lines.
    pub(crate) ink: egui::Rect,
    /// The room it's cut to.
    pub(crate) clip: egui::Rect,
}

impl Drawn {
    /// Whether some of it can be seen (it isn't wholly outside its room).
    pub(crate) fn seen(&self) -> bool {
        self.clip.intersects(self.rect)
    }
}

/// The texts among a frame's shapes.
pub(crate) fn texts_in(shapes: &[egui::epaint::ClippedShape]) -> Vec<Drawn> {
    shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Text(t) => {
                let ink = t.galley.rows.iter().flat_map(|r| r.row.glyphs.iter().map(move |g| (r.pos.x + g.pos.x, r.pos.x + g.max_x())));
                let (l, r) = ink.fold((f32::INFINITY, f32::NEG_INFINITY), |(l, r), (a, b)| (l.min(a), r.max(b)));
                let size = t.galley.size();
                Some(Drawn {
                    text: t.galley.job.text.clone(),
                    rect: egui::Rect::from_min_size(t.pos, size),
                    ink: egui::Rect::from_x_y_ranges(t.pos.x + l..=t.pos.x + r, t.pos.y..=t.pos.y + size.y),
                    clip: s.clip_rect,
                })
            }
            _ => None,
        })
        .collect()
}

/// The texts a frame in a window of `size` given `events` draws.
pub(crate) fn drawn_with(ctx: &egui::Context, a: &mut MailApp, size: egui::Vec2, events: Vec<Event>) -> Vec<Drawn> {
    texts_in(&sized_frame(ctx, a, size, events).shapes)
}

/// The texts a frame in a window of `size` draws.
pub(crate) fn drawn_text(ctx: &egui::Context, a: &mut MailApp, size: egui::Vec2) -> Vec<Drawn> {
    drawn_with(ctx, a, size, vec![])
}

/// The texts a frame in a window of `size` draws, the words alone.
pub(crate) fn drawn_strings(ctx: &egui::Context, a: &mut MailApp, size: egui::Vec2) -> Vec<String> {
    drawn_text(ctx, a, size).into_iter().map(|d| d.text).collect()
}

/// Whether one of `texts` says `what`.
pub(crate) fn has(texts: &[String], what: &str) -> bool {
    texts.iter().any(|t| t.contains(what))
}

/// A node a screen reader is told of.
#[derive(Clone, Debug)]
pub(crate) struct Named {
    pub(crate) role: egui::accesskit::Role,
    pub(crate) label: Option<String>,
    pub(crate) value: Option<String>,
    /// Where it is, if it's on screen.
    pub(crate) rect: Option<egui::Rect>,
    pub(crate) disabled: bool,
}

impl Named {
    /// What it's called: its label, else its value.
    pub(crate) fn name(&self) -> Option<&str> {
        self.label.as_deref().or(self.value.as_deref())
    }

    /// Where a click on it lands: its middle.
    pub(crate) fn centre(&self) -> Option<egui::Pos2> {
        self.rect.map(|r| r.center())
    }
}

/// The nodes a frame told a screen reader of (the context's AccessKit on).
pub(crate) fn named_in(out: &egui::FullOutput) -> Vec<Named> {
    let tree = out.platform_output.accesskit_update.as_ref().expect("AccessKit is on");
    tree.nodes
        .iter()
        .map(|(_, n)| Named {
            role: n.role(),
            label: n.label().map(str::to_string),
            value: n.value().map(str::to_string),
            rect: n.bounds().map(|b| egui::Rect::from_min_max(egui::pos2(b.x0 as f32, b.y0 as f32), egui::pos2(b.x1 as f32, b.y1 as f32))),
            disabled: n.is_disabled(),
        })
        .collect()
}

/// The nodes a frame in a window of `size` tells a screen reader of (the context's AccessKit on).
pub(crate) fn named(ctx: &egui::Context, a: &mut MailApp, size: egui::Vec2) -> Vec<Named> {
    named_in(&sized_frame(ctx, a, size, vec![]))
}

/// A click of `button` at `at` with `modifiers` held for the whole of it, as the input's
/// modifiers say and not only the pointer's events: a frame each for the pointer coming, the
/// button going down and coming up, and one letting go of the keys, if any were held. What it
/// did shows in the frame after, the caller's.
pub(crate) fn click_at(ctx: &egui::Context, a: &mut MailApp, at: egui::Pos2, button: egui::PointerButton, modifiers: Modifiers) {
    let press = |pressed| Event::PointerButton { pos: at, button, pressed, modifiers };
    frame(ctx, a, vec![Event::ModifiersChanged(modifiers), Event::PointerMoved(at)]);
    frame(ctx, a, vec![press(true)]);
    frame(ctx, a, vec![press(false)]);
    if modifiers != Modifiers::NONE {
        frame(ctx, a, vec![Event::ModifiersChanged(Modifiers::NONE)]);
    }
}

/// A click of the primary button at `at`, no keys held ([`click_at`]).
pub(crate) fn click(ctx: &egui::Context, a: &mut MailApp, at: egui::Pos2) {
    click_at(ctx, a, at, egui::PointerButton::Primary, Modifiers::NONE);
}

/// A click on the control a screen reader names `label` (the first that's on and on screen).
pub(crate) fn click_named(ctx: &egui::Context, a: &mut MailApp, label: &str) {
    let size = windows::layout::SIZE.with(std::cell::Cell::get);
    let all = named(ctx, a, size);
    let at = all.iter().filter(|n| n.label.as_deref() == Some(label) && !n.disabled).find_map(Named::centre);
    let at = at.unwrap_or_else(|| panic!("no {label} to click: {:?}", all.iter().filter_map(Named::name).collect::<Vec<_>>()));
    click(ctx, a, at);
}

/// A message of this source filed first in the sample's Inbox at `uid`, as mail coming in.
pub(crate) fn arrive(a: &mut MailApp, uid: u32, raw: &[u8]) -> MessageKey {
    let store = a.session.store.clone().unwrap();
    let mut inbox = store.load_folder("sample", "INBOX").unwrap();
    let key = MessageKey { account: "sample".into(), folder: "INBOX".into(), validity: inbox.validity, uid };
    inbox.messages.insert(0, mailcraft_engine::mime::summary_from_full(raw, key.clone(), Default::default(), 0, NOW).into());
    store.save_folder("sample", "INBOX", &inbox).unwrap();
    store.write_body(&key, raw).unwrap();
    a.session.apply(MailEvent::FolderChanged { account: "sample".into(), folder: "INBOX".into() });
    key
}

/// A message of this source put first in the Inbox, and the Inbox shown.
pub(crate) fn with_raw(a: &mut MailApp, uid: u32, raw: &[u8]) -> MessageKey {
    let key = arrive(a, uid, raw);
    a.run("folder.open", json!({"account": "sample", "folder": "INBOX"}));
    key
}

/// A message with these files put first in the Inbox, and the Inbox shown.
pub(crate) fn with_files(a: &mut MailApp, uid: u32, files: Vec<(String, &str, Vec<u8>)>) -> MessageKey {
    use mailcraft_engine::mime::{self, Draft, OutAttachment};
    let d = Draft {
        subject: format!("Files {uid}"),
        body: "Here they are.".into(),
        attachments: files.into_iter().map(|(name, mime, data)| OutAttachment { name, mime: mime.into(), data, ..Default::default() }).collect(),
        ..Default::default()
    };
    let raw = mime::build(&d, &mailcraft_engine::model::EmailAddress::new("Sam", "sam@studio.example"), NOW, &format!("<files{uid}@x>")).unwrap();
    with_raw(a, uid, &raw)
}

/// The sample's Archive made of these summaries alone (keys in Archive, validity 1), and shown.
pub(crate) fn archive_of(a: &mut MailApp, messages: Vec<Summary>) {
    let store = a.session.store.clone().unwrap();
    let uid_next = messages.iter().map(|m| m.key.uid).max().unwrap_or(0) + 1;
    let state = FolderState { validity: 1, uid_next, highest_modseq: 0, messages: messages.into_iter().map(Into::into).collect(), window: 0 };
    store.save_folder("sample", "Archive", &state).unwrap();
    a.session.apply(MailEvent::FolderChanged { account: "sample".into(), folder: "Archive".into() });
    a.run("folder.open", json!({"account": "sample", "folder": "Archive"}));
}

/// Every pane layout, state and dialog, in light and dark (also run in the pseudo-language by
/// the translation tests).
pub(crate) fn tour() {
    static RUN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = RUN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let (mut a, _) = app(&format!("states-{n}"));
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
        // A message's source, whole and its header only (issue #27); several selected (#26).
        a.act(crate::Action::Run("message.source", json!({"key": k.to_id()})));
        frame(&ctx, &mut a, vec![]);
        a.ui.source_headers.insert(k.clone());
        frame(&ctx, &mut a, vec![]);
        a.run("message.closeSource", json!({"key": k.to_id()}));
        a.run("list.extendNext", json!({}));
        frame(&ctx, &mut a, vec![]);
        a.run("list.open", json!({"key": k.to_id()}));
        backstage_tour(&mut a, &ctx);
        pim_tour(&mut a, &ctx);
        updates::tour(&mut a, &ctx);
        accounts::importing::tour(&mut a, &ctx);
        accounts::compacting::tour(&mut a, &ctx);
        crate::app::boot::tests::tour(&ctx);
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
            Dialog::CheckNames(Box::new(crate::dialogs::CheckNamesForm {
                asks: vec![crate::dialogs::NameChoice {
                    field: "to".into(),
                    typed: "ana".into(),
                    people: vec![("Ana Lima".into(), "ana@soundlab.example".into()), (String::new(), "ana.r@studio.example".into())],
                }],
                picked: Some(1),
            })),
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
                archive: None,
            },
            // A folder's own AutoArchive setting (issue #180), deleting its old mail.
            Dialog::FolderProperties {
                name: "Projects".into(),
                account: "riley@studio.example".into(),
                location: "Projects".into(),
                total: 2,
                unread: 0,
                size: 4_096,
                archive: crate::dialogs::FolderArchiveForm::new(&a.session, "sample", "Projects", "")
                    .map(|f| Box::new(crate::dialogs::FolderArchiveForm { mode: crate::dialogs::FolderMode::Own, ..f })),
            },
            Dialog::OpenSharedCalendar { email: "ana@contoso.example".into(), error: Some("no access".into()) },
            Dialog::SubscribeCalendar { url: "webcal://holidays.example/de.ics".into(), name: String::new(), error: Some("HTTP 404".into()) },
            Dialog::AddSharedMailbox { account: "sample".into(), address: "sales@contoso.example".into(), error: Some("already open".into()) },
            Dialog::Address(crate::dialogs::AddressForm {
                account: "sample".into(),
                address: "shop@studio.example".into(),
                name: "The Shop".into(),
                reply_to: "help@studio.example".into(),
                error: Some("that's the account's own address".into()),
                ..Default::default()
            }),
            Dialog::Dav(crate::dialogs::DavForm {
                account: "sample".into(),
                caldav: "https://dav.example.com/calendars/riley/default".into(),
                own: true,
                username: "riley@studio.example".into(),
                kept: Some("riley@studio.example".into()),
                error: Some("no calendars were found".into()),
                ..Default::default()
            }),
            Dialog::RemoveAccount { account: "sample".into(), email: "x".into() },
            Dialog::MailboxClosed {
                account: "sample".into(),
                email: "zoe@hotmail.com".into(),
                error: format!("{}. Turn on IMAP on Outlook.com.", mailcraft_engine::model::MICROSOFT_IMAP_CLOSED),
            },
            Dialog::TrustCertificate {
                account: "sample".into(),
                shown: Box::new(mailcraft_engine::model::UntrustedCertificate {
                    host: "127.0.0.1".into(),
                    fingerprint: "0a1b2c3d".repeat(8),
                    ..Default::default()
                }),
            },
            Dialog::TrustCertificate {
                account: "sample".into(),
                shown: Box::new(mailcraft_engine::model::UntrustedCertificate {
                    host: "mail.corp.test".into(),
                    fingerprint: "0a1b2c3d".repeat(8),
                    remote: true,
                    subject: "mail.corp.test".into(),
                    issuer: "Corp Internal CA".into(),
                    names: vec!["mail.corp.test".into(), "imap.corp.test".into()],
                    not_before: 1_700_000_000,
                    not_after: 1_900_000_000,
                }),
            },
            Dialog::add_account_with("zoe@hotmail.com", true),
            Dialog::Signature(Box::new(crate::dialogs::SignatureForm::new(&a.session, "sample", false))),
            Dialog::Signature(Box::new(crate::dialogs::SignatureForm::new(&a.session, "sample", true))),
            Dialog::Password { account: "sample".into(), email: "x".into(), user: "riley".into(), password: String::new() },
            Dialog::Certificate { account: "sample".into(), path: "riley.p12".into(), data: None, password: String::new() },
            Dialog::Certificate { account: "sample".into(), path: String::new(), data: None, password: String::new() },
            Dialog::Backup(Default::default()),
            Dialog::Archive(Box::new(crate::dialogs::ArchiveForm::new(&a.session, "sample"))),
            Dialog::Archive(Box::new(crate::dialogs::ArchiveForm {
                folder: Some("Projects".into()),
                path: r"C:\Users\riley\Documents\archive.pst".into(),
                error: Some("Old mail is being archived already: wait until that's done.".into()),
                ..crate::dialogs::ArchiveForm::new(&a.session, "sample")
            })),
            Dialog::AutoArchive(Box::new(crate::dialogs::AutoArchiveForm::new(&a.session, r"C:\Users\riley\Documents\MailCraft Files\archive.pst"))),
            Dialog::AutoArchiveDue,
            Dialog::Restore(crate::dialogs::RestoreForm { name: "MailCraft backup 2026-11-13.mailcraft-backup".into(), ..Default::default() }),
            Dialog::PgpKey { account: "sample".into(), path: "ana-secret.asc".into(), passphrase: String::new() },
            Dialog::SmartCard { account: "sample".into(), module: "/usr/lib/opensc-pkcs11.so".into(), pin: "1234".into(), system: false },
            Dialog::SmartCard { account: "sample".into(), module: String::new(), pin: String::new(), system: true },
            Dialog::new_folder("sample", Some("Projects")),
            Dialog::new_folder("sample", None),
            Dialog::rename_folder(&projects),
            Dialog::move_folder(&projects),
            Dialog::delete_folder(&a.session, &projects),
            Dialog::NewCategory { name: "Clients".into(), color: 3, keyword: String::new(), editing: None, error: Some("taken".into()) },
            Dialog::NewCategory { name: "Work".into(), color: 1, keyword: "bad word".into(), editing: Some("Work".into()), error: None },
            Dialog::name_keyword("r=c3=bcckfrage"),
            Dialog::Categories,
            Dialog::Rules,
            Dialog::rule_from("zoe@studio.example"),
            Dialog::add_account_with("zoe@gmail.com", true),
            accounts::jmap_form(),
            accounts::graph_form(),
            accounts::pop_form(),
            accounts::calendar_form(),
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
            Dialog::Preview {
                title: "favour.eml".into(),
                header: vec![("From".into(), "Zoe Park".into())],
                text: "Are you at your desk?".into(),
                warning: Some(mailcraft_engine::model::lookalike::SenderWarning::NameOfKnownPerson {
                    name: "Zoe Park".into(),
                    known: "zoe@studio.example".into(),
                    real: "zoe.park.director@gmail.com".into(),
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
        // The Search tab, and the classic ribbon's (their labels are looked up only there).
        a.ui.tab = crate::RibbonTab::Search;
        frame(&ctx, &mut a, vec![]);
        a.run("view.classicRibbon", json!({"on": true}));
        frame(&ctx, &mut a, vec![]);
        a.run("view.classicRibbon", json!({"on": false}));
        a.ui.tab = crate::RibbonTab::Home;
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
    for st in
        [SyncStatus::Connecting, SyncStatus::NeedsPassword { error: "no".into() }, SyncStatus::error("x".repeat(300)), SyncStatus::Online { at: NOW }]
    {
        a.session.apply(MailEvent::Status { account: "sample".into(), status: st });
        frame(&ctx, &mut a, vec![]);
    }
}

/// The browser's services: a popped-out message goes to a browser window of its own.
struct PageServices(std::rc::Rc<std::cell::RefCell<Vec<String>>>);

impl crate::Services for PageServices {
    stub_services!(clock, pick_files, save_path);
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

/// The File page: every page, in the tour's languages and themes; Options again with what this
/// version's settings show only when chosen (a meeting room, answered invitations to a folder,
/// the External tag), as they were after.
pub(crate) fn backstage_tour(a: &mut MailApp, ctx: &egui::Context) {
    for page in [crate::backstage::Page::Info, crate::backstage::Page::Open, crate::backstage::Page::Options, crate::backstage::Page::About] {
        a.act(crate::Action::Backstage(page));
        frame(ctx, a, vec![]);
    }
    let prefs = a.session.prefs.clone();
    a.run("meetings.addRoom", json!({"name": "My room", "url": "https://meet.example.org/rooms/abc", "code": "123", "details": "+1 555 0100"}));
    a.run("meetings.addHost", json!({"host": "jitsi.example.org"}));
    a.run("options.answeredInvites", json!({"after": "move", "replies": true}));
    a.run("view.externalTag", json!({"on": true, "domains": "studio.example"}));
    a.act(crate::Action::Backstage(crate::backstage::Page::Options));
    frame(ctx, a, vec![]);
    a.session.prefs = prefs;
    a.ui.backstage = None;
}

/// Calendar (each view, an event open), People (someone picked, the form open) and Tasks.
pub(crate) fn pim_tour(a: &mut MailApp, ctx: &egui::Context) {
    // The To-Do Bar beside the mail, every part of it.
    a.run("view.todoBar", json!({"part": "calendar", "on": true}));
    a.run("view.todoBar", json!({"part": "appointments", "on": true}));
    a.run("view.todoBar", json!({"part": "tasks", "on": true}));
    for _ in 0..30 {
        frame(ctx, a, vec![]);
    }
    // Follow Up › Custom… on its flagged mail (issue #160).
    let flagged = a.session.todo_items().into_iter().find_map(|i| match i.what {
        mailcraft_engine::TodoWhat::Mail { key } => Some(key),
        mailcraft_engine::TodoWhat::Task { .. } => None,
    });
    if let Some(key) = flagged {
        a.ui.dialog = crate::pim_dialogs::FollowUpForm::new(&a.session, &key).map(|f| Dialog::FollowUp(Box::new(f)));
        frame(ctx, a, vec![]);
        frame(ctx, a, vec![]);
        a.ui.dialog = None;
    }
    a.run("view.todoBar", json!({"off": true}));
    a.run("view.module", json!({"name": "calendar"}));
    for mode in ["schedule", "day", "week", "month"] {
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
    // The calendar's search: what it finds, each switch, and nothing found.
    for (q, when) in [
        ("a", mailcraft_engine::EventWhen::All),
        ("stand-up", mailcraft_engine::EventWhen::Past),
        ("nothing-like-it", mailcraft_engine::EventWhen::Upcoming),
    ] {
        a.ui.cal_search.query = q.into();
        a.ui.cal_search.when = when;
        frame(ctx, a, vec![]);
    }
    a.ui.cal_search = Default::default();
    a.run("view.module", json!({"name": "people"}));
    frame(ctx, a, vec![]);
    a.ui.person = a.session.people("").first().map(|p| p.card.uid.clone());
    frame(ctx, a, vec![]);
    if let Some(p) = a.session.people("").first() {
        a.ui.dialog = Some(Dialog::edit_contact(p.card));
        frame(ctx, a, vec![]);
    }
    a.ui.dialog = None;
    a.ui.people_query = "nobody-at-all".into();
    frame(ctx, a, vec![]);
    a.ui.people_query.clear();
    // Tasks: the list (one picked, sorted each way, the done ones hidden), a task open, a new
    // one, and nothing found.
    a.run("view.module", json!({"name": "tasks"}));
    frame(ctx, a, vec![]);
    a.ui.task = a.session.tasks("").first().map(|t| t.task.uid.clone());
    for by in ["due", "priority", "title", "modified"] {
        a.run("task.arrange", json!({"by": by}));
        frame(ctx, a, vec![]);
    }
    a.run("task.hideDone", json!({"on": true}));
    frame(ctx, a, vec![]);
    a.run("task.hideDone", json!({"on": false}));
    if let Some(t) = a.session.tasks("").first() {
        a.ui.dialog = Dialog::open_task(&a.session, &t.task.uid);
        frame(ctx, a, vec![]);
    }
    a.ui.dialog = Some(Dialog::new_task(&a.session, None));
    frame(ctx, a, vec![]);
    a.ui.dialog = None;
    a.ui.tasks_query = "nothing-like-it".into();
    frame(ctx, a, vec![]);
    a.ui.tasks_query.clear();
    a.run("view.module", json!({"name": "mail"}));
    frame(ctx, a, vec![]);
}

/// Services that remember the notifications shown.
struct Recorder(std::sync::Arc<std::sync::Mutex<Vec<(String, String)>>>);

impl crate::Services for Recorder {
    stub_services!();
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
    picked: Vec<(crate::Picked, String, Vec<u8>)>,
    asked: std::rc::Rc<std::cell::RefCell<Vec<crate::Picked>>>,
    /// The file the picker gives next (else a note).
    hands: std::rc::Rc<std::cell::RefCell<Option<Vec<u8>>>>,
}

impl crate::Services for WebLike {
    stub_services!(clock);
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
    fn pick_later(&mut self, purpose: &crate::Picked) -> bool {
        self.asked.borrow_mut().push(purpose.clone());
        let file = self.hands.borrow_mut().take().unwrap_or_else(|| b"remember".to_vec());
        self.picked.push((purpose.clone(), "notes.txt".into(), file));
        true
    }
    fn take_picked(&mut self) -> Vec<(crate::Picked, String, Vec<u8>)> {
        std::mem::take(&mut self.picked)
    }
}

/// Services whose clock the test moves, remembering the notifications shown.
struct Ticking(std::sync::Arc<std::sync::Mutex<i64>>, std::sync::Arc<std::sync::Mutex<Vec<(String, String)>>>);

impl crate::Services for Ticking {
    stub_services!(pick_files, save_path, open_path);
    fn clock(&self) -> (i64, i32) {
        (*self.0.lock().unwrap(), 0)
    }
    fn notify(&mut self, title: &str, body: &str) {
        self.1.lock().unwrap().push((title.into(), body.into()));
    }
}

mod access;
mod accounts;
mod calendar;
mod organize;
mod pictures;
mod reading;
mod selecting;
mod soak;
mod typing;
mod updates;
mod windows;
mod writing;
