//! The ribbon: its tabs and their buttons.

use crate::*;

impl MailApp {
    pub(crate) fn ribbon(&mut self, ui: &mut Ui, acts: &mut Vec<Action>) {
        let t = self.ui.tokens;
        let s = &self.session;
        // The Search tab goes when the search does (and the box isn't being typed in).
        let on_ribbon = self.ui.ribbon_rect.zip(ui.ctx().pointer_latest_pos()).is_some_and(|(r, p)| r.contains(p));
        if self.ui.tab == RibbonTab::Search && s.search.is_none() && !self.ui.search_focused && !on_ribbon {
            self.ui.tab = RibbonTab::Home;
        }
        let searching = self.ui.tab == RibbonTab::Search || s.search.is_some();
        ui.horizontal(|ui| {
            if ui.add(egui::Button::new(RichText::new(tl!("File")).color(t.dim).font(theme::regular(13.5))).frame_when_inactive(false)).clicked() {
                acts.push(Action::Backstage(backstage::Page::Info));
            }
            let tabs =
                [(RibbonTab::Home, "Home"), (RibbonTab::SendReceive, "Send / Receive"), (RibbonTab::View, "View"), (RibbonTab::Search, "Search")];
            for (tab, label) in tabs.into_iter().filter(|(tab, _)| *tab != RibbonTab::Search || searching) {
                let on = self.ui.tab == tab;
                let r = ui.add(
                    egui::Button::new(RichText::new(tl!(label)).color(if on { t.accent } else { t.dim }).font(if on {
                        theme::bold(13.5)
                    } else {
                        theme::regular(13.5)
                    }))
                    .frame_when_inactive(false),
                );
                if on {
                    // The underline glides from the tab left to the one chosen.
                    let rect = r.rect;
                    let ctx = ui.ctx().clone();
                    let left = motion::glide(&ctx, egui::Id::unique("ribbon-tab-left"), rect.left() + 6.0, motion::NORMAL);
                    let right = motion::glide(&ctx, egui::Id::unique("ribbon-tab-right"), rect.right() - 6.0, motion::NORMAL);
                    ui.painter().line_segment(
                        [egui::pos2(left, rect.bottom() + 1.0), egui::pos2(right, rect.bottom() + 1.0)],
                        egui::Stroke::new(2.0, t.accent),
                    );
                }
                if r.clicked() {
                    self.ui.tab = tab;
                }
            }
        });
        ui.add_space(2.0);
        let enabled = |id: &str| s.registry().get(id).is_some_and(|c| (c.enabled)(s).is_none());
        if s.module != Module::Mail {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                if s.module == Module::Calendar {
                    if widgets::accent_button(ui, Icon::Calendar, "New event", &t, !s.accounts.is_empty()).clicked() {
                        acts.push(Action::Dialog(Dialog::new_event(s, None)));
                    }
                    widgets::group_sep(ui, &t);
                    if widgets::tool_button(ui, Icon::Clock, "Today", &t, true).clicked() {
                        acts.push(Action::Run("calendar.goto", json!({"date": "today"})));
                    }
                    widgets::group_sep(ui, &t);
                    for (mode, label, code) in [
                        (CalMode::Day, "Day", "day"),
                        (CalMode::WorkWeek, "Work week", "workWeek"),
                        (CalMode::Week, "Week", "week"),
                        (CalMode::Month, "Month", "month"),
                    ] {
                        let r = widgets::tool_button(ui, Icon::Layout, label, &t, s.cal_mode != mode);
                        if r.clicked() {
                            acts.push(Action::Run("calendar.view", json!({"mode": code})));
                        }
                    }
                    // Each calendar in a column of its own (Outlook's side by side), or together.
                    if s.pim.values().map(|p| p.calendars.len()).sum::<usize>() > 1 {
                        widgets::group_sep(ui, &t);
                        if widgets::tool_toggle(ui, Icon::Calendar, "Side by side", &t, s.prefs.calendars_side_by_side).clicked() {
                            acts.push(Action::Run("calendar.sideBySide", json!({})));
                        }
                    }
                } else if s.module == Module::Notes {
                    if widgets::accent_button(ui, Icon::Note, "New note", &t, !s.accounts.is_empty()).clicked() {
                        notes::new_note(&mut self.ui, acts);
                    }
                    widgets::group_sep(ui, &t);
                    let editing = self.ui.note_edit.as_ref().map(|(u, _)| u.clone());
                    if widgets::tool_button(ui, Icon::Delete, "Delete", &t, editing.is_some()).clicked()
                        && let Some(uid) = editing.clone()
                    {
                        self.ui.note_edit = None;
                        acts.push(Action::Run("note.delete", json!({"uid": uid})));
                    }
                    // The note's colour, as Outlook's Categorize on a note.
                    let r = widgets::tool_button(ui, Icon::Tag, "Color", &t, editing.is_some());
                    egui::Popup::menu(&r).show(|ui| {
                        for (i, name) in notes::NAMES.iter().enumerate() {
                            if ui.button(tl!(name)).clicked()
                                && let Some(uid) = &editing
                            {
                                acts.push(Action::Run("note.update", json!({"uid": uid, "color": i})));
                                ui.close();
                            }
                        }
                    });
                    // How they're listed: the last changed first, A to Z, or by colour.
                    widgets::group_sep(ui, &t);
                    let r = widgets::tool_button(ui, Icon::ListBullet, "Arrange by", &t, true);
                    egui::Popup::menu(&r).show(|ui| {
                        use mailcraft_engine::NotesOrder as O;
                        for (order, label, code) in
                            [(O::Modified, "Last changed", "modified"), (O::Title, "Title (A to Z)", "title"), (O::Color, "Color", "color")]
                        {
                            if ui.radio(s.prefs.notes_order == order, tl!(label)).clicked() {
                                acts.push(Action::Run("note.arrange", json!({"by": code})));
                                ui.close();
                            }
                        }
                    });
                } else {
                    if widgets::accent_button(ui, Icon::Person, "New contact", &t, !s.accounts.is_empty()).clicked() {
                        acts.push(Action::Dialog(Dialog::new_contact()));
                    }
                    widgets::group_sep(ui, &t);
                    let picked = self.ui.person.as_deref().and_then(|u| s.find_person(u));
                    // Someone in a shared (read-only) address book can't be changed here.
                    let picked = picked.filter(|p| !s.person_read_only(&p.card.uid));
                    if widgets::tool_button(ui, Icon::Drafts, "Edit", &t, picked.is_some()).clicked()
                        && let Some(p) = &picked
                    {
                        acts.push(Action::Dialog(Dialog::edit_contact(&p.card)));
                    }
                    if widgets::tool_button(ui, Icon::Delete, "Delete", &t, picked.is_some()).clicked()
                        && let Some(p) = &picked
                    {
                        acts.push(Action::Dialog(Dialog::ConfirmDeleteContact { uid: p.card.uid.clone(), name: p.card.display() }));
                    }
                    // Outlook's Change View: the list with a card, business cards, a phone list.
                    widgets::group_sep(ui, &t);
                    for v in people::PeopleView::ALL {
                        if widgets::tool_toggle(ui, v.icon(), v.label(), &t, self.ui.people_view == v).clicked() {
                            self.ui.people_view = v;
                        }
                    }
                    // The list's columns (also from its header's right-click).
                    if self.ui.people_view == people::PeopleView::Phone {
                        let r = widgets::tool_button(ui, Icon::Layout, "Columns", &t, true);
                        egui::Popup::menu(&r)
                            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                            .show(|ui| people::column_menu(ui, &mut self.ui.people_list));
                    }
                }
            });
            return;
        }
        let confirm_delete = self.needs_delete_confirm();
        let ust = &mut self.ui;
        // A narrow window scrolls the ribbon sideways rather than cutting buttons off; each tab
        // keeps its own place (one shared place opened View scrolled as far as Home was, which
        // gridcraft's ribbon found).
        // Right to left it starts scrolled to the right, where the row starts.
        let rtl = i18n::current().rtl();
        egui::ScrollArea::horizontal().id_salt(("ribbon", format!("{:?}", ust.tab))).stick_to_right(rtl).show(ui, |ui| {
            widgets::scrolled_row(ui, |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                widgets::start_group(ui);
                match ust.tab {
                    RibbonTab::Home => {
                        if widgets::accent_button(ui, Icon::NewMail, "New mail", &t, enabled("mail.new"))
                            .on_hover_text(tl!("New mail (Ctrl+N)"))
                            .clicked()
                        {
                            acts.push(Action::Run("mail.new", json!({})));
                        }
                        // New items of other kinds, as Outlook's New Email drop-down.
                        let r = widgets::tiny_icon_button(ui, Icon::ChevronDown, "New items", &t);
                        egui::Popup::menu(&r).show(|ui| {
                            if ui.button(tl!("Email message")).clicked() {
                                acts.push(Action::Run("mail.new", json!({})));
                                ui.close();
                            }
                            if ui.button(tl!("Event")).clicked() {
                                acts.push(Action::Dialog(Dialog::new_event(s, None)));
                                ui.close();
                            }
                            if ui.button(tl!("Contact")).clicked() {
                                acts.push(Action::Dialog(Dialog::new_contact()));
                                ui.close();
                            }
                        });
                        widgets::group(ui, &t, "New", false);
                        if widgets::tool_button(ui, Icon::Delete, "Delete", &t, enabled("mail.delete")).on_hover_text(tl!("Delete (Del)")).clicked() {
                            if confirm_delete {
                                acts.push(Action::Dialog(Dialog::ConfirmDelete { count: s.selection.len() }));
                            } else {
                                acts.push(Action::Run("mail.delete", json!({})));
                            }
                        }
                        // Ignore the conversation, as Outlook's Delete drop-down.
                        let r = widgets::tiny_icon_button(ui, Icon::ChevronDown, "More delete options", &t);
                        egui::Popup::menu(&r).show(|ui| {
                            let ignoring = !s.selection.is_empty() && s.selected().iter().all(|m| s.is_ignored(m));
                            let label = if ignoring { "Stop ignoring the conversation" } else { "Ignore the conversation" };
                            if ui
                                .add_enabled(enabled("mail.ignore"), egui::Button::new(tl!(label)))
                                .on_hover_text(tl!("Its messages, and replies that arrive later, go to Deleted Items"))
                                .clicked()
                            {
                                acts.push(Action::Run("mail.ignore", json!({})));
                                ui.close();
                            }
                            // Clean Up: what a later reply quotes in full goes to Deleted Items.
                            ui.separator();
                            let tip = tl!("Messages quoted in full by a later reply go to Deleted Items");
                            if ui.add_enabled(enabled("mail.cleanUp"), egui::Button::new(tl!("Clean up conversation"))).on_hover_text(tip).clicked() {
                                acts.push(Action::Run("mail.cleanUp", json!({})));
                                ui.close();
                            }
                            if ui.add_enabled(enabled("folder.cleanUp"), egui::Button::new(tl!("Clean up folder"))).on_hover_text(tip).clicked() {
                                acts.push(Action::Run("folder.cleanUp", json!({})));
                                ui.close();
                            }
                        });
                        if widgets::tool_button(ui, Icon::Archive, "Archive", &t, enabled("mail.archive"))
                            .on_hover_text(tl!("Archive (Backspace)"))
                            .clicked()
                        {
                            acts.push(Action::Run("mail.archive", json!({})));
                        }
                        let in_junk = s.view_role() == Some(mailcraft_engine::model::FolderRole::Junk);
                        let (jid, jl) = if in_junk { ("mail.notJunk", "Not junk") } else { ("mail.junk", "Report junk") };
                        if widgets::tool_button(ui, Icon::Junk, jl, &t, enabled(jid)).clicked() {
                            acts.push(Action::Run(jid, json!({})));
                        }
                        // The Junk drop-down: the sender's lists, as Outlook's.
                        let r = widgets::tiny_icon_button(ui, Icon::ChevronDown, "Junk options", &t);
                        egui::Popup::menu(&r).show(|ui| junk_menu(ui, s, acts));
                        widgets::group(ui, &t, "Delete", false);
                        if widgets::tool_button(ui, Icon::Move, "Move to", &t, enabled("mail.move"))
                            .on_hover_text(tl!("Move to a folder (Ctrl+Shift+V)"))
                            .clicked()
                        {
                            acts.push(Action::Dialog(Dialog::move_to()));
                        }
                        // Quick Steps: each done to the selection in one click (Ctrl+Shift+1…9).
                        let r = widgets::tool_button(ui, Icon::Steps, "Quick steps", &t, true);
                        egui::Popup::menu(&r).show(|ui| quick_steps_menu(ui, s, acts));
                        widgets::group(ui, &t, "Move", false);
                        for (id, icon, label, tip) in [
                            ("mail.reply", Icon::Reply, "Reply", "Reply (Ctrl+R)"),
                            ("mail.replyAll", Icon::ReplyAll, "Reply all", "Reply all (Ctrl+Shift+R)"),
                            ("mail.forward", Icon::Forward, "Forward", "Forward (Ctrl+F)"),
                        ] {
                            if widgets::tool_button(ui, icon, label, &t, enabled(id)).on_hover_text(tl!(tip)).clicked() {
                                acts.push(Action::Run(id, json!({})));
                            }
                        }
                        widgets::group(ui, &t, "Respond", false);
                        let first_unread = s.selected().first().is_some_and(|m| !m.flags.seen);
                        let (ricon, rlabel) = if first_unread { (Icon::Read, "Mark read") } else { (Icon::Unread, "Mark unread") };
                        if widgets::tool_button(ui, ricon, rlabel, &t, enabled("mail.toggleRead"))
                            .on_hover_text("Read / Unread (Ctrl+Q / Ctrl+U)")
                            .clicked()
                        {
                            acts.push(Action::Run("mail.toggleRead", json!({})));
                        }
                        if widgets::tool_button(ui, Icon::Flag, "Flag", &t, enabled("mail.flag"))
                            .on_hover_text(tl!("Flag / Unflag (Insert)"))
                            .clicked()
                        {
                            acts.push(Action::Run("mail.flag", json!({})));
                        }
                        // Follow Up: when it's due, as Outlook's flag drop-down.
                        let r = widgets::tiny_icon_button(ui, Icon::ChevronDown, "Follow up", &t);
                        egui::Popup::menu(&r).show(|ui| organize::follow_up_menu(ui, s, &t, acts));
                        let sel = !s.selection.is_empty();
                        let r = widgets::tool_button(ui, Icon::Tag, "Categorize", &t, sel);
                        egui::Popup::menu(&r).show(|ui| organize::categorize_menu(ui, s, &t, acts));
                        let pinned = s.selected().first().is_some_and(|m| m.flags.pinned());
                        if widgets::tool_button(ui, Icon::Pin, if pinned { "Unpin" } else { "Pin" }, &t, sel).clicked() {
                            acts.push(Action::Run("mail.pin", json!({"on": !pinned})));
                        }
                        let r = widgets::tool_button(ui, Icon::Clock, "Snooze", &t, sel);
                        egui::Popup::menu(&r).show(|ui| organize::snooze_menu(ui, s, &t, acts));
                        widgets::group(ui, &t, "Tags", false);
                        // Show only some mail, as Outlook's Filter Email.
                        let r = widgets::tool_button(ui, Icon::Funnel, "Filter", &t, true);
                        egui::Popup::menu(&r).show(|ui| {
                            for f in mailcraft_engine::ListFilter::EVERY {
                                if ui.selectable_label(s.filter == f, tl!(f.label())).clicked() {
                                    acts.push(Action::Run("list.filter", json!({"value": f.code()})));
                                    ui.close();
                                }
                            }
                        });
                        widgets::group(ui, &t, "Find", true);
                    }
                    RibbonTab::SendReceive => {
                        if widgets::tool_button(ui, Icon::Sync, "Send / Receive all folders", &t, enabled("mail.sync")).on_hover_text("F9").clicked()
                        {
                            acts.push(Action::Run("mail.sync", json!({})));
                        }
                        if widgets::tool_button(ui, Icon::Folder, "Update folder", &t, enabled("folder.sync")).on_hover_text("Shift+F9").clicked() {
                            acts.push(Action::Run("folder.sync", json!({})));
                        }
                        if widgets::tool_button(ui, Icon::Send, "Send all", &t, enabled("outbox.send") && !s.prefs.work_offline)
                            .on_hover_text(tl!("Send what's waiting in the Outbox"))
                            .clicked()
                        {
                            acts.push(Action::Run("outbox.send", json!({})));
                        }
                        widgets::group(ui, &t, "Send & Receive", !ust.has_files);
                        // Working offline: no connections; mail you send waits in the Outbox.
                        if ust.has_files {
                            let off = s.prefs.work_offline;
                            if ui
                                .selectable_label(off, tl!("Work offline"))
                                .on_hover_text(tl!("Close every connection: changes, mail you send and drafts wait on this computer"))
                                .clicked()
                            {
                                acts.push(Action::Run("mail.workOffline", json!({"on": !off})));
                            }
                            widgets::group(ui, &t, "Preferences", true);
                        }
                    }
                    RibbonTab::Search => {
                        let query = s.search.as_ref().map(|(q, _)| q.clone()).unwrap_or_else(|| ust.search_text.clone());
                        let current = s.search.as_ref().map_or(s.prefs.search_scope, |(_, scope)| *scope);
                        ui.label(RichText::new(tl!("Look in")).color(t.dim));
                        for scope in SearchScope::ALL {
                            if ui.selectable_label(scope == current, scope_label(scope)).clicked() && scope != current {
                                acts.push(Action::Run("search.run", json!({"query": query, "scope": scope.code()})));
                            }
                        }
                        widgets::group(ui, &t, "Scope", false);
                        // Terms to type a name after: put in the box, ready for it.
                        for (icon, label, term) in
                            [(Icon::Person, "From", "from:"), (Icon::Send, "Sent To", "to:"), (Icon::Read, "Subject", "subject:")]
                        {
                            if widgets::tool_button(ui, icon, label, &t, true).clicked() {
                                let text = &mut ust.search_text;
                                if !text.is_empty() && !text.ends_with(' ') {
                                    text.push(' ');
                                }
                                text.push_str(term);
                                ust.focus_search = true;
                                ust.search_cursor_end = true;
                            }
                        }
                        // Terms that narrow at once: on, or off again.
                        for (icon, label, term) in [
                            (Icon::Attach, "Has Attachments", "has:attachment"),
                            (Icon::Unread, "Unread", "is:unread"),
                            (Icon::Tag, "Categorized", "is:categorized"),
                            (Icon::Flag, "Flagged", "is:flagged"),
                            (Icon::Warning, "Important", "is:important"),
                            (Icon::Send, "Awaiting reply", "is:awaiting"),
                        ] {
                            let words: Vec<&str> = query.split_whitespace().collect();
                            let on = words.iter().any(|w| w.eq_ignore_ascii_case(term));
                            if widgets::tool_toggle(ui, icon, label, &t, on).clicked() {
                                let q = if on {
                                    words.iter().filter(|w| !w.eq_ignore_ascii_case(term)).copied().collect::<Vec<_>>().join(" ")
                                } else if query.trim().is_empty() {
                                    term.to_string()
                                } else {
                                    format!("{} {term}", query.trim_end())
                                };
                                ust.search_text = q.clone();
                                if q.trim().is_empty() {
                                    acts.push(Action::Run("search.clear", json!({})));
                                } else {
                                    acts.push(Action::Run("search.run", json!({"query": q, "scope": current.code()})));
                                }
                            }
                        }
                        widgets::group(ui, &t, "Refine", false);
                        if widgets::tool_button(ui, Icon::Close, "Close Search", &t, true).clicked() {
                            acts.push(Action::Run("search.clear", json!({})));
                            ust.search_text.clear();
                            ust.tab = RibbonTab::Home;
                        }
                        widgets::group(ui, &t, "Close", true);
                    }
                    RibbonTab::View => {
                        ui.label(RichText::new(tl!("Reading pane")).color(t.dim));
                        for (v, label) in [(ReadingPane::Right, "Right"), (ReadingPane::Bottom, "Bottom"), (ReadingPane::Off, "Off")] {
                            if ui.selectable_label(s.prefs.reading_pane == v, tl!(label)).clicked() {
                                acts.push(Action::Run("view.readingPane", json!({"value": label.to_lowercase()})));
                            }
                        }
                        widgets::group(ui, &t, "Reading pane", false);
                        if ui
                            .selectable_label(s.prefs.focused_inbox, tl!("Focused Inbox"))
                            .on_hover_text(tl!("Put newsletters and automatic mail in Other"))
                            .clicked()
                        {
                            acts.push(Action::Run("view.focused", json!({})));
                        }
                        if ui
                            .selectable_label(s.prefs.conversations, tl!("Conversations"))
                            .on_hover_text(tl!("Group messages by conversation"))
                            .clicked()
                        {
                            acts.push(Action::Run("view.conversations", json!({})));
                        }
                        if ui.selectable_label(s.prefs.show_preview, tl!("Message preview")).clicked() {
                            acts.push(Action::Run("view.preview", json!({})));
                        }
                        if ui.selectable_label(s.prefs.compact, tl!("Compact")).clicked() {
                            acts.push(Action::Run("view.compact", json!({})));
                        }
                        if ui.selectable_label(!s.prefs.folder_pane_minimized, tl!("Folder pane")).clicked() {
                            acts.push(Action::Run("view.folderPane", json!({})));
                        }
                        widgets::group(ui, &t, "Layout", false);
                        let r = ui.button(tf!("Arrange by: {field}", field = tl!(s.prefs.arrange.label())));
                        egui::Popup::menu(&r).show(|ui| list::arrange_menu(ui, s, acts));
                        if ui.selectable_label(s.prefs.reverse_sort, tl!("Reverse sort")).clicked() {
                            acts.push(Action::Run("view.reverseSort", json!({})));
                        }
                        widgets::group(ui, &t, "Arrangement", false);
                        ui.label(RichText::new(tl!("Theme")).color(t.dim));
                        for (v, label) in [(None, "System"), (Some(false), "Light"), (Some(true), "Dark")] {
                            if ui.selectable_label(ust.dark == v, tl!(label)).clicked() {
                                acts.push(Action::SetDark(v));
                            }
                        }
                        // Big icons over their labels, in named groups (Outlook's classic
                        // ribbon), or one row of small ones.
                        if ui
                            .selectable_label(s.prefs.classic_ribbon, tl!("Classic ribbon"))
                            .on_hover_text(tl!("Big buttons with their names under them, in named groups"))
                            .clicked()
                        {
                            acts.push(Action::Run("view.classicRibbon", json!({})));
                        }
                        widgets::group(ui, &t, "Appearance", true);
                    }
                }
            });
        });
    }
}

/// The ribbon's height (its tabs and its row of buttons), the same whichever tab is shown.
pub(crate) const RIBBON_H: f32 = 65.0;
/// The classic ribbon's: its tabs, then big buttons with the groups' names under them.
pub(crate) const RIBBON_H_CLASSIC: f32 = RIBBON_H - 30.0 + widgets::CLASSIC_BUTTON_H + widgets::CLASSIC_CAPTION_H;

/// Mail, Calendar and People, at the bottom of the side pane.
pub(crate) fn module_switch(ui: &mut Ui, t: &Tokens, module: Module, acts: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for (m, icon, label, code) in [
            (Module::Mail, Icon::Unread, "Mail", "mail"),
            (Module::Calendar, Icon::Calendar, "Calendar", "calendar"),
            (Module::People, Icon::People, "People", "people"),
            (Module::Notes, Icon::Note, "Notes", "notes"),
        ] {
            let (r, resp) = ui.allocate_exact_size(vec2(40.0, 34.0), Sense::click());
            let on = m == module;
            widgets::named(ui, &resp, tl!(label), true, Some(on));
            if on || resp.hovered() {
                ui.painter().rect_filled(r, 6.0, if on { t.accent_soft } else { t.hover });
            }
            icons::paint(ui.painter(), icons::centered(r, 19.0), icon, if on { t.accent } else { t.dim });
            if resp.on_hover_text(tl!(label)).clicked() {
                acts.push(Action::Run("view.module", json!({"name": code})));
            }
        }
    });
}

/// Mail, Calendar and People one above another (the minimized folder pane).
pub(crate) fn module_switch_vertical(ui: &mut Ui, t: &Tokens, module: Module, acts: &mut Vec<Action>) {
    for (m, icon, label, code) in [
        (Module::Mail, Icon::Unread, "Mail", "mail"),
        (Module::Calendar, Icon::Calendar, "Calendar", "calendar"),
        (Module::People, Icon::People, "People", "people"),
        (Module::Notes, Icon::Note, "Notes", "notes"),
    ] {
        let (r, resp) = ui.allocate_exact_size(vec2(40.0, 34.0), Sense::click());
        let on = m == module;
        widgets::named(ui, &resp, tl!(label), true, Some(on));
        if on || resp.hovered() {
            ui.painter().rect_filled(r, 6.0, if on { t.accent_soft } else { t.hover });
        }
        icons::paint(ui.painter(), icons::centered(r, 19.0), icon, if on { t.accent } else { t.dim });
        if resp.on_hover_text(tl!(label)).clicked() {
            acts.push(Action::Run("view.module", json!({"name": code})));
        }
    }
}

/// The Quick Steps menu: each step (done to the selection), then making and changing them.
pub(crate) fn quick_steps_menu(ui: &mut egui::Ui, s: &Session, acts: &mut Vec<Action>) {
    let has = !s.selection.is_empty();
    if s.prefs.quick_steps.is_empty() {
        ui.label(RichText::new(tl!("No Quick Steps yet.")).weak());
    }
    for (i, q) in s.prefs.quick_steps.iter().enumerate() {
        let mut b = egui::Button::new(q.name.as_str());
        if i < 9 {
            b = b.shortcut_text(format!("Ctrl+Shift+{}", i + 1));
        }
        if ui.add_enabled(has, b).clicked() {
            acts.push(Action::Run("quickStep.run", json!({"index": i})));
            ui.close();
        }
    }
    ui.separator();
    if ui.button(tl!("New Quick Step…")).clicked() {
        acts.push(Action::Dialog(Dialog::QuickStep(Box::new(crate::dialogs::QuickStepForm::new(s)))));
        ui.close();
    }
    if !s.prefs.quick_steps.is_empty() {
        ui.menu_button(tl!("Edit a Quick Step"), |ui| {
            for q in &s.prefs.quick_steps {
                if ui.button(q.name.as_str()).clicked() {
                    acts.push(Action::Dialog(Dialog::QuickStep(Box::new(crate::dialogs::QuickStepForm::edit(s, q)))));
                    ui.close();
                }
            }
        });
    }
}

/// The Junk menu: block the selection's senders, or never block them (or their domain), and
/// the lists themselves (File › Options).
pub(crate) fn junk_menu(ui: &mut egui::Ui, s: &Session, acts: &mut Vec<Action>) {
    let has = !s.selection.is_empty();
    for (id, label, tip) in [
        ("junk.blockSender", "Block sender", "Their mail goes to Junk Email from now on"),
        ("junk.neverBlockSender", "Never block sender", "Their mail is never treated as junk"),
        ("junk.neverBlockDomain", "Never block sender's domain", "Mail from anyone at their domain is never treated as junk"),
    ] {
        if ui.add_enabled(has, egui::Button::new(tl!(label))).on_hover_text(tl!(tip)).clicked() {
            acts.push(Action::Run(id, json!({})));
            ui.close();
        }
    }
    ui.separator();
    if ui.button(tl!("Junk Email options…")).clicked() {
        acts.push(Action::Backstage(backstage::Page::Options));
        ui.close();
    }
}
