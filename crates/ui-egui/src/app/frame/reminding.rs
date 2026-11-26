//! Calendar reminders each frame, in `logic` as well as `ui` (issue #299): eframe runs only `logic`
//! while the window is minimized, hidden or covered (on a Mac a window behind another app's
//! counts as covered), so the Reminders window, a window of its own over other apps', is opened
//! from there, and the desktop told of a reminder when it comes, not when MailCraft is switched to.

use super::*;

impl MailApp {
    /// Reminders come due: the desktop told of each as it comes (by whichever of `logic` and `ui`
    /// runs first: [`reminders::refresh`] gives each once), and the window shown, by `logic` when
    /// it's one of its own, by `ui` in MailCraft's.
    pub(in crate::app) fn remind(&mut self, ctx: &egui::Context, in_logic: bool, acts: &mut Vec<Action>) {
        // A message's own page leaves them to the main window's (each page would tell of them again).
        if self.ui.page_message.is_some() {
            return;
        }
        // Nothing drawn: what's left is looked at again soon, as a drawn window is (each frame
        // asks for the next one at most 20 seconds on).
        let undrawn = in_logic && ctx.input(|i| i.viewport().visible() == Some(false));
        // Flagged mail's (gap 9) are found in every folder's flagged mail, read a folder a frame
        // as the To-Do Bar reads it, when there may be some.
        if (!in_logic || undrawn) && self.session.mail_reminders_wanted() && self.session.todo_step() {
            ctx.request_repaint();
        }
        for r in reminders::refresh(&mut self.ui.reminders, &self.session, false) {
            let when = if r.all_day { String::new() } else { crate::calendar::hm(r.start, &self.session) };
            self.services.notify_reminder(&r.summary, &if r.location.is_empty() { when } else { format!("{when} · {}", r.location) });
        }
        if reminders::in_own_window(ctx, &self.session) == in_logic {
            let before = acts.len();
            let t = self.ui.tokens;
            reminders::show(ctx, &mut self.ui.reminders, &self.session, &t, acts);
            // An item opened from a window of its own: MailCraft comes forward, where its form is.
            if in_logic && acts.get(before..).is_some_and(|new| new.iter().any(|a| matches!(a, Action::Dialog(_)))) {
                self.show_window(ctx);
            }
        }
        if reminders::showing(&self.ui.reminders) || undrawn {
            ctx.request_repaint_after(std::time::Duration::from_secs(15));
        }
    }
}
