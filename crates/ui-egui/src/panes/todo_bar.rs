//! The To-Do Bar beside the mail (Outlook's View › To-Do Bar): a month to go to a day from,
//! what's coming up in the calendars, and the tasks and flagged mail by when they're due. Each
//! part is shown or closed on its own.

use egui::{Margin, Rect, RichText, Sense, Ui, pos2, vec2};
use mailcraft_engine::model::dates;
use mailcraft_engine::{AgendaItem, Session, TodoGroup, TodoItem, TodoWhat};
use serde_json::json;

use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};
use crate::{Action, Dialog, UiState, widgets};

/// How far ahead the appointments look.
const DAYS_AHEAD: i64 = 7;
/// Appointments listed at most: every one of the week's (issue #163: they stopped at 8, today's
/// and tomorrow's on a busy calendar), the part scrolling; a bound all the same.
const APPOINTMENTS: usize = 500;
const ROW: f32 = 26.0;
/// How soon an appointment starts for its countdown to show (issue #195): within the hour, as
/// Outlook for Mac's.
const SOON: i64 = 3600;

/// The bar as a pane at the window's end (its left in a right-to-left interface), when a part of
/// it is on and the window has room for it beside the `least` the mail needs. Dragged wider or
/// narrower, its width is kept (`view.todoBar`).
pub fn panel(s: &mut Session, st: &mut UiState, ui: &mut Ui, least: f32, acts: &mut Vec<Action>) {
    let bar = s.prefs.todo_bar;
    let (narrowest, widest) = (f32::from(*mailcraft_engine::TODO_BAR_WIDTH.start()), f32::from(*mailcraft_engine::TODO_BAR_WIDTH.end()));
    let room = ui.available_width() - least;
    if !bar.shown() || room < narrowest {
        return;
    }
    let t = st.tokens;
    let side = if crate::i18n::current().rtl() { egui::Panel::left("todo-bar") } else { egui::Panel::right("todo-bar") };
    let shown = side
        .resizable(true)
        .default_size(f32::from(bar.width).min(room))
        .size_range(narrowest..=widest.min(room))
        .frame(egui::Frame::new().fill(t.chrome))
        .show(ui, |ui| show(s, st, ui, acts));
    st.panes.push((crate::Pane::TodoBar, shown.response.rect));
    // Its width as dragged, kept once the button is up.
    let width = shown.response.rect.width().round().clamp(narrowest, widest);
    if (width - f32::from(bar.width)).abs() >= 1.0 && width < room && !ui.input(|i| i.pointer.any_down()) {
        acts.push(Action::Run("view.todoBar", json!({"width": width})));
    }
}

/// The bar: its parts, top to bottom. The task list takes the room that's left.
fn show(s: &mut Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    let t = st.tokens;
    let bar = s.prefs.todo_bar;
    // The flagged mail of every folder, a folder a frame until they're all read.
    if bar.tasks && s.todo_step() {
        ui.ctx().request_repaint();
    }
    egui::Frame::new().inner_margin(Margin::symmetric(8, 6)).show(ui, |ui| {
        if bar.calendar {
            head(ui, &t, "Calendar", "calendar", acts);
            super::calendar::bar_month(s, st, ui, acts);
            ui.add_space(6.0);
        }
        if bar.appointments {
            head(ui, &t, "Appointments", "appointments", acts);
            if bar.tasks {
                // Its share of what's left, the tasks under the line having the rest, each with a
                // scroll bar of its own; the line is dragged to share it otherwise (issue #196: it
                // was half, always).
                let (top, room) = (ui.cursor().top(), (ui.available_height() - TASKS_HEAD - SPLIT_LINE).max(ROW * 4.0));
                let share = st.todo_split.unwrap_or(f32::from(bar.split) / 100.0);
                let h = (room * share).max(ROW * 2.0);
                egui::ScrollArea::vertical().id_salt("todo-bar-appointments").auto_shrink(false).max_height(h).show(ui, |ui| {
                    appointments(s, &t, st.todo_day, ui, acts);
                });
                split_line(ui, &t, (top, room), bar.split, st, acts);
            } else {
                egui::ScrollArea::vertical().id_salt("todo-bar-appointments").max_height(ui.available_height()).show(ui, |ui| {
                    ui.set_min_width(ui.available_width());
                    appointments(s, &t, st.todo_day, ui, acts);
                });
                ui.add_space(6.0);
            }
        }
        if bar.tasks {
            head(ui, &t, "Tasks", "tasks", acts);
            tasks(s, st, ui, acts);
        }
    });
}

/// The tasks' heading's height, under the line.
const TASKS_HEAD: f32 = 22.0;
/// The line between the appointments and the tasks, and the room round it that takes the pointer.
const SPLIT_LINE: f32 = 10.0;

/// The line between the appointments and the tasks (issue #196), as Outlook's: dragged up or
/// down, it gives either more room, and stays where it's let go (`view.todoBar`'s `split`, `kept`
/// now); with the keyboard, Up and Down move it. `top` is where the appointments start, `room`
/// the height the two share.
fn split_line(ui: &mut Ui, t: &Tokens, (top, room): (f32, f32), kept: u8, st: &mut UiState, acts: &mut Vec<Action>) {
    let (least, most) = (f32::from(*mailcraft_engine::TODO_BAR_SPLIT.start()) / 100.0, f32::from(*mailcraft_engine::TODO_BAR_SPLIT.end()) / 100.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), SPLIT_LINE), Sense::click_and_drag());
    widgets::named(ui, &resp, tl!("Line between the appointments and the tasks"), true, None);
    // A click puts the keyboard on it, for Up and Down.
    if resp.clicked() || resp.drag_started() {
        resp.request_focus();
    }
    let active = resp.hovered() || resp.dragged() || resp.has_focus();
    if active {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
    }
    let y = rect.center().y;
    let stroke = egui::Stroke::new(if active { 2.0 } else { 1.0 }, if active { t.accent } else { t.border });
    ui.painter().line_segment([pos2(rect.min.x, y), pos2(rect.max.x, y)], stroke);
    let kept = f32::from(kept) / 100.0;
    let moved = if resp.dragged()
        && let Some(p) = resp.interact_pointer_pos()
    {
        Some(((p.y - top) / room.max(1.0)).clamp(least, most))
    } else if resp.has_focus() {
        let step = ui.input(|i| i32::from(i.key_pressed(egui::Key::ArrowDown)) - i32::from(i.key_pressed(egui::Key::ArrowUp)));
        (step != 0).then(|| (st.todo_split.unwrap_or(kept) + 0.05 * step as f32).clamp(least, most))
    } else {
        None
    };
    if let Some(share) = moved {
        st.todo_split = Some(share);
    }
    // Kept once let go, or at once from the keyboard.
    if (resp.drag_stopped() || (moved.is_some() && !resp.dragged()))
        && let Some(share) = st.todo_split.take()
    {
        acts.push(Action::Run("view.todoBar", json!({"split": (share * 100.0).round() as u64})));
    }
}

/// A part's name, and the ✕ that closes it.
fn head(ui: &mut Ui, t: &Tokens, name: &str, part: &str, acts: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(tl!(name)).font(theme::bold(12.0)).color(t.dim));
        ui.with_layout(crate::bidi::row_from_end(egui::Align::Center), |ui| {
            if widgets::tiny_icon_button(ui, Icon::Close, "Close", t).clicked() {
                acts.push(Action::Run("view.todoBar", json!({"part": part, "on": false})));
            }
        });
    });
}

/// What's coming up: the next week's events not yet over, under their days; or the events of the
/// day picked in the bar's month (`picked`, issue #154), all of them, over or not.
fn appointments(s: &Session, t: &Tokens, picked: Option<i64>, ui: &mut Ui, acts: &mut Vec<Action>) {
    let today = picked.unwrap_or_else(|| dates::today(&s.clock));
    let (days, now) = if picked.is_some() { (1, i64::MIN) } else { (DAYS_AHEAD, s.clock.now) };
    let coming: Vec<AgendaItem> = s
        .todo_appointments(s.local_midnight(today), s.local_midnight(today + days))
        .into_iter()
        .filter(|it| !it.task && !it.busy_only && it.end > now && it.status != "CANCELLED")
        .collect();
    if coming.is_empty() {
        let none = if picked.is_some() { tl!("Nothing on this day") } else { tl!("Nothing in the next 7 days") };
        if let Some(day) = picked {
            ui.label(RichText::new(super::tasks::relative_day(day, s)).font(theme::bold(11.5)).color(t.accent));
        }
        ui.label(RichText::new(none).color(t.faint).small());
        return;
    }
    // By the day each is on from here: one begun earlier is on the first day shown.
    let day_of = |it: &AgendaItem| s.clock.local(it.start).div_euclid(86_400).max(today);
    let mut by_day: Vec<&AgendaItem> = coming.iter().collect();
    by_day.sort_by_key(|it| (day_of(it), !it.all_day, it.start));
    let mut last = None;
    for it in by_day.into_iter().take(APPOINTMENTS) {
        let day = day_of(it);
        if last != Some(day) {
            last = Some(day);
            ui.label(RichText::new(super::tasks::relative_day(day, s)).font(theme::bold(11.5)).color(t.accent));
        }
        let when = if it.all_day { tl!("All day").to_string() } else { super::calendar::hm(it.start, s) };
        // How long it is under its time, and how soon it starts when that's within the hour
        // (issue #195), as Outlook for Mac's day list.
        let length = (!it.all_day).then(|| length_text(it.end - it.start));
        let soon = starts_soon(it, s.clock.now);
        if soon.is_some() {
            // Counted down a minute at a time.
            ui.ctx().request_repaint_after(std::time::Duration::from_secs(u64::try_from(60 - s.clock.now.rem_euclid(60)).unwrap_or(60)));
        }
        // The countdown on a line of its own above the time, as Outlook for Mac's (its comment on
        // issue #195): at the row's end it took the title's room, and hid the end of it.
        let above = if soon.is_some() { 15.0 } else { 0.0 };
        let h = above + if it.location.trim().is_empty() && length.is_none() { ROW } else { ROW + 14.0 };
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(rect, 4.0, t.hover_row);
        }
        let mr = crate::bidi::Mirror::new(rect);
        // Its calendar's colour down its start edge, as the calendar draws it.
        let stripe = mr.rect(Rect::from_min_size(rect.min + vec2(1.0, 3.0), vec2(3.0, h - 6.0)));
        ui.painter().rect_filled(stripe, 1.5, it.category.map_or_else(|| super::calendar::color(it.color, t), |i| theme::category_color(i, t.dark)));
        let p = ui.painter().with_clip_rect(rect);
        let time = widgets::one_line(ui, &when, theme::regular(11.5), t.dim, 70.0);
        let laid = length.as_ref().map(|l| widgets::one_line(ui, l, theme::regular(11.0), t.faint, 70.0));
        // The time's column as wide as the wider of its time and its length (`15 min` ran into
        // the place beside it).
        let time_w = time.size().x.max(laid.as_ref().map_or(0.0, |l| l.size().x));
        let top = rect.min.y + 5.0 + above;
        p.galley(pos2(mr.start(rect.min.x + 10.0, time.size().x), top + 1.0), time, t.dim);
        if let Some(l) = laid {
            p.galley(pos2(mr.start(rect.min.x + 10.0, l.size().x), top + 18.0), l, t.faint);
        }
        let room = (rect.width() - 22.0 - time_w).max(10.0);
        if let Some(soon) = &soon {
            let badge = widgets::one_line(ui, soon, theme::bold(11.0), t.accent, rect.width() - 16.0);
            p.galley(pos2(mr.start(rect.min.x + 10.0, badge.size().x), rect.min.y + 4.0), badge, t.accent);
        }
        let title = if it.summary.trim().is_empty() { tl!("(no title)").to_string() } else { it.summary.clone() };
        let title = widgets::one_line(ui, &title, theme::regular(13.0), t.text, room);
        p.galley(pos2(mr.start(rect.min.x + 18.0 + time_w, title.size().x), top), title, t.text);
        if !it.location.trim().is_empty() {
            let place = widgets::one_line(ui, it.location.trim(), theme::regular(11.5), t.faint, room);
            p.galley(pos2(mr.start(rect.min.x + 18.0 + time_w, place.size().x), top + 17.0), place, t.faint);
        }
        // Read as it's shown: its time, how long and how soon, then what it is.
        let heard = [Some(when.clone()), length.clone(), soon.clone(), Some(it.summary.clone())].into_iter().flatten().collect::<Vec<_>>().join(", ");
        widgets::named(ui, &resp, &heard, true, None);
        if resp.clicked()
            && let Some(d) = Dialog::open_event(s, &it.uid, it.occurrence)
        {
            acts.push(Action::Dialog(d));
        }
    }
}

/// How soon an event starts, when it does within [`SOON`] of `now`: `In 5 min`.
fn starts_soon(it: &AgendaItem, now: i64) -> Option<String> {
    let ahead = it.start - now;
    (!it.all_day && ahead > 0 && ahead <= SOON).then(|| tf!("In {n} min", n = (ahead + 59) / 60))
}

/// An event's length as the day list shows it: `30 min`, `1 h`, `1 h 30 min` (to the minute).
fn length_text(secs: i64) -> String {
    let minutes = (secs.max(0) + 30) / 60;
    match (minutes / 60, minutes % 60) {
        (0, m) => tf!("{n} min", n = m),
        (h, 0) => tf!("{h} h", h = h),
        (h, m) => tf!("{h} h {m} min", h = h, m = m),
    }
}

/// The box to type a new task in (due today, as Outlook's), then the tasks and the flagged mail
/// under when they're due.
fn tasks(s: &Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    let t = st.tokens;
    let can_add = !s.accounts.is_empty();
    let typed = ui.add_enabled(
        can_add,
        egui::TextEdit::singleline(&mut st.todo_new).hint_text(tl!("Type a new task")).desired_width(f32::INFINITY).font(theme::regular(13.0)),
    );
    if typed.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !st.todo_new.trim().is_empty() {
        acts.push(Action::Run("task.create", json!({"title": st.todo_new.trim(), "due": dates::day_string(dates::today(&s.clock))})));
        st.todo_new.clear();
        typed.request_focus();
    }
    ui.add_space(4.0);
    let items = s.todo_items();
    if items.is_empty() {
        ui.label(RichText::new(tl!("Nothing to do")).color(t.faint).small());
        return;
    }
    egui::ScrollArea::vertical().auto_shrink(false).id_salt("todo-bar-tasks").show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let mut group: Option<TodoGroup> = None;
        for it in &items {
            if group != Some(it.group) {
                group = Some(it.group);
                ui.add_space(4.0);
                let color = if it.group == TodoGroup::Overdue { t.danger } else { t.accent };
                ui.label(RichText::new(tl!(it.group.label())).font(theme::bold(11.5)).color(color));
                ui.add_space(2.0);
            }
            row(s, &t, ui, it, acts);
        }
    });
}

/// One thing to do: what it is, its name, and the mark that says it's done (a task's circle, a
/// message's flag).
fn row(s: &Session, t: &Tokens, ui: &mut Ui, it: &TodoItem, acts: &mut Vec<Action>) {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 4.0, t.hover_row);
    }
    let mr = crate::bidi::Mirror::new(rect);
    let mail = matches!(it.what, TodoWhat::Mail { .. });
    // Done: at the end of the row, where Outlook's flag is.
    let mark = mr.rect(Rect::from_center_size(pos2(rect.max.x - 13.0, rect.center().y), vec2(18.0, 18.0)));
    let id = match &it.what {
        TodoWhat::Task { uid } => ui.scope_id().with(("todo-done", uid)),
        TodoWhat::Mail { key } => ui.scope_id().with(("todo-done", key.to_id())),
    };
    let tick = ui.interact(mark.expand(3.0), id, Sense::click());
    if mail {
        icons::paint(ui.painter(), mark.shrink(1.0), Icon::Flag, if tick.hovered() { t.accent } else { t.danger });
    } else {
        let ring = if tick.hovered() { t.accent } else { t.dim };
        ui.painter().circle_stroke(mark.center(), 8.0, egui::Stroke::new(1.5, ring));
        if tick.hovered() {
            let c = mark.center();
            ui.painter().line(vec![c + vec2(-3.5, 0.0), c + vec2(-1.0, 2.5), c + vec2(4.0, -3.0)], egui::Stroke::new(1.6, t.accent));
        }
    }
    let title = if it.title.trim().is_empty() { tl!(if mail { "(no subject)" } else { "(no title)" }).to_string() } else { it.title.clone() };
    widgets::named(ui, &tick, &format!("{}: {title}", tl!("Mark complete")), true, None);
    if tick.on_hover_text(tl!("Mark complete")).clicked() {
        acts.push(match &it.what {
            TodoWhat::Task { uid } => Action::Run("task.complete", json!({"uid": uid})),
            TodoWhat::Mail { key } => Action::Run("todo.complete", json!({"key": key.to_id()})),
        });
        return;
    }
    // What it is, at the start: a task, or a message.
    let kind = mr.rect(Rect::from_center_size(pos2(rect.min.x + 11.0, rect.center().y), vec2(14.0, 14.0)));
    icons::paint(ui.painter(), kind, if mail { Icon::Read } else { Icon::Task }, t.dim);
    let room = (rect.width() - 50.0).max(10.0);
    let galley = widgets::one_line(ui, &title, theme::regular(13.0), t.text, room);
    let at = pos2(mr.start(rect.min.x + 24.0, galley.size().x), rect.center().y - galley.size().y / 2.0);
    ui.painter().with_clip_rect(rect).galley(at, galley, t.text);
    widgets::named(ui, &resp, &title, true, None);
    if resp.clicked() {
        open(s, it, acts);
    }
    resp.context_menu(|ui| menu(s, t, ui, it, acts));
}

/// Open a row's task (its form) or message (a window of its own).
fn open(s: &Session, it: &TodoItem, acts: &mut Vec<Action>) {
    match &it.what {
        TodoWhat::Task { uid } => {
            if let Some(d) = Dialog::open_task(s, uid) {
                acts.push(Action::Dialog(d));
            }
        }
        TodoWhat::Mail { key } => acts.push(Action::Run("todo.open", json!({"key": key.to_id()}))),
    }
}

/// A row's menu, as Outlook's To-Do Bar has it (issue #160): Open; Follow Up by a day, none or
/// one picked (a task's form, for a task); Mark Complete; and Clear Flag for mail, or a task's
/// categories and Delete.
fn menu(s: &Session, t: &Tokens, ui: &mut Ui, it: &TodoItem, acts: &mut Vec<Action>) {
    if ui.button(tl!("Open")).clicked() {
        open(s, it, acts);
        ui.close();
    }
    ui.menu_button(tl!("Follow up"), |ui| {
        for f in dates::FollowUp::ALL {
            let day = f.due(&s.clock);
            let when = crate::i18n::date(&dates::day_text(day, &s.clock));
            let clicked = ui
                .horizontal(|ui| {
                    let r = ui.selectable_label(it.due == Some(day), tl!(f.label()));
                    ui.label(RichText::new(when).color(t.faint).small());
                    r.clicked()
                })
                .inner;
            if clicked {
                acts.push(follow_up(it, Some(day)));
                ui.close();
            }
        }
        if ui.selectable_label(it.due.is_none(), tl!("No date")).clicked() {
            acts.push(follow_up(it, None));
            ui.close();
        }
        if ui.selectable_label(false, tl!("Custom…")).clicked() {
            match &it.what {
                TodoWhat::Mail { key } => {
                    if let Some(f) = crate::pim_dialogs::FollowUpForm::new(s, key) {
                        acts.push(Action::Dialog(Dialog::FollowUp(Box::new(f))));
                    }
                }
                TodoWhat::Task { .. } => open(s, it, acts),
            }
            ui.close();
        }
        // A flag's reminder (gap 9); a task's is in its form.
        if let TodoWhat::Mail { key } = &it.what
            && ui.selectable_label(false, tl!("Add reminder…")).clicked()
        {
            if let Some(mut f) = crate::pim_dialogs::FollowUpForm::new(s, key) {
                f.remind = true;
                acts.push(Action::Dialog(Dialog::FollowUp(Box::new(f))));
            }
            ui.close();
        }
    });
    ui.separator();
    if ui.button(tl!("Mark complete")).clicked() {
        acts.push(match &it.what {
            TodoWhat::Task { uid } => Action::Run("task.complete", json!({"uid": uid})),
            TodoWhat::Mail { key } => Action::Run("todo.flag", json!({"key": key.to_id(), "complete": true})),
        });
        ui.close();
    }
    match &it.what {
        TodoWhat::Mail { key } => {
            if ui.button(tl!("Clear flag")).clicked() {
                acts.push(Action::Run("todo.flag", json!({"key": key.to_id()})));
                ui.close();
            }
        }
        TodoWhat::Task { uid } => {
            let chosen = s.find_task(uid).map(|r| r.task.categories).unwrap_or_default();
            if !s.prefs.categories.is_empty() {
                ui.menu_button(tl!("Categorize"), |ui| {
                    for c in &s.prefs.categories {
                        let on = chosen.iter().any(|x| x.eq_ignore_ascii_case(&c.name));
                        let clicked = ui
                            .horizontal(|ui| {
                                crate::panes::organize::dot(ui, c.color, t.dark);
                                ui.selectable_label(on, tl!(&c.name)).clicked()
                            })
                            .inner;
                        if clicked {
                            let mut now: Vec<String> = chosen.iter().filter(|x| !x.eq_ignore_ascii_case(&c.name)).cloned().collect();
                            if !on {
                                now.push(c.name.clone());
                            }
                            acts.push(Action::Run("task.update", json!({"uid": uid, "categories": now})));
                            ui.close();
                        }
                    }
                });
            }
            ui.separator();
            if ui.button(tl!("Delete")).clicked() {
                acts.push(Action::Run("task.delete", json!({"uid": uid})));
                ui.close();
            }
        }
    }
}

/// Following a row up by `day` (`None`: no date): a message's flag, a task's due day.
fn follow_up(it: &TodoItem, day: Option<i64>) -> Action {
    match &it.what {
        TodoWhat::Mail { key } => {
            Action::Run("todo.flag", json!({"key": key.to_id(), "due": day.map_or_else(|| "none".to_string(), dates::day_string)}))
        }
        TodoWhat::Task { uid } => Action::Run("task.update", json!({"uid": uid, "due": day.map(dates::day_string).unwrap_or_default()})),
    }
}
