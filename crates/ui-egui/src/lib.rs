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

mod a11y;
mod app;
mod dialogs;
pub mod i18n;
mod keytips;
mod look;
mod panes;
mod reading;
mod rich_edit;
mod services;

// Each part by its own name, wherever its file sits (`crate::list`, `mailcraft_ui_egui::theme`).
use dialogs::pim_dialogs;
pub use look::thumbs::VideoFrame;
pub use look::{bidi, craft_fonts, faces, icons, mail_fonts, motion, theme};
use look::{drop_down, photos, progress, thumbs, widgets};
pub use panes::backstage;
use panes::{calendar, list, nav, notes, organize, people, reminders, shown, tasks, todo_bar};

use std::collections::HashMap;
use std::path::PathBuf;

use egui::{Align, Color32, Key, Layout, Margin, RichText, Sense, Ui, vec2};
use mailcraft_engine::model::{MessageKey, SyncStatus};
use mailcraft_engine::{CalMode, Module, ReadingPane, SearchScope, Session};
use serde_json::{Value, json};

pub use app::caption::{MIN_WINDOW_SIZE, take_zooming};
pub use app::{
    Action, BootCard, ClipboardRead, FramePart, MailApp, Monitors, PaneSizes, RibbonTab, UiState, WindowPlace, after_open, alt_code_pages, alt_codes,
    clipboard_keys, control_click, set_monitors,
};
pub(crate) use app::{Pane, SearchDrop, junk_menu, scope_label, use_editor};
pub use dialogs::{AccountKind, Dialog};

/// MailCraft's id: the Linux app id (its `.desktop` file and icon are named so), the macOS bundle
/// id and the Windows notifications' app id.
pub const APP_ID: &str = "io.github.zidan44.MailCraft";

/// A window of MailCraft's own beyond the main one (a message being written, one popped out, a
/// message's source), carrying [`APP_ID`]: without it a Wayland dock shows no icon for it and a
/// compositor's window rules can't match it (as PhotoCraft's extra windows).
pub(crate) fn os_window() -> egui::ViewportBuilder {
    egui::ViewportBuilder::default().with_app_id(APP_ID)
}

/// A lock that outlives a panic elsewhere (what the window's work shares with its threads is
/// whole between steps).
pub(crate) fn lock<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}
use icons::Icon;
pub use services::{Backend, InstallAsks, KeptOnClose, Lane, NullBackend, NullServices, Picked, Services, UpdateState, Work};
use theme::Tokens;

/// How the app reads HTML from the clipboard, set once at start (the clipboard is the
/// platform's; a build without one has none).
static CLIPBOARD_HTML: std::sync::OnceLock<fn() -> Option<String>> = std::sync::OnceLock::new();

/// Tell the editor how to read HTML from the clipboard, so text pasted from Outlook, Word or a
/// web page keeps its look.
pub fn set_clipboard_html(read: fn() -> Option<String>) {
    let _ = CLIPBOARD_HTML.set(read);
}

#[cfg(test)]
thread_local! {
    /// The HTML a test has on its clipboard (each test runs on a thread of its own).
    pub(crate) static TEST_CLIPBOARD_HTML: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// HTML on the clipboard (`None` when there is none).
pub(crate) fn clipboard_html() -> Option<String> {
    #[cfg(test)]
    if let Some(html) = TEST_CLIPBOARD_HTML.with(|h| h.borrow().clone()) {
        return Some(html);
    }
    CLIPBOARD_HTML.get().and_then(|read| read())
}

/// The user's folder `name` for files handed to other apps ([`mailcraft_engine::store::scratch_dir`]);
/// in the tests, the test's own (the user's is shared with a running MailCraft and other runs).
pub(crate) fn scratch_dir(name: &str) -> Option<std::path::PathBuf> {
    #[cfg(test)]
    if let Some(own) = tests::own_scratch_dir(name) {
        return Some(own);
    }
    mailcraft_engine::store::scratch_dir(name)
}

/// Whether the message being written shows in the reading pane: when it isn't popped out, or
/// where windows of its own can't open (the browser).
pub(crate) fn compose_inline(c: &mailcraft_engine::Compose, ctx: &egui::Context) -> bool {
    !c.window || ctx.embed_viewports()
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
