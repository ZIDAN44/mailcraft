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

mod app;
mod dialogs;
pub mod i18n;
mod look;
mod panes;
mod reading;
mod rich_edit;
mod services;

// Each part by its own name, wherever its file sits (`crate::list`, `mailcraft_ui_egui::theme`).
use dialogs::pim_dialogs;
pub use look::thumbs::VideoFrame;
pub use look::{bidi, craft_fonts, icons, motion, theme};
use look::{thumbs, widgets};
pub use panes::backstage;
use panes::{calendar, list, nav, notes, organize, people, reminders};

use std::collections::HashMap;
use std::path::PathBuf;

use egui::{Align, Color32, Key, Layout, Margin, RichText, Sense, Ui, vec2};
use mailcraft_engine::model::{MessageKey, SyncStatus};
use mailcraft_engine::{CalMode, Module, ReadingPane, SearchScope, Session};
use serde_json::{Value, json};

pub(crate) use app::{SearchDrop, junk_menu, scope_label};
pub use app::{WindowPlace, after_open};
pub use dialogs::Dialog;
use icons::Icon;
pub use services::{Backend, NullBackend, NullServices, Services, UpdateState};
use theme::Tokens;

/// Whether the message being written shows in the reading pane: when it isn't popped out, or
/// where windows of its own can't open (the browser).
/// How the app reads HTML from the clipboard, set once at start (the clipboard is the
/// platform's; a build without one has none).
static CLIPBOARD_HTML: std::sync::OnceLock<fn() -> Option<String>> = std::sync::OnceLock::new();

/// Tell the editor how to read HTML from the clipboard, so text pasted from Outlook, Word or a
/// web page keeps its look.
pub fn set_clipboard_html(read: fn() -> Option<String>) {
    let _ = CLIPBOARD_HTML.set(read);
}

/// HTML on the clipboard (`None` when there is none).
pub(crate) fn clipboard_html() -> Option<String> {
    CLIPBOARD_HTML.get().and_then(|read| read())
}

pub(crate) fn compose_inline(c: &mailcraft_engine::Compose, ctx: &egui::Context) -> bool {
    !c.window || ctx.embed_viewports()
}

/// Put message `id`'s editor at hand (`st.editor`), keeping the one there for later.
pub(crate) fn use_editor(st: &mut UiState, id: u64) {
    if st.editor.as_ref().is_some_and(|e| e.id == id) {
        return;
    }
    if let Some(e) = st.editor.take() {
        st.editors.insert(e.id, e);
    }
    st.editor = st.editors.remove(&id);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RibbonTab {
    Home,
    SendReceive,
    View,
    /// While searching, as Outlook's Search tab: the scope, terms to narrow it, Close Search.
    Search,
}

/// Something a pane asks for; run after drawing so panes only borrow what they read.
pub enum Action {
    Run(&'static str, Value),
    /// Close MailCraft though online-only accounts hold unsent mail or changes (issue #4).
    CloseAnyway,
    /// Close, install the downloaded version, and open it.
    RestartToUpdate,
    /// Closing, with the update asked about: installed as MailCraft closes (true), or kept for
    /// later (false).
    UpdateOnClose(bool),
    Open(MessageKey),
    Dialog(Dialog),
    /// An action from one message's window: done to that message (several can be written at
    /// once, each in a window of its own).
    ForCompose(u64, Box<Action>),
    OpenAttachment(usize),
    SaveAttachment(usize),
    AttachFiles,
    FocusSearch,
    /// Make a note and write on it.
    NewNote,
    /// Search every folder for this (shown in the box, with the Search tab).
    Search(String),
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
    /// Save some attachments of the open message (by index) into a folder.
    SaveAttachments(Vec<usize>),
    /// The rules to a file, or rules from one (Outlook's Export and Import Rules).
    ExportRules,
    ImportRules,
    /// Pick a `.p12` / `.pfx` file for an account's certificate, then ask its password.
    ImportCertificate(String),
    /// The browser: open the account's certificate it keeps, with its password, for this page.
    UnlockCertificate(String),
    /// Choose a certificate on a smart card (or in the system's keychain) for an account.
    UseSmartCard(String),
    /// Choose one of the system's own certificates (Windows' store, the macOS Keychain).
    UseSystemCertificate(String),
    /// Sign an account in again: the browser for OAuth accounts, else the password dialog.
    SignInAgain(String),
    /// Insert a picture at the cursor of the message being written (from a file).
    InsertPicture,
    /// The account's signature at the cursor of the message being written.
    InsertSignature,
    /// Paste a picture from the clipboard, if there is one.
    PastePicture,
    /// Choose the interface language (`auto` or a code).
    SetLanguage(String),
    /// Open the signature editor for an account (`insert`: from a message, putting it in).
    EditSignature {
        account: String,
        insert: bool,
    },
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

/// UI-only state.
pub struct UiState {
    /// Where the main window, the last message window and the last compose window were.
    pub places: std::collections::BTreeMap<String, WindowPlace>,
    /// Closing was confirmed though it loses online-only accounts' unsent mail (issue #4); and
    /// the window is to close on the next frame.
    pub closing_anyway: bool,
    pub close_now: bool,
    /// When the close was last decided (the input's time): [`MailApp::logic`] decides it, and
    /// `ui` in the same pass doesn't again.
    pub(crate) close_checked: Option<f64>,
    /// Where the updater is, from [`Services::update_state`].
    pub update: UpdateState,
    /// The ready version whose bar was put away (Later), and the one the system was told of.
    pub(crate) update_later: Option<String>,
    pub(crate) update_told: Option<String>,
    /// What to do with the update as MailCraft closes was answered: closing asks no more.
    pub(crate) update_answered: bool,
    /// Restart now was chosen: the new version opens once installed, if the close goes ahead.
    pub(crate) restart_wanted: bool,
    /// A question about closing is (or was) on screen; put away, the close is off.
    pub(crate) close_asked: bool,
    /// Calendar reminders come due ([`reminders`]).
    pub reminders: reminders::State,
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
    /// The search box has the keyboard focus (the Search tab stays while it does).
    search_focused: bool,
    /// Put the search box's cursor at the end when it next takes the focus.
    search_cursor_end: bool,
    /// When to search for what's typed (a moment after the last key, so typing stays quick).
    search_due: Option<f64>,
    /// The search box's recent searches or advanced search, when open.
    search_drop: SearchDrop,
    /// Why the message being written couldn't be sent or saved, shown over it until dismissed.
    pub compose_alert: Option<String>,
    /// The attachment shown in place of its message's text (a click on its chip), as Outlook
    /// previews attachments in the reading pane.
    pub attachment_view: Option<(MessageKey, usize)>,
    /// The attachments selected in a message's strip (a click picks one, Ctrl+click adds or
    /// removes one, Ctrl+A takes them all), and whether the strip has the keyboard: it was the
    /// last thing clicked, so Ctrl+A selects attachments rather than mail.
    pub attachment_selected: Option<(MessageKey, Vec<usize>)>,
    pub attachments_active: bool,
    /// The pointer was over an attachment chip in the last frame drawn.
    pub attachment_hovered: bool,
    /// Where the ribbon was last drawn (a click on the Search tab's buttons takes the focus
    /// from the box, and mustn't close the tab).
    ribbon_rect: Option<egui::Rect>,
    /// The ribbon's buttons from this one on go without their labels: it didn't fit the window
    /// ([`widgets::RibbonFit`]).
    ribbon_icons_from: usize,
    pub dialog: Option<Dialog>,
    previews: HashMap<(MessageKey, usize), Option<egui::TextureHandle>>,
    /// Decoded web pictures, by address.
    web_pictures: HashMap<String, Option<egui::TextureHandle>>,
    pub toast: Option<(String, bool, f64)>,
    seen_notices: usize,
    scroll_to_focus: bool,
    /// The named font families the UI uses are registered (`install_fonts`).
    fonts_ready: bool,
    /// The messages being written that have been shown, to focus a new one's first field once.
    pub(crate) compose_shown: std::collections::HashSet<u64>,
    /// The editors of the messages being written other than the one acting now (`editor`).
    pub(crate) editors: HashMap<u64, rich_edit::Editor>,
    /// The message whose window a dialog for messages belongs to (Save changes, Address Book).
    pub(crate) dialog_for: Option<u64>,
    /// The body's cursor (characters), where pictures go.
    /// The message editor's state (selection, undo, what's typed next).
    pub(crate) editor: Option<rich_edit::Editor>,
    /// The recipient box showing suggestions (`to`, `cc`, `bcc`), the highlighted one, and the
    /// text for which Escape hid them.
    pub(crate) suggest_field: Option<&'static str>,
    pub(crate) suggest_index: usize,
    pub(crate) suggest_dismissed: Option<String>,
    /// The recipient box a suggestion was just picked into: it keeps the cursor through the rest
    /// of that click (the release lands outside the box, which would otherwise end its focus).
    pub(crate) suggest_refocus: Option<&'static str>,
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
    /// The day and week views have been scrolled to the working day once (each has its own scroll).
    pub(crate) cal_scrolled: [bool; 2],
    pub(crate) people_query: String,
    /// The address book People shows (account, book id); `None`: all of them.
    pub(crate) people_book: Option<(String, String)>,
    /// The message list's rows and where each starts, kept while the list stays the same.
    pub(crate) list_rows: Option<list::ListRows>,
    /// How People lays contacts out (Outlook's Change View).
    pub people_view: people::PeopleView,
    /// The People list's columns and sort.
    pub people_list: people::ListSettings,
    /// Notes: the search, the note being written on (and its text), a new note to write on,
    /// and the keyboard to give its text box.
    pub(crate) notes_query: String,
    /// How far the calendar's month grip has been dragged since it last added or took a month.
    pub(crate) months_drag: f32,
    pub(crate) note_edit: Option<(String, String)>,
    pub(crate) note_focus: bool,
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
        session.set_zone(services.zone().as_deref());
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
                closing_anyway: false,
                close_now: false,
                close_checked: None,
                update: UpdateState::None,
                update_later: None,
                update_told: None,
                update_answered: false,
                restart_wanted: false,
                close_asked: false,
                reminders: reminders::State::default(),
                has_files,
                data_folder,
                pending_move,
                dark: None,
                tab: RibbonTab::Home,
                search_text: String::new(),
                focus_search: false,
                search_focused: false,
                search_cursor_end: false,
                search_due: None,
                search_drop: SearchDrop::None,
                compose_alert: None,
                attachment_view: None,
                attachment_selected: None,
                attachments_active: false,
                attachment_hovered: false,
                ribbon_rect: None,
                ribbon_icons_from: usize::MAX,
                dialog: None,
                previews: HashMap::new(),
                web_pictures: HashMap::new(),
                toast: None,
                seen_notices: seen,
                scroll_to_focus: false,
                fonts_ready: false,
                compose_shown: Default::default(),
                editors: HashMap::new(),
                dialog_for: None,
                editor: None,
                suggest_field: None,
                suggest_index: 0,
                suggest_dismissed: None,
                suggest_refocus: None,
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
                cal_scrolled: [false; 2],
                people_query: String::new(),
                people_book: None,
                list_rows: None,
                people_view: people::PeopleView::default(),
                people_list: people::ListSettings::default(),
                notes_query: String::new(),
                months_drag: 0.0,
                note_edit: None,
                note_focus: false,
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
        theme::install_fonts_with(ctx, regular, bold, Vec::new(), self.ui.rtl_font.clone());
        self.ui.fonts_ready = true;
    }

    /// Draw in the chosen language. Chinese, Japanese and Korean fonts are always there as
    /// fallbacks (the language menu names them in their own script, and mail may be in them),
    /// the chosen language's own first: they draw shared characters differently.
    fn apply_language(&mut self, ctx: &egui::Context) {
        let lang = i18n::Lang::from_pref(&self.ui.language);
        i18n::set_current(lang);
        // egui's own parts mirrored for Arabic and Hebrew (the Craft apps' egui): submenus open
        // to the left, scroll bars on the left, closed arrows pointing left.
        if ctx.global_style().right_to_left != lang.rtl() {
            ctx.all_styles_mut(|s| s.right_to_left = lang.rtl());
        }
        let first = if lang.cjk() { lang.code() } else { "zh-hans" };
        if self.ui.cjk_loaded != Some(first) {
            self.ui.cjk_loaded = Some(first);
            // Korean has its own font; Simplified Chinese's covers Japanese kana too.
            let second = if first == "ko" { "zh-hans" } else { "ko" };
            let mut cjk: Vec<(Vec<u8>, u32)> = Vec::new();
            for code in [first, second] {
                if let Some(f) = self.services.cjk_font(code)
                    && !cjk.iter().any(|(b, i)| *i == f.1 && b.len() == f.0.len())
                {
                    cjk.push(f);
                }
            }
            if !cjk.is_empty() {
                let (r, b) = self.ui.fonts.clone();
                theme::install_fonts_with(ctx, r, b, cjk, self.ui.rtl_font.clone());
                ctx.request_repaint();
            } else if lang.cjk() && !(craft_fonts::has_script("Jpan") || craft_fonts::has_script("Hans")) {
                // (Otherwise the built-in fallbacks, installed with the other fonts, draw them.)
                log::warn!("no Chinese/Japanese/Korean font found: those characters can't be drawn");
            }
        }
    }

    /// Run a command, reporting failures in a toast.
    pub fn run(&mut self, id: &str, params: Value) {
        use mailcraft_engine::CmdError;
        match self.session.run(id, &params) {
            // Held to be looked at first: said in the message's window when it has one.
            Ok(v) if id == "compose.send" && v.get("check").is_some() => {
                let found = serde_json::from_value(v["check"].clone()).unwrap_or_default();
                self.ui.dialog_for = self.session.compose.as_ref().filter(|c| c.window).map(|c| c.id);
                self.ui.dialog = Some(Dialog::SendCheck(found));
            }
            Ok(_) => {}
            Err(CmdError::Disabled(_)) => {}
            // What went wrong with the message being written is said over it, in its own
            // window if it has one, as Outlook does (not in the main window's corner).
            Err(CmdError::Params(m) | CmdError::Failed(m)) if id.starts_with("compose.") && self.session.compose.is_some() => {
                self.ui.compose_alert = Some(m);
            }
            Err(CmdError::Params(m) | CmdError::Failed(m)) => self.session.notice(m, true),
            Err(e) => self.session.notice(e.to_string(), true),
        }
    }

    /// A `mailto:` link or a mail file the system handed MailCraft (as its mail app, or through
    /// Open with): a new message to write, the message shown, or the events or people added and
    /// shown. Nothing is sent and nothing is attached.
    pub fn open_handed(&mut self, what: &str) {
        use mailcraft_engine::CmdError;
        let mailto = mailcraft_engine::model::mailto::is_mailto(what);
        let r = if mailto { self.session.run("mail.mailto", &json!({"url": what})) } else { self.session.run("file.open", &json!({"path": what})) };
        let name = what.rsplit(['/', '\\']).next().unwrap_or(what).to_string();
        // How many the answer says were added or left out (none when it doesn't say).
        let tally = |answer: &Value, field: &str| answer[field].as_u64().unwrap_or(0);
        match r {
            Ok(_) if mailto => self.ui.backstage = None,
            Ok(v) => {
                self.ui.backstage = None;
                let left_out = tally(&v, "skipped");
                let left_out =
                    if left_out > 0 { format!(" {}", tf!("Left out because they couldn't be read: {n}", n = left_out)) } else { String::new() };
                match v.get("kind").and_then(Value::as_str) {
                    Some("calendar") => {
                        self.run("view.calendar", json!({}));
                        let added = tf!("Added to your calendar from {file}: {n}", file = name, n = tally(&v, "added"));
                        self.session.notice(format!("{added}{left_out}"), false);
                    }
                    Some("contacts") => {
                        self.run("view.people", json!({}));
                        let added = tf!("Added to your contacts from {file}: {n}", file = name, n = tally(&v, "added"));
                        self.session.notice(format!("{added}{left_out}"), false);
                    }
                    _ => self.ui.dialog = Some(Dialog::preview(&v, &self.session.clock)),
                }
            }
            Err(CmdError::Disabled(_)) if mailto => {
                self.session.notice(tl!("Add an account first: a new message needs an account to send from.").to_string(), true)
            }
            Err(CmdError::Disabled(_)) => {
                self.session.notice(tl!("Add an account first: events and people are kept with an account.").to_string(), true)
            }
            Err(e) => self.session.notice(tf!("Couldn't open {file}: {error}", file = name, error = e.to_string()), true),
        }
    }

    fn pump(&mut self) {
        let (now, offset) = self.services.clock();
        let before = self.session.clock.offset;
        self.session.set_clock(now, offset);
        // The offset changed (summer time, or the system's zone did): which zone it is, again.
        if offset != before {
            self.session.set_zone(self.services.zone().as_deref());
        }
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
            if let Some(account) = purpose.strip_prefix("certificate:") {
                self.ui.dialog = Some(Dialog::Certificate { account: account.to_string(), path: name, data: Some(data), password: String::new() });
            } else if purpose == "picture" {
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

    /// Preferences and theme for the app to save.
    pub fn saved_state(&self) -> Value {
        json!({ "prefs": self.session.prefs, "dark": self.ui.dark, "language": self.ui.language, "windows": self.ui.places, "peopleView": self.ui.people_view, "peopleList": self.ui.people_list })
    }

    pub fn restore_state(&mut self, v: &Value) {
        if let Some(p) = v.get("prefs").and_then(|p| serde_json::from_value::<mailcraft_engine::Prefs>(p.clone()).ok()) {
            let offline = p.work_offline;
            let keep = (p.keep_messages, p.keep_offline);
            self.session.prefs = p;
            self.session.refresh_list();
            // How much mail to keep, before any account connects (when it isn't the default).
            let d = mailcraft_engine::Prefs::default();
            if keep != (d.keep_messages, d.keep_offline) {
                self.session.apply_sync_prefs();
            }
            // Still working offline from last time: before any account connects.
            if offline {
                self.session.set_work_offline(true);
            }
        }
        self.ui.dark = v.get("dark").and_then(Value::as_bool);
        if let Some(pv) = v.get("peopleView").and_then(|w| serde_json::from_value(w.clone()).ok()) {
            self.ui.people_view = pv;
        }
        if let Some(pl) = v.get("peopleList").and_then(|w| serde_json::from_value(w.clone()).ok()) {
            self.ui.people_list = pl;
        }
        if let Some(w) = v.get("windows").and_then(|w| serde_json::from_value(w.clone()).ok()) {
            self.ui.places = w;
        }
        if let Some(l) = v.get("language").and_then(Value::as_str).filter(|l| *l == i18n::AUTO || i18n::Lang::from_code(l).is_some()) {
            self.ui.language = l.to_string();
        }
    }
}

/// The list's title in the interface language.
pub(crate) fn view_title(s: &Session) -> String {
    if let Some(name) = s.open_search_folder_name() {
        return name.to_string();
    }
    match &s.search {
        Some((q, _)) => tf!("Results for “{query}”", query = q),
        None => i18n::tc("folder", &s.view_title()).to_string(),
    }
}

#[cfg(test)]
mod tests;
