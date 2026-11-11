//! The window, frame by frame: what comes in, the panes, the windows, what was asked.

use crate::*;

use super::ribbon::{RIBBON_H, RIBBON_H_CLASSIC, module_switch, module_switch_vertical};

/// The top bar's height: the ribbon's tabs and the search box, one row (issue #22).
pub(crate) const TOP_BAR_H: f32 = 40.0;
/// The top bar's margins, across and down.
pub(crate) const TOP_BAR_MARGIN: (i8, i8) = (8, 5);
/// The narrowest the message list goes beside the reading pane, and the width the reading pane
/// keeps while the folders and the list have any to give.
const LIST_LEAST: f32 = 280.0;
const READING_LEAST: f32 = 300.0;

impl MailApp {
    /// Every frame, the window minimized or hidden too: eframe runs only `logic` then, and closes
    /// the window unless `logic` cancels, so the close is decided here, not in `ui` alone (craft-libs'
    /// quit crate found this: a close from the taskbar lost the messages being written).
    pub fn logic(&mut self, ctx: &egui::Context) {
        // In a pass that draws, `ui` then leaves it be (egui gives `logic` alone the last pass's
        // input time, so a minimized window's close is always decided).
        self.ui.close_checked = Some(ctx.input(|i| i.time));
        self.decide_close(ctx);
        // What the mail servers sent is taken in and new mail announced here too: a minimized
        // window draws no frame, and mail that came meanwhile was neither shown in the title
        // nor announced until the window was opened again (issue #68).
        self.pump();
        self.announce(ctx);
        self.flush_jobs();
    }

    /// The window back from the notification area (its icon clicked, MailCraft started again).
    pub fn show_window(&mut self, ctx: &egui::Context) {
        self.ui.in_tray = false;
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    }

    /// Exit, from the notification area's icon: MailCraft closes as it does without one, asking
    /// what it asks.
    pub fn exit(&mut self, ctx: &egui::Context) {
        self.ui.quit_wanted = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    /// What the notification area's icon says: its tooltip (the window's title, with the unread
    /// count) and its menu's two lines, in the language shown.
    pub fn tray_words(&self) -> (String, String, String) {
        (self.ui.title_shown.clone(), tl!("Open MailCraft").to_string(), tl!("Exit").to_string())
    }

    /// Where the platform has a quit of its own (macOS: `terminate:`, `quit` returning true), a
    /// close of the main window that nothing held open goes that way instead: call it last in
    /// `logic`. Leaving a Mac by closing the eframe window can crash in AppKit's Touch Bar code
    /// once the window is gone, and `terminate:` ends the process before that runs (craft-libs'
    /// quit crate). Whether it took the close over.
    pub fn quit_instead_of_close(ctx: &egui::Context, quit: impl FnOnce() -> bool) -> bool {
        if ctx.viewport_id() != egui::ViewportId::ROOT || !ctx.input(|i| i.viewport().close_requested()) {
            return false;
        }
        // A question (save what's being written?) holds the window open.
        if ctx.viewport(|v| v.commands.contains(&egui::ViewportCommand::CancelClose)) {
            return false;
        }
        if !quit() {
            return false;
        }
        // The window stays for the quit to close.
        ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        true
    }

    /// The window's GPU device is gone (a driver reset, after the computer slept): no frame can be
    /// drawn any more, so nobody could answer a question. Every message being written is kept as a
    /// draft, and MailCraft closes without asking (as PhotoCraft does); it opens them again from
    /// Drafts. Once only, however often it's said.
    pub fn graphics_lost(&mut self, ctx: &egui::Context) {
        if self.ui.graphics_lost {
            return;
        }
        self.ui.graphics_lost = true;
        log::error!("graphics: the window can't be drawn any more; keeping what's being written as drafts and closing");
        // Kept as MailCraft keeps them closing (on this computer at once, a server account's in
        // Drafts at the next connection): a save to a server could still be on its way as the
        // app ends, and be lost (found in review).
        let kept = self.session.keep_drafts();
        log::info!("graphics: {kept} message(s) being written kept as drafts");
        self.flush_jobs();
        self.ui.dialog = None;
        self.ui.closing_anyway = true;
        self.ui.close_now = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    /// Closing with messages being written, online-only accounts' unsent mail or changes held in
    /// memory, a data file being imported or to be compacted, or an update waiting: ask first.
    fn decide_close(&mut self, ctx: &egui::Context) {
        self.forget_cancelled_close();
        // Closing with online-only accounts' unsent mail or changes held in memory: ask first.
        if self.ui.close_now {
            self.ui.close_now = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else if ctx.input(|i| i.viewport().close_requested()) {
            // Kept running in the notification area (issue #67): the window is hidden, nothing
            // ends and nothing is asked. Exit, an update's restart and a close already being
            // answered go on to close it.
            let ending = self.ui.quit_wanted || self.ui.closing_anyway || self.ui.restart_wanted || self.ui.graphics_lost;
            if self.session.prefs.close_to_tray && !ending && self.services.tray_shown() {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
                self.ui.in_tray = true;
                return;
            }
            // Messages being written: kept as drafts if wanted, as Outlook asks; never lost.
            let written = self.session.all_composes().filter(|c| self.session.written(c) && !c.sending).count();
            let (unsent, changes) = self.session.lost_on_close();
            // Messages still on their way: not in the Outbox yet, so closing now would lose them.
            let sending = self.session.all_composes().filter(|c| c.sending).count();
            let asked = if self.ui.closing_anyway {
                false
            } else if written > 0 {
                self.ui.dialog = Some(Dialog::SaveOnClose(written));
                self.ui.dialog_for = None;
                true
            } else if unsent + changes + sending > 0 {
                self.ui.dialog = Some(Dialog::LostOnClose { unsent, changes, sending });
                true
            } else if super::importing::ask_on_close(&self.session) {
                // It pauses, and goes on as MailCraft opens again: said, not asked to be undone.
                self.ask_import_on_close();
                true
            } else if self.ask_compact_on_close() {
                // Data files compacted first, when that was chosen and is short (or offered by
                // hand, when it isn't); the part that has a file to itself is waited for.
                true
            } else if self.ask_about_update() {
                // An update about to be installed, or still downloading: said first.
                self.ask_update_on_close();
                true
            } else {
                false
            };
            if asked {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.ui.close_asked = true;
                // Exit from the notification area's icon: the question needs the window.
                if self.ui.in_tray {
                    self.show_window(ctx);
                }
                // Closed from the taskbar or the Dock while minimized: the window comes back to ask.
                if ctx.input(|i| i.viewport().minimized == Some(true)) {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
            } else if self.ui.restart_wanted {
                // Restart now, and nothing stopped the close: the new version opens once installed.
                self.services.restart_to_update();
            }
            if !asked {
                // Closing, however it was decided (nothing to ask, Don't save, Close anyway, the
                // graphics lost): the messages held while their drafts were on the way to the
                // server were chosen to be kept, and the save may never finish (#53). On this
                // computer now, with the pool, as a minimized window draws no frame to send them.
                if self.session.keep_held() > 0 {
                    self.flush_jobs();
                }
            }
        }
    }

    /// The pictures (and the video thumbnails) this frame drew ask for their work, and what's
    /// done of it becomes textures.
    fn pictures_drawn(&mut self, ctx: &egui::Context) {
        let open = self.session.reading.as_ref().map(|r| (&r.key, r.parsed.as_ref().map_or(&[][..], |p| &p.attachments[..])));
        self.ui.pictures.end_frame(ctx, self.session.store.as_ref(), open, &self.ui.fonts, &mut *self.services);
        self.ui.videos.end_frame(ctx, &mut *self.services);
    }

    /// The window is told before this frame's maximize or restore by its title bar, if there is
    /// one: the command runs after the frame.
    fn tell_before_zooming(&mut self, ctx: &egui::Context) {
        if super::caption::take_zooming(ctx) && self.services.own_title_bar() {
            self.services.before_zoom();
            // A frame after Windows' zoom animation, whatever else is going on then.
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        }
    }

    /// The whole window.
    pub fn ui(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        // The window draws its own title bar (Windows): kept borderless each frame (winit puts the
        // system's frame back when the window is maximized or restored).
        let own_title_bar = self.services.own_title_bar();
        if own_title_bar {
            self.services.keep_frame();
            super::caption::clear_scroll_bars(&ctx);
        }
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
        // Done in `logic` already when the app runs it, as the close is.
        let in_logic = self.ui.close_checked == Some(ctx.input(|i| i.time));
        if !in_logic {
            self.pump();
        }
        self.show_finished_import();
        self.step_search(&ctx);
        self.follow_compaction(&ctx);
        self.take_body();
        self.settle_composes(&ctx);
        // Again after `logic`'s: the title follows what this frame opened.
        self.announce(&ctx);
        self.follow_update(&ctx);
        // Decided in `logic` already when the app runs it (the desktop app; not the tests or the browser).
        if self.ui.close_checked != Some(ctx.input(|i| i.time)) {
            self.decide_close(&ctx);
        }
        // A search started by a command or agent shows its query in the box.
        if let Some((q, _)) = &self.session.search
            && self.ui.search_text.is_empty()
            && !ctx.text_edit_focused()
            && self.session.open_search_folder_name().is_none()
        {
            self.ui.search_text = q.clone();
        }
        let dark = self.ui.dark.unwrap_or_else(|| match ctx.system_theme() {
            Some(theme) => theme == egui::Theme::Dark,
            None => self.services.system_dark().unwrap_or(false),
        });
        // The theme: light or dark, with the accent colour and top bar chosen (issue #8).
        let want = Tokens::themed(dark, mailcraft_engine::accent_rgb(&self.session.prefs.accent), self.session.prefs.plain_top_bar);
        if want != self.ui.tokens || ctx.global_style().visuals.panel_fill != self.ui.tokens.surface {
            self.ui.tokens = want;
            theme::apply(&ctx, &self.ui.tokens);
        }
        let file = self.keytips(&ctx);
        self.shortcuts(&ctx);

        let mut acts: Vec<Action> = Vec::new();
        if file {
            acts.push(Action::Backstage(backstage::Page::Info));
        }
        self.files_from_desktop(&ctx, &mut acts);
        let t = self.ui.tokens;
        if let Some(key) = self.ui.page_message.clone() {
            self.message_page(ui, &key, &mut acts);
            if let Some(mut d) = self.ui.dialog.take() {
                let keep = dialogs::show(&ctx, &mut d, &self.session, &mut self.ui.pictures, &t, &mut acts);
                if keep && self.ui.dialog.is_none() {
                    self.ui.dialog = Some(d);
                }
            }
            self.toasts(&ctx);
            self.pictures_drawn(&ctx);
            for a in acts {
                self.act(a);
            }
            self.send_body_work(&ctx);
            self.flush_jobs();
            self.tell_before_zooming(&ctx);
            return;
        }
        // The File page has the window to itself, under the window's own title bar (as Outlook's).
        if self.ui.backstage.is_none() {
            let height = if self.ui.bar_two_rows { TOP_BAR_H + super::bars::BAR_ROW_H + 6.0 } else { TOP_BAR_H };
            let bar = egui::Panel::top("topbar")
                .exact_size(height)
                .frame(egui::Frame::new().fill(t.bar).inner_margin(Margin::symmetric(TOP_BAR_MARGIN.0, TOP_BAR_MARGIN.1)))
                .show(ui, |ui| {
                    self.top_bar(ui, &mut acts);
                });
            self.ui.top_bar_rect = Some(bar.response.rect);
            egui::Panel::top("ribbon")
                .frame(
                    egui::Frame::new()
                        .fill(t.chrome)
                        .inner_margin(Margin { left: 8, right: 8, top: 4, bottom: 6 })
                        .stroke(egui::Stroke::new(1.0, t.border)),
                )
                .show(ui, |ui| {
                    // Too wide for the window: its last buttons lose their labels, as few as it
                    // takes (worked out from this frame for the next).
                    // One height on every tab: switching tabs doesn't nudge the window below.
                    let classic = self.session.prefs.classic_ribbon;
                    ui.set_min_height(if classic { RIBBON_H_CLASSIC } else { RIBBON_H });
                    let id = widgets::ribbon_fit_id();
                    let start = widgets::RibbonFit {
                        active: true,
                        classic,
                        icons_from: self.ui.ribbon_icons_from,
                        right: ui.max_rect().left(),
                        left: ui.max_rect().right(),
                        ..Default::default()
                    };
                    ui.ctx().data_mut(|d| d.insert_temp(id, start));
                    // Right to left: its rows from the right (egui runs a row in a layout aligned
                    // right from the right), File first.
                    let rtl = i18n::current().rtl();
                    let room = ui.max_rect();
                    ui.with_layout(Layout::top_down(if rtl { Align::Max } else { Align::Min }), |ui| {
                        crate::keytips::on_ribbon(ui, |ui| self.ribbon(ui, &mut acts))
                    });
                    super::ribbon::card_under_ribbon(ui, &t, room);
                    let fit = ui.ctx().data_mut(|d| std::mem::take(d.get_temp_mut_or_default::<widgets::RibbonFit>(id)));
                    self.ui.ribbon_rect = Some(ui.min_rect());
                    let from = fit.fit(room, rtl);
                    if from != self.ui.ribbon_icons_from.min(fit.count) {
                        self.ui.ribbon_icons_from = from;
                        ui.ctx().request_repaint();
                    }
                });
            // A newer MailCraft ready to install (or a copy left from before): a bar of its own.
            if self.update_bar_wanted() {
                egui::Panel::top("update-bar")
                    .frame(egui::Frame::new().fill(t.accent_soft).inner_margin(Margin::symmetric(12, 6)).stroke(egui::Stroke::new(1.0, t.border)))
                    .show(ui, |ui| self.update_bar(ui, &mut acts));
            }
            // MailCraft's earlier Flatpak, whose mail came over at this start, still installed.
            if self.ui.old_flatpak.is_some() {
                egui::Panel::top("old-flatpak-bar")
                    .frame(egui::Frame::new().fill(t.accent_soft).inner_margin(Margin::symmetric(12, 6)).stroke(egui::Stroke::new(1.0, t.border)))
                    .show(ui, |ui| self.old_flatpak_bar(ui));
            }
        } else {
            self.ui.top_bar_rect = None;
            self.ui.ribbon_rect = None;
            // The File page keeps the window's title bar when it's MailCraft's: its mark, the
            // window's buttons, and the space between to drag it by.
            if own_title_bar {
                egui::Panel::top("file-title-bar")
                    .exact_size(TOP_BAR_H)
                    .frame(egui::Frame::new().fill(t.bar).inner_margin(Margin::symmetric(TOP_BAR_MARGIN.0, TOP_BAR_MARGIN.1)))
                    .show(ui, |ui| {
                        let inner = ui.max_rect();
                        let bar = inner.expand2(vec2(f32::from(TOP_BAR_MARGIN.0), f32::from(TOP_BAR_MARGIN.1)));
                        super::caption::drag_area(ui, bar, &[]);
                        let live = !super::caption::laid_over_a_dialog(ui.ctx());
                        super::caption::caption_buttons(ui, &t, bar, live);
                        super::caption::note_bar(ui.ctx(), bar, bar, Vec::new());
                        // The mark at the bar's start, clear of the buttons (right to left too).
                        let room = super::caption::CAPTIONS_W - f32::from(TOP_BAR_MARGIN.0);
                        let free = egui::Rect::from_min_max(inner.min, egui::pos2(inner.max.x - room, inner.max.y));
                        ui.scope_builder(egui::UiBuilder::new().max_rect(free).layout(bidi::row_from_start(Align::Center)), |ui| {
                            super::caption::mark(ui, &t)
                        });
                    });
            }
        }
        egui::Panel::bottom("status")
            .exact_size(26.0)
            .frame(egui::Frame::new().fill(t.chrome).inner_margin(Margin::symmetric(10, 3)).stroke(egui::Stroke::new(1.0, t.border)))
            .show(ui, |ui| {
                self.status_bar(ui);
            });

        if self.ui.backstage.is_some() {
            backstage::show(&self.session, &mut self.ui, ui, &mut acts);
            self.ui.backstage_typing = ctx.text_edit_focused();
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
            // In a small window the folders and the list give way, so what's read or written
            // keeps a width it can be read in (it was a strip a word wide).
            let mail_beside = module == Module::Mail && self.session.prefs.reading_pane == ReadingPane::Right;
            let most_nav = if mail_beside { (ui.available_width() - LIST_LEAST - READING_LEAST).clamp(170.0, 380.0) } else { 380.0 };
            let full = start("nav")
                .resizable(true)
                .default_size(if module == Module::Mail { 230.0 } else { 260.0 })
                .size_range(170.0..=most_nav)
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
                    Module::Tasks => tasks::nav(&self.session, &mut self.ui, ui, &mut acts),
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
                (Module::Tasks, _) => {
                    egui::CentralPanel::default().frame(egui::Frame::new().fill(t.surface)).show(ui, |ui| {
                        tasks::show(&self.session, &mut self.ui, ui, &mut acts);
                    });
                }
                (Module::Notes, _) => {
                    egui::CentralPanel::default().frame(egui::Frame::new().fill(t.surface)).show(ui, |ui| {
                        notes::show(&self.session, &mut self.ui, ui, &mut acts);
                    });
                }
                (Module::Mail, pane) => match pane {
                    ReadingPane::Right => {
                        let most = (ui.available_width() - READING_LEAST).clamp(LIST_LEAST, 720.0);
                        start("list").resizable(true).default_size(390.0).size_range(LIST_LEAST..=most).frame(list_frame).show(ui, |ui| {
                            list::show(&self.session, &mut self.ui, ui, &mut acts);
                        });
                        egui::CentralPanel::default().frame(egui::Frame::new().fill(t.surface)).show(ui, |ui| {
                            reading::show(&mut self.session, &mut self.ui, ui, &mut acts);
                        });
                    }
                    ReadingPane::Bottom => {
                        // A name of its own: under "list" it took the size the list beside the
                        // pane had saved, the whole height, and the pane had no room (issue #17).
                        // The pane always keeps some.
                        let most = (ui.available_height() - 160.0).max(140.0);
                        egui::Panel::top("list-above").resizable(true).default_size(300.0).size_range(140.0..=most).frame(list_frame).show(
                            ui,
                            |ui| {
                                list::show(&self.session, &mut self.ui, ui, &mut acts);
                            },
                        );
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
        self.source_windows(&ctx, &mut acts);
        self.compose_windows(&ctx, &mut acts);

        // The address book reads the session's people.
        if let Some(Dialog::SelectNames(f)) = self.ui.dialog.as_mut()
            && (f.stale || f.directory_rev != self.session.directory.rev)
        {
            // Opened: with the address book chosen to open with (one gone is everyone).
            if f.source.is_empty() {
                f.books = self.session.address_books();
                f.default_source = self.session.prefs.address_book.clone();
                let known = f.default_source == "people" || f.books.iter().any(|(b, _, _)| *b == f.default_source);
                f.source = if known { f.default_source.clone() } else { "all".into() };
            }
            f.directory = !self.session.directory_accounts().is_empty();
            // The person selected stays selected when answers come in and the list moves.
            let picked = f.selected.and_then(|i| f.list.get(i)).map(|a| a.email.clone());
            f.list = self.session.address_book(&f.source, &f.query);
            f.selected = picked.and_then(|e| f.list.iter().position(|a| a.email.eq_ignore_ascii_case(&e)));
            let asks = f.directory && (f.source == "all" || f.source == "directory");
            f.searching = asks && self.session.directory_pending(&f.query);
            f.directory_errors = if asks { self.session.directory_errors(&f.query) } else { Vec::new() };
            f.consent = asks && !self.session.directory.consent.is_empty();
            f.directory_rev = self.session.directory.rev;
            f.stale = false;
        }
        // A message window's own dialogs show in that window.
        let in_window = self.session.compose.as_ref().is_some_and(|c| !compose_inline(c, &ctx));
        if let Some(mut d) = self.ui.dialog.take() {
            if in_window && d.for_compose() {
                self.ui.dialog = Some(d);
            } else {
                let keep = dialogs::show(&ctx, &mut d, &self.session, &mut self.ui.pictures, &t, &mut acts);
                if keep && self.ui.dialog.is_none() {
                    self.ui.dialog = Some(d);
                }
            }
        }
        // The window's resize edges, over everything, when it has no system frame to resize by;
        // its title bar over a dialog holding the window.
        if own_title_bar {
            super::caption::over_a_modal(&ctx, &t);
            super::caption::resize_zones(ui);
        }
        self.toasts(&ctx);
        // Calendar reminders: the window, and the desktop told of each as it comes.
        for r in reminders::refresh(&mut self.ui.reminders, &self.session, false) {
            let when = if r.all_day { String::new() } else { crate::calendar::hm(r.start, &self.session) };
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
        self.pictures_drawn(&ctx);

        // A search a command started here (the box's, once the typing pauses) runs after the
        // frame was drawn and takes none of its units there: the next frame steps and shows it
        // now, not at the next wake (the settings tick: on a real file the results waited 2–5 s).
        let mut searched = false;
        for a in acts {
            searched |= matches!(a, Action::Run(id, _) if id.starts_with("search")) || matches!(a, Action::Search(_));
            self.act(a);
        }
        if searched || self.session.search_progress().is_some_and(|p| !p.done) {
            ctx.request_repaint();
        }
        self.send_body_work(&ctx);
        self.flush_jobs();
        if std::mem::take(&mut self.ui.focus_main) {
            ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Focus);
        }
        self.tell_before_zooming(&ctx);
        let busy = self.session.accounts.iter().any(|a| matches!(a.status, SyncStatus::Connecting | SyncStatus::Syncing { .. }));
        ctx.request_repaint_after(std::time::Duration::from_secs(if busy || self.ui.toast.is_some() { 1 } else { 20 }));
    }
}
