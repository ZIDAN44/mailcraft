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

/// How far ahead the appointments look, and how many are listed.
const DAYS_AHEAD: i64 = 7;
const APPOINTMENTS: usize = 8;
const ROW: f32 = 26.0;

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
            // Half of what's left when the tasks are under it (they keep the other half), with a
            // scroll bar of its own when there's more coming up than fits.
            let most = if bar.tasks { (ui.available_height() / 2.0).max(ROW * 2.0) } else { ui.available_height() };
            egui::ScrollArea::vertical().id_salt("todo-bar-appointments").max_height(most).show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                appointments(s, &t, ui, acts);
            });
            ui.add_space(6.0);
        }
        if bar.tasks {
            head(ui, &t, "Tasks", "tasks", acts);
            tasks(s, st, ui, acts);
        }
    });
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

/// A day as the lists name it: Today, Tomorrow, else its date.
fn day_name(day: i64, s: &Session) -> String {
    super::tasks::due_label(day.saturating_mul(86_400), s)
}

/// What's coming up: the next week's events not yet over, under their days.
fn appointments(s: &Session, t: &Tokens, ui: &mut Ui, acts: &mut Vec<Action>) {
    let today = dates::today(&s.clock);
    let now = s.clock.now;
    let coming: Vec<AgendaItem> = s
        .agenda(s.local_midnight(today), s.local_midnight(today + DAYS_AHEAD))
        .into_iter()
        .filter(|it| !it.task && it.end > now && it.status != "CANCELLED")
        .collect();
    if coming.is_empty() {
        ui.label(RichText::new(tl!("Nothing in the next 7 days")).color(t.faint).small());
        return;
    }
    // By the day each is on from here: one begun earlier is on today.
    let day_of = |it: &AgendaItem| s.clock.local(it.start).div_euclid(86_400).max(today);
    let mut by_day: Vec<&AgendaItem> = coming.iter().collect();
    by_day.sort_by_key(|it| (day_of(it), !it.all_day, it.start));
    let mut last = None;
    for it in by_day.into_iter().take(APPOINTMENTS) {
        let day = day_of(it);
        if last != Some(day) {
            last = Some(day);
            ui.label(RichText::new(day_name(day, s)).font(theme::bold(11.5)).color(t.accent));
        }
        let when = if it.all_day { tl!("All day").to_string() } else { super::calendar::hm(it.start, s) };
        let h = if it.location.trim().is_empty() { ROW } else { ROW + 14.0 };
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
        let time_w = time.size().x;
        let top = rect.min.y + 5.0;
        p.galley(pos2(mr.start(rect.min.x + 10.0, time_w), top + 1.0), time, t.dim);
        let room = (rect.width() - 22.0 - time_w).max(10.0);
        let title = if it.summary.trim().is_empty() { tl!("(no title)").to_string() } else { it.summary.clone() };
        let title = widgets::one_line(ui, &title, theme::regular(13.0), t.text, room);
        p.galley(pos2(mr.start(rect.min.x + 18.0 + time_w, title.size().x), top), title, t.text);
        if !it.location.trim().is_empty() {
            let place = widgets::one_line(ui, it.location.trim(), theme::regular(11.5), t.faint, room);
            p.galley(pos2(mr.start(rect.min.x + 18.0 + time_w, place.size().x), top + 17.0), place, t.faint);
        }
        widgets::named(ui, &resp, &format!("{when} {}", it.summary), true, None);
        if resp.clicked()
            && let Some(d) = Dialog::open_event(s, &it.uid, it.occurrence)
        {
            acts.push(Action::Dialog(d));
        }
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
        let (y, m, d) = dates::civil_from_days(dates::today(&s.clock));
        acts.push(Action::Run("task.create", json!({"title": st.todo_new.trim(), "due": format!("{y:04}-{m:02}-{d:02}")})));
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
        match &it.what {
            TodoWhat::Task { uid } => {
                if let Some(d) = Dialog::open_task(s, uid) {
                    acts.push(Action::Dialog(d));
                }
            }
            TodoWhat::Mail { key } => acts.push(Action::Run("todo.open", json!({"key": key.to_id()}))),
        }
    }
}
