//! Tasks, as Outlook's Tasks module: a list of what's to do, each with its done box, title,
//! due day, status and priority, the late ones in red and the done ones struck through, last.
//! A click on its box marks it done; a click on the row picks it, a double click opens it. The
//! columns' headings sort the list (Arrange by). Laid out from the left, mirrored for a
//! right-to-left interface. Tasks live on this computer, or on the account's server (Exchange's
//! Tasks folder; a CalDAV server's task lists, listed in the side pane, issue #201).

use egui::{Align2, Margin, Rect, RichText, Sense, Ui, pos2, vec2};
use mailcraft_engine::model::dates;
use mailcraft_engine::model::pim::{TaskPriority, TaskStatus};
use mailcraft_engine::{ListedTask, Session, TasksOrder};
use serde_json::json;

use crate::icons::Icon;
use crate::shown;
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
        let (n, late) = s.task_counts(&st.tasks_query, dates::today(&s.clock));
        ui.label(RichText::new(tf!("{n} tasks", n = n)).color(t.faint).small());
        if late > 0 {
            ui.label(RichText::new(tf!("{n} overdue", n = late)).color(t.danger).small());
        }
        task_lists(s, ui, acts);
    });
}

/// The CalDAV servers' task lists (issue #201), as Outlook lists its task folders: every task,
/// or one list's alone. Nothing when no server keeps them.
fn task_lists(s: &Session, ui: &mut Ui, acts: &mut Vec<Action>) {
    let lists = s.task_lists();
    if lists.is_empty() {
        return;
    }
    let several_accounts = lists.iter().any(|(a, _)| Some(*a) != lists.first().map(|f| f.0));
    ui.add_space(12.0);
    ui.label(RichText::new(tl!("Task lists")).strong().small());
    let shown = s.shown_task_list();
    if ui.selectable_label(shown.is_none(), tl!("All task lists")).clicked() && shown.is_some() {
        acts.push(Action::Run("task.showList", json!({})));
    }
    for (account, l) in lists {
        let r = ui.selectable_label(shown == Some(l.id.as_str()), &l.name);
        let r = if several_accounts { r.on_hover_text(s.account(account).map(|a| a.config.email.clone()).unwrap_or_default()) } else { r };
        if r.clicked() && shown != Some(l.id.as_str()) {
            acts.push(Action::Run("task.showList", json!({"list": l.id})));
        }
    }
}

/// The name of a task's list, when it's worth saying: there are several, and they're all shown.
fn list_name<'a>(s: &'a Session, account: &str, list: &str) -> Option<&'a str> {
    if list.is_empty() || s.shown_task_list().is_some() {
        return None;
    }
    let p = s.pim.get(account)?;
    let several = s.pim.values().map(|p| p.task_lists.len()).sum::<usize>() > 1;
    p.task_lists.iter().find(|l| l.id == list).filter(|_| several).map(|l| l.name.as_str())
}

/// A local day (days since the epoch) as the task list and the To-Do Bar name it: Today,
/// Tomorrow, Yesterday, else its date as the Dates option writes it ([`dates::day_short`]).
pub(crate) fn relative_day(day: i64, s: &Session) -> String {
    near_day(day, dates::today(&s.clock)).map_or_else(|| crate::i18n::date(&dates::day_short(day, &s.clock)), str::to_string)
}

/// Today, Tomorrow or Yesterday, when the local day `day` is one of them from `today`.
pub(crate) fn near_day(day: i64, today: i64) -> Option<&'static str> {
    match day.saturating_sub(today) {
        0 => Some(tl!("Today")),
        1 => Some(tl!("Tomorrow")),
        -1 => Some(tl!("Yesterday")),
        _ => None,
    }
}

/// The main pane: the columns' headings, then the tasks.
pub fn show(s: &Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    let t = st.tokens;
    let list = shown::tasks(s, &mut st.shown, &st.tasks_query);
    if list.is_empty() {
        let empty = st.tasks_query.trim().is_empty() && !s.prefs.tasks_hide_done;
        widgets::empty_state(ui, &t, Icon::Task, tl!(if empty { "No tasks yet: New task adds one." } else { "No task matches." }));
        return;
    }
    egui::Frame::new().inner_margin(Margin::symmetric(14, 8)).show(ui, |ui| {
        headings(s, &t, ui, acts);
    });
    egui::ScrollArea::vertical().auto_shrink(false).id_salt("tasks").show_viewport(ui, |ui, viewport| {
        egui::Frame::new().inner_margin(Margin::symmetric(14, 0)).show(ui, |ui| {
            // Only the rows in view are laid out, each where it would be with all of them.
            let pitch = ROW + ui.spacing().item_spacing.y;
            shown::rows(
                ui,
                viewport,
                list.len(),
                |k| k as f32 * pitch,
                None,
                |ui, k| match list.get(k).and_then(|at| shown::task(s, at)) {
                    Some(r) => row(s, &t, st, ui, &r, acts),
                    None => {
                        ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::hover());
                    }
                },
            );
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
    let title_rtl = crate::bidi::reads_rtl(&title);
    let title_color = if done { t.faint } else { t.text };
    // Cut with "…" where it doesn't fit, as the list's subjects are (it stopped mid-letter at
    // the next column).
    let room = (x_due - x_title - 10.0).max(10.0);
    let galley = widgets::one_line(ui, &title, theme::bold(13.0), title_color, room);
    let title_w = galley.size().x.min(room);
    let title_x = if title_rtl && !mr.on() { rect.min.x + x_title + room - title_w } else { rect.min.x + x_title };
    // Under it: its task list (when there are several), that it repeats, its categories, its
    // notes' first line.
    let mut under: Vec<String> = Vec::new();
    if let Some(name) = list_name(s, r.account, r.list) {
        under.push(name.to_string());
    }
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
    let due_text = task.due_day().map_or_else(|| tl!("None").to_string(), |d| relative_day(d, s));
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
