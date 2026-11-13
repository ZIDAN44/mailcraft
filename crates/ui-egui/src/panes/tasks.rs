//! Tasks, as Outlook's Tasks module: a list of what's to do, each with its done box, title,
//! due day, status and priority, the late ones in red and the done ones struck through, last.
//! A click on its box marks it done; a click on the row picks it, a double click opens it. The
//! columns' headings sort the list (Arrange by). Laid out from the left, mirrored for a
//! right-to-left interface. Tasks live on this computer (no server keeps them).

use egui::{Align2, Margin, Rect, RichText, Sense, Ui, pos2, vec2};
use mailcraft_engine::model::dates;
use mailcraft_engine::model::pim::{TaskPriority, TaskStatus};
use mailcraft_engine::{ListedTask, Session, TasksOrder};
use serde_json::json;

use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};
use crate::{Action, Dialog, UiState, widgets};

/// A row's height.
const ROW: f32 = 44.0;
/// The columns after the title: due, status, priority (their widths).
const DUE_W: f32 = 120.0;
const STATUS_W: f32 = 150.0;
const PRIORITY_W: f32 = 70.0;

/// The side pane: a new task, the view (every task, or those not done), how many there are.
pub fn nav(s: &Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    let t = st.tokens;
    if widgets::accent_button(ui, Icon::Task, "New task", &t, !s.accounts.is_empty()).clicked() {
        acts.push(Action::Dialog(Dialog::new_task(s, None)));
    }
    ui.add_space(10.0);
    ui.with_layout(crate::bidi::column(), |ui| {
        for (hide, label) in [(false, "All tasks"), (true, "Active tasks")] {
            if ui.selectable_label(s.prefs.tasks_hide_done == hide, tl!(label)).clicked() && s.prefs.tasks_hide_done != hide {
                acts.push(Action::Run("task.hideDone", json!({"on": hide})));
            }
        }
        ui.add_space(8.0);
        let list = s.tasks(&st.tasks_query);
        let today = dates::today(&s.clock);
        let late = list.iter().filter(|r| r.task.overdue(today)).count();
        ui.label(RichText::new(tf!("{n} tasks", n = list.len())).color(t.faint).small());
        if late > 0 {
            ui.label(RichText::new(tf!("{n} overdue", n = late)).color(t.danger).small());
        }
    });
}

/// When a task is due, as the list shows it: today, tomorrow, yesterday, else its date.
pub(crate) fn due_label(day: i64, s: &Session) -> String {
    let d = day.div_euclid(86_400);
    let today = dates::today(&s.clock);
    match d - today {
        0 => tl!("Today").to_string(),
        1 => tl!("Tomorrow").to_string(),
        -1 => tl!("Yesterday").to_string(),
        _ => {
            let (y, m, dd) = dates::civil_from_days(d);
            let (now_y, _, _) = dates::civil_from_days(today);
            let wd = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"].get((d + 3).rem_euclid(7) as usize).copied().unwrap_or("");
            let month =
                ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"].get(m as usize - 1).copied().unwrap_or("");
            if y == now_y { crate::i18n::date(&format!("{wd} {month} {dd}")) } else { format!("{y:04}-{m:02}-{dd:02}") }
        }
    }
}

/// The main pane: the columns' headings, then the tasks.
pub fn show(s: &Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    let t = st.tokens;
    let list = s.tasks(&st.tasks_query);
    if list.is_empty() {
        ui.centered_and_justified(|ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(ui.available_height() * 0.35);
                let (r, _) = ui.allocate_exact_size(vec2(56.0, 56.0), Sense::hover());
                icons::paint(ui.painter(), r, Icon::Task, t.border);
                ui.add_space(8.0);
                let empty = st.tasks_query.trim().is_empty() && !s.prefs.tasks_hide_done;
                ui.label(RichText::new(tl!(if empty { "No tasks yet: New task adds one." } else { "No task matches." })).color(t.faint));
            });
        });
        return;
    }
    egui::Frame::new().inner_margin(Margin::symmetric(14, 8)).show(ui, |ui| {
        headings(s, &t, ui, acts);
    });
    egui::ScrollArea::vertical().auto_shrink(false).id_salt("tasks").show(ui, |ui| {
        egui::Frame::new().inner_margin(Margin::symmetric(14, 0)).show(ui, |ui| {
            for r in &list {
                row(s, &t, st, ui, r, acts);
            }
            ui.add_space(10.0);
        });
    });
}

/// Where the columns start, from the row's left (as laid out left to right).
fn columns(width: f32) -> (f32, f32, f32, f32) {
    let title = 34.0;
    let priority = width - PRIORITY_W;
    let status = priority - STATUS_W;
    let due = status - DUE_W;
    (title, due.max(title + 80.0), status.max(title + 80.0), priority.max(title + 80.0))
}

/// The columns' headings: a click sorts by that column.
fn headings(s: &Session, t: &Tokens, ui: &mut Ui, acts: &mut Vec<Action>) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::hover());
    let mr = crate::bidi::Mirror::new(rect);
    let (title, due, status, priority) = columns(rect.width());
    for (x, w, label, order) in [
        (title, due - title, "Task subject", Some(TasksOrder::Title)),
        (due, DUE_W, "Due date", Some(TasksOrder::Due)),
        (status, STATUS_W, "Status", None),
        (priority, PRIORITY_W, "Priority", Some(TasksOrder::Priority)),
    ] {
        let r = mr.rect(Rect::from_min_size(pos2(rect.min.x + x, rect.min.y), vec2(w - 6.0, rect.height())));
        let resp = ui.interact(r, ui.scope_id().with(("task-heading", label)), Sense::click());
        let on = order == Some(s.prefs.tasks_order);
        let color = if on {
            t.accent
        } else if resp.hovered() {
            t.text
        } else {
            t.dim
        };
        let text = if on { format!("{} ▾", tl!(label)) } else { tl!(label).to_string() };
        ui.painter().with_clip_rect(r).text(
            mr.pos(pos2(rect.min.x + x, r.center().y)),
            mr.align(Align2::LEFT_CENTER),
            text,
            theme::bold(12.0),
            color,
        );
        if let Some(o) = order
            && resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
        {
            acts.push(Action::Run("task.arrange", json!({"by": o.code()})));
        }
    }
    ui.painter().line_segment([rect.left_bottom(), rect.right_bottom()], egui::Stroke::new(1.0, t.border));
}

/// One task: its done box, title (and its notes' first line or categories under it), due day,
/// status and priority.
fn row(s: &Session, t: &Tokens, st: &mut UiState, ui: &mut Ui, r: &ListedTask<'_>, acts: &mut Vec<Action>) {
    let task = r.task;
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
    let picked = st.task.as_deref() == Some(task.uid.as_str());
    if picked {
        ui.painter().rect_filled(rect, 4.0, t.accent_soft);
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, 4.0, t.hover_row);
    }
    let mr = crate::bidi::Mirror::new(rect);
    let start = mr.align(Align2::LEFT_CENTER);
    let done = task.is_done();
    let late = task.overdue(dates::today(&s.clock));
    // The done box: a circle, ticked when done.
    let boxr = mr.rect(Rect::from_center_size(pos2(rect.min.x + 14.0, rect.center().y), vec2(18.0, 18.0)));
    let tick = ui.interact(boxr.expand(4.0), ui.scope_id().with(("task-done", &task.uid)), Sense::click());
    let ring = if tick.hovered() || done { t.accent } else { t.dim };
    if done {
        ui.painter().circle_filled(boxr.center(), 9.0, t.accent);
    } else {
        ui.painter().circle_stroke(boxr.center(), 9.0, egui::Stroke::new(1.5, ring));
    }
    if done || tick.hovered() {
        let c = boxr.center();
        let ink = if done { t.on_accent } else { t.accent };
        let pts = [c + vec2(-4.0, 0.0), c + vec2(-1.0, 3.0), c + vec2(4.5, -3.0)];
        ui.painter().line(pts.to_vec(), egui::Stroke::new(1.6, ink));
    }
    let label = if done { tl!("Mark not complete") } else { tl!("Mark complete") };
    widgets::named(ui, &tick, &format!("{label}: {}", task.title), true, Some(done));
    if tick.on_hover_text(label).clicked() {
        acts.push(Action::Run("task.complete", json!({"uid": task.uid, "done": !done})));
    }
    let (x_title, x_due, x_status, x_priority) = columns(rect.width());
    let p = ui.painter().with_clip_rect(rect);
    // The title, struck through when done.
    let title = if task.title.trim().is_empty() { tl!("(no title)").to_string() } else { task.title.clone() };
    let (shown, title_rtl) = crate::bidi::shown(&title);
    let title_color = if done { t.faint } else { t.text };
    // Cut with "…" where it doesn't fit, as the list's subjects are (it stopped mid-letter at
    // the next column).
    let room = (x_due - x_title - 10.0).max(10.0);
    let galley = widgets::one_line(ui, &shown, theme::bold(13.0), title_color, room);
    let title_w = galley.size().x.min(room);
    let title_x = if title_rtl && !mr.on() { rect.min.x + x_title + room - title_w } else { rect.min.x + x_title };
    // Under it: that it repeats, its categories, its notes' first line.
    let mut under: Vec<String> = Vec::new();
    if task.rrule.is_some() {
        under.push(tl!("Repeats").to_string());
    }
    if !task.categories.is_empty() {
        under.push(task.categories.join(", "));
    }
    if let Some(line) = task.notes.lines().map(str::trim).find(|l| !l.is_empty()) {
        under.push(line.to_string());
    }
    // Alone, the title is in the middle of the row, in line with the columns.
    let top = if under.is_empty() { rect.center().y - galley.size().y / 2.0 } else { rect.min.y + 6.0 };
    let title_rect = mr.rect(Rect::from_min_size(pos2(title_x, top), vec2(title_w, galley.size().y)));
    let clip = mr.rect(Rect::from_min_size(pos2(rect.min.x + x_title, rect.min.y), vec2(room, ROW)));
    p.with_clip_rect(clip).galley(title_rect.min, galley, title_color);
    if done {
        let y = title_rect.center().y;
        p.with_clip_rect(clip).line_segment([pos2(title_rect.min.x, y), pos2(title_rect.max.x, y)], egui::Stroke::new(1.0, t.faint));
    }
    let under = widgets::one_line(ui, &under.join(" · "), theme::regular(11.5), t.dim, room);
    let under_at = start.anchor_size(mr.pos(pos2(rect.min.x + x_title, rect.min.y + 31.0)), under.size());
    p.with_clip_rect(clip).galley(under_at.min, under, t.dim);
    let due_text = task.due.map_or_else(|| tl!("None").to_string(), |d| due_label(d, s));
    let due_color = if late {
        t.danger
    } else if task.due.is_none() {
        t.faint
    } else {
        t.text
    };
    p.text(mr.pos(pos2(rect.min.x + x_due, rect.center().y)), start, due_text, theme::regular(12.5), due_color);
    let status = match task.status {
        TaskStatus::InProgress | TaskStatus::Waiting | TaskStatus::Deferred if task.percent > 0 => {
            format!("{} · {}%", tl!(crate::pim_dialogs::status_label(task.status)), task.percent)
        }
        other => tl!(crate::pim_dialogs::status_label(other)).to_string(),
    };
    let status_clip = mr.rect(Rect::from_min_size(pos2(rect.min.x + x_status, rect.min.y), vec2(STATUS_W - 8.0, ROW)));
    let status = widgets::one_line(ui, &status, theme::regular(12.5), t.dim, STATUS_W - 8.0);
    let status_at = start.anchor_size(mr.pos(pos2(rect.min.x + x_status, rect.center().y)), status.size());
    p.with_clip_rect(status_clip).galley(status_at.min, status, t.dim);
    if task.priority != TaskPriority::Normal {
        let high = task.priority == TaskPriority::High;
        let text = if high { "!" } else { "↓" };
        let color = if high { t.danger } else { t.dim };
        p.text(mr.pos(pos2(rect.min.x + x_priority + 6.0, rect.center().y)), start, text, theme::bold(15.0), color);
    }
    ui.painter().line_segment([rect.left_bottom(), rect.right_bottom()], egui::Stroke::new(1.0, t.border));
    widgets::named(ui, &resp, &title, true, Some(picked));
    if resp.clicked() {
        st.task = Some(task.uid.clone());
    }
    if resp.double_clicked()
        && let Some(d) = Dialog::open_task(s, &task.uid)
    {
        acts.push(Action::Dialog(d));
    }
    resp.context_menu(|ui| {
        if ui.button(tl!("Open")).clicked() {
            if let Some(d) = Dialog::open_task(s, &task.uid) {
                acts.push(Action::Dialog(d));
            }
            ui.close();
        }
        if ui.button(tl!(if done { "Mark not complete" } else { "Mark complete" })).clicked() {
            acts.push(Action::Run("task.complete", json!({"uid": task.uid, "done": !done})));
            ui.close();
        }
        ui.separator();
        if ui.button(RichText::new(tl!("Delete")).color(t.danger)).clicked() {
            acts.push(Action::Run("task.delete", json!({"uid": task.uid})));
            ui.close();
        }
    });
}
