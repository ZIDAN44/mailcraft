//! The ribbon of Calendar, Tasks, Notes and People: one row of their buttons.

use crate::*;

impl MailApp {
    /// Calendar's: New event, Today, the views, and its calendars side by side.
    pub(super) fn ribbon_calendar(&mut self, ui: &mut Ui, acts: &mut Vec<Action>) {
        let t = self.ui.tokens;
        let s = &self.session;
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
    }

    /// Tasks': New task, the one picked opened, marked or deleted, and how they're listed.
    pub(super) fn ribbon_tasks(&mut self, ui: &mut Ui, acts: &mut Vec<Action>) {
        let t = self.ui.tokens;
        let s = &self.session;
        if widgets::accent_button(ui, Icon::Task, "New task", &t, !s.accounts.is_empty()).clicked() {
            acts.push(Action::Dialog(Dialog::new_task(s, None)));
        }
        widgets::group_sep(ui, &t);
        // The task picked in the list.
        let picked = self.ui.task.as_deref().and_then(|u| s.find_task(u));
        let done = picked.as_ref().is_some_and(|r| r.task.is_done());
        if widgets::tool_button(ui, Icon::Drafts, "Open", &t, picked.is_some()).clicked()
            && let Some(d) = picked.as_ref().and_then(|r| Dialog::open_task(s, &r.task.uid))
        {
            acts.push(Action::Dialog(d));
        }
        let mark = if done { "Mark not complete" } else { "Mark complete" };
        if widgets::tool_button(ui, Icon::Task, mark, &t, picked.is_some()).clicked()
            && let Some(r) = &picked
        {
            acts.push(Action::Run("task.complete", json!({"uid": r.task.uid, "done": !done})));
        }
        if widgets::tool_button(ui, Icon::Delete, "Delete", &t, picked.is_some()).clicked()
            && let Some(r) = &picked
        {
            acts.push(Action::Run("task.delete", json!({"uid": r.task.uid})));
        }
        // How they're listed, and whether the done ones are.
        widgets::group_sep(ui, &t);
        let r = widgets::tool_button(ui, Icon::ListBullet, "Arrange by", &t, true);
        egui::Popup::menu(&r).show(|ui| {
            use mailcraft_engine::TasksOrder as O;
            for (order, label) in [(O::Due, "Due date"), (O::Priority, "Priority"), (O::Title, "Title (A to Z)"), (O::Modified, "Last changed")] {
                if ui.radio(s.prefs.tasks_order == order, tl!(label)).clicked() {
                    acts.push(Action::Run("task.arrange", json!({"by": order.code()})));
                    ui.close();
                }
            }
        });
        if widgets::tool_toggle(ui, Icon::Funnel, "Hide completed", &t, s.prefs.tasks_hide_done).clicked() {
            acts.push(Action::Run("task.hideDone", json!({})));
        }
    }

    /// Notes': New note, the one being written deleted or coloured, and how they're listed.
    pub(super) fn ribbon_notes(&mut self, ui: &mut Ui, acts: &mut Vec<Action>) {
        let t = self.ui.tokens;
        let s = &self.session;
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
    }

    /// People's: New contact, the one picked edited or deleted, and how they're shown.
    pub(super) fn ribbon_people(&mut self, ui: &mut Ui, acts: &mut Vec<Action>) {
        let t = self.ui.tokens;
        let s = &self.session;
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
            acts.push(Action::Dialog(Dialog::edit_contact(p.card)));
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
}
