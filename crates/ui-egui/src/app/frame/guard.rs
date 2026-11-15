//! The last guard around each frame (PdfCraft's): a panic in what a frame does or draws (a
//! dialog's button, a pane, a message laid out) used to end MailCraft, with the messages being
//! written in it. The app (`apps/mailcraft`) runs each part of a frame under `catch_unwind` and
//! tells MailCraft how it went: a panic is said, what was on top (a dialog) is put away, and
//! MailCraft goes on. Only a window that panics frame after frame (nothing in it can be used, or
//! even closed) is let go as before, with what's being written kept as drafts first.

use std::any::Any;

use crate::*;

/// Frames in a row that may panic before the panic is let go to end MailCraft.
const PANICKING_FRAMES: u32 = 3;

/// The part of a frame run under the guard: eframe runs `logic` every frame, and `ui` after it
/// only while the window is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FramePart {
    Logic,
    Ui,
}

/// How the frames under the guard have gone.
#[derive(Default)]
pub(crate) struct FrameGuard {
    /// Frames in a row that panicked.
    panicking: u32,
    /// This frame panicked (counted once, in either part).
    panicked: bool,
    /// The last frame ran `logic` with no `ui` after it (a minimized or hidden window).
    logic_alone: bool,
    /// The panic is being let go to end MailCraft (what's written is kept once, not again as
    /// it passes the main window's guard on its way out).
    letting_go: bool,
}

/// Run `draw`, a window of its own's drawing (`show_viewport_immediate`, which draws it in a pass
/// of its own inside the main window's): a panic in it is kept in `caught`, for
/// [`MailApp::frame_part_ended`] once that pass is over, and `T::default()` stands for what it
/// would have given. Let out of the window's pass, the panic would leave egui unable to draw it
/// again.
pub(crate) fn guarded_window<T: Default>(caught: &mut Option<Box<dyn Any + Send>>, draw: impl FnOnce() -> T) -> T {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(draw)) {
        Ok(v) => v,
        Err(payload) => {
            *caught = Some(payload);
            T::default()
        }
    }
}

impl MailApp {
    /// Before a part of a frame runs under the guard.
    pub fn frame_part_starts(&mut self, part: FramePart) {
        let g = &mut self.ui.frame_guard;
        match part {
            FramePart::Logic => {
                // A hidden window's frames are `logic` alone: one that ran cleanly ends the run.
                if g.logic_alone && !g.panicked {
                    g.panicking = 0;
                }
                g.panicked = false;
                g.logic_alone = true;
            }
            FramePart::Ui => g.logic_alone = false,
        }
    }

    /// After a part of a frame ran under the guard: `ran` is what `catch_unwind` gave. A frame
    /// whose drawing ran cleanly ends a run of panicking ones. After a panic, the dialog on top
    /// is put away and the panic is said (where it happened is in the log already, and what it
    /// said isn't shown: it can quote a message); the [`PANICKING_FRAMES`]th frame in a row that
    /// panics keeps what's being written as drafts and lets the panic go on.
    pub fn frame_part_ended(&mut self, ctx: &egui::Context, part: FramePart, ran: std::thread::Result<()>) {
        let payload = match ran {
            Ok(()) => {
                if part == FramePart::Ui && !self.ui.frame_guard.panicked {
                    self.ui.frame_guard.panicking = 0;
                }
                return;
            }
            Err(payload) => payload,
        };
        if let Some(payload) = self.frame_panicked(part, payload) {
            std::panic::resume_unwind(payload);
        }
        ctx.request_repaint();
    }

    /// After a window of its own was drawn under [`guarded_window`]: its panic, if any, as a
    /// panic of the frame's drawing.
    pub(crate) fn window_drawn(&mut self, ctx: &egui::Context, caught: Option<Box<dyn Any + Send>>) {
        if let Some(payload) = caught {
            self.frame_part_ended(ctx, FramePart::Ui, Err(payload));
        }
    }

    /// [`MailApp::frame_part_ended`] after a panic: the panic back when it's to end MailCraft.
    fn frame_panicked(&mut self, part: FramePart, payload: Box<dyn Any + Send>) -> Option<Box<dyn Any + Send>> {
        let g = &mut self.ui.frame_guard;
        if g.letting_go {
            return Some(payload);
        }
        if !g.panicked {
            g.panicked = true;
            g.panicking = g.panicking.saturating_add(1);
        }
        let panicking = g.panicking;
        if panicking >= PANICKING_FRAMES {
            g.letting_go = true;
            log::error!("the window's {part:?} panicked in {panicking} frames in a row: keeping what's being written as drafts, and closing");
            // Guarded too: the session may be what panics.
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.session.keep_drafts())) {
                Ok(kept) => log::info!("{kept} message(s) being written kept as drafts"),
                Err(_) => log::error!("keeping what's being written as drafts panicked too"),
            }
            self.flush_jobs();
            return Some(payload);
        }
        log::error!("the window's {part:?} panicked and was stopped; MailCraft goes on");
        self.ui.dialog = None;
        self.session.notice(
            tl!("Something went wrong in the window, and what it was doing was stopped. MailCraft goes on; the log says where it happened."),
            true,
        );
        None
    }
}

#[cfg(test)]
mod tests {
    use std::panic::{AssertUnwindSafe, catch_unwind};

    use serde_json::json;

    use super::*;
    use crate::tests::{app, frame};

    /// A frame as eframe runs the app's: `logic`, then the drawing inside egui's pass, each under
    /// the guard. The drawing panics part way, deep inside a window of its own, when `panics`.
    /// What the guard let go, if anything.
    fn guarded_frame(ctx: &egui::Context, a: &mut MailApp, panics: bool) -> std::thread::Result<()> {
        catch_unwind(AssertUnwindSafe(|| {
            a.frame_part_starts(FramePart::Logic);
            let ran = catch_unwind(AssertUnwindSafe(|| a.logic(ctx)));
            a.frame_part_ended(ctx, FramePart::Logic, ran);
            let input =
                egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| {
                a.frame_part_starts(FramePart::Ui);
                let ran = catch_unwind(AssertUnwindSafe(|| {
                    a.ui(ui);
                    egui::Window::new("guard test").show(ui.ctx(), |ui| {
                        ui.horizontal(|ui| {
                            ui.label("drawn");
                            assert!(!panics, "a frame that panics part way");
                        })
                    });
                }));
                a.frame_part_ended(ui.ctx(), FramePart::Ui, ran);
            });
            // No renderer here: the texture uploads are dropped deliberately.
            out.textures_delta.clear();
        }))
    }

    /// A panic in a frame no longer ends MailCraft: it's said, the dialog it may have come from is
    /// put away, and the next frames draw as before; the run of panicking frames starts again
    /// after a good one. Only a window that panics frame after frame is let go, with what's being
    /// written kept in Drafts first.
    #[test]
    fn a_panic_in_a_frame_is_said_and_mailcraft_goes_on() {
        let (mut a, _) = app("frame-guard");
        a.session.prefs.new_mail_window = false;
        let ctx = egui::Context::default();
        frame(&ctx, &mut a, vec![]);
        a.run("mail.new", json!({}));
        a.run("compose.update", json!({"subject": "Not lost", "body": "Written before the window failed."}));
        a.ui.dialog = Some(Dialog::add_account_with("zoe@example.com", false));
        assert!(guarded_frame(&ctx, &mut a, true).is_ok(), "the first panic is caught");
        assert!(a.ui.dialog.is_none());
        assert!(a.session.notices.last().is_some_and(|n| n.error && n.text.starts_with("Something went wrong in the window")));
        assert!(guarded_frame(&ctx, &mut a, true).is_ok(), "and the second");
        // A good frame: the window draws again, and the run is over.
        assert!(guarded_frame(&ctx, &mut a, false).is_ok());
        assert_eq!(a.ui.frame_guard.panicking, 0);
        assert_eq!(a.session.all_composes().count(), 1, "the message being written is still open");
        // Frame after frame: let go on the third, the message kept in Drafts first.
        assert!(guarded_frame(&ctx, &mut a, true).is_ok() && guarded_frame(&ctx, &mut a, true).is_ok());
        assert!(guarded_frame(&ctx, &mut a, true).is_err(), "the third in a row ends MailCraft");
        a.run("folder.open", json!({"account": "sample", "folder": "Drafts"}));
        assert_eq!(a.session.list.iter().filter(|m| m.subject == "Not lost").count(), 1);
    }

    /// A hidden window runs `logic` alone: one that panics every time is let go too, and one that
    /// runs cleanly ends a run as a drawn frame does.
    #[test]
    fn a_hidden_windows_frames_count_too() {
        let (mut a, _) = app("frame-guard-hidden");
        let ctx = egui::Context::default();
        let logic = |a: &mut MailApp, panics: bool| {
            catch_unwind(AssertUnwindSafe(|| {
                a.frame_part_starts(FramePart::Logic);
                let ran = if panics { Err(Box::new("logic panicked") as Box<dyn Any + Send>) } else { Ok(()) };
                a.frame_part_ended(&ctx, FramePart::Logic, ran);
            }))
        };
        assert!(logic(&mut a, true).is_ok() && logic(&mut a, true).is_ok());
        assert!(logic(&mut a, false).is_ok() && logic(&mut a, false).is_ok());
        assert_eq!(a.ui.frame_guard.panicking, 0);
        assert!(logic(&mut a, true).is_ok() && logic(&mut a, true).is_ok());
        assert!(logic(&mut a, true).is_err());
    }
}
