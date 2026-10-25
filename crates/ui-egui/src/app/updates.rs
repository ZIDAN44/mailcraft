//! A newer MailCraft on its way, in a build that updates itself (`apps/mailcraft/src/self_update.rs`):
//! the status bar says how far its download is; once it's downloaded and checked, a bar above the
//! panes (and the system's notification, if MailCraft isn't in front) offers to restart into it;
//! and closing asks first, so it never installs an update, or pauses a download, without a word.

use crate::*;

/// How often the updater is looked at when nothing else draws a frame.
const LOOK_EVERY: std::time::Duration = std::time::Duration::from_secs(30);

impl MailApp {
    /// This frame's look at the updater; the system is told once when a version is ready. The
    /// window calls it; the desktop app too where eframe draws nothing (minimized), when woken.
    pub fn follow_update(&mut self, ctx: &egui::Context) {
        let state = self.services.update_state();
        if let Some(Dialog::UpdateOnClose(asked)) = &mut self.ui.dialog {
            // The question follows the download (finished while asked: it's about installing).
            asked.clone_from(&state);
        }
        if let UpdateState::Ready { version, .. } = &state
            && self.ui.update_told.as_deref() != Some(version.as_str())
        {
            self.ui.update_told = Some(version.clone());
            if !ctx.input(|i| i.focused) {
                let title = tf!("MailCraft {version} is ready", version = version);
                self.services.notify(&title, tl!("Restart MailCraft to install it now, or it installs when you close MailCraft."));
            }
        }
        if state != UpdateState::None || self.ui.update != UpdateState::None {
            ctx.request_repaint_after(if matches!(state, UpdateState::Downloading { .. }) { std::time::Duration::from_secs(1) } else { LOOK_EVERY });
        }
        self.ui.update = state;
    }

    /// Whether closing now should first say what becomes of the update.
    pub(crate) fn ask_about_update(&self) -> bool {
        !self.ui.update_answered && self.ui.update != UpdateState::None
    }

    /// Ask, as MailCraft is about to close, what becomes of the update.
    pub(crate) fn ask_update_on_close(&mut self) {
        self.ui.dialog = Some(Dialog::UpdateOnClose(self.ui.update.clone()));
        self.ui.dialog_for = None;
        self.ui.close_asked = true;
    }

    /// A question about closing put away without closing (Cancel, Escape): Restart now, and what
    /// was answered about the update, go with it, so a later close asks again.
    pub(crate) fn forget_cancelled_close(&mut self) {
        let asking = self.ui.dialog.as_ref().is_some_and(Dialog::about_closing);
        if self.ui.close_asked && !asking && !self.ui.close_now && !self.ui.closing_anyway {
            self.ui.close_asked = false;
            self.ui.restart_wanted = false;
            self.ui.update_answered = false;
        }
    }

    /// Whether the bar above the panes has something to say.
    pub(crate) fn update_bar_wanted(&self) -> bool {
        self.ui.update.ready().is_some_and(|v| self.ui.update_later.as_deref() != Some(v))
    }

    /// The bar: a version ready to install (Restart now, Later).
    pub(crate) fn update_bar(&mut self, ui: &mut Ui, acts: &mut Vec<Action>) {
        let t = self.ui.tokens;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            let (r, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
            icons::paint(ui.painter(), r, Icon::Sync, t.accent);
            if let UpdateState::Ready { version, admin } = &self.ui.update
                && self.ui.update_later.as_deref() != Some(version.as_str())
            {
                let mut text =
                    tf!("MailCraft {version} is ready. Restart now to install it, or it installs when you close MailCraft.", version = version);
                if *admin {
                    text.push(' ');
                    text.push_str(tl!("Windows asks for an administrator's permission."));
                }
                ui.label(RichText::new(text).color(t.text));
                if widgets::accent_button(ui, Icon::Sync, "Restart now", &t, true).clicked() {
                    acts.push(Action::RestartToUpdate);
                }
                if ui.button(tl!("Later")).on_hover_text(tl!("It installs when you close MailCraft.")).clicked() {
                    self.ui.update_later = Some(version.clone());
                }
            }
        });
    }

    /// The status bar's part: how far a download is, or the way to restart into it.
    pub(crate) fn update_status(&self, ui: &mut Ui) -> bool {
        let t = self.ui.tokens;
        match &self.ui.update {
            UpdateState::Downloading { version, done, total } => {
                let text = if *total > 0 {
                    let percent = done.saturating_mul(100) / total;
                    tf!("Downloading MailCraft {version}… {percent}%", version = version, percent = percent.min(100))
                } else {
                    tf!("Downloading MailCraft {version}…", version = version)
                };
                ui.label(RichText::new(text).color(t.dim).small()).on_hover_text(tl!(
                    "If you close MailCraft now, the download pauses, and carries on from where it stopped the next time you open MailCraft."
                ));
                false
            }
            UpdateState::Ready { version, .. } => {
                let hint = tf!("MailCraft {version} is downloaded. It installs when you close MailCraft.", version = version);
                ui.link(RichText::new(tl!("Restart to update")).color(t.accent).small()).on_hover_text(hint).clicked()
            }
            UpdateState::None => false,
        }
    }
}
