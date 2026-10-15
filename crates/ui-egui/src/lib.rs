//! MailCraft's egui front end: the familiar three-pane mail layout (folders, message list,
//! reading pane) with a simplified ribbon, search box, inline compose and account dialogs.
//!
//! The UI is thin: it reads [`Session`] state and changes it through commands (by id), so the
//! toolbar, menus, shortcuts and agents all behave the same. The network sits behind
//! [`Backend`] and platform services (file dialogs, opening files, the clock) behind
//! [`Services`], so nothing here depends on threads, sockets or a windowing crate.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

/// An interface string in the current language (see [`i18n`]).
macro_rules! tl {
    ($s:expr) => {
        $crate::i18n::t($s)
    };
}

/// A translated template with `{name}` values filled in: `tf!("Saved {path}", path = p.display())`.
macro_rules! tf {
    ($s:expr $(, $k:ident = $v:expr)* $(,)?) => {
        $crate::i18n::tfmt($s, &[$((stringify!($k), &$v.to_string())),*])
    };
}

pub mod backstage;
pub mod bidi;
mod calendar;
pub mod craft_fonts;
mod dialogs;
pub mod i18n;
pub mod icons;
mod list;
mod nav;
mod organize;
mod people;
mod pim_dialogs;
mod reading;
mod rich_edit;
pub mod theme;
mod thumbs;
pub use thumbs::VideoFrame;
mod widgets;

use std::collections::HashMap;
use std::path::PathBuf;

use egui::{Align, Color32, Key, Layout, Margin, RichText, Sense, Ui, vec2};
use mailcraft_engine::model::{Event, Job, MessageKey, SyncStatus};
use mailcraft_engine::{CalMode, Module, ReadingPane, Session};
use serde_json::{Value, json};

pub use dialogs::Dialog;
use icons::Icon;
use theme::Tokens;

/// Whether the message being written shows in the reading pane: when it isn't popped out, or
/// where windows of its own can't open (the browser).
pub(crate) fn compose_inline(c: &mailcraft_engine::Compose, ctx: &egui::Context) -> bool {
    !c.window || ctx.embed_viewports()
}

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
    /// Where an installed Craft app is (`pdfcraft`, `wordcraft`…), if it is.
    fn craft_app(&mut self, _command: &str) -> Option<PathBuf> {
        None
    }
    /// Open a file with a given app.
    fn open_with(&mut self, _app: &std::path::Path, _file: &std::path::Path) -> Result<(), String> {
        Err("not available".into())
    }
    /// A folder to save into (`None` when cancelled).
    fn pick_folder(&mut self) -> Option<PathBuf> {
        None
    }
    /// An installed font for a Chinese, Japanese or Korean interface language (`ja`, `ko`,
    /// `zh-hans`, `zh-hant`): its bytes and the index in a collection.
    fn cjk_font(&mut self, _lang: &str) -> Option<(Vec<u8>, u32)> {
        None
    }
    /// An installed font with Arabic and Hebrew, for when the main font hasn't them.
    fn rtl_font(&mut self) -> Option<Vec<u8>> {
        None
    }
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
    /// Open a message (its key) in a browser window of its own, as web mail does; `false` when windows are
    /// windows here.
    fn open_message_window(&mut self, _key: &str) -> bool {
        false
    }
    /// Close this page (a message's own window, when it's done).
    fn close_page(&mut self) {}
    /// An Outlook data file to open (`None` when cancelled).
    fn pick_data_file(&mut self) -> Option<PathBuf> {
        None
    }
    /// The folder MailCraft keeps its data in, and whether it can be moved from here (not when
    /// it was chosen at start, or in the browser).
    fn data_folder(&self) -> Option<(PathBuf, bool)> {
        None
    }
    /// How a move carried out at this start went: the new folder, or why nothing moved.
    fn take_data_moved(&mut self) -> Option<Result<PathBuf, String>> {
        None
    }
    /// Where the data moves when MailCraft next starts, if a move was asked for.
    fn pending_data_move(&self) -> Option<PathBuf> {
        None
    }
    /// Ask for the data to move into `to` when MailCraft next starts (`None` forgets the move);
    /// the folder it will be in.
    fn move_data_folder(&mut self, _to: Option<&std::path::Path>) -> Result<Option<PathBuf>, String> {
        Err("not available".into())
    }
    /// A frame of a video file for its thumbnail. Called each frame until it isn't
    /// [`VideoFrame::Pending`], so slow work belongs on another thread.
    fn video_frame(&mut self, _path: &std::path::Path) -> VideoFrame {
        VideoFrame::Unavailable
    }
    /// Whether files can be dragged out of the window to other apps (see
    /// [`MailApp::take_drag_out`]).
    fn can_drag_out(&self) -> bool {
        false
    }
    /// Whether files are handed over as downloads (the browser, which has no files) instead of
    /// saved through [`Services::save_path`].
    fn downloads(&self) -> bool {
        false
    }
    /// Hand `bytes` to the user as a download named `name`.
    fn download(&mut self, _name: &str, _bytes: &[u8]) {}
    /// Start choosing files without waiting (the browser): `purpose` is `attach` or `picture`,
    /// and the files arrive through [`Services::take_picked`]. `false` (the default) means
    /// [`Services::pick_files`] answers at once.
    fn pick_later(&mut self, _purpose: &str) -> bool {
        false
    }
    /// Files chosen since the last call: (purpose, name, bytes).
    fn take_picked(&mut self) -> Vec<(String, String, Vec<u8>)> {
        Vec::new()
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
    SendReceive,
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
    /// Print the open message (as a PDF opened in the system's viewer).
    Print,
    /// Save the open message as a PDF, or as its `.eml` source.
    ExportPdf,
    SaveMessage,
    /// Open an attachment in a Craft app (its command name).
    OpenInCraft(usize, &'static str),
    /// Save every attachment of the open message into a folder.
    SaveAll,
    /// Sign an account in again: the browser for OAuth accounts, else the password dialog.
    SignInAgain(String),
    /// Insert a picture at the cursor of the message being written (from a file).
    InsertPicture,
    /// Paste a picture from the clipboard, if there is one.
    PastePicture,
    /// Choose the interface language (`auto` or a code).
    SetLanguage(String),
    /// A link in a message was clicked (web pages open in the browser; `mailto:` starts a message).
    OpenUrl(String),
    /// Run a command for the open dialog: it closes on success and shows the error otherwise.
    Submit(&'static str, Value),
    /// Bring the main window to the front (after acting from a message window).
    FocusMain,
    /// Open the File page at a page.
    Backstage(backstage::Page),
    /// An attachment is being dragged out of the window: hand it to the system as a file.
    DragOut(usize),
    /// Choose an Outlook data file and open it.
    OpenDataFile,
    /// Choose an existing data file for the account being added (its mail comes in first).
    PickAccountDataFile,
    /// An action from a message's own window, on that message (its attachments, Save As, Print).
    InWindow(MessageKey, Box<Action>),
    /// Choose where to write an account's mail as an Outlook data file.
    ExportDataFile(String),
    /// Show a folder in the system's file manager.
    OpenFolder(PathBuf),
    /// Choose a folder to move MailCraft's data into (at the next start).
    MoveDataFolder,
    /// Forget a move asked for.
    CancelDataMove,
}

/// Where a window was and how big (in points), and whether it was maximized: its normal place is
/// kept while maximized, so Restore goes back to it, as Outlook does.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct WindowPlace {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    #[serde(default)]
    pub maximized: bool,
}

impl WindowPlace {
    /// Note where a window is now (nothing while it's minimized; while maximized, only that).
    pub fn note(place: &mut Option<WindowPlace>, ui: &Ui) {
        let v = ui.input(|i| i.viewport().clone());
        if v.minimized == Some(true) {
            return;
        }
        let maximized = v.maximized == Some(true);
        if maximized {
            if let Some(p) = place.as_mut() {
                p.maximized = true;
            }
            return;
        }
        if let (Some(outer), Some(inner)) = (v.outer_rect, v.inner_rect) {
            *place = Some(WindowPlace { x: outer.min.x, y: outer.min.y, w: inner.width(), h: inner.height(), maximized: false });
        }
    }

    /// A place that's sensible to open at: a real size, and on a screen as far as can be told.
    pub fn usable(self) -> Option<WindowPlace> {
        let fine = |v: f32| v.is_finite();
        (fine(self.x) && fine(self.y) && self.w >= 200.0 && self.h >= 150.0 && self.w < 20_000.0 && self.h < 20_000.0)
            .then_some(self)
            .filter(|p| p.x > -2_000.0 && p.y > -200.0 && p.x < 20_000.0 && p.y < 20_000.0)
    }

    /// Open a window where this place says.
    pub fn apply(self, b: egui::ViewportBuilder) -> egui::ViewportBuilder {
        match self.usable() {
            Some(p) => b.with_position([p.x, p.y]).with_inner_size([p.w, p.h]).with_maximized(p.maximized),
            None => b,
        }
    }
}

/// UI-only state.
pub struct UiState {
    /// Where the main window, the last message window and the last compose window were.
    pub places: std::collections::BTreeMap<String, WindowPlace>,
    pub tokens: Tokens,
    /// The platform has files (temporary ones for other apps): not in the browser.
    pub has_files: bool,
    /// Where the data is kept and whether it can be moved; where it moves at the next start.
    pub data_folder: Option<(PathBuf, bool)>,
    pub pending_move: Option<PathBuf>,
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
    /// The message editor's state (selection, undo, what's typed next).
    pub(crate) editor: Option<rich_edit::Editor>,
    /// The recipient box showing suggestions (`to`, `cc`, `bcc`), the highlighted one, and the
    /// text for which Escape hid them.
    pub(crate) suggest_field: Option<&'static str>,
    pub(crate) suggest_index: usize,
    pub(crate) suggest_dismissed: Option<String>,
    /// Installed Craft apps (command, path), found at start-up.
    pub(crate) craft_apps: Vec<(&'static str, PathBuf)>,
    /// The interface language setting (`auto` or a language code).
    pub language: String,
    /// The fonts installed, kept to add a CJK fallback later (and for PDF thumbnails).
    pub(crate) fonts: (Option<Vec<u8>>, Option<Vec<u8>>),
    /// A fallback font for Arabic and Hebrew (when the main font hasn't them).
    rtl_font: Option<Vec<u8>>,
    /// The language whose CJK font is loaded.
    cjk_loaded: Option<&'static str>,
    /// The app's name in the window title (`MailCraft`, or `MailCraft – Sample mailbox`).
    pub title_base: String,
    /// This page is a message's own window (the browser): only that message is shown.
    pub page_message: Option<MessageKey>,
    pub(crate) title_shown: String,
    /// Raise the main window on the next frame.
    focus_main: bool,
    /// Video attachments written out for the platform to take a frame from, and the frames.
    pub(crate) video_files: HashMap<(MessageKey, usize), PathBuf>,
    pub(crate) video_frames: HashMap<PathBuf, VideoFrame>,
    /// The File page, while it's open.
    pub backstage: Option<backstage::Backstage>,
    /// The month the calendar's small month shows (`year * 12 + month - 1`; `None`: the day on
    /// screen's).
    pub(crate) mini_month: Option<i64>,
    /// The hours were scrolled to the working day once.
    pub(crate) cal_scrolled: bool,
    pub(crate) people_query: String,
    /// The person whose card is shown (uid).
    pub(crate) person: Option<String>,
    /// The invitation in the open message, worked out once per message and calendar change.
    pub(crate) invite: Option<(MessageKey, u64, Option<mailcraft_engine::Invite>)>,
    /// A file to drag out of the window (the app starts the system drag).
    drag_out: Option<PathBuf>,
    /// The platform can drag files out.
    pub(crate) can_drag_out: bool,
}

pub struct MailApp {
    pub session: Session,
    pub backend: Box<dyn Backend>,
    pub services: Box<dyn Services>,
    pub ui: UiState,
}

impl MailApp {
    pub fn new(session: Session, backend: Box<dyn Backend>, mut services: Box<dyn Services>) -> MailApp {
        let seen = session.notices.len();
        let has_files = !services.downloads();
        let (data_folder, pending_move) = (services.data_folder(), services.pending_data_move());
        let mut session = session;
        match services.take_data_moved() {
            Some(Ok(to)) => session.notice(tf!("Your mail is now kept in {folder}", folder = to.display().to_string()), false),
            Some(Err(e)) => session.notice(tf!("Your mail couldn't be moved, so it stays where it was: {error}", error = e), true),
            None => {}
        }
        let craft_apps: Vec<(&'static str, PathBuf)> = ["pdfcraft", "wordcraft", "gridcraft", "deckcraft", "photocraft", "filmcraft", "soundcraft"]
            .into_iter()
            .filter_map(|c| services.craft_app(c).map(|p| (c, p)))
            .collect();
        let mut app = MailApp {
            session,
            backend,
            services,
            ui: UiState {
                tokens: Tokens::light(),
                has_files,
                data_folder,
                pending_move,
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
                editor: None,
                suggest_field: None,
                suggest_index: 0,
                suggest_dismissed: None,
                craft_apps: Vec::new(),
                language: i18n::AUTO.into(),
                fonts: (None, None),
                rtl_font: None,
                cjk_loaded: None,
                title_base: "MailCraft".into(),
                places: Default::default(),
                page_message: None,
                title_shown: String::new(),
                focus_main: false,
                video_files: HashMap::new(),
                video_frames: HashMap::new(),
                backstage: None,
                mini_month: None,
                cal_scrolled: false,
                people_query: String::new(),
                person: None,
                invite: None,
                drag_out: None,
                can_drag_out: false,
            },
        };
        app.ui.craft_apps = craft_apps;
        app.ui.can_drag_out = app.services.can_drag_out();
        app
    }

    /// A file the user started dragging out of the window, for the app to hand to the system's
    /// drag and drop (egui can't).
    pub fn take_drag_out(&mut self) -> Option<PathBuf> {
        self.ui.drag_out.take()
    }

    /// Register the UI fonts, preferring an installed system font (call before the first frame).
    pub fn install_fonts(&mut self, ctx: &egui::Context, regular: Option<Vec<u8>>, bold: Option<Vec<u8>>) {
        self.ui.fonts = (regular.clone(), bold.clone());
        bidi::set_coverage(regular.as_deref());
        if !bidi::covers_rtl() {
            self.ui.rtl_font = self.services.rtl_font();
        }
        theme::install_fonts_with(ctx, regular, bold, None, self.ui.rtl_font.clone());
        self.ui.fonts_ready = true;
    }

    /// Draw in the chosen language; Chinese and Japanese need a CJK font, loaded on first use.
    fn apply_language(&mut self, ctx: &egui::Context) {
        let lang = i18n::Lang::from_pref(&self.ui.language);
        i18n::set_current(lang);
        if lang.cjk() && self.ui.cjk_loaded != Some(lang.code()) {
            self.ui.cjk_loaded = Some(lang.code());
            match self.services.cjk_font(lang.code()) {
                Some(cjk) => {
                    let (r, b) = self.ui.fonts.clone();
                    theme::install_fonts_with(ctx, r, b, Some(cjk), self.ui.rtl_font.clone());
                    ctx.request_repaint();
                }
                // The built-in fallbacks draw them (installed with the other fonts).
                None if craft_fonts::has_script("Jpan") || craft_fonts::has_script("Hans") => {}
                None => log::warn!("no Chinese/Japanese font found: those characters can't be drawn"),
            }
        }
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
        // Sign-in pages go to the browser.
        for url in std::mem::take(&mut self.session.open_urls) {
            if let Err(e) = self.services.open_url(&url) {
                self.session.notice(tf!("The sign-in page couldn't be opened: {error}", error = e), true);
            }
        }
        // Files chosen in the browser's picker.
        for (purpose, name, data) in self.services.take_picked() {
            if purpose == "picture" {
                let r = self.session.run("compose.insertPicture", &json!({"name": name, "data": data, "at": self.picture_at()}));
                self.place_picture(r);
            } else {
                self.run("compose.attach", json!({"name": name, "data": data}));
            }
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
        let mut main = self.ui.places.get("main").copied();
        WindowPlace::note(&mut main, ui);
        if let Some(p) = main {
            self.ui.places.insert("main".into(), p);
        }
        if !self.ui.fonts_ready {
            // Fonts take effect from the next frame; drawing now would miss the bold family.
            self.install_fonts(&ctx, None, None);
            ctx.request_repaint();
            return;
        }
        self.apply_language(&ctx);
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
        if let Some(key) = self.ui.page_message.clone() {
            self.message_page(ui, &key, &mut acts);
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
            return;
        }
        egui::Panel::top("topbar").exact_size(46.0).frame(egui::Frame::new().fill(t.accent).inner_margin(Margin::symmetric(12, 6))).show(ui, |ui| {
            self.top_bar(ui, &mut acts);
        });
        if self.ui.backstage.is_none() {
            egui::Panel::top("ribbon")
                .frame(egui::Frame::new().fill(t.chrome).inner_margin(Margin::symmetric(8, 4)).stroke(egui::Stroke::new(1.0, t.border)))
                .show(ui, |ui| {
                    self.ribbon(ui, &mut acts);
                });
        }
        egui::Panel::bottom("status").exact_size(26.0).frame(egui::Frame::new().fill(t.chrome).inner_margin(Margin::symmetric(10, 3))).show(
            ui,
            |ui| {
                self.status_bar(ui);
            },
        );

        if self.ui.backstage.is_some() {
            backstage::show(&self.session, &mut self.ui, ui, &mut acts);
        } else if self.session.accounts.is_empty() {
            egui::CentralPanel::default().frame(egui::Frame::new().fill(t.bg)).show(ui, |ui| {
                widgets::welcome(ui, &t, self.session.jmap_only, &mut acts);
            });
        } else {
            let module = self.session.module;
            // Right-to-left languages mirror the window: folders on the right, then the list,
            // the reading pane on the left.
            let rtl = i18n::current().rtl();
            let start = |id: &'static str| if rtl { egui::Panel::right(id) } else { egui::Panel::left(id) };
            // Minimized: a narrow strip with the way back and the modules, as Outlook's.
            let minimized = self.session.prefs.folder_pane_minimized;
            if minimized {
                start("nav-min")
                    .resizable(false)
                    .exact_size(52.0)
                    .frame(egui::Frame::new().fill(t.chrome).inner_margin(Margin::symmetric(4, 8)))
                    .show(ui, |ui| {
                        ui.vertical_centered(|ui| {
                            let chevron = if rtl { Icon::ChevronLeft } else { Icon::ChevronRight };
                            if widgets::tiny_icon_button(ui, chevron, "Show the folder pane", &t).clicked() {
                                acts.push(Action::Run("view.folderPane", json!({"minimized": false})));
                            }
                            ui.add_space(10.0);
                            module_switch_vertical(ui, &t, module, &mut acts);
                        });
                    });
            }
            if !minimized {
                start("nav")
                    .resizable(true)
                    .default_size(if module == Module::Mail { 230.0 } else { 260.0 })
                    .size_range(170.0..=380.0)
                    .frame(egui::Frame::new().fill(t.chrome).inner_margin(Margin::symmetric(6, 8)))
                    .show(ui, |ui| {
                        egui::Panel::bottom("modules").frame(egui::Frame::new().inner_margin(Margin::symmetric(0, 4))).show(ui, |ui| {
                            module_switch(ui, &t, module, &mut acts);
                        });
                        // Minimize, as Outlook's ‹ at the top of its folder pane.
                        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                            let chevron = if rtl { Icon::ChevronRight } else { Icon::ChevronLeft };
                            if widgets::tiny_icon_button(ui, chevron, "Minimize the folder pane", &t).clicked() {
                                acts.push(Action::Run("view.folderPane", json!({"minimized": true})));
                            }
                        });
                        match module {
                            Module::Mail => nav::show(&self.session, &t, ui, &mut acts),
                            Module::Calendar => calendar::nav(&self.session, &mut self.ui, ui, &mut acts),
                            Module::People => people::nav(&self.session, &mut self.ui, ui, &mut acts),
                        }
                    });
            }
            let pane = self.session.prefs.reading_pane;
            let list_frame = egui::Frame::new().fill(t.surface).stroke(egui::Stroke::new(1.0, t.border));
            match (module, pane) {
                (Module::Calendar, _) => {
                    egui::CentralPanel::default().frame(egui::Frame::new().fill(t.surface)).show(ui, |ui| {
                        calendar::show(&self.session, &mut self.ui, ui, &mut acts);
                    });
                }
                (Module::People, _) => {
                    egui::CentralPanel::default().frame(egui::Frame::new().fill(t.surface)).show(ui, |ui| {
                        people::show(&self.session, &mut self.ui, ui, &mut acts);
                    });
                }
                (Module::Mail, pane) => match pane {
                    ReadingPane::Right => {
                        start("list").resizable(true).default_size(390.0).size_range(280.0..=720.0).frame(list_frame).show(ui, |ui| {
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
                },
            }
        }

        self.message_windows(&ctx, &mut acts);
        self.compose_window(&ctx, &mut acts);

        // The address book reads the session's people.
        if let Some(Dialog::SelectNames(f)) = self.ui.dialog.as_mut()
            && f.stale
        {
            f.list = self.session.address_book(if f.people_only { "people" } else { "all" }, &f.query);
            f.stale = false;
        }
        // A message window's own dialogs show in that window.
        let in_window = self.session.compose.as_ref().is_some_and(|c| !compose_inline(c, &ctx));
        if let Some(mut d) = self.ui.dialog.take() {
            if in_window && d.for_compose() {
                self.ui.dialog = Some(d);
            } else {
                let keep = dialogs::show(&ctx, &mut d, &self.session, &t, &mut acts);
                if keep && self.ui.dialog.is_none() {
                    self.ui.dialog = Some(d);
                }
            }
        }
        self.toasts(&ctx);
        // Video thumbnails the cards asked for.
        for (path, frame) in self.ui.video_frames.iter_mut().filter(|(_, f)| **f == VideoFrame::Pending) {
            *frame = self.services.video_frame(path);
        }

        for a in acts {
            self.act(a);
        }
        self.flush_jobs();
        if std::mem::take(&mut self.ui.focus_main) {
            ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Focus);
        }
        let busy = self.session.accounts.iter().any(|a| matches!(a.status, SyncStatus::Connecting | SyncStatus::Syncing { .. }));
        ctx.request_repaint_after(std::time::Duration::from_secs(if busy || self.ui.toast.is_some() { 1 } else { 20 }));
    }

    /// The message being written, when it's in a window of its own (new mail, or popped out).
    /// Closing the window keeps it in Drafts.
    fn compose_window(&mut self, ctx: &egui::Context, acts: &mut Vec<Action>) {
        let Some(c) = self.session.compose.as_ref().filter(|c| !compose_inline(c, ctx)) else { return };
        let t = self.ui.tokens;
        let subject = if c.subject.trim().is_empty() { tl!("Untitled").to_string() } else { c.subject.trim().to_string() };
        let id = egui::ViewportId::from_hash_of(("compose-window", c.id));
        let builder = egui::ViewportBuilder::default()
            .with_title(i18n::as_written(|| tf!("{subject} – Message", subject = subject)))
            .with_inner_size([900.0, 720.0])
            .with_min_inner_size([520.0, 420.0]);
        // Where the last one was, and its size.
        let builder = match self.ui.places.get("compose") {
            Some(p) => p.apply(builder),
            None => builder,
        };
        let (session, st) = (&mut self.session, &mut self.ui);
        let close = ctx.show_viewport_immediate(id, builder, |ui, _| {
            let mut place = st.places.get("compose").copied();
            WindowPlace::note(&mut place, ui);
            if let Some(p) = place {
                st.places.insert("compose".into(), p);
            }
            let mut mine: Vec<Action> = Vec::new();
            let mut closing = ui.input(|i| i.viewport().close_requested());
            // Something written: ask first, as Outlook does.
            if closing && session.compose_written() {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::CancelClose);
                st.dialog = Some(Dialog::SaveChanges);
                closing = false;
            }
            egui::CentralPanel::default().frame(egui::Frame::new().fill(t.surface)).show(ui, |ui| {
                reading::compose_window(session, st, ui, &mut mine);
            });
            if let Some(mut d) = st.dialog.take() {
                if d.for_compose() {
                    let keep = dialogs::show(ui.ctx(), &mut d, session, &t, &mut mine);
                    if keep && st.dialog.is_none() {
                        st.dialog = Some(d);
                    }
                } else {
                    st.dialog = Some(d);
                }
            }
            acts.extend(mine);
            closing
        });
        if close {
            acts.push(Action::Run("compose.close", json!({"save": false})));
        }
    }

    /// Show only one message, as its own window does (`key` from [`MessageKey::to_id`]).
    pub fn show_message_page(&mut self, key: &str) {
        let Some(k) = MessageKey::from_id(key) else { return };
        if self.session.run("message.popOut", &json!({"key": key})).is_ok() {
            self.ui.page_message = Some(k);
        }
    }

    /// A message's own window: its toolbar and the message; a reply written here; closed when done.
    fn message_page(&mut self, ui: &mut Ui, key: &MessageKey, acts: &mut Vec<Action>) {
        let t = self.ui.tokens;
        if self.session.compose.is_some() {
            egui::CentralPanel::default().frame(egui::Frame::new().fill(t.surface)).show(ui, |ui| {
                reading::show(&mut self.session, &mut self.ui, ui, acts);
            });
            return;
        }
        let Some(w) = self.session.windows.iter().find(|w| &w.key == key).cloned() else {
            // Done with: the window goes (a browser may refuse; then this says so).
            self.services.close_page();
            egui::CentralPanel::default().frame(egui::Frame::new().fill(t.surface)).show(ui, |ui| {
                ui.centered_and_justified(|ui| ui.label(RichText::new(tl!("You can close this window.")).color(t.dim)));
            });
            return;
        };
        let subject =
            w.parsed.as_ref().map(|p| p.subject.clone()).filter(|s| !s.trim().is_empty()).unwrap_or_else(|| tl!("(no subject)").to_string());
        let title = i18n::as_written(|| tf!("{subject} – Message", subject = subject));
        if title != self.ui.title_shown {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.ui.title_shown = title;
        }
        egui::Panel::top("message-page-bar")
            .frame(egui::Frame::new().fill(t.chrome).inner_margin(Margin::symmetric(8, 6)).stroke(egui::Stroke::new(1.0, t.border)))
            .show(ui, |ui| window_toolbar(ui, &self.session, &t, key, acts));
        egui::CentralPanel::default().frame(egui::Frame::new().fill(t.surface)).show(ui, |ui| {
            reading::window(&mut self.session, &mut self.ui, ui, w, acts);
        });
    }

    /// Messages opened in windows of their own (double-click), each a native window with the
    /// message's own toolbar; in the browser, each in a browser window of its own.
    fn message_windows(&mut self, ctx: &egui::Context, acts: &mut Vec<Action>) {
        let t = self.ui.tokens;
        if self.ui.page_message.is_none() {
            for w in self.session.windows.clone() {
                if self.services.open_message_window(&w.key.to_id()) {
                    self.session.close_window(&w.key);
                }
            }
        }
        for w in self.session.windows.clone() {
            let key = w.key.clone();
            let subject =
                w.parsed.as_ref().map(|p| p.subject.clone()).filter(|s| !s.trim().is_empty()).unwrap_or_else(|| tl!("(no subject)").to_string());
            let id = egui::ViewportId::from_hash_of(("message-window", key.to_id()));
            let builder = egui::ViewportBuilder::default()
                .with_title(i18n::as_written(|| tf!("{subject} – Message", subject = subject)))
                .with_inner_size([940.0, 740.0])
                .with_min_inner_size([480.0, 360.0]);
            // Where the last one was, and its size.
            let builder = match self.ui.places.get("message") {
                Some(p) => p.apply(builder),
                None => builder,
            };
            let (session, st) = (&mut self.session, &mut self.ui);
            let close = ctx.show_viewport_immediate(id, builder, |ui, _| {
                let mut place = st.places.get("message").copied();
                WindowPlace::note(&mut place, ui);
                if let Some(p) = place {
                    st.places.insert("message".into(), p);
                }
                let mut mine: Vec<Action> = Vec::new();
                let closing = ui.input(|i| i.viewport().close_requested() || i.key_pressed(Key::Escape));
                egui::Panel::top(egui::Id::new(("message-window-bar", key.to_id())))
                    .frame(egui::Frame::new().fill(t.chrome).inner_margin(Margin::symmetric(8, 6)).stroke(egui::Stroke::new(1.0, t.border)))
                    .show(ui, |ui| window_toolbar(ui, session, &t, &key, &mut mine));
                egui::CentralPanel::default().frame(egui::Frame::new().fill(t.surface)).show(ui, |ui| {
                    reading::window(session, st, ui, w.clone(), &mut mine);
                });
                let leave = mine.iter().any(|a| matches!(a, Action::Run("message.closeWindow", _)));
                // What works on "the open message" works on this window's message.
                acts.extend(mine.into_iter().map(|a| match a {
                    Action::OpenAttachment(_)
                    | Action::SaveAttachment(_)
                    | Action::OpenInCraft(..)
                    | Action::DragOut(_)
                    | Action::SaveAll
                    | Action::Print
                    | Action::ExportPdf
                    | Action::SaveMessage
                    | Action::Run("attachment.preview", _) => Action::InWindow(key.clone(), Box::new(a)),
                    a => a,
                }));
                closing && !leave
            });
            if close {
                acts.push(Action::Run("message.closeWindow", json!({"key": key.to_id()})));
            }
        }
    }

    /// New mail: a desktop notification, the taskbar asking for attention when the window is in
    /// the background, and the unread count in the window title.
    fn announce(&mut self, ctx: &egui::Context) {
        let news = self.session.take_new_mail();
        if !news.is_empty() {
            // A system notification: text as written.
            let (title, body) = i18n::as_written(|| match news.as_slice() {
                [m] => (m.from.display().to_string(), if m.subject.is_empty() { tl!("(no subject)").to_string() } else { m.subject.clone() }),
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
                        [a, b] => tf!("{a} and {b}", a = a, b = b),
                        [a, b, ..] => tf!("{a}, {b} and others", a = a, b = b),
                        [] => String::new(),
                    };
                    (tf!("{n} new messages", n = many.len()), tf!("From {names}", names = names))
                }
            });
            self.services.notify(&title, &body);
            if !ctx.input(|i| i.focused) {
                ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(egui::UserAttentionType::Informational));
            }
        }
        let unread = self.session.inbox_unread();
        // The system lays out the title (right-to-left text too): as written.
        let (view, base) = i18n::as_written(|| {
            let view = match self.session.module {
                Module::Mail => view_title(&self.session),
                Module::Calendar => tl!("Calendar").to_string(),
                Module::People => tl!("People").to_string(),
            };
            (view, tl!(&self.ui.title_base).to_string())
        });
        let title = if self.session.view == mailcraft_engine::View::Empty {
            base.to_string()
        } else if unread > 0 {
            format!("({unread}) {view} – {base}")
        } else {
            format!("{view} – {base}")
        };
        if title != self.ui.title_shown {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.ui.title_shown = title;
        }
    }

    fn act(&mut self, a: Action) {
        match a {
            Action::Run("attachment.preview", p) => match self.session.run("attachment.preview", &p) {
                Ok(v) => self.ui.dialog = Some(Dialog::preview(&v, &self.session.clock)),
                Err(e) => self.session.notice(e.to_string(), true),
            },
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
            Action::FocusMain => self.ui.focus_main = true,
            Action::Backstage(p) => self.ui.backstage = Some(backstage::Backstage::new(p)),
            Action::InWindow(key, inner) => {
                // The window's message stands in as the open one while the action runs.
                if let Some(w) = self.session.windows.iter().find(|w| w.key == key).cloned() {
                    let main = self.session.reading.replace(w);
                    self.act(*inner);
                    self.session.reading = main;
                }
            }
            Action::PickAccountDataFile => {
                if let Some(path) = self.services.pick_data_file()
                    && let Some(dialogs::Dialog::AddAccount(f)) = self.ui.dialog.as_mut()
                {
                    f.pst = path.to_string_lossy().into_owned();
                }
            }
            Action::OpenDataFile => {
                if let Some(path) = self.services.pick_data_file() {
                    match self.session.run("account.importPst", &json!({"path": path.to_string_lossy()})) {
                        Ok(_) => {
                            self.ui.backstage = None;
                            self.session.notice(tf!("Opening {file}…", file = path.file_name().unwrap_or_default().to_string_lossy()), false);
                        }
                        Err(e) => self.session.notice(e.to_string(), true),
                    }
                }
            }
            Action::ExportDataFile(account) => {
                let name = self.session.account(&account).map(|a| a.config.name.clone()).unwrap_or_default();
                let name = mailcraft_engine::mime::sanitize_filename(if name.trim().is_empty() { "Mail" } else { name.trim() });
                if let Some(path) = self.services.save_path(&format!("{name}.pst")) {
                    match self.session.run("account.exportPst", &json!({"account": account, "path": path.to_string_lossy()})) {
                        Ok(_) => {
                            self.ui.backstage = None;
                            self.session.notice(tf!("Exporting to {file}…", file = path.file_name().unwrap_or_default().to_string_lossy()), false);
                        }
                        Err(e) => self.session.notice(e.to_string(), true),
                    }
                }
            }
            Action::OpenFolder(path) => {
                if let Err(e) = self.services.open_path(&path) {
                    self.session.notice(e, true);
                }
            }
            Action::MoveDataFolder => {
                if let Some(to) = self.services.pick_folder() {
                    match self.services.move_data_folder(Some(&to)) {
                        Ok(Some(target)) if self.ui.data_folder.as_ref().is_some_and(|(d, _)| *d == target) => {
                            self.ui.pending_move = None;
                        }
                        Ok(target) => {
                            self.ui.pending_move = target;
                            self.session.notice(tl!("Your mail moves there the next time MailCraft starts.").to_string(), false);
                        }
                        Err(e) => self.session.notice(e, true),
                    }
                }
            }
            Action::CancelDataMove => {
                if let Err(e) = self.services.move_data_folder(None) {
                    self.session.notice(e, true);
                }
                self.ui.pending_move = None;
            }
            Action::DragOut(i) => {
                if self.services.can_drag_out() {
                    self.ui.drag_out = self.attachment_to_temp(i);
                }
            }
            Action::Submit(id, p) => match self.session.run(id, &p) {
                Ok(v) => {
                    if id == "contact.create" {
                        self.ui.person = v.get("uid").and_then(Value::as_str).map(str::to_string);
                    }
                    self.ui.dialog = None;
                }
                Err(e) => {
                    if let Some(d) = self.ui.dialog.as_mut() {
                        d.set_error(e.to_string().trim_start_matches("bad parameters: ").to_string());
                    }
                }
            },
            Action::SetDark(d) => self.ui.dark = d,
            Action::SetLanguage(l) => self.ui.language = l,
            Action::OpenUrl(url) => self.open_link(&url),
            Action::SignInAgain(id) => self.sign_in_again(&id),
            Action::OpenInCraft(i, cmd) => {
                let Some(app) = self.ui.craft_apps.iter().find(|(c, _)| *c == cmd).map(|(_, p)| p.clone()) else { return };
                if let Some(path) = self.attachment_to_temp(i)
                    && let Err(e) = self.services.open_with(&app, &path)
                {
                    self.session.notice(tf!("{app} couldn't open it: {error}", app = app.display(), error = e), true);
                }
            }
            Action::Print | Action::ExportPdf if self.services.downloads() => {
                let name = format!("{}.pdf", self.message_file_name());
                match self.session.message_pdf() {
                    Ok(b) => self.services.download(&name, &b),
                    Err(e) => self.session.notice(e.to_string(), true),
                }
            }
            Action::SaveMessage if self.services.downloads() => {
                let name = format!("{}.eml", self.message_file_name());
                match self.session.message_source() {
                    Ok(b) => self.services.download(&name, &b),
                    Err(e) => self.session.notice(e.to_string(), true),
                }
            }
            Action::SaveAttachment(i) | Action::OpenAttachment(i) if self.services.downloads() => {
                let name = mailcraft_engine::mime::sanitize_filename(&self.attachment_name(i));
                match self.session.attachment_bytes(i) {
                    Ok(b) => self.services.download(&name, &b),
                    Err(e) => self.session.notice(e.to_string(), true),
                }
            }
            Action::SaveAll if self.services.downloads() => {
                let files: Vec<(usize, String)> = self
                    .session
                    .reading
                    .as_ref()
                    .and_then(|r| r.parsed.as_ref())
                    .map(|p| p.attachments.iter().filter(|a| !a.inline).map(|a| (a.index, a.name.clone())).collect())
                    .unwrap_or_default();
                for (i, name) in files {
                    if let Ok(b) = self.session.attachment_bytes(i) {
                        self.services.download(&mailcraft_engine::mime::sanitize_filename(&name), &b);
                    }
                }
            }
            Action::InsertPicture if self.services.pick_later("picture") => {}
            Action::AttachFiles if self.services.pick_later("attach") => {}
            Action::Print => {
                let dir = std::env::temp_dir().join("mailcraft-print");
                let _ = std::fs::create_dir_all(&dir);
                let path = dir.join(format!("{}.pdf", self.message_file_name()));
                match self.session.run("message.exportPdf", &json!({"path": path.to_string_lossy()})) {
                    Ok(_) => match self.services.open_path(&path) {
                        Ok(()) => self.session.notice(tl!("Opened as a PDF: print it from the viewer"), false),
                        Err(e) => self.session.notice(tf!("The PDF couldn't be opened: {error}", error = e), true),
                    },
                    Err(e) => self.session.notice(e.to_string(), true),
                }
            }
            Action::ExportPdf | Action::SaveMessage => {
                let (ext, id) = if matches!(a, Action::ExportPdf) { ("pdf", "message.exportPdf") } else { ("eml", "message.saveAs") };
                if let Some(path) = self.services.save_path(&format!("{}.{ext}", self.message_file_name())) {
                    match self.session.run(id, &json!({"path": path.to_string_lossy()})) {
                        Ok(_) => self.session.notice(tf!("Saved {path}", path = path.display()), false),
                        Err(e) => self.session.notice(e.to_string(), true),
                    }
                }
            }
            Action::SaveAll => {
                if let Some(dir) = self.services.pick_folder() {
                    match self.session.run("attachment.saveAll", &json!({"dir": dir.to_string_lossy()})) {
                        Ok(v) => {
                            let n = v.get("saved").and_then(Value::as_array).map_or(0, Vec::len);
                            self.session.notice(tf!("Saved {n} attachments in {folder}", n = n, folder = dir.display()), false);
                        }
                        Err(e) => self.session.notice(e.to_string(), true),
                    }
                }
            }
            Action::InsertPicture => {
                for p in self.services.pick_pictures() {
                    let r = self.session.run("compose.insertPicture", &json!({"path": p.to_string_lossy(), "at": self.picture_at()}));
                    self.place_picture(r);
                }
            }
            Action::PastePicture => {
                if let Some(png) = self.services.clipboard_picture() {
                    let r = self.session.run("compose.insertPicture", &json!({"name": "pasted.png", "data": png, "at": self.picture_at()}));
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
                if let Some(path) = self.attachment_to_temp(i)
                    && let Err(e) = self.services.open_path(&path)
                {
                    self.session.notice(tf!("{name} could not be opened: {error}", name = name, error = e), true);
                }
            }
        }
    }

    fn sign_in_again(&mut self, id: &str) {
        let Some(config) = self.session.account(id).map(|a| a.config.clone()) else { return };
        if config.oauth.is_some() {
            self.run("account.signIn", json!({"account": id}));
        } else {
            self.ui.dialog = Some(Dialog::password(&config));
        }
    }

    /// Put a picture's mark at the body's cursor.
    /// Where a picture goes: at the cursor in the message.
    fn picture_at(&self) -> usize {
        let n = self.session.compose.as_ref().map_or(0, |c| c.body.chars().count());
        self.ui.editor.as_ref().filter(|e| self.session.compose.as_ref().is_some_and(|c| c.id == e.id)).map_or(n, |e| e.sel.start.min(n))
    }

    fn place_picture(&mut self, r: mailcraft_engine::CmdResult) {
        match r {
            // In place: the cursor goes after it.
            Ok(_) => {
                if let Some(e) = self.ui.editor.as_mut() {
                    e.sel = e.sel.start + 1..e.sel.start + 1;
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
                self.session.notice(tf!("The link couldn't be opened: {error}", error = e), true);
            }
        } else {
            self.session.notice(tf!("Links like {link} aren't opened", link = widgets::shorten(u, 40)), true);
        }
    }

    /// A file name for the open message (its subject, made safe).
    fn message_file_name(&self) -> String {
        let subject = self.session.reading.as_ref().and_then(|r| r.parsed.as_ref()).map(|p| p.subject.clone()).unwrap_or_default();
        let name = mailcraft_engine::mime::sanitize_filename(if subject.trim().is_empty() { "message" } else { &subject });
        name.chars().take(80).collect()
    }

    /// Write an attachment to a temporary file to hand to another app.
    fn attachment_to_temp(&mut self, i: usize) -> Option<PathBuf> {
        if !self.ui.has_files {
            return None;
        }
        let name = self.attachment_name(i);
        let dir = std::env::temp_dir().join("mailcraft-open");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(mailcraft_engine::mime::sanitize_filename(&name));
        match self.session.run("attachment.save", &json!({"index": i, "path": path.to_string_lossy()})) {
            Ok(_) => Some(path),
            Err(e) => {
                self.session.notice(e.to_string(), true);
                None
            }
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
        if self.ui.backstage.is_some() {
            if ctx.input(|i| i.key_pressed(Key::Escape)) {
                self.ui.backstage = None;
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
            // Writing a message: its own keys (formatting, Ctrl+R aligns right…), not the main
            // window's; Ctrl+Shift+B opens the address book from any of its boxes.
            if typing && let Some(c) = self.session.compose.as_ref().filter(|c| compose_inline(c, ctx)) {
                if combo == "Mod+Shift+B" {
                    self.ui.dialog = Some(Dialog::select_names(c, "to"));
                }
                if !matches!(combo.as_str(), "Mod+S" | "Mod+Enter") {
                    continue;
                }
            }
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
            // In a text box these edit the text (select all, undo, redo).
            if typing && matches!(combo.as_str(), "Mod+A" | "Mod+Z" | "Mod+Y" | "Mod+Shift+Z") {
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
            ui.label(RichText::new(tl!("MailCraft")).font(theme::bold(16.0)).color(Color32::WHITE));
            ui.add_space(6.0);
            // Quick access, as Outlook's title bar: Send / Receive all folders, and Undo the last
            // move, delete, flag or category (Ctrl+Z).
            let can_sync = self.session.registry().get("mail.sync").is_some_and(|c| (c.enabled)(&self.session).is_none());
            if can_sync && widgets::bar_icon_button(ui, Icon::Sync, tl!("Send / Receive all folders (F9)")).clicked() {
                acts.push(Action::Run("mail.sync", json!({})));
            }
            if !self.session.undo.is_empty() && widgets::bar_icon_button(ui, Icon::Back, tl!("Undo (Ctrl+Z)")).clicked() {
                acts.push(Action::Run("edit.undo", json!({})));
            }
            ui.add_space(10.0);
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
                        .hint_text(tl!("Search mail (from:, subject:, has:attachment…)"))
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
                let (icon, tip) = if dark { (Icon::Sun, tl!("Switch to light mode")) } else { (Icon::Moon, tl!("Switch to dark mode")) };
                if widgets::bar_icon_button(ui, icon, tip).clicked() {
                    acts.push(Action::SetDark(Some(!dark)));
                }
                if widgets::bar_icon_button(ui, Icon::Gear, "Accounts and settings").clicked() {
                    acts.push(Action::Backstage(backstage::Page::Info));
                }
            });
        });
    }

    fn ribbon(&mut self, ui: &mut Ui, acts: &mut Vec<Action>) {
        let t = self.ui.tokens;
        let s = &self.session;
        ui.horizontal(|ui| {
            if ui.add(egui::Button::new(RichText::new(tl!("File")).color(t.dim).font(theme::regular(13.5))).frame(false)).clicked() {
                acts.push(Action::Backstage(backstage::Page::Info));
            }
            for (tab, label) in [(RibbonTab::Home, "Home"), (RibbonTab::SendReceive, "Send / Receive"), (RibbonTab::View, "View")] {
                let on = self.ui.tab == tab;
                let r = ui.add(
                    egui::Button::new(RichText::new(tl!(label)).color(if on { t.accent } else { t.dim }).font(if on {
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
        if s.module != Module::Mail {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                if s.module == Module::Calendar {
                    if widgets::accent_button(ui, Icon::Calendar, "New event", &t, !s.accounts.is_empty()).clicked() {
                        acts.push(Action::Dialog(Dialog::new_event(s, None)));
                    }
                    widgets::group_sep(ui, &t);
                    if widgets::tool_button(ui, Icon::Clock, "Today", &t, true).clicked() {
                        acts.push(Action::Run("calendar.goto", json!({"date": "today"})));
                    }
                    widgets::group_sep(ui, &t);
                    for (mode, label, code) in [(CalMode::Day, "Day", "day"), (CalMode::Week, "Week", "week"), (CalMode::Month, "Month", "month")] {
                        let r = widgets::tool_button(ui, Icon::Layout, label, &t, s.cal_mode != mode);
                        if r.clicked() {
                            acts.push(Action::Run("calendar.view", json!({"mode": code})));
                        }
                    }
                } else {
                    if widgets::accent_button(ui, Icon::Person, "New contact", &t, !s.accounts.is_empty()).clicked() {
                        acts.push(Action::Dialog(Dialog::new_contact()));
                    }
                    widgets::group_sep(ui, &t);
                    let picked = self.ui.person.as_deref().and_then(|u| s.find_person(u));
                    if widgets::tool_button(ui, Icon::Drafts, "Edit", &t, picked.is_some()).clicked()
                        && let Some(p) = &picked
                    {
                        acts.push(Action::Dialog(Dialog::edit_contact(&p.card)));
                    }
                    if widgets::tool_button(ui, Icon::Delete, "Delete", &t, picked.is_some()).clicked()
                        && let Some(p) = &picked
                    {
                        acts.push(Action::Run("contact.delete", json!({"uid": p.card.uid})));
                    }
                }
            });
            return;
        }
        // A narrow window scrolls the ribbon sideways rather than cutting buttons off.
        egui::ScrollArea::horizontal().id_salt("ribbon").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                match self.ui.tab {
                    RibbonTab::Home => {
                        if widgets::accent_button(ui, Icon::NewMail, "New mail", &t, enabled("mail.new"))
                            .on_hover_text(tl!("New mail (Ctrl+N)"))
                            .clicked()
                        {
                            acts.push(Action::Run("mail.new", json!({})));
                        }
                        // New items of other kinds, as Outlook's New Email drop-down.
                        let r = widgets::tiny_icon_button(ui, Icon::ChevronDown, "New items", &t);
                        egui::Popup::menu(&r).show(|ui| {
                            if ui.button(tl!("Email message")).clicked() {
                                acts.push(Action::Run("mail.new", json!({})));
                                ui.close();
                            }
                            if ui.button(tl!("Event")).clicked() {
                                acts.push(Action::Dialog(Dialog::new_event(s, None)));
                                ui.close();
                            }
                            if ui.button(tl!("Contact")).clicked() {
                                acts.push(Action::Dialog(Dialog::new_contact()));
                                ui.close();
                            }
                        });
                        widgets::group_sep(ui, &t);
                        if widgets::tool_button(ui, Icon::Delete, "Delete", &t, enabled("mail.delete")).on_hover_text(tl!("Delete (Del)")).clicked() {
                            if self.needs_delete_confirm() {
                                acts.push(Action::Dialog(Dialog::ConfirmDelete { count: s.selection.len() }));
                            } else {
                                acts.push(Action::Run("mail.delete", json!({})));
                            }
                        }
                        // Ignore the conversation, as Outlook's Delete drop-down.
                        let r = widgets::tiny_icon_button(ui, Icon::ChevronDown, "More delete options", &t);
                        egui::Popup::menu(&r).show(|ui| {
                            let ignoring = !s.selection.is_empty() && s.selected().iter().all(|m| s.is_ignored(m));
                            let label = if ignoring { "Stop ignoring the conversation" } else { "Ignore the conversation" };
                            if ui
                                .add_enabled(enabled("mail.ignore"), egui::Button::new(tl!(label)))
                                .on_hover_text(tl!("Its messages, and replies that arrive later, go to Deleted Items"))
                                .clicked()
                            {
                                acts.push(Action::Run("mail.ignore", json!({})));
                                ui.close();
                            }
                        });
                        if widgets::tool_button(ui, Icon::Archive, "Archive", &t, enabled("mail.archive"))
                            .on_hover_text(tl!("Archive (Backspace)"))
                            .clicked()
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
                            .on_hover_text(tl!("Move to a folder (Ctrl+Shift+V)"))
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
                            if widgets::tool_button(ui, icon, label, &t, enabled(id)).on_hover_text(tl!(tip)).clicked() {
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
                        if widgets::tool_button(ui, Icon::Flag, "Flag", &t, enabled("mail.flag"))
                            .on_hover_text(tl!("Flag / Unflag (Insert)"))
                            .clicked()
                        {
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
                        // Show only some mail, as Outlook's Filter Email.
                        let r = widgets::tool_button(ui, Icon::Funnel, "Filter", &t, true);
                        egui::Popup::menu(&r).show(|ui| {
                            for f in mailcraft_engine::ListFilter::EVERY {
                                if ui.selectable_label(s.filter == f, tl!(f.label())).clicked() {
                                    acts.push(Action::Run("list.filter", json!({"value": f.code()})));
                                    ui.close();
                                }
                            }
                        });
                    }
                    RibbonTab::SendReceive => {
                        if widgets::tool_button(ui, Icon::Sync, "Send / Receive all folders", &t, enabled("mail.sync")).on_hover_text("F9").clicked()
                        {
                            acts.push(Action::Run("mail.sync", json!({})));
                        }
                        if widgets::tool_button(ui, Icon::Folder, "Update folder", &t, enabled("folder.sync")).on_hover_text("Shift+F9").clicked() {
                            acts.push(Action::Run("folder.sync", json!({})));
                        }
                        if widgets::tool_button(ui, Icon::Send, "Send all", &t, enabled("outbox.send") && !s.prefs.work_offline)
                            .on_hover_text(tl!("Send what's waiting in the Outbox"))
                            .clicked()
                        {
                            acts.push(Action::Run("outbox.send", json!({})));
                        }
                        // Working offline: no connections; mail you send waits in the Outbox.
                        if self.ui.has_files {
                            widgets::group_sep(ui, &t);
                            let off = s.prefs.work_offline;
                            if ui
                                .selectable_label(off, tl!("Work offline"))
                                .on_hover_text(tl!("Close every connection: changes, mail you send and drafts wait on this computer"))
                                .clicked()
                            {
                                acts.push(Action::Run("mail.workOffline", json!({"on": !off})));
                            }
                        }
                    }
                    RibbonTab::View => {
                        ui.label(RichText::new(tl!("Reading pane")).color(t.dim));
                        for (v, label) in [(ReadingPane::Right, "Right"), (ReadingPane::Bottom, "Bottom"), (ReadingPane::Off, "Off")] {
                            if ui.selectable_label(s.prefs.reading_pane == v, tl!(label)).clicked() {
                                acts.push(Action::Run("view.readingPane", json!({"value": label.to_lowercase()})));
                            }
                        }
                        widgets::group_sep(ui, &t);
                        if ui
                            .selectable_label(s.prefs.focused_inbox, tl!("Focused Inbox"))
                            .on_hover_text(tl!("Put newsletters and automatic mail in Other"))
                            .clicked()
                        {
                            acts.push(Action::Run("view.focused", json!({})));
                        }
                        if ui
                            .selectable_label(s.prefs.conversations, tl!("Conversations"))
                            .on_hover_text(tl!("Group messages by conversation"))
                            .clicked()
                        {
                            acts.push(Action::Run("view.conversations", json!({})));
                        }
                        if ui.selectable_label(s.prefs.show_preview, tl!("Message preview")).clicked() {
                            acts.push(Action::Run("view.preview", json!({})));
                        }
                        if ui.selectable_label(s.prefs.compact, tl!("Compact")).clicked() {
                            acts.push(Action::Run("view.compact", json!({})));
                        }
                        if ui.selectable_label(!s.prefs.folder_pane_minimized, tl!("Folder pane")).clicked() {
                            acts.push(Action::Run("view.folderPane", json!({})));
                        }
                        widgets::group_sep(ui, &t);
                        let r = ui.button(tf!("Arrange by: {field}", field = tl!(s.prefs.arrange.label())));
                        egui::Popup::menu(&r).show(|ui| list::arrange_menu(ui, s, acts));
                        if ui.selectable_label(s.prefs.reverse_sort, tl!("Reverse sort")).clicked() {
                            acts.push(Action::Run("view.reverseSort", json!({})));
                        }
                        widgets::group_sep(ui, &t);
                        ui.label(RichText::new(tl!("Theme")).color(t.dim));
                        for (v, label) in [(None, "System"), (Some(false), "Light"), (Some(true), "Dark")] {
                            if ui.selectable_label(self.ui.dark == v, tl!(label)).clicked() {
                                acts.push(Action::SetDark(v));
                            }
                        }
                    }
                }
            });
        });
    }

    fn status_bar(&mut self, ui: &mut Ui) {
        let t = self.ui.tokens;
        let s = &self.session;
        let mut again: Option<String> = None;
        ui.horizontal_centered(|ui| {
            let unread = s.list.iter().filter(|m| !m.flags.seen).count();
            ui.label(RichText::new(tf!("Items: {n}", n = s.list.len())).color(t.dim).small());
            if unread > 0 {
                ui.label(RichText::new(tf!("Unread: {n}", n = unread)).color(t.dim).small());
            }
            if !s.outbox.is_empty() {
                ui.label(RichText::new(tf!("Outbox: {n}", n = s.outbox.len())).color(t.warn).small());
            }
            if s.prefs.work_offline {
                ui.label(RichText::new(tl!("Working offline")).color(t.warn).small());
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                for a in s.accounts.iter().rev() {
                    let (color, text) = match &a.status {
                        SyncStatus::Online { at } => {
                            (t.ok, tf!("Updated {time}", time = i18n::date(&mailcraft_engine::model::dates::list_time(*at, &s.clock))))
                        }
                        SyncStatus::Syncing { .. } if a.is_data_file() => (t.accent, tf!("Opening {file}…", file = a.config.email)),
                        SyncStatus::Syncing { folder } => (
                            t.accent,
                            tf!(
                                "Updating {folder}…",
                                folder =
                                    s.folder(&a.config.id, folder).map(|f| i18n::tc("folder", &f.display_name()).to_string()).unwrap_or_default()
                            ),
                        ),
                        SyncStatus::Connecting => (t.accent, tl!("Connecting…").to_string()),
                        SyncStatus::Offline if a.is_data_file() => (t.faint, tl!("Outlook data file").to_string()),
                        SyncStatus::Offline if a.is_local() => (t.faint, tl!("Sample mailbox (offline)").to_string()),
                        SyncStatus::Offline => (t.faint, tl!("Offline").to_string()),
                        SyncStatus::NeedsPassword { .. } => (t.danger, tl!("Password needed").to_string()),
                        SyncStatus::Error { error } => (t.warn, tf!("Disconnected: {error}", error = widgets::shorten(error, 60))),
                    };
                    let r = ui.label(RichText::new(text).color(t.dim).small());
                    let label = a.config.email.clone();
                    let r = r.on_hover_text(label);
                    if matches!(a.status, SyncStatus::NeedsPassword { .. }) && r.interact(Sense::click()).clicked() {
                        again = Some(a.config.id.clone());
                    }
                    let (dot, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
                    ui.painter().circle_filled(dot.center(), 4.0, color);
                    ui.add_space(10.0);
                }
            });
        });
        if let Some(id) = again {
            self.sign_in_again(&id);
        }
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
                        // Notices from the engine are English; the ones with a fixed text have a translation.
                        ui.label(RichText::new(tl!(&text)).color(if error { Color32::WHITE } else { t.surface }));
                    });
            },
        );
    }

    /// Preferences and theme for the app to save.
    pub fn saved_state(&self) -> Value {
        json!({ "prefs": self.session.prefs, "dark": self.ui.dark, "language": self.ui.language, "windows": self.ui.places })
    }

    pub fn restore_state(&mut self, v: &Value) {
        if let Some(p) = v.get("prefs").and_then(|p| serde_json::from_value::<mailcraft_engine::Prefs>(p.clone()).ok()) {
            let offline = p.work_offline;
            self.session.prefs = p;
            self.session.refresh_list();
            // Still working offline from last time: before any account connects.
            if offline {
                self.session.set_work_offline(true);
            }
        }
        self.ui.dark = v.get("dark").and_then(Value::as_bool);
        if let Some(w) = v.get("windows").and_then(|w| serde_json::from_value(w.clone()).ok()) {
            self.ui.places = w;
        }
        if let Some(l) = v.get("language").and_then(Value::as_str).filter(|l| *l == i18n::AUTO || i18n::Lang::from_code(l).is_some()) {
            self.ui.language = l.to_string();
        }
    }
}

/// The toolbar of a message window. Replying and forwarding happen in the main window, so
/// they close this one; so do Delete and Archive.
fn window_toolbar(ui: &mut Ui, s: &Session, t: &Tokens, key: &MessageKey, acts: &mut Vec<Action>) {
    let close = || Action::Run("message.closeWindow", json!({"key": key.to_id()}));
    let flagged = s.summary(key).is_some_and(|m| m.flags.flagged);
    let unread = s.summary(key).is_some_and(|m| !m.flags.seen);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for (id, icon, label) in
            [("mail.reply", Icon::Reply, "Reply"), ("mail.replyAll", Icon::ReplyAll, "Reply all"), ("mail.forward", Icon::Forward, "Forward")]
        {
            if widgets::tool_button(ui, icon, label, t, true).clicked() {
                acts.push(Action::Open(key.clone()));
                acts.push(Action::Run(id, json!({})));
                acts.push(Action::FocusMain);
                acts.push(close());
            }
        }
        widgets::group_sep(ui, t);
        for (id, icon, label) in [("mail.delete", Icon::Delete, "Delete"), ("mail.archive", Icon::Archive, "Archive")] {
            if widgets::tool_button(ui, icon, label, t, true).clicked() {
                acts.push(Action::Open(key.clone()));
                acts.push(Action::Run(id, json!({})));
                acts.push(close());
            }
        }
        if widgets::tool_button(ui, Icon::Move, "Move to", t, true).clicked() {
            acts.push(Action::Open(key.clone()));
            acts.push(Action::Dialog(Dialog::move_to()));
            acts.push(Action::FocusMain);
            acts.push(close());
        }
        widgets::group_sep(ui, t);
        let (ricon, rlabel) = if unread { (Icon::Read, "Mark read") } else { (Icon::Unread, "Mark unread") };
        if widgets::tool_button(ui, ricon, rlabel, t, true).clicked() {
            acts.push(Action::Run("list.select", json!({"key": key.to_id()})));
            acts.push(Action::Run(if unread { "mail.markRead" } else { "mail.markUnread" }, json!({})));
        }
        if widgets::tool_button(ui, Icon::Flag, if flagged { "Unflag" } else { "Flag" }, t, true).clicked() {
            acts.push(Action::Run("list.select", json!({"key": key.to_id()})));
            acts.push(Action::Run("mail.flag", json!({"on": !flagged})));
        }
        widgets::group_sep(ui, t);
        if widgets::tool_button(ui, Icon::Print, "Print", t, true).clicked() {
            acts.push(Action::Open(key.clone()));
            acts.push(Action::Print);
        }
        // Drawn inside the page (the browser), the window has no title bar to close it from.
        if ui.ctx().embed_viewports() {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if widgets::tool_button(ui, Icon::Close, "Close", t, true).clicked() {
                    acts.push(close());
                }
            });
        }
    });
}

/// Mail, Calendar and People, at the bottom of the side pane.
fn module_switch(ui: &mut Ui, t: &Tokens, module: Module, acts: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for (m, icon, label, code) in [
            (Module::Mail, Icon::Unread, "Mail", "mail"),
            (Module::Calendar, Icon::Calendar, "Calendar", "calendar"),
            (Module::People, Icon::People, "People", "people"),
        ] {
            let (r, resp) = ui.allocate_exact_size(vec2(40.0, 34.0), Sense::click());
            let on = m == module;
            if on || resp.hovered() {
                ui.painter().rect_filled(r, 6.0, if on { t.accent_soft } else { t.hover });
            }
            icons::paint(ui.painter(), icons::centered(r, 19.0), icon, if on { t.accent } else { t.dim });
            if resp.on_hover_text(tl!(label)).clicked() {
                acts.push(Action::Run("view.module", json!({"name": code})));
            }
        }
    });
}

/// Mail, Calendar and People one above another (the minimized folder pane).
fn module_switch_vertical(ui: &mut Ui, t: &Tokens, module: Module, acts: &mut Vec<Action>) {
    for (m, icon, label, code) in [
        (Module::Mail, Icon::Unread, "Mail", "mail"),
        (Module::Calendar, Icon::Calendar, "Calendar", "calendar"),
        (Module::People, Icon::People, "People", "people"),
    ] {
        let (r, resp) = ui.allocate_exact_size(vec2(40.0, 34.0), Sense::click());
        let on = m == module;
        if on || resp.hovered() {
            ui.painter().rect_filled(r, 6.0, if on { t.accent_soft } else { t.hover });
        }
        icons::paint(ui.painter(), icons::centered(r, 19.0), icon, if on { t.accent } else { t.dim });
        if resp.on_hover_text(tl!(label)).clicked() {
            acts.push(Action::Run("view.module", json!({"name": code})));
        }
    }
}

/// The list's title in the interface language.
pub(crate) fn view_title(s: &Session) -> String {
    match &s.search {
        Some((q, _)) => tf!("Results for “{query}”", query = q),
        None => i18n::tc("folder", &s.view_title()).to_string(),
    }
}

#[cfg(test)]
mod tests;
