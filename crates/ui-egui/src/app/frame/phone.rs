use super::*;
use egui::{Align2, FontId, Vec2, pos2};
use mailcraft_engine::model::FolderRole;

impl MailApp {
    /// A compact primary action and a way to expand the existing ribbon on phones.
    pub(super) fn phone_action_bar(&mut self, ui: &mut Ui, t: &Tokens, acts: &mut Vec<Action>) {
        let (icon, label, action) = match self.session.module {
            Module::Mail => (Icon::NewMail, "New mail", Action::Run("mail.new", json!({}))),
            Module::Calendar => (Icon::Calendar, "New event", Action::Dialog(Dialog::new_event(&self.session, None))),
            Module::People => (Icon::Person, "New contact", Action::Dialog(Dialog::new_contact())),
            Module::Tasks => (Icon::Task, "New task", Action::Dialog(Dialog::new_task(&self.session, None))),
            Module::Notes => (Icon::Note, "New note", Action::NewNote),
        };
        let show_direct_read = ui.available_width() >= 330.0;
        let can_sync = !self.session.prefs.work_offline
            && self.session.accounts.iter().any(|account| !account.is_local() && !account.is_data_file())
            && self.session.registry().get("mail.sync").is_some_and(|command| (command.enabled)(&self.session).is_none());
        let sync_running = self.session.accounts.iter().any(|account| {
            !account.is_local() && !account.is_data_file() && matches!(account.status, SyncStatus::Connecting | SyncStatus::Syncing { .. })
        });
        ui.with_layout(crate::bidi::row_from_start(egui::Align::Center), |ui| {
            let selected = if self.session.module == Module::Mail { self.session.selection.len() } else { 0 };
            if selected == 0 {
                if widgets::accent_button(ui, icon, label, t, true).clicked() {
                    acts.push(action);
                }
                if self.session.module == Module::Mail && can_sync {
                    if sync_running {
                        let (rect, response) = ui.allocate_exact_size(vec2(44.0, 44.0), Sense::hover());
                        widgets::named(ui, &response, tl!("Send / Receive all folders"), false, None);
                        ui.put(rect, egui::Spinner::new().size(20.0));
                        response.on_hover_text(tl!("Send / Receive all folders"));
                    } else if widgets::bar_icon_button(ui, Icon::Sync, "Send / Receive all folders", t).clicked() {
                        acts.push(Action::Run("mail.sync", json!({})));
                    }
                }
            } else {
                ui.label(RichText::new(tf!("Items: {n}", n = selected)).strong().color(t.text));
                let keys = self.session.selection.clone();
                if widgets::bar_icon_button(ui, Icon::Close, "Clear", t).clicked() {
                    for key in keys {
                        acts.push(Action::Run("list.select", json!({"key": key.to_id(), "add": true})));
                    }
                }
                ui.spacing_mut().item_spacing.x = 4.0;
                let can_run = |id: &str| self.session.registry().get(id).is_some_and(|command| (command.enabled)(&self.session).is_none());
                if can_run("mail.archive") && widgets::bar_icon_button(ui, Icon::Archive, "Archive", t).clicked() {
                    acts.push(Action::Run("mail.archive", json!({})));
                }
                let all_seen = self.session.selected_iter().all(|message| message.flags.seen);
                let read_command = if all_seen { "mail.markUnread" } else { "mail.markRead" };
                if show_direct_read
                    && can_run(read_command)
                    && widgets::bar_icon_button(
                        ui,
                        if all_seen { Icon::Unread } else { Icon::Read },
                        if all_seen { "Mark as unread" } else { "Mark as read" },
                        t,
                    )
                    .clicked()
                {
                    acts.push(Action::Run(read_command, json!({})));
                }
                if can_run("mail.delete") && widgets::bar_icon_button(ui, Icon::Delete, "Delete", t).clicked() {
                    let ask = self.session.prefs.confirm_permanent_delete && self.session.view_role() == Some(FolderRole::Trash);
                    if ask {
                        acts.push(Action::Dialog(Dialog::ConfirmDelete { count: selected }));
                    } else {
                        acts.push(Action::Run("mail.delete", json!({})));
                    }
                }
            }
            let more = if selected > 0 {
                ui.add_sized(vec2(44.0, 44.0), egui::Button::new(RichText::new("⋯").size(22.0)))
            } else {
                let height = if ui.ctx().input(|input| input.has_touch_screen()) { 44.0 } else { 36.0 };
                ui.add_sized(vec2(ui.available_width(), height), egui::Button::new(tl!("More actions")))
            };
            widgets::named(ui, &more, tl!("More actions"), true, None);
            crate::bidi::popup_menu(&more).show(|ui| {
                ui.spacing_mut().interact_size.y = 44.0;
                let can_work_offline = self.session.accounts.iter().any(|account| !account.is_local() && !account.is_data_file());
                if can_work_offline {
                    let mut work_offline = self.session.prefs.work_offline;
                    if ui.checkbox(&mut work_offline, tl!("Work offline")).changed() {
                        acts.push(Action::Run("mail.workOffline", json!({"on": work_offline})));
                    }
                    ui.separator();
                }
                if selected < self.session.list.len() && self.session.module == Module::Mail && !self.ui.phone_reader_open {
                    if ui.button(tl!("Select all")).clicked() {
                        acts.push(Action::Run("list.selectAll", json!({})));
                        ui.close();
                    }
                    ui.separator();
                }
                if selected > 0 {
                    let all_seen = self.session.selected_iter().all(|message| message.flags.seen);
                    if ui.button(tl!(if all_seen { "Mark as unread" } else { "Mark as read" })).clicked() {
                        acts.push(Action::Run(if all_seen { "mail.markUnread" } else { "mail.markRead" }, json!({})));
                        ui.close();
                    }
                    let all_flagged = self.session.selected_iter().all(|message| message.flags.flagged);
                    if ui.button(tl!(if all_flagged { "Unflag" } else { "Flag" })).clicked() {
                        acts.push(Action::Run("mail.flag", json!({"on": !all_flagged})));
                        ui.close();
                    }
                    if ui.button(tl!("Move to…")).clicked() {
                        acts.push(Action::Dialog(Dialog::move_to()));
                        ui.close();
                    }
                    ui.separator();
                }
                for (tab, name) in
                    [(RibbonTab::Home, "Home"), (RibbonTab::SendReceive, "Send / Receive"), (RibbonTab::View, "View"), (RibbonTab::Search, "Search")]
                {
                    if ui.selectable_label(self.ui.tab == tab, tl!(name)).clicked() {
                        self.ui.tab = tab;
                        self.ui.phone_actions_open = true;
                        ui.close();
                    }
                }
                ui.separator();
                let toggle = if self.ui.phone_actions_open { "Hide" } else { "Show" };
                if ui.button(tl!(toggle)).clicked() {
                    self.ui.phone_actions_open = !self.ui.phone_actions_open;
                    ui.close();
                }
            });
        });
    }

    /// Keep all five MailCraft modules available from the phone's bottom navigation.
    pub(super) fn phone_module_navigation(&mut self, ui: &mut Ui, t: &Tokens, acts: &mut Vec<Action>) {
        let items = [
            (Module::Mail, Icon::Inbox, "Mail", "mail"),
            (Module::Calendar, Icon::Calendar, "Calendar", "calendar"),
            (Module::People, Icon::People, "People", "people"),
            (Module::Tasks, Icon::Task, "Tasks", "tasks"),
            (Module::Notes, Icon::Note, "Notes", "notes"),
        ];
        let width = ui.available_width() / items.len() as f32;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            for (module, icon, label, code) in items {
                let (rect, response) = ui.allocate_exact_size(vec2(width, 56.0), Sense::click());
                let active = module == self.session.module;
                widgets::named(ui, &response, tl!(label), true, Some(active));
                if active || response.hovered() {
                    ui.painter().rect_filled(rect.shrink2(vec2(3.0, 2.0)), 8.0, if active { t.accent_soft } else { t.hover });
                }
                let color = if active { t.accent } else { t.dim };
                let label_text = tl!(label).to_string();
                let label_font = FontId::proportional(10.0);
                let label_width = ui.painter().layout_no_wrap(label_text.clone(), label_font.clone(), color).size().x;
                let show_label = label_width <= (width - 8.0).max(0.0);
                let icon_offset_y = if show_label { -9.0 } else { 0.0 };
                icons::paint(ui.painter(), icons::centered(rect, 20.0).translate(vec2(0.0, icon_offset_y)), icon, color);
                if show_label {
                    ui.painter().with_clip_rect(rect).text(
                        pos2(rect.center().x, rect.top() + 42.0),
                        Align2::CENTER_CENTER,
                        label_text,
                        label_font,
                        color,
                    );
                }
                let mut response = response.on_hover_text(tl!(label));
                if module == Module::Mail {
                    let unread = self
                        .session
                        .accounts
                        .iter()
                        .filter_map(|account| account.folder(FolderRole::Inbox))
                        .fold(0u32, |total, folder| total.saturating_add(folder.unread));
                    if unread > 0 {
                        let center = pos2(rect.center().x + 13.0, rect.top() + 14.0);
                        ui.painter().circle_filled(center, 9.0, t.danger);
                        let count = if unread > 99 { "99+".to_owned() } else { unread.to_string() };
                        ui.painter().text(center, Align2::CENTER_CENTER, count, FontId::proportional(9.0), t.surface);
                        response = response.on_hover_text(tf!("Unread: {n}", n = unread));
                    }
                }
                if response.clicked() {
                    acts.push(Action::Run("view.module", json!({"name": code})));
                }
            }
        });
    }

    /// On a phone or narrow window, folders cover the current page instead of squeezing it beside
    /// the navigation strip. The backdrop makes dismissal clear and keeps taps out of the mail.
    pub(super) fn mobile_folder_drawer(&mut self, ctx: &egui::Context, content_rect: egui::Rect, t: &Tokens, rtl: bool, acts: &mut Vec<Action>) {
        if !self.ui.mobile_folder_open {
            return;
        }
        if self.ui.backstage.is_some() || self.ui.dialog.is_some() {
            self.ui.mobile_folder_open = false;
            return;
        }
        let id = egui::Id::unique("mobile_folder_drawer");
        let area_rect = ctx.content_rect();
        let screen = content_rect.intersect(area_rect);
        let width = screen.width().clamp(0.0, 340.0);
        let anchor = if rtl { egui::Align2::RIGHT_TOP } else { egui::Align2::LEFT_TOP };
        let response =
            egui::Modal::new(id).area(egui::Modal::default_area(id).anchor(anchor, egui::Vec2::ZERO)).frame(egui::Frame::NONE).show(ctx, |ui| {
                ui.set_min_size(area_rect.size());
                ui.set_max_size(area_rect.size());
                let drawer_rect = if rtl {
                    egui::Rect::from_min_max(egui::pos2(screen.right() - width, screen.top()), screen.right_bottom())
                } else {
                    egui::Rect::from_min_max(screen.left_top(), egui::pos2(screen.left() + width, screen.bottom()))
                };
                let mut close = false;
                ui.scope_builder(egui::UiBuilder::new().max_rect(drawer_rect).layout(Layout::top_down(Align::Min)), |ui| {
                    egui::Frame::new().fill(t.chrome).stroke(egui::Stroke::new(1.0, t.border)).inner_margin(Margin::symmetric(6, 8)).show(ui, |ui| {
                        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                            let chevron = if rtl { Icon::ChevronRight } else { Icon::ChevronLeft };
                            if widgets::tiny_icon_button(ui, chevron, "Minimize the folder pane", t).clicked() {
                                close = true;
                            }
                        });
                        egui::Panel::bottom("mobile-drawer-modules")
                            .frame(egui::Frame::new().inner_margin(Margin::symmetric(0, 4)))
                            .show(ui, |ui| module_switch(ui, t, self.session.module, acts));
                        let actions_before_navigation = acts.len();
                        match self.session.module {
                            Module::Mail => nav::show(&self.session, t, self.ui.can_open_path, ui, acts),
                            Module::Calendar => calendar::nav(&self.session, &mut self.ui, ui, acts),
                            Module::People => people::nav(&self.session, &mut self.ui, ui, acts),
                            Module::Tasks => tasks::nav(&self.session, &mut self.ui, ui, acts),
                            Module::Notes => notes::nav(&self.session, &mut self.ui, ui, acts),
                        }
                        close |= acts.len() > actions_before_navigation;
                    });
                });
                close
            });
        if response.inner || response.backdrop_response.clicked() {
            self.ui.mobile_folder_open = false;
        }
    }
}
