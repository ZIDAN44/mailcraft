//! The ribbon of Calendar, Tasks, Notes and People: one row of their buttons, Home's and View's
//! (how the module shows what it holds, which Home has too, as Outlook's).

use crate::*;

impl MailApp {
    /// Calendar's: New event, Today, a time free for everyone, the views, and its calendars side
    /// by side.
    pub(super) fn ribbon_calendar(&mut self, ui: &mut Ui, acts: &mut Vec<Action>) {
        let t = self.ui.tokens;
        let s = &self.session;
        if widgets::accent_button(ui, Icon::Calendar, "New event", &t, !s.accounts.is_empty()).clicked() {
            acts.push(Action::Dialog(Dialog::new_event(s, None)));
        }
        widgets::group_sep(ui, &t);
        free_time(ui, s, t, acts);
        // Today and the views are over the calendar (and the views on View): twice, in two
        // orders, they were 70 points apart.
        side_by_side(ui, s, t, acts);
    }

    /// The View tab of the module shown: how it shows what it holds, and the folder pane.
    pub(super) fn ribbon_module_view(&mut self, ui: &mut Ui, acts: &mut Vec<Action>) {
        let t = self.ui.tokens;
        let s = &self.session;
        match s.module {
            Module::Calendar => calendar_views(ui, s, t, acts),
            Module::Tasks => tasks_arrangement(ui, s, t, acts),
            Module::Notes => notes_arrangement(ui, s, t, acts),
            _ => people_views(ui, &mut self.ui, t),
        }
        widgets::group_sep(ui, &t);
        if widgets::tool_toggle(ui, Icon::Folder, "Folder pane", &t, !s.prefs.folder_pane_minimized).clicked() {
            acts.push(Action::Run("view.folderPane", json!({})));
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
        widgets::group_sep(ui, &t);
        tasks_arrangement(ui, s, t, acts);
    }

    /// Notes': New note, the one being written deleted or coloured, and how they're listed.
    pub(super) fn ribbon_notes(&mut self, ui: &mut Ui, acts: &mut Vec<Action>) {
        let t = self.ui.tokens;
        let s = &self.session;
        if widgets::accent_button(ui, Icon::Note, "New note", &t, !s.accounts.is_empty()).clicked() {
            notes::new_note(&mut self.ui, acts);
        }
        widgets::group_sep(ui, &t);
        // The note written on, else the one picked.
        let editing = self.ui.note_edit.as_ref().map(|(u, _)| u.clone()).or_else(|| self.ui.note_picked.clone());
        if widgets::tool_button(ui, Icon::Delete, "Delete", &t, editing.is_some()).clicked()
            && let Some(uid) = editing.clone()
        {
            self.ui.note_edit = None;
            acts.push(Action::Run("note.delete", json!({"uid": uid})));
        }
        // The note's colour, as Outlook's Categorize on a note.
        let r = widgets::menu_tool_button(ui, Icon::Tag, "Color", &t, editing.is_some());
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
        widgets::group_sep(ui, &t);
        notes_arrangement(ui, s, t, acts);
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
        widgets::group_sep(ui, &t);
        people_views(ui, &mut self.ui, t);
    }
}

/// A time when everyone shown is free (issue #186), as Outlook's AutoPick finds one: how long,
/// and the one before or next.
fn free_time(ui: &mut Ui, s: &Session, t: Tokens, acts: &mut Vec<Action>) {
    let label = |m: u16| crate::pim_dialogs::reminder_label(Some(i64::from(m)));
    let length = label(s.prefs.free_time_minutes);
    let r = widgets::menu_tool_button(ui, Icon::Clock, widgets::Label::shown("Meeting length", &length), &t, true)
        .on_hover_text(tl!("How long a time free for everyone is looked for"));
    egui::Popup::menu(&r).show(|ui| {
        for m in mailcraft_engine::FREE_LENGTHS {
            if ui.selectable_label(s.prefs.free_time_minutes == m, label(m)).clicked() {
                acts.push(Action::Run("calendar.freeLength", json!({"minutes": m})));
                ui.close();
            }
        }
    });
    let any = !s.shown_calendars().is_empty();
    let r = widgets::tool_button(ui, Icon::ChevronLeft, "Previous free time", &t, any);
    if r.on_hover_text(tl!("The time before when everyone shown is free, within work hours")).clicked() {
        acts.push(Action::Run("calendar.findFree", json!({"back": true})));
    }
    let r = widgets::tool_button(ui, Icon::ChevronRight, "Next free time", &t, any);
    if r.on_hover_text(tl!("The next time when everyone shown is free, within work hours")).clicked() {
        acts.push(Action::Run("calendar.findFree", json!({})));
    }
}

/// The calendar's views (Day, Work week, Week, Month, Schedule), the one shown lit (it was drawn
/// off, as if it couldn't be used), each with its own icon; and its calendars side by side.
fn calendar_views(ui: &mut Ui, s: &Session, t: Tokens, acts: &mut Vec<Action>) {
    for (mode, icon, label, code) in [
        (CalMode::Day, Icon::ViewDay, "Day", "day"),
        (CalMode::WorkWeek, Icon::ViewWorkWeek, "Work week", "workWeek"),
        (CalMode::Week, Icon::ViewWeek, "Week", "week"),
        (CalMode::Month, Icon::ViewMonth, "Month", "month"),
        (CalMode::Schedule, Icon::ViewSchedule, "Schedule", "schedule"),
    ] {
        if widgets::tool_toggle(ui, icon, label, &t, s.cal_mode == mode).clicked() && s.cal_mode != mode {
            acts.push(Action::Run("calendar.view", json!({"mode": code})));
        }
    }
    side_by_side(ui, s, t, acts);
}

/// Each calendar in a column of its own (Outlook's side by side), or together.
fn side_by_side(ui: &mut Ui, s: &Session, t: Tokens, acts: &mut Vec<Action>) {
    if s.pim.values().map(|p| p.calendars.len()).sum::<usize>() > 1 {
        widgets::group_sep(ui, &t);
        if widgets::tool_toggle(ui, Icon::Calendar, "Side by side", &t, s.prefs.calendars_side_by_side).clicked() {
            acts.push(Action::Run("calendar.sideBySide", json!({})));
        }
    }
}

/// How tasks are listed, and whether the done ones are.
fn tasks_arrangement(ui: &mut Ui, s: &Session, t: Tokens, acts: &mut Vec<Action>) {
    let r = widgets::menu_tool_button(ui, Icon::ListBullet, "Arrange by", &t, true);
    egui::Popup::menu(&r).show(|ui| {
        use mailcraft_engine::TasksOrder as O;
        for (order, label) in [(O::Due, "Due date"), (O::Priority, "Priority"), (O::Title, "Title (A to Z)"), (O::Modified, "Last changed")] {
            if ui.radio(s.prefs.tasks_order == order, tl!(label)).clicked() {
                acts.push(Action::Run("task.arrange", json!({"by": order.code()})));
                ui.close();
            }
        }
    });
    // Hiding what's done is the side pane's Active tasks (this did the same).
}

/// How notes are listed: the last changed first, A to Z, or by colour.
fn notes_arrangement(ui: &mut Ui, s: &Session, t: Tokens, acts: &mut Vec<Action>) {
    let r = widgets::menu_tool_button(ui, Icon::ListBullet, "Arrange by", &t, true);
    egui::Popup::menu(&r).show(|ui| {
        use mailcraft_engine::NotesOrder as O;
        for (order, label, code) in [(O::Modified, "Last changed", "modified"), (O::Title, "Title (A to Z)", "title"), (O::Color, "Color", "color")] {
            if ui.radio(s.prefs.notes_order == order, tl!(label)).clicked() {
                acts.push(Action::Run("note.arrange", json!({"by": code})));
                ui.close();
            }
        }
    });
}

/// Outlook's Change View for people: the list with a card, business cards, a phone list; and the
/// phone list's columns (also from its header's right-click).
fn people_views(ui: &mut Ui, st: &mut UiState, t: Tokens) {
    for v in people::PeopleView::ALL {
        if widgets::tool_toggle(ui, v.icon(), v.label(), &t, st.people_view == v).clicked() {
            st.people_view = v;
        }
    }
    // Outlook's Sort by: first name or last name (issue #244).
    let r = widgets::menu_tool_button(ui, Icon::ListBullet, "Sort by", &t, true);
    egui::Popup::menu(&r).show(|ui| people::sort_menu(ui, &mut st.people_list));
    if st.people_view == people::PeopleView::Phone {
        let r = widgets::menu_tool_button(ui, Icon::Layout, "Columns", &t, true);
        egui::Popup::menu(&r).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| people::column_menu(ui, &mut st.people_list));
    }
}
