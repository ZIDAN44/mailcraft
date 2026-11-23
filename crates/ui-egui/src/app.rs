//! The window (`MailApp`), in parts: each continues `impl MailApp` with what it draws or does.

mod actions;
mod agents;
mod alt_code;
mod bodies;
pub(crate) mod boot;
mod chrome;
mod control_click;
mod data_files;
mod files;
pub(crate) mod frame;
mod keys;
pub(crate) mod source;
mod state;
mod updates;
mod windows;

use crate::*;

pub(crate) use chrome::caption;
use chrome::{bars, ribbon, search_box};
pub(crate) use data_files::{compacting, importing};
use files::dropped;

pub use actions::Action;
pub use agents::{AGENTS_OFF, AgentAnswer, AgentLogLine, AgentOutcome, Agents};
pub use alt_code::{alt_code_pages, alt_codes};
pub(crate) use bodies::Bodies;
pub use boot::BootCard;
pub use control_click::control_click;
pub use dropped::{ClipboardRead, clipboard_keys};
pub use frame::FramePart;
pub(crate) use keys::Pane;
pub use ribbon::RibbonTab;
pub(crate) use ribbon::junk_menu;
pub(crate) use search_box::{SearchDrop, SearchTimes, scope_label};
pub(crate) use state::use_editor;
pub use state::{PaneSizes, UiState};
#[cfg(test)]
pub(crate) use windows::work_area;
pub use windows::{Monitors, WindowPlace, after_open, set_monitors};

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
        let keeps_running = services.keeps_running();
        let (data_folder, pending_move) = (services.data_folder(), services.pending_data_move());
        let old_flatpak = services.carried_over_from();
        let mut session = session;
        session.set_zone(services.zone().as_deref());
        // The window's network side keeps each account's search index up, and the window steps
        // searches a little each frame rather than waiting for them: none of a search in the
        // command that starts it, after the frame was drawn, but in the next frame's share.
        session.set_indexing(true);
        session.set_background_search(true);
        session.set_search_first_units(0);
        match services.take_data_moved() {
            Some(Ok(to)) => session.notice(tf!("Your mail is now kept in {folder}", folder = to.display().to_string()), false),
            Some(Err(e)) => session.notice(tf!("Your mail couldn't be moved, so it stays where it was: {error}", error = e), true),
            None => {}
        }
        let craft_apps: Vec<(&'static str, PathBuf)> = ["pdfcraft", "wordcraft", "gridcraft", "deckcraft", "photocraft", "filmcraft", "soundcraft"]
            .into_iter()
            .filter_map(|c| services.craft_app(c).map(|p| (c, p)))
            .collect();
        let ui = UiState {
            has_files,
            keeps_running,
            data_folder,
            pending_move,
            old_flatpak,
            seen_notices: seen,
            craft_apps,
            can_drag_out: services.can_drag_out(),
            // Until the ribbon and the list are first drawn: every button labelled, a page of ten.
            ribbon_icons_from: usize::MAX,
            list_page: 10,
            language: i18n::AUTO.into(),
            title_base: "MailCraft".into(),
            ..UiState::default()
        };
        MailApp { session, backend, services, ui }
    }

    /// The files the user started dragging out of the window (none when no drag started), for
    /// the app to hand to the system's drag and drop (egui can't).
    pub fn take_drag_out(&mut self) -> Vec<PathBuf> {
        std::mem::take(&mut self.ui.drag_out)
    }

    /// Register the UI fonts, preferring an installed system font (call before the first frame).
    pub fn install_fonts(&mut self, ctx: &egui::Context, regular: Option<Vec<u8>>, bold: Option<Vec<u8>>) {
        self.ui.fonts = (regular.clone().map(std::sync::Arc::new), bold.clone().map(std::sync::Arc::new));
        bidi::set_coverage(regular.as_deref());
        if !bidi::covers_rtl() {
            self.ui.rtl_font = self.services.rtl_font();
        }
        theme::install_fonts_with(ctx, regular, bold, Vec::new(), self.ui.rtl_font.clone());
        self.ui.fonts_ready = true;
        // Every path into the window comes here first, with its context: from now on a screen
        // reader's update always carries its focus (`a11y`; added once, however often called).
        ctx.add_plugin(a11y::FocusGuard);
    }

    /// Draw in the chosen language. Chinese, Japanese and Korean fonts are always there as
    /// fallbacks (the language menu names them in their own script, and mail may be in them),
    /// the chosen language's own first: they draw shared characters differently.
    pub(super) fn apply_language(&mut self, ctx: &egui::Context) {
        let lang = i18n::Lang::from_pref(&self.ui.language);
        i18n::set_current(lang);
        // egui's own parts mirrored for Arabic and Hebrew (the Craft apps' egui): submenus open
        // to the left, scroll bars on the left, closed arrows pointing left.
        if ctx.global_style().right_to_left != lang.rtl() {
            ctx.all_styles_mut(|s| s.right_to_left = lang.rtl());
        }
        let first = if lang.cjk() { lang.code() } else { "zh-hans" };
        // Shown text with Chinese characters no font has (the browser has only the fonts it
        // brought; Simplified Chinese's is too large to bring, so it's fetched when wanted).
        if std::mem::take(&mut self.ui.han_missing) {
            self.services.want_font("zh-hans");
            self.ui.cjk_loaded = None;
        }
        if lang.code() == "zh-hans" {
            self.services.want_font("zh-hans");
        }
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
                let (r, b) = (self.ui.fonts.0.as_ref().map(|f| f.to_vec()), self.ui.fonts.1.as_ref().map(|f| f.to_vec()));
                theme::install_fonts_with(ctx, r, b, cjk, self.ui.rtl_font.clone());
                ctx.request_repaint();
            } else if self.services.font_pending() {
                // Asked again once it has come.
                self.ui.cjk_loaded = None;
            } else if lang.cjk() && !(craft_fonts::has_script("Jpan") || craft_fonts::has_script("Hans")) {
                // (Otherwise the built-in fallbacks, installed with the other fonts, draw them.)
                log::warn!("no Chinese/Japanese/Korean font found: those characters can't be drawn");
            }
        }
    }

    /// Run a command, reporting failures in a toast.
    pub fn run(&mut self, id: &str, params: Value) {
        self.answer(id, params);
    }

    /// Run a command as [`MailApp::run`] does: its answer, when it worked.
    pub(crate) fn answer(&mut self, id: &str, params: Value) -> Option<Value> {
        use mailcraft_engine::CmdError;
        let ran = self.session.run(id, &params);
        let answer = ran.as_ref().ok().cloned();
        match ran {
            // Held to be looked at first: said in the message's window when it has one.
            Ok(v) if id == "compose.send" && v.get("check").is_some() => {
                // One that can't be read (none ever should) still holds the message, the dialog
                // saying so rather than nothing.
                let found = serde_json::from_value(v["check"].clone()).unwrap_or_else(|e| {
                    log::warn!("what the check before sending found couldn't be read: {e}");
                    Vec::new()
                });
                self.ui.dialog_for = self.session.compose.as_ref().filter(|c| c.window).map(|c| c.id);
                self.ui.dialog = Some(Dialog::SendCheck(found));
            }
            // A message opened by a command (a script's, a conversation's row in the reading
            // pane) is brought into view in the list: it could be opened out of sight.
            Ok(_) if id == "list.open" => self.ui.scroll_to_focus = true,
            Ok(_) => {}
            // Not from an account that can't send: said over the message, as why it didn't go.
            Err(CmdError::Disabled(_))
                if id == "compose.send"
                    && let Some(b) = self.session.compose_send_block() =>
            {
                self.ui.compose_alert = Some(reading::ComposeAlert::Blocked(b));
            }
            Err(CmdError::Disabled(_)) => {}
            // What went wrong with the message being written is said over it, in its own
            // window if it has one, as Outlook does (not in the main window's corner). Sent to
            // no one: said in the window's own words (the engine's are for scripts).
            Err(CmdError::Params(_)) if id == "compose.send" && self.session.compose_has_no_recipient() => {
                self.ui.compose_alert = Some(reading::ComposeAlert::NoRecipient);
            }
            Err(CmdError::Params(m) | CmdError::Failed(m)) if id.starts_with("compose.") && self.session.compose.is_some() => {
                self.ui.compose_alert = Some(reading::ComposeAlert::Other(m));
            }
            Err(CmdError::Params(m) | CmdError::Failed(m)) => self.session.notice(m, true),
            Err(e) => self.session.notice(e.to_string(), true),
        }
        answer
    }

    /// A `mailto:` link or a mail file the system handed MailCraft (as its mail app, or through
    /// Open with): a new message to write, the message shown, or the events or people added and
    /// shown. Nothing is sent and nothing is attached.
    pub fn open_handed(&mut self, what: &str) {
        use mailcraft_engine::CmdError;
        let mailto = mailcraft_engine::model::mailto::is_mailto(what);
        let r = if mailto { self.session.run("mail.mailto", &json!({"url": what})) } else { self.session.run("file.open", &json!({"path": what})) };
        let name = mailcraft_engine::model::file_name(what);
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
                    // An Outlook message file: shown once it's read (`pump`).
                    Some("reading") => {}
                    // A message file: in a message window of its own, as one double-clicked is.
                    Some("message") if v.get("key").is_some() => {}
                    _ => self.ui.dialog = Some(Dialog::preview(&v, &self.session.clock)),
                }
            }
            Err(CmdError::Disabled(_)) if mailto => {
                self.session.notice(tl!("Add an account first: a new message needs an account to send from.").to_string(), true)
            }
            Err(CmdError::Disabled(_)) => {
                self.session.notice(tl!("Add an account first: events and people are kept with an account.").to_string(), true)
            }
            // One that names the file says all there is to say, by its name and not its whole path.
            Err(CmdError::Params(m) | CmdError::Failed(m)) if m.contains(what) || m.contains(name) => {
                self.session.notice(m.replace(what, name), true);
            }
            Err(e) => self.session.notice(tf!("Couldn't open {file}: {error}", file = name, error = e.to_string()), true),
        }
    }

    /// Files handed to MailCraft to send (issue #96: Windows' Send to › MailCraft, the file
    /// managers' Send with MailCraft, files dropped on its Dock icon): a new message with them
    /// attached, as files dropped on the window start one. A folder isn't attached; one that
    /// can't be read is said over the message.
    pub fn attach_handed(&mut self, files: &[String]) {
        use mailcraft_engine::CmdError;
        let (folders, files): (Vec<&String>, Vec<&String>) = files.iter().partition(|f| std::path::Path::new(f.as_str()).is_dir());
        for f in folders.iter().take(3) {
            self.session.notice(tf!("{folder} isn't attached: it's a folder", folder = mailcraft_engine::model::file_name(f)), true);
        }
        if files.is_empty() {
            return;
        }
        match self.session.run("mail.new", &json!({})) {
            Ok(_) => {
                self.ui.backstage = None;
                self.ui.compose_to_front = self.session.compose.as_ref().map(|c| c.id);
            }
            Err(CmdError::Disabled(_)) => {
                return self.session.notice(tl!("Add an account first: a new message needs an account to send from.").to_string(), true);
            }
            Err(e) => return self.session.notice(e.to_string(), true),
        }
        for f in files {
            self.run("compose.attach", json!({"path": f}));
        }
    }

    pub(super) fn pump(&mut self) {
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
        self.files_filed();
        // Sign-in pages go to the browser.
        for url in std::mem::take(&mut self.session.open_urls) {
            if let Err(e) = self.services.open_url(&url) {
                self.session.notice(tf!("The sign-in page couldn't be opened: {error}", error = e), true);
            }
        }
        // Files chosen in the browser's picker.
        for (purpose, name, data) in self.services.take_picked() {
            match purpose {
                Picked::Certificate(account) => {
                    self.ui.dialog = Some(Dialog::Certificate { account, path: name, data: Some(data), password: String::new() });
                }
                Picked::Backup => self.backup_picked(name, data),
                Picked::Rules => self.rules_picked(&data),
                Picked::Categories => self.categories_picked(&data),
                Picked::Picture => {
                    let size = rich_edit::own_size(&data);
                    let (width, line) = (rich_edit::fitted_width(size), rich_edit::own_line(size));
                    // Handed over as they are, not as a JSON list (a value per byte).
                    let handed = self.session.hand(data);
                    let params = self.picture_place(json!({"name": name, "handed": handed, "width": width, "line": line}));
                    let r = self.session.run("compose.insertPicture", &params);
                    self.place_picture(r);
                }
                Picked::Attach => {
                    let handed = self.session.hand(data);
                    self.run("compose.attach", json!({"name": name, "handed": handed}));
                }
            }
        }
        // Snoozed mail that is due comes back.
        self.session.tick();
        // Spelling as you type, and the words added to the dictionary (issue #174).
        rich_edit::spelling::follow(self.session.prefs.spelling, &self.session.prefs.dictionary);
    }

    pub(super) fn flush_jobs(&mut self) {
        for j in self.session.take_jobs() {
            self.backend.submit(j);
        }
    }
}
