//! MailCraft's egui front end: the familiar three-pane mail layout (folders, message list,
//! reading pane) with a simplified ribbon, search box, inline compose and account dialogs.
//!
//! The UI is thin: it reads [`Session`] state and changes it through commands (by id), so the
//! toolbar, menus, shortcuts and agents all behave the same. The network sits behind
//! [`Backend`] and platform services (file dialogs, opening files, the clock) behind
//! [`Services`], so nothing here depends on threads, sockets or a windowing crate.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod dialogs;
mod editor;
pub mod icons;
mod list;
mod nav;
mod organize;
mod reading;
pub mod theme;
mod widgets;

use std::collections::HashMap;
use std::path::PathBuf;

use egui::{Align, Color32, Key, Layout, Margin, RichText, Sense, Ui, vec2};
use mailcraft_engine::model::{Event, Job, MessageKey, SyncStatus};
use mailcraft_engine::{ReadingPane, Session};
use serde_json::{Value, json};

pub use dialogs::Dialog;
use icons::Icon;
use theme::Tokens;

/// The network side: the desktop app plugs in `mailcraft-net`'s worker pool.
pub trait Backend {
    fn submit(&mut self, job: Job);
    fn poll(&mut self) -> Vec<Event>;
}

/// No network (the sample mailbox, tests).
pub struct NullBackend;

impl Backend for NullBackend {
    fn submit(&mut self, _job: Job) {}
    fn poll(&mut self) -> Vec<Event> {
        Vec::new()
    }
}

/// Platform services the UI needs.
pub trait Services {
    /// Unix seconds and the local UTC offset in seconds.
    fn clock(&self) -> (i64, i32);
    /// Files to attach (an empty list when cancelled).
    fn pick_files(&mut self) -> Vec<PathBuf>;
    /// Where to save a file (`None` when cancelled).
    fn save_path(&mut self, suggested: &str) -> Option<PathBuf>;
    /// Open a file with the system's default app.
    fn open_path(&mut self, path: &std::path::Path) -> Result<(), String>;
    /// Show a desktop notification (new mail). The default does nothing.
    fn notify(&mut self, _title: &str, _body: &str) {}
    /// A picture on the clipboard, as PNG bytes (`None` when there is none).
    fn clipboard_picture(&mut self) -> Option<Vec<u8>> {
        None
    }
    /// Pictures to insert (an empty list when cancelled). The default offers the file picker.
    fn pick_pictures(&mut self) -> Vec<PathBuf> {
        self.pick_files()
    }
    /// Open a web address in the browser. The default can't.
    fn open_url(&mut self, _url: &str) -> Result<(), String> {
        Err("not available".into())
    }
}

/// Services for tests: a fixed clock, no dialogs.
pub struct NullServices(pub i64);

impl Services for NullServices {
    fn clock(&self) -> (i64, i32) {
        (self.0, 0)
    }
    fn pick_files(&mut self) -> Vec<PathBuf> {
        Vec::new()
    }
    fn save_path(&mut self, _: &str) -> Option<PathBuf> {
        None
    }
    fn open_path(&mut self, _: &std::path::Path) -> Result<(), String> {
        Err("not available".into())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RibbonTab {
    Home,
    View,
}

/// Something a pane asks for; run after drawing so panes only borrow what they read.
pub enum Action {
    Run(&'static str, Value),
    Open(MessageKey),
    Dialog(Dialog),
    OpenAttachment(usize),
    SaveAttachment(usize),
    AttachFiles,
    FocusSearch,
    SetDark(Option<bool>),
    /// Insert a picture at the cursor of the message being written (from a file).
    InsertPicture,
    /// Paste a picture from the clipboard, if there is one.
    PastePicture,
    /// A link in a message was clicked (web pages open in the browser; `mailto:` starts a message).
    OpenUrl(String),
    /// Run a command for the open dialog: it closes on success and shows the error otherwise.
    Submit(&'static str, Value),
}

/// UI-only state.
pub struct UiState {
    pub tokens: Tokens,
    /// `None` follows the system.
    pub dark: Option<bool>,
    pub tab: RibbonTab,
    pub search_text: String,
    focus_search: bool,
    pub dialog: Option<Dialog>,
    previews: HashMap<(MessageKey, usize), Option<egui::TextureHandle>>,
    /// Decoded web pictures, by address.
    web_pictures: HashMap<String, Option<egui::TextureHandle>>,
    pub toast: Option<(String, bool, f64)>,
    seen_notices: usize,
    scroll_to_focus: bool,
    /// The named font families the UI uses are registered (`install_fonts`).
    fonts_ready: bool,
    /// The compose form last shown, to focus its first field when a new one opens.
    pub(crate) compose_shown: Option<u64>,
    /// The body's cursor (characters), where pictures go.
    pub(crate) compose_cursor: usize,
    /// The recipient box showing suggestions (`to`, `cc`, `bcc`), the highlighted one, and the
    /// text for which Escape hid them.
    pub(crate) suggest_field: Option<&'static str>,
    pub(crate) suggest_index: usize,
    pub(crate) suggest_dismissed: Option<String>,
    /// The app's name in the window title (`MailCraft`, or `MailCraft – Sample mailbox`).
    pub title_base: String,
    pub(crate) title_shown: String,
}

pub struct MailApp {
    pub session: Session,
    pub backend: Box<dyn Backend>,
    pub services: Box<dyn Services>,
    pub ui: UiState,
}

impl MailApp {
    pub fn new(session: Session, backend: Box<dyn Backend>, services: Box<dyn Services>) -> MailApp {
        let seen = session.notices.len();
        MailApp {
            session,
            backend,
            services,
            ui: UiState {
                tokens: Tokens::light(),
                dark: None,
                tab: RibbonTab::Home,
                search_text: String::new(),
                focus_search: false,
                dialog: None,
                previews: HashMap::new(),
                web_pictures: HashMap::new(),
                toast: None,
                seen_notices: seen,
                scroll_to_focus: false,
                fonts_ready: false,
                compose_shown: None,
                compose_cursor: 0,
                suggest_field: None,
                suggest_index: 0,
                suggest_dismissed: None,
                title_base: "MailCraft".into(),
                title_shown: String::new(),
            },
        }
    }

    /// Register the UI fonts, preferring an installed system font (call before the first frame).
    pub fn install_fonts(&mut self, ctx: &egui::Context, regular: Option<Vec<u8>>, bold: Option<Vec<u8>>) {
        theme::install_fonts(ctx, regular, bold);
        self.ui.fonts_ready = true;
    }

    /// Run a command, reporting failures in a toast.
    pub fn run(&mut self, id: &str, params: Value) {
        match self.session.run(id, &params) {
            Ok(_) => {}
            Err(mailcraft_engine::CmdError::Disabled(_)) => {}
            Err(e) => self.session.notice(e.to_string(), true),
        }
    }

    fn pump(&mut self) {
        let (now, offset) = self.services.clock();
        self.session.set_clock(now, offset);
        for e in self.backend.poll() {
            self.session.apply(e);
        }
        // Snoozed mail that is due comes back.
        self.session.tick();
    }

    fn flush_jobs(&mut self) {
        for j in self.session.take_jobs() {
            self.backend.submit(j);
        }
    }

    /// The whole window.
    pub fn ui(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        if !self.ui.fonts_ready {
            // Fonts take effect from the next frame; drawing now would miss the bold family.
            self.install_fonts(&ctx, None, None);
            ctx.request_repaint();
            return;
        }
        self.pump();
        self.announce(&ctx);
        // A search started by a command or agent shows its query in the box.
        if let Some((q, _)) = &self.session.search
            && self.ui.search_text.is_empty()
            && !ctx.text_edit_focused()
        {
            self.ui.search_text = q.clone();
        }
        let dark = self.ui.dark.unwrap_or_else(|| ctx.system_theme() == Some(egui::Theme::Dark));
        if dark != self.ui.tokens.dark || ctx.global_style().visuals.panel_fill != self.ui.tokens.surface {
            self.ui.tokens = Tokens::for_dark(dark);
            theme::apply(&ctx, &self.ui.tokens);
        }
        self.shortcuts(&ctx);

        let mut acts: Vec<Action> = Vec::new();
        let t = self.ui.tokens;
        egui::Panel::top("topbar").exact_size(46.0).frame(egui::Frame::new().fill(t.accent).inner_margin(Margin::symmetric(12, 6))).show(ui, |ui| {
            self.top_bar(ui, &mut acts);
        });
        egui::Panel::top("ribbon")
            .frame(egui::Frame::new().fill(t.chrome).inner_margin(Margin::symmetric(8, 4)).stroke(egui::Stroke::new(1.0, t.border)))
            .show(ui, |ui| {
                self.ribbon(ui, &mut acts);
            });
        egui::Panel::bottom("status").exact_size(26.0).frame(egui::Frame::new().fill(t.chrome).inner_margin(Margin::symmetric(10, 3))).show(
            ui,
            |ui| {
                self.status_bar(ui);
            },
        );

        if self.session.accounts.is_empty() {
            egui::CentralPanel::default().frame(egui::Frame::new().fill(t.bg)).show(ui, |ui| {
                widgets::welcome(ui, &t, &mut acts);
            });
        } else {
            egui::Panel::left("nav")
                .resizable(true)
                .default_size(230.0)
                .size_range(170.0..=380.0)
                .frame(egui::Frame::new().fill(t.chrome).inner_margin(Margin::symmetric(6, 8)))
                .show(ui, |ui| {
                    nav::show(&self.session, &t, ui, &mut acts);
                });
            let pane = self.session.prefs.reading_pane;
            let list_frame = egui::Frame::new().fill(t.surface).stroke(egui::Stroke::new(1.0, t.border));
            match pane {
                ReadingPane::Right => {
                    egui::Panel::left("list").resizable(true).default_size(390.0).size_range(280.0..=720.0).frame(list_frame).show(ui, |ui| {
                        list::show(&self.session, &mut self.ui, ui, &mut acts);
                    });
                    egui::CentralPanel::default().frame(egui::Frame::new().fill(t.surface)).show(ui, |ui| {
                        reading::show(&mut self.session, &mut self.ui, ui, &mut acts);
                    });
                }
                ReadingPane::Bottom => {
                    egui::Panel::top("list").resizable(true).default_size(300.0).size_range(140.0..=900.0).frame(list_frame).show(ui, |ui| {
                        list::show(&self.session, &mut self.ui, ui, &mut acts);
                    });
                    egui::CentralPanel::default().frame(egui::Frame::new().fill(t.surface)).show(ui, |ui| {
                        reading::show(&mut self.session, &mut self.ui, ui, &mut acts);
                    });
                }
                ReadingPane::Off => {
                    egui::CentralPanel::default().frame(list_frame).show(ui, |ui| {
                        if self.session.compose.is_some() {
                            reading::show(&mut self.session, &mut self.ui, ui, &mut acts);
                        } else {
                            list::show(&self.session, &mut self.ui, ui, &mut acts);
                        }
                    });
                }
            }
        }

        if let Some(mut d) = self.ui.dialog.take() {
            let keep = dialogs::show(&ctx, &mut d, &self.session, &t, &mut acts);
            if keep && self.ui.dialog.is_none() {
                self.ui.dialog = Some(d);
            }
        }
        self.toasts(&ctx);

        for a in acts {
            self.act(a);
        }
        self.flush_jobs();
        let busy = self.session.accounts.iter().any(|a| matches!(a.status, SyncStatus::Connecting | SyncStatus::Syncing { .. }));
        ctx.request_repaint_after(std::time::Duration::from_secs(if busy || self.ui.toast.is_some() { 1 } else { 20 }));
    }

    /// New mail: a desktop notification, the taskbar asking for attention when the window is in
    /// the background, and the unread count in the window title.
    fn announce(&mut self, ctx: &egui::Context) {
        let news = self.session.take_new_mail();
        if !news.is_empty() {
            let (title, body) = match news.as_slice() {
                [m] => (m.from.display().to_string(), if m.subject.is_empty() { "(no subject)".to_string() } else { m.subject.clone() }),
                many => {
                    let mut who: Vec<&str> = Vec::new();
                    for m in many.iter().rev() {
                        let n = m.from.display();
                        if !who.contains(&n) {
                            who.push(n);
                        }
                    }
                    let names = match who.as_slice() {
                        [a] => (*a).to_string(),
                        [a, b] => format!("{a} and {b}"),
                        [a, b, ..] => format!("{a}, {b} and others"),
                        [] => String::new(),
                    };
                    (format!("{} new messages", many.len()), format!("From {names}"))
                }
            };
            self.services.notify(&title, &body);
            if !ctx.input(|i| i.focused) {
                ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(egui::UserAttentionType::Informational));
            }
        }
        let unread = self.session.inbox_unread();
        let view = self.session.view_title();
        let title = if self.session.view == mailcraft_engine::View::Empty {
            self.ui.title_base.clone()
        } else if unread > 0 {
            format!("({unread}) {view} – {}", self.ui.title_base)
        } else {
            format!("{view} – {}", self.ui.title_base)
        };
        if title != self.ui.title_shown {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.ui.title_shown = title;
        }
    }

    fn act(&mut self, a: Action) {
        match a {
            Action::Run(id, p) => {
                if id == "search.clear" {
                    self.ui.search_text.clear();
                }
                self.run(id, p);
                if matches!(id, "list.next" | "list.prev") {
                    self.ui.scroll_to_focus = true;
                }
            }
            Action::Open(k) => {
                self.run("list.open", json!({"key": k.to_id()}));
            }
            Action::Dialog(d) => self.ui.dialog = Some(d),
            Action::FocusSearch => self.ui.focus_search = true,
            Action::Submit(id, p) => match self.session.run(id, &p) {
                Ok(_) => self.ui.dialog = None,
                Err(e) => {
                    if let Some(d) = self.ui.dialog.as_mut() {
                        d.set_error(e.to_string().trim_start_matches("bad parameters: ").to_string());
                    }
                }
            },
            Action::SetDark(d) => self.ui.dark = d,
            Action::OpenUrl(url) => self.open_link(&url),
            Action::InsertPicture => {
                for p in self.services.pick_pictures() {
                    let r = self.session.run("compose.insertPicture", &json!({"path": p.to_string_lossy(), "insert": false}));
                    self.place_picture(r);
                }
            }
            Action::PastePicture => {
                if let Some(png) = self.services.clipboard_picture() {
                    let r = self.session.run("compose.insertPicture", &json!({"name": "pasted.png", "data": png, "insert": false}));
                    self.place_picture(r);
                }
            }
            Action::AttachFiles => {
                for p in self.services.pick_files() {
                    self.run("compose.attach", json!({"path": p.to_string_lossy()}));
                }
            }
            Action::SaveAttachment(i) => {
                let name = self.attachment_name(i);
                if let Some(path) = self.services.save_path(&name) {
                    self.run("attachment.save", json!({"index": i, "path": path.to_string_lossy()}));
                }
            }
            Action::OpenAttachment(i) => {
                let name = self.attachment_name(i);
                let dir = std::env::temp_dir().join("mailcraft-open");
                let _ = std::fs::create_dir_all(&dir);
                let path = dir.join(mailcraft_engine::mime::sanitize_filename(&name));
                match self.session.run("attachment.save", &json!({"index": i, "path": path.to_string_lossy()})) {
                    Ok(_) => {
                        if let Err(e) = self.services.open_path(&path) {
                            self.session.notice(format!("{name} could not be opened: {e}"), true);
                        }
                    }
                    Err(e) => self.session.notice(e.to_string(), true),
                }
            }
        }
    }

    /// Put a picture's mark at the body's cursor.
    fn place_picture(&mut self, r: mailcraft_engine::CmdResult) {
        match r {
            Ok(v) => {
                let mark = v.get("mark").and_then(Value::as_str).unwrap_or("").to_string();
                if let Some(c) = self.session.compose.as_mut() {
                    let at = c.body.char_indices().nth(self.ui.compose_cursor).map_or(c.body.len(), |(b, _)| b);
                    c.body.insert_str(at, &format!("{mark}\n"));
                    self.ui.compose_cursor += mark.chars().count() + 1;
                }
            }
            Err(e) => self.session.notice(e.to_string(), true),
        }
    }

    /// Follow a link from a message: only web pages and mail addresses, never local files or
    /// other schemes.
    fn open_link(&mut self, url: &str) {
        let u = url.trim();
        let lower = u.to_ascii_lowercase();
        if let Some(rest) = u.get(7..).filter(|_| lower.starts_with("mailto:")) {
            let to = rest.split('?').next().unwrap_or("").replace("%40", "@");
            self.run("mail.new", json!({}));
            self.run("compose.update", json!({"to": to}));
        } else if lower.starts_with("https://") || lower.starts_with("http://") {
            if let Err(e) = self.services.open_url(u) {
                self.session.notice(format!("The link couldn't be opened: {e}"), true);
            }
        } else {
            self.session.notice(format!("Links like {} aren't opened", widgets::shorten(u, 40)), true);
        }
    }

    fn attachment_name(&self, i: usize) -> String {
        self.session
            .reading
            .as_ref()
            .and_then(|r| r.parsed.as_ref())
            .and_then(|p| p.attachments.iter().find(|a| a.index == i))
            .map(|a| a.name.clone())
            .unwrap_or_else(|| "attachment".into())
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if self.ui.dialog.is_some() {
            if ctx.input(|i| i.key_pressed(Key::Escape)) {
                self.ui.dialog = None;
            }
            return;
        }
        let typing = ctx.text_edit_focused();
        let pressed: Vec<(Key, egui::Modifiers)> = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Key { key, pressed: true, modifiers, .. } => Some((*key, *modifiers)),
                    _ => None,
                })
                .collect()
        });
        let reg = self.session.registry();
        for (key, m) in pressed {
            let name = match key {
                Key::ArrowDown => "Down".to_string(),
                Key::ArrowUp => "Up".to_string(),
                Key::Enter => "Enter".to_string(),
                k => k.name().to_string(),
            };
            let plain = !(m.command || m.ctrl || m.alt);
            // Single keys type text when a text box has focus.
            if typing && plain && !matches!(key, Key::F9 | Key::Escape) {
                continue;
            }
            let mut combo = String::new();
            if m.command || m.ctrl {
                combo.push_str("Mod+");
            }
            if m.alt {
                combo.push_str("Alt+");
            }
            if m.shift {
                combo.push_str("Shift+");
            }
            combo.push_str(&name);
            if combo == "Mod+E" {
                self.ui.focus_search = true;
                continue;
            }
            if combo == "Escape" {
                if self.session.search.is_some() {
                    self.ui.search_text.clear();
                    self.run("search.clear", json!({}));
                }
                continue;
            }
            if typing && combo == "Mod+A" {
                continue;
            }
            if let Some(spec) = reg.by_shortcut(&combo) {
                let id = spec.id;
                if id == "search.run" {
                    continue;
                }
                if id == "mail.delete" && self.needs_delete_confirm() {
                    self.ui.dialog = Some(Dialog::ConfirmDelete { count: self.session.selection.len() });
                    continue;
                }
                if id == "mail.move" {
                    self.ui.dialog = Some(Dialog::move_to());
                    continue;
                }
                self.run(id, json!({}));
                if matches!(id, "list.next" | "list.prev") {
                    self.ui.scroll_to_focus = true;
                }
            }
        }
        if ctx.input(|i| i.modifiers.command && i.key_pressed(Key::Enter)) && self.session.compose.is_some() {
            self.run("compose.send", json!({}));
        }
    }

    /// Deleting from Deleted Items is permanent: ask first.
    pub fn needs_delete_confirm(&self) -> bool {
        self.session.prefs.confirm_permanent_delete
            && !self.session.selection.is_empty()
            && self.session.selected().iter().all(|m| {
                self.session
                    .account(&m.key.account)
                    .and_then(|a| a.folder(mailcraft_engine::model::FolderRole::Trash))
                    .is_some_and(|t| t.path == m.key.folder)
            })
    }

    fn top_bar(&mut self, ui: &mut Ui, acts: &mut Vec<Action>) {
        let t = self.ui.tokens;
        ui.horizontal_centered(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
            ui.painter().rect_filled(r, 5.0, Color32::WHITE);
            icons::paint(ui.painter(), icons::centered(r, 15.0), Icon::Unread, t.accent);
            ui.label(RichText::new("MailCraft").font(theme::bold(16.0)).color(Color32::WHITE));
            ui.add_space(16.0);
            let avail = ui.available_width();
            let search_w = (avail * 0.45).clamp(220.0, 620.0);
            ui.add_space(((avail - search_w) / 2.0 - 160.0).max(0.0));
            let frame =
                egui::Frame::new().fill(if t.dark { t.surface } else { Color32::WHITE }).corner_radius(6).inner_margin(Margin::symmetric(8, 3));
            frame.show(ui, |ui| {
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                    icons::paint(ui.painter(), r, Icon::Search, t.dim);
                    let te = egui::TextEdit::singleline(&mut self.ui.search_text)
                        .hint_text("Search mail (from:, subject:, has:attachment…)")
                        .frame(egui::Frame::NONE)
                        .desired_width(search_w - 60.0)
                        .text_color(t.text);
                    let resp = ui.add(te);
                    if self.ui.focus_search {
                        resp.request_focus();
                        self.ui.focus_search = false;
                    }
                    if resp.changed() {
                        let q = self.ui.search_text.clone();
                        acts.push(Action::Run("search.run", json!({"query": q})));
                    }
                    if !self.ui.search_text.is_empty() && widgets::tiny_icon_button(ui, Icon::Close, "Clear search", &t).clicked() {
                        acts.push(Action::Run("search.clear", json!({})));
                    }
                });
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let dark = t.dark;
                let (icon, tip) = if dark { (Icon::Sun, "Switch to light mode") } else { (Icon::Moon, "Switch to dark mode") };
                if widgets::bar_icon_button(ui, icon, tip).clicked() {
                    acts.push(Action::SetDark(Some(!dark)));
                }
                if widgets::bar_icon_button(ui, Icon::Gear, "Accounts and settings").clicked() {
                    acts.push(Action::Dialog(Dialog::Settings));
                }
            });
        });
    }

    fn ribbon(&mut self, ui: &mut Ui, acts: &mut Vec<Action>) {
        let t = self.ui.tokens;
        let s = &self.session;
        ui.horizontal(|ui| {
            for (tab, label) in [(RibbonTab::Home, "Home"), (RibbonTab::View, "View")] {
                let on = self.ui.tab == tab;
                let r = ui.add(
                    egui::Button::new(RichText::new(label).color(if on { t.accent } else { t.dim }).font(if on {
                        theme::bold(13.5)
                    } else {
                        theme::regular(13.5)
                    }))
                    .frame(false),
                );
                if on {
                    let rect = r.rect;
                    ui.painter().line_segment(
                        [egui::pos2(rect.left() + 6.0, rect.bottom() + 1.0), egui::pos2(rect.right() - 6.0, rect.bottom() + 1.0)],
                        egui::Stroke::new(2.0, t.accent),
                    );
                }
                if r.clicked() {
                    self.ui.tab = tab;
                }
            }
        });
        ui.add_space(2.0);
        let enabled = |id: &str| s.registry().get(id).is_some_and(|c| (c.enabled)(s).is_none());
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            match self.ui.tab {
                RibbonTab::Home => {
                    if widgets::accent_button(ui, Icon::NewMail, "New mail", &t, enabled("mail.new")).on_hover_text("New mail (Ctrl+N)").clicked() {
                        acts.push(Action::Run("mail.new", json!({})));
                    }
                    widgets::group_sep(ui, &t);
                    if widgets::tool_button(ui, Icon::Delete, "Delete", &t, enabled("mail.delete")).on_hover_text("Delete (Del)").clicked() {
                        if self.needs_delete_confirm() {
                            acts.push(Action::Dialog(Dialog::ConfirmDelete { count: s.selection.len() }));
                        } else {
                            acts.push(Action::Run("mail.delete", json!({})));
                        }
                    }
                    if widgets::tool_button(ui, Icon::Archive, "Archive", &t, enabled("mail.archive")).on_hover_text("Archive (Backspace)").clicked()
                    {
                        acts.push(Action::Run("mail.archive", json!({})));
                    }
                    let in_junk = s.view_role() == Some(mailcraft_engine::model::FolderRole::Junk);
                    let (jid, jl) = if in_junk { ("mail.notJunk", "Not junk") } else { ("mail.junk", "Report junk") };
                    if widgets::tool_button(ui, Icon::Junk, jl, &t, enabled(jid)).clicked() {
                        acts.push(Action::Run(jid, json!({})));
                    }
                    widgets::group_sep(ui, &t);
                    if widgets::tool_button(ui, Icon::Move, "Move to", &t, enabled("mail.move"))
                        .on_hover_text("Move to a folder (Ctrl+Shift+V)")
                        .clicked()
                    {
                        acts.push(Action::Dialog(Dialog::move_to()));
                    }
                    widgets::group_sep(ui, &t);
                    for (id, icon, label, tip) in [
                        ("mail.reply", Icon::Reply, "Reply", "Reply (Ctrl+R)"),
                        ("mail.replyAll", Icon::ReplyAll, "Reply all", "Reply all (Ctrl+Shift+R)"),
                        ("mail.forward", Icon::Forward, "Forward", "Forward (Ctrl+F)"),
                    ] {
                        if widgets::tool_button(ui, icon, label, &t, enabled(id)).on_hover_text(tip).clicked() {
                            acts.push(Action::Run(id, json!({})));
                        }
                    }
                    widgets::group_sep(ui, &t);
                    let first_unread = s.selected().first().is_some_and(|m| !m.flags.seen);
                    let (ricon, rlabel) = if first_unread { (Icon::Read, "Mark read") } else { (Icon::Unread, "Mark unread") };
                    if widgets::tool_button(ui, ricon, rlabel, &t, enabled("mail.toggleRead"))
                        .on_hover_text("Read / Unread (Ctrl+Q / Ctrl+U)")
                        .clicked()
                    {
                        acts.push(Action::Run("mail.toggleRead", json!({})));
                    }
                    if widgets::tool_button(ui, Icon::Flag, "Flag", &t, enabled("mail.flag")).on_hover_text("Flag / Unflag (Insert)").clicked() {
                        acts.push(Action::Run("mail.flag", json!({})));
                    }
                    let sel = !s.selection.is_empty();
                    let r = widgets::tool_button(ui, Icon::Tag, "Categorize", &t, sel);
                    egui::Popup::menu(&r).show(|ui| organize::categorize_menu(ui, s, &t, acts));
                    let pinned = s.selected().first().is_some_and(|m| m.flags.pinned());
                    if widgets::tool_button(ui, Icon::Pin, if pinned { "Unpin" } else { "Pin" }, &t, sel).clicked() {
                        acts.push(Action::Run("mail.pin", json!({"on": !pinned})));
                    }
                    let r = widgets::tool_button(ui, Icon::Clock, "Snooze", &t, sel);
                    egui::Popup::menu(&r).show(|ui| organize::snooze_menu(ui, s, &t, acts));
                    widgets::group_sep(ui, &t);
                    if widgets::tool_button(ui, Icon::Sync, "Send / Receive", &t, enabled("mail.sync"))
                        .on_hover_text("Send / Receive all folders (F9)")
                        .clicked()
                    {
                        acts.push(Action::Run("mail.sync", json!({})));
                    }
                }
                RibbonTab::View => {
                    ui.label(RichText::new("Reading pane").color(t.dim));
                    for (v, label) in [(ReadingPane::Right, "Right"), (ReadingPane::Bottom, "Bottom"), (ReadingPane::Off, "Off")] {
                        if ui.selectable_label(s.prefs.reading_pane == v, label).clicked() {
                            acts.push(Action::Run("view.readingPane", json!({"value": label.to_lowercase()})));
                        }
                    }
                    widgets::group_sep(ui, &t);
                    if ui
                        .selectable_label(s.prefs.focused_inbox, "Focused Inbox")
                        .on_hover_text("Put newsletters and automatic mail in Other")
                        .clicked()
                    {
                        acts.push(Action::Run("view.focused", json!({})));
                    }
                    if ui.selectable_label(s.prefs.conversations, "Conversations").on_hover_text("Group messages by conversation").clicked() {
                        acts.push(Action::Run("view.conversations", json!({})));
                    }
                    if ui.selectable_label(s.prefs.show_preview, "Message preview").clicked() {
                        acts.push(Action::Run("view.preview", json!({})));
                    }
                    if ui.selectable_label(s.prefs.compact, "Compact").clicked() {
                        acts.push(Action::Run("view.compact", json!({})));
                    }
                    widgets::group_sep(ui, &t);
                    ui.label(RichText::new("Theme").color(t.dim));
                    for (v, label) in [(None, "System"), (Some(false), "Light"), (Some(true), "Dark")] {
                        if ui.selectable_label(self.ui.dark == v, label).clicked() {
                            acts.push(Action::SetDark(v));
                        }
                    }
                }
            }
        });
    }

    fn status_bar(&mut self, ui: &mut Ui) {
        let t = self.ui.tokens;
        let s = &self.session;
        ui.horizontal_centered(|ui| {
            let unread = s.list.iter().filter(|m| !m.flags.seen).count();
            ui.label(RichText::new(format!("Items: {}", s.list.len())).color(t.dim).small());
            if unread > 0 {
                ui.label(RichText::new(format!("Unread: {unread}")).color(t.dim).small());
            }
            if !s.outbox.is_empty() {
                ui.label(RichText::new(format!("Outbox: {}", s.outbox.len())).color(t.warn).small());
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                for a in s.accounts.iter().rev() {
                    let (color, text) = match &a.status {
                        SyncStatus::Online { at } => (t.ok, format!("Updated {}", mailcraft_engine::model::dates::list_time(*at, &s.clock))),
                        SyncStatus::Syncing { folder } => {
                            (t.accent, format!("Updating {}…", s.folder(&a.config.id, folder).map(|f| f.display_name()).unwrap_or_default()))
                        }
                        SyncStatus::Connecting => (t.accent, "Connecting…".to_string()),
                        SyncStatus::Offline if a.is_local() => (t.faint, "Sample mailbox (offline)".to_string()),
                        SyncStatus::Offline => (t.faint, "Offline".to_string()),
                        SyncStatus::NeedsPassword { .. } => (t.danger, "Password needed".to_string()),
                        SyncStatus::Error { error } => (t.warn, format!("Disconnected: {}", widgets::shorten(error, 60))),
                    };
                    let r = ui.label(RichText::new(text).color(t.dim).small());
                    let label = a.config.email.clone();
                    let r = r.on_hover_text(label);
                    if matches!(a.status, SyncStatus::NeedsPassword { .. }) && r.interact(Sense::click()).clicked() {
                        self.ui.dialog = Some(Dialog::password(&a.config));
                    }
                    let (dot, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
                    ui.painter().circle_filled(dot.center(), 4.0, color);
                    ui.add_space(10.0);
                }
            });
        });
    }

    fn toasts(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        if self.session.notices.len() < self.ui.seen_notices {
            self.ui.seen_notices = 0;
        }
        if let Some(n) = self.session.notices.get(self.ui.seen_notices..).and_then(|v| v.last()) {
            self.ui.toast = Some((n.text.clone(), n.error, now));
        }
        self.ui.seen_notices = self.session.notices.len();
        let Some((text, error, at)) = self.ui.toast.clone() else { return };
        let ttl = if error { 7.0 } else { 3.5 };
        if now - at > ttl {
            self.ui.toast = None;
            return;
        }
        let t = self.ui.tokens;
        egui::Area::new(egui::Id::new("toast")).anchor(egui::Align2::RIGHT_BOTTOM, vec2(-16.0, -36.0)).order(egui::Order::Foreground).show(
            ctx,
            |ui| {
                egui::Frame::new()
                    .fill(if error { t.danger } else { t.text })
                    .corner_radius(8)
                    .inner_margin(Margin::symmetric(14, 10))
                    .shadow(egui::epaint::Shadow { offset: [0, 2], blur: 10, spread: 0, color: Color32::from_black_alpha(60) })
                    .show(ui, |ui| {
                        ui.set_max_width(420.0);
                        ui.label(RichText::new(text).color(if error { Color32::WHITE } else { t.surface }));
                    });
            },
        );
    }

    /// Preferences and theme for the app to save.
    pub fn saved_state(&self) -> Value {
        json!({ "prefs": self.session.prefs, "dark": self.ui.dark })
    }

    pub fn restore_state(&mut self, v: &Value) {
        if let Some(p) = v.get("prefs").and_then(|p| serde_json::from_value(p.clone()).ok()) {
            self.session.prefs = p;
            self.session.refresh_list();
        }
        self.ui.dark = v.get("dark").and_then(Value::as_bool);
    }
}

#[cfg(test)]
mod tests;
