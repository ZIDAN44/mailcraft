//! The window, frame by frame: what comes in, the panes, the windows, what was asked.

use crate::*;

use super::ribbon::{RIBBON_H, module_switch, module_switch_vertical};

impl MailApp {
    /// The whole window.
    pub fn ui(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let mut main = self.ui.places.get("main").copied();
        after_open(ui, main);
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
        motion::set_enabled(&ctx, self.session.prefs.animations);
        // Ctrl+Q is Mark as read, as Outlook's: egui's own Ctrl+Q closed the window too. On a
        // Mac, Cmd+Q quits, as every Mac app does.
        if !cfg!(target_os = "macos") && ctx.options(|o| !o.quit_shortcuts.is_empty()) {
            ctx.options_mut(|o| o.quit_shortcuts.clear());
        }
        self.pump();
        self.settle_composes(&ctx);
        self.announce(&ctx);
        self.ui.pending_update = self.services.pending_update();
        // Closing with online-only accounts' unsent mail or changes held in memory: ask first.
        if self.ui.close_now {
            self.ui.close_now = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else if ctx.input(|i| i.viewport().close_requested()) && !self.ui.closing_anyway {
            // Messages being written: kept as drafts if wanted, as Outlook asks; never lost.
            let written = self.session.all_composes().filter(|c| self.session.written(c) && !c.sending).count();
            let (unsent, changes) = self.session.lost_on_close();
            if written > 0 {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.ui.dialog = Some(Dialog::SaveOnClose(written));
                self.ui.dialog_for = None;
            } else if unsent + changes > 0 {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.ui.dialog = Some(Dialog::LostOnClose { unsent, changes });
            }
        }
        // A search started by a command or agent shows its query in the box.
        if let Some((q, _)) = &self.session.search
            && self.ui.search_text.is_empty()
            && !ctx.text_edit_focused()
            && self.session.open_search_folder_name().is_none()
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
                    // Too wide for the window: its last buttons lose their labels, as few as it
                    // takes (worked out from this frame for the next).
                    // One height on every tab: switching tabs doesn't nudge the window below.
                    ui.set_min_height(RIBBON_H);
                    let id = widgets::ribbon_fit_id();
                    let start =
                        widgets::RibbonFit { active: true, icons_from: self.ui.ribbon_icons_from, right: ui.max_rect().left(), ..Default::default() };
                    ui.ctx().data_mut(|d| d.insert_temp(id, start));
                    self.ribbon(ui, &mut acts);
                    let fit = ui.ctx().data_mut(|d| std::mem::take(d.get_temp_mut_or_default::<widgets::RibbonFit>(id)));
                    self.ui.ribbon_rect = Some(ui.min_rect());
                    let from = fit.fit(ui.max_rect().right());
                    if from != self.ui.ribbon_icons_from.min(fit.count) {
                        self.ui.ribbon_icons_from = from;
                        ui.ctx().request_repaint();
                    }
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
            // Minimized: a narrow strip with the way back and the modules, as Outlook's. Between
            // the two, the pane slides (or changes at once, with animations off).
            let mut expanded = !self.session.prefs.folder_pane_minimized;
            let strip =
                start("nav-min").resizable(false).exact_size(52.0).frame(egui::Frame::new().fill(t.chrome).inner_margin(Margin::symmetric(4, 8)));
            let full = start("nav")
                .resizable(true)
                .default_size(if module == Module::Mail { 230.0 } else { 260.0 })
                .size_range(170.0..=380.0)
                .frame(egui::Frame::new().fill(t.chrome).inner_margin(Margin::symmetric(6, 8)));
            egui::Panel::show_switched(ui, &mut expanded, strip, full, |ui, wide| {
                if !wide {
                    ui.vertical_centered(|ui| {
                        let chevron = if rtl { Icon::ChevronLeft } else { Icon::ChevronRight };
                        if widgets::tiny_icon_button(ui, chevron, "Show the folder pane", &t).clicked() {
                            acts.push(Action::Run("view.folderPane", json!({"minimized": false})));
                        }
                        ui.add_space(10.0);
                        module_switch_vertical(ui, &t, module, &mut acts);
                    });
                    return;
                }
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
                    Module::Notes => notes::nav(&self.session, &mut self.ui, ui, &mut acts),
                }
            });
            // Dragged narrower than it goes: minimized, as the button does.
            if expanded == self.session.prefs.folder_pane_minimized {
                acts.push(Action::Run("view.folderPane", json!({"minimized": !expanded})));
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
                (Module::Notes, _) => {
                    egui::CentralPanel::default().frame(egui::Frame::new().fill(t.surface)).show(ui, |ui| {
                        notes::show(&self.session, &mut self.ui, ui, &mut acts);
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
        self.compose_windows(&ctx, &mut acts);

        // The address book reads the session's people.
        if let Some(Dialog::SelectNames(f)) = self.ui.dialog.as_mut()
            && f.stale
        {
            // Opened: with the address book chosen to open with (one gone is everyone).
            if f.source.is_empty() {
                f.books = self.session.address_books();
                f.default_source = self.session.prefs.address_book.clone();
                let known = f.default_source == "people" || f.books.iter().any(|(b, _, _)| *b == f.default_source);
                f.source = if known { f.default_source.clone() } else { "all".into() };
            }
            f.list = self.session.address_book(&f.source, &f.query);
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
        // Calendar reminders: the window, and the desktop told of each as it comes.
        for r in reminders::refresh(&mut self.ui.reminders, &self.session, false) {
            let start = mailcraft_engine::model::dates::civil(r.start, &self.session.clock);
            let when = if r.all_day { String::new() } else { format!("{:02}:{:02}", start.hour, start.minute) };
            self.services.notify(&r.summary, &if r.location.is_empty() { when } else { format!("{when} · {}", r.location) });
        }
        reminders::show(&ctx, &mut self.ui.reminders, &self.session, &t, &mut acts);
        if reminders::showing(&self.ui.reminders) {
            ctx.request_repaint_after(std::time::Duration::from_secs(15));
        }
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
}
