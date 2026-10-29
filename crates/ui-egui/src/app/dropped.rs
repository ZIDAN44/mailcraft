//! Files from the desktop: copied in a file manager and pasted into a message (attached, as
//! Outlook attaches them), or dropped on the main window where nothing is being written (a new
//! message with them attached, as Outlook starts one). Files dropped on a message being written
//! are its own (`reading::compose`).

use crate::*;

/// Most files taken from one paste or drop.
const MAX_FILES: usize = 100;

/// Set while the text of a message being written has the keyboard in a window: Ctrl+V there looks
/// for files first ([`clipboard_keys`]).
fn files_wanted(viewport: egui::ViewportId) -> egui::Id {
    egui::Id::unique("paste-files-wanted").with(viewport)
}

/// Files Ctrl+V found on the clipboard in a window that wants them, for [`pasted_files`] to take.
fn files_pasted(viewport: egui::ViewportId) -> egui::Id {
    egui::Id::unique("files-pasted").with(viewport)
}

/// What the desktop's clipboard holds, read when a key pastes.
pub trait ClipboardRead {
    /// Files copied in a file manager.
    fn files(&mut self) -> Vec<PathBuf>;
    /// Its text, else its picture, as egui's paste events (`None`: neither).
    fn paste(&mut self) -> Option<egui::Event>;
}

/// The desktop's Cut, Copy and Paste keys, taken from egui-winit (the app turns eframe's
/// `clipboard_shortcuts` off) and run on each window's input before egui reads it. egui-winit
/// read only text and pictures for Ctrl+V, and with neither on the clipboard did nothing: files
/// copied in Explorer or PCManFM, which put no text with them, were never pasted (issue #10).
/// The same keys make the same events here (the clipboard's text or picture read as egui-winit
/// reads it), except Ctrl+V in the text of a message being written: files on the clipboard
/// first, attached by [`pasted_files`].
pub fn clipboard_keys(ctx: &egui::Context, raw: &mut egui::RawInput, clipboard: &mut dyn ClipboardRead) {
    use egui::Key;
    let viewport = raw.viewport_id;
    let windows = cfg!(target_os = "windows");
    let (wanted_id, files_id) = (files_wanted(viewport), files_pasted(viewport));
    let wanted = ctx.data(|d| d.get_temp::<bool>(wanted_id)).unwrap_or(false);
    let mut out = Vec::with_capacity(raw.events.len());
    for e in raw.events.drain(..) {
        let egui::Event::Key { key, pressed: true, modifiers: m, .. } = e else {
            out.push(e);
            continue;
        };
        if key == Key::Cut || (m.command_only() && key == Key::X) || (windows && m.shift && key == Key::Delete) {
            out.push(egui::Event::Cut);
        } else if key == Key::Copy || (m.command_only() && key == Key::C) || (windows && m.ctrl && key == Key::Insert) {
            out.push(egui::Event::Copy);
        } else if key == Key::Paste || (m.command_only() && key == Key::V) || (windows && m.shift && key == Key::Insert) {
            let files: Vec<PathBuf> =
                if wanted { clipboard.files().into_iter().filter(|p| p.is_absolute() && p.is_file()).take(MAX_FILES).collect() } else { Vec::new() };
            if files.is_empty() {
                out.extend(clipboard.paste());
            } else {
                ctx.data_mut(|d| d.insert_temp(files_id, files));
            }
        } else {
            out.push(e);
        }
    }
    raw.events = out;
}

/// Files copied in a file manager and pasted into the text of the message being written: the
/// paths to attach. Ctrl+V there finds them first ([`clipboard_keys`]). A paste that comes with
/// the files' names as text (the browser's; or the right-click Paste's, which pastes the text) is
/// taken out of this frame's input, so the names don't go into the text as well. The text's
/// right-click Paste attaches them too ([`MailApp::paste_files`]).
pub(crate) fn pasted_files(ctx: &egui::Context, services: &mut dyn Services, s: &Session) -> Vec<PathBuf> {
    let viewport = ctx.viewport_id();
    let (wanted, files_id) = (files_wanted(viewport), files_pasted(viewport));
    let focused = s.compose.as_ref().map(rich_edit::body_id).is_some_and(|body| ctx.memory(|m| m.has_focus(body)));
    ctx.data_mut(|d| d.insert_temp(wanted, focused));
    let keyed: Vec<PathBuf> = ctx.data_mut(|d| d.remove_temp::<Vec<PathBuf>>(files_id)).unwrap_or_default();
    if !keyed.is_empty() {
        return keyed;
    }
    if !focused || !ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Paste(_) | egui::Event::PasteImage(_)))) {
        return Vec::new();
    }
    let files: Vec<PathBuf> = services.clipboard_files().into_iter().filter(|p| p.is_absolute() && p.is_file()).take(MAX_FILES).collect();
    if !files.is_empty() {
        ctx.input_mut(|i| i.events.retain(|e| !matches!(e, egui::Event::Paste(_) | egui::Event::PasteImage(_))));
    }
    files
}

/// Forget that a window wants files on Ctrl+V (it shows no message being written now).
pub(crate) fn no_files_wanted(ctx: &egui::Context) {
    let id = files_wanted(ctx.viewport_id());
    ctx.data_mut(|d| d.insert_temp(id, false));
}

impl MailApp {
    /// The main window's files from the desktop: pasted into a message written in it, or dropped
    /// on it with none being written there (a new message with them attached). While files are
    /// held over it, it says so.
    pub(crate) fn files_from_desktop(&mut self, ctx: &egui::Context, acts: &mut Vec<Action>) {
        let inline = self.session.module == Module::Mail && self.session.compose.as_ref().is_some_and(|c| compose_inline(c, ctx));
        if inline {
            for p in pasted_files(ctx, self.services.as_mut(), &self.session) {
                acts.push(Action::Run("compose.attach", json!({"path": p.to_string_lossy()})));
            }
            return;
        }
        no_files_wanted(ctx);
        // Somewhere else to put them first: the File page, a dialog, no account yet.
        if self.session.module != Module::Mail || self.ui.backstage.is_some() || self.ui.dialog.is_some() || self.session.accounts.is_empty() {
            return;
        }
        let (held, dropped) = ctx.input(|i| (!i.raw.hovered_files.is_empty(), i.raw.dropped_files.clone()));
        if held {
            let rect = ctx.content_rect();
            let t = self.ui.tokens;
            let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("files-held-over")));
            p.rect_stroke(rect.shrink(3.0), 6.0, egui::Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
            let g = p.layout_no_wrap(tl!("Drop to attach them to a new message").to_string(), theme::regular(15.0), t.text);
            let card = egui::Rect::from_center_size(rect.center(), g.size() + vec2(32.0, 20.0));
            p.rect(card, 8.0, t.surface, egui::Stroke::new(1.0, t.accent), egui::StrokeKind::Inside);
            p.galley(card.center() - g.size() / 2.0, g, t.text);
        }
        let files: Vec<String> = dropped
            .iter()
            .filter(|f| f.path().is_absolute() && f.path().is_file())
            .take(MAX_FILES)
            .map(|f| f.path().to_string_lossy().to_string())
            .collect();
        // Folders aren't attached: none, no message.
        if files.is_empty() {
            return;
        }
        acts.push(Action::Run("mail.new", json!({})));
        for f in files {
            acts.push(Action::Run("compose.attach", json!({"path": f})));
        }
    }

    /// The text's right-click Paste in a message: files copied in a file manager are attached
    /// (the text paste that follows is called off), as Outlook attaches them.
    pub(crate) fn paste_files(&mut self) {
        let files: Vec<PathBuf> = self.services.clipboard_files().into_iter().filter(|p| p.is_absolute() && p.is_file()).take(MAX_FILES).collect();
        if files.is_empty() {
            return;
        }
        if let Some(ed) = self.ui.editor.as_mut() {
            ed.paste = false;
        }
        for p in files {
            self.run("compose.attach", json!({"path": p.to_string_lossy()}));
        }
    }
}
