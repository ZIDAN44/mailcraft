//! Files from the desktop: copied in a file manager and pasted into a message (attached, as
//! Outlook attaches them), or dropped on the main window where nothing is being written (a new
//! message with them attached, as Outlook starts one). Files dropped on a message being written
//! are its own (`reading::compose`).

use crate::*;

/// Most files taken from one paste or drop.
const MAX_FILES: usize = 100;

/// Files copied in a file manager and pasted into the text of the message being written: the
/// paths to attach. The paste is taken out of this frame's input, so their names don't go into
/// the text as well (file managers on Linux and the Finder put them on the clipboard as text
/// too). Explorer puts no text there, and egui sends nothing for its Ctrl+V: those come by the
/// text's right-click Paste ([`MailApp::paste_files`]).
pub(crate) fn pasted_files(ctx: &egui::Context, services: &mut dyn Services, s: &Session) -> Vec<PathBuf> {
    let Some(c) = s.compose.as_ref() else { return Vec::new() };
    if !ctx.memory(|m| m.has_focus(rich_edit::body_id(c))) {
        return Vec::new();
    }
    if !ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::Paste(_) | egui::Event::PasteImage(_)))) {
        return Vec::new();
    }
    let files: Vec<PathBuf> = services.clipboard_files().into_iter().filter(|p| p.is_absolute() && p.is_file()).take(MAX_FILES).collect();
    if !files.is_empty() {
        ctx.input_mut(|i| i.events.retain(|e| !matches!(e, egui::Event::Paste(_) | egui::Event::PasteImage(_))));
    }
    files
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
