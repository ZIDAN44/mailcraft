//! The Calendar: a month to pick days from and the calendars in the side pane; a day, a week
//! or a month in the main one. Click an empty hour to add an event there, an event to open it.

use egui::{Align2, Color32, Margin, Pos2, Rect, RichText, Sense, Ui, Vec2, pos2, vec2};
use mailcraft_engine::model::dates::{self, civil_from_days, days_from_civil};
use mailcraft_engine::model::ical::{Privacy, ShowAs};
use mailcraft_engine::model::pim::work_hours::{outside, overlap};
use mailcraft_engine::{AgendaItem, CalMode, Session};
use serde_json::json;

mod days;
mod found;
mod moving;
mod schedule;
mod showing;
use days::days;
#[cfg(test)]
pub(crate) use found::open_hit;
pub(crate) use found::{CalSearch, found};
use schedule::schedule;
pub use showing::*;

use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};
use crate::{Action, Dialog, UiState, widgets};

mod side;
pub use side::nav;

/// Height of an hour in the day and week views.
const HOUR: f32 = 46.0;
const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

/// A month's name (1 = January) in the interface's language; nothing for a month no calendar has.
pub(crate) fn month_name(m: u32) -> &'static str {
    crate::i18n::tc("month", m.checked_sub(1).and_then(|i| MONTHS.get(i as usize)).copied().unwrap_or(""))
}

/// A month's name in a date, after its day (`17 December 2026`): the form a language gives it
/// there (Russian's and Slovak's genitive, `17 декабря 2026`, issue #215), its name alone where
/// that's the same.
pub(crate) fn date_month_name(m: u32) -> &'static str {
    let english = m.checked_sub(1).and_then(|i| MONTHS.get(i as usize)).copied().unwrap_or("");
    crate::i18n::tc_own("date", english).unwrap_or_else(|| month_name(m))
}

/// The calendar on a day (`calendar.goto`).
pub(crate) fn goto(day: i64) -> Action {
    Action::Run("calendar.goto", json!({"date": dates::day_string(day)}))
}

/// The Calendar opened on a day, in the day view: from the To-Do Bar's month, an invitation.
pub(crate) fn show_day(day: i64, acts: &mut Vec<Action>) {
    acts.push(Action::Run("view.module", json!({"name": "calendar"})));
    acts.push(Action::Run("calendar.view", json!({"mode": "day"})));
    acts.push(goto(day));
}

/// A calendar's colour: an account's own calendar in the account's colour, others in the
/// calendar palette (`Session::calendar_color`).
pub fn color(i: u8, t: &Tokens) -> Color32 {
    let account = mailcraft_engine::ACCOUNT_COLOR;
    if i >= account {
        theme::account_color(i - account)
    } else if i == 0 {
        t.accent
    } else {
        theme::category_color(i.saturating_sub(1), t.dark)
    }
}

/// The time found free for everyone (issue #186), at `r`: outlined in the accent; clicked, a
/// meeting then with everyone whose calendar is shown.
fn marked_slot(s: &Session, t: &Tokens, ui: &mut Ui, r: Rect, id: (&str, i64, usize)) -> Option<Action> {
    let slot = s.free_slot()?;
    let resp = ui.interact(r, ui.scope_id().with(id), Sense::click());
    let fill = t.accent.gamma_multiply(if resp.hovered() { 0.3 } else { 0.15 });
    ui.painter().rect(r, 3.0, fill, egui::Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
    crate::widgets::named(ui, &resp, tl!("Free for everyone"), true, None);
    let clicked = resp.on_hover_text(tl!("Free for everyone: click to make a meeting with them")).clicked();
    clicked.then(|| schedule::new_meeting(s, slot, &None, &s.shown_calendars()))
}

/// The shade over the hours not worked (issue #185).
fn off_hours(t: &Tokens) -> egui::Color32 {
    if t.dark { egui::Color32::from_black_alpha(70) } else { egui::Color32::from_black_alpha(13) }
}

fn today(s: &Session) -> i64 {
    dates::today(&s.clock)
}

/// The time of day, local: `14:05`, or `2:05 PM` on a 12-hour clock (issue #46), its AM or PM
/// in the interface's language.
pub fn hm(t: i64, s: &Session) -> String {
    crate::i18n::date(&dates::hm(t, &s.clock))
}

/// The local day a time falls on.
fn day_of(t: i64, s: &Session) -> i64 {
    s.clock.local(t).div_euclid(86_400)
}

/// `October 2026`, `12 – 18 October 2026`, `Monday, 12 October 2026`.
pub fn title(s: &Session) -> String {
    let (d0, d1) = s.cal_days();
    match s.cal_mode {
        CalMode::Month => {
            let (y, m, _) = civil_from_days(s.cal_day());
            tf!("{month} {year}", month = month_name(m), year = y)
        }
        CalMode::WorkWeek | CalMode::Week => {
            let ((y0, m0, a), (y1, m1, b)) = (civil_from_days(d0), civil_from_days(d1 - 1));
            if (y0, m0) == (y1, m1) {
                tf!("{from} – {to} {month} {year}", from = a, to = b, month = date_month_name(m1), year = y1)
            } else {
                tf!(
                    "{from} {month_from} – {to} {month} {year}",
                    from = a,
                    month_from = date_month_name(m0),
                    to = b,
                    month = date_month_name(m1),
                    year = y1
                )
            }
        }
        CalMode::Day | CalMode::Schedule => long_date(d0),
    }
}

/// A day written out: `Thursday, 22 October 2026`, in the interface's language.
pub fn long_date(day: i64) -> String {
    let (y, m, d) = civil_from_days(day);
    tf!("{weekday}, {day} {month} {year}", weekday = tl!(dates::weekday_long(day)), day = d, month = date_month_name(m), year = y)
}

/// The months of the side pane, one under another (Outlook's date navigator, pulled down for
/// more), and a grip under them to drag for more or fewer.
fn months(s: &Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    let t = st.tokens;
    let shown = st.mini_month.unwrap_or_else(|| {
        let (y, m, _) = civil_from_days(s.cal_day());
        y * 12 + i64::from(m) - 1
    });
    let count = s.prefs.calendar_months.clamp(1, 6);
    // The days with events, looked up once for all the months shown.
    let busy = busy_days(s, grid_start(shown), grid_start(shown + i64::from(count) - 1) + 42);
    for k in 0..i64::from(count) {
        if k > 0 {
            ui.add_space(8.0);
        }
        mini_month(s, st, ui, acts, MiniMonth { first: shown, month: shown + k, arrows: k == 0, bar: false, busy: &busy });
    }
    // The grip: dragged down, one more month for each month's height; up, one fewer.
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 12.0), Sense::click_and_drag());
    let c = if resp.hovered() || resp.dragged() { t.dim } else { t.border };
    ui.painter().line_segment([pos2(r.center().x - 14.0, r.center().y), pos2(r.center().x + 14.0, r.center().y)], egui::Stroke::new(2.0, c));
    let resp = resp.on_hover_text(tl!("Drag down to show more months")).on_hover_cursor(egui::CursorIcon::ResizeVertical);
    if resp.drag_started() {
        st.months_drag = 0.0;
    }
    if resp.dragged() {
        st.months_drag += resp.drag_delta().y;
        let step = 170.0;
        if st.months_drag > step && count < 6 {
            st.months_drag -= step;
            acts.push(Action::Run("calendar.months", json!({"count": count + 1})));
        } else if st.months_drag < -step && count > 1 {
            st.months_drag += step;
            acts.push(Action::Run("calendar.months", json!({"count": count - 1})));
        }
    }
    resp.context_menu(|ui| {
        for n in 1..=6u8 {
            let label = if n == 1 { tl!("1 month").to_string() } else { tf!("{n} months", n = n) };
            if ui.radio(count == n, label).clicked() {
                acts.push(Action::Run("calendar.months", json!({"count": n})));
                ui.close();
            }
        }
    });
}

/// The To-Do Bar's month beside the mail: this month until its arrows move it, the days with
/// something on them in bold; a day clicked lists its events under it (clicked again, or today:
/// what's coming up), a day double-clicked opens the calendar on it (issue #154).
pub(crate) fn bar_month(s: &Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    let shown = st.todo_month.unwrap_or_else(|| {
        let (y, m, _) = civil_from_days(today(s));
        y * 12 + i64::from(m) - 1
    });
    let busy = bar_busy_days(s, grid_start(shown), grid_start(shown) + 42);
    mini_month(s, st, ui, acts, MiniMonth { first: shown, month: shown, arrows: true, bar: true, busy: &busy });
}

/// The first day a small month's grid shows (`month`: months since year 0): the Monday on or
/// before its first.
fn grid_start(month: i64) -> i64 {
    let first = days_from_civil(month.div_euclid(12), u32::try_from(month.rem_euclid(12) + 1).unwrap_or(1), 1);
    first - i64::from(dates::weekday_index(first))
}

/// The days from `from` up to `to` that have an event on them (Outlook's bold days).
pub(crate) fn busy_days(s: &Session, from: i64, to: i64) -> std::collections::BTreeSet<i64> {
    days_with(s, &s.agenda(s.local_midnight(from), s.local_midnight(to)), from, to)
}

/// [`busy_days`] in the To-Do Bar's month: the days of the bar's own appointments (issue #205),
/// the user's calendars'. A colleague's holiday made a day bold whose list then had nothing.
pub(crate) fn bar_busy_days(s: &Session, from: i64, to: i64) -> std::collections::BTreeSet<i64> {
    days_with(s, &s.todo_appointments(s.local_midnight(from), s.local_midnight(to)), from, to)
}

/// The days from `from` up to `to` that these appointments are on.
fn days_with(s: &Session, items: &[mailcraft_engine::AgendaItem], from: i64, to: i64) -> std::collections::BTreeSet<i64> {
    let mut days = std::collections::BTreeSet::new();
    // Colleagues' times taken aren't the user's, and are only had around the days shown.
    for it in items.iter().filter(|it| !it.task && !it.busy_only) {
        // Only the days asked about: an event years long is no longer to walk than these.
        let last = day_of(it.end.saturating_sub(1).max(it.start), s).min(to - 1);
        days.extend(day_of(it.start, s).max(from)..=last);
    }
    days
}

/// A small month to draw (months counted since year 0).
struct MiniMonth<'a> {
    /// The first of the small months shown.
    first: i64,
    /// This one.
    month: i64,
    /// It has the arrows that move them all (the first of them does).
    arrows: bool,
    /// The To-Do Bar's, which marks no days as shown and opens the calendar on the day clicked.
    bar: bool,
    /// The days with events, bold.
    busy: &'a std::collections::BTreeSet<i64>,
}

/// A small month: click a day to show it.
fn mini_month(s: &Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>, m: MiniMonth) {
    let MiniMonth { first: shown, month, arrows, bar, busy } = m;
    let t = st.tokens;
    let (y, m) = (month.div_euclid(12), u32::try_from(month.rem_euclid(12) + 1).unwrap_or(1));
    ui.horizontal(|ui| {
        ui.label(RichText::new(tf!("{month} {year}", month = month_name(m), year = y)).font(theme::bold(13.0)).color(t.text));
        if !arrows {
            return;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let moved = if bar { &mut st.todo_month } else { &mut st.mini_month };
            if widgets::tiny_icon_button(ui, Icon::ChevronRight, "Next month", &t).clicked() {
                *moved = Some(shown + 1);
            }
            if widgets::tiny_icon_button(ui, Icon::ChevronLeft, "Previous month", &t).clicked() {
                *moved = Some(shown - 1);
            }
        });
    });
    let w = ((ui.available_width() - 4.0) / 7.0).clamp(20.0, 34.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for d in dates::WEEKDAYS {
            let (r, _) = ui.allocate_exact_size(vec2(w, 18.0), Sense::hover());
            ui.painter().text(
                r.center(),
                Align2::CENTER_CENTER,
                tl!(d).chars().next().map(String::from).unwrap_or_default(),
                theme::regular(11.0),
                t.faint,
            );
        }
    });
    let start = grid_start(month);
    let (sel0, sel1) = s.cal_days();
    let now = today(s);
    for week in 0..6 {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            for i in 0..7 {
                let d = start + week * 7 + i;
                let (r, resp) = ui.allocate_exact_size(vec2(w, 22.0), Sense::click());
                let (_, dm, dd) = civil_from_days(d);
                let picked = if bar { st.todo_day == Some(d) } else { d >= sel0 && d < sel1 && s.cal_mode != CalMode::Month };
                if picked {
                    ui.painter().rect_filled(r.shrink(1.0), 4.0, t.accent_soft);
                }
                if d == now {
                    ui.painter().circle_filled(r.center(), 10.0, t.accent);
                } else if resp.hovered() {
                    ui.painter().circle_filled(r.center(), 10.0, t.hover);
                }
                let c = if d == now {
                    t.on_accent
                } else if dm != m {
                    t.faint
                } else {
                    t.text
                };
                let font = if busy.contains(&d) { theme::bold(12.0) } else { theme::regular(12.0) };
                ui.painter().text(r.center(), Align2::CENTER_CENTER, dd.to_string(), font, c);
                // The To-Do Bar's: a look at the day under it, the mail kept on screen (issue #154:
                // the calendar opening at every click lost one's place); the calendar on it at a
                // double click, as it was at a click (#79).
                if bar && resp.double_clicked() {
                    show_day(d, acts);
                    st.todo_month = None;
                    st.todo_day = None;
                } else if resp.clicked() {
                    if bar {
                        st.todo_day = if st.todo_day == Some(d) || d == now { None } else { Some(d) };
                    } else {
                        acts.push(goto(d));
                        st.mini_month = None;
                    }
                }
            }
        });
    }
}

/// The main pane.
pub fn show(s: &Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    // Searching: the events found, in place of the days.
    if st.cal_search.active() {
        found(s, st, ui, acts);
        return;
    }
    let t = st.tokens;
    egui::Frame::new().inner_margin(Margin::symmetric(14, 10)).show(ui, |ui| {
        ui.horizontal(|ui| {
            if ui.button(tl!("Today")).clicked() {
                acts.push(Action::Run("calendar.goto", json!({"date": "today"})));
            }
            if widgets::tiny_icon_button(ui, Icon::ChevronLeft, "Previous", &t).clicked() {
                acts.push(Action::Run("calendar.goto", json!({"step": -1})));
            }
            if widgets::tiny_icon_button(ui, Icon::ChevronRight, "Next", &t).clicked() {
                acts.push(Action::Run("calendar.goto", json!({"step": 1})));
            }
            ui.add_space(6.0);
            let modes = [
                (CalMode::Month, "Month", "month"),
                (CalMode::Week, "Week", "week"),
                (CalMode::WorkWeek, "Work week", "workWeek"),
                (CalMode::Day, "Day", "day"),
                (CalMode::Schedule, "Schedule", "schedule"),
            ];
            let pad = ui.spacing().button_padding.x * 2.0 + ui.spacing().item_spacing.x;
            let buttons: f32 = modes
                .iter()
                .map(|(_, label, _)| ui.painter().layout_no_wrap(tl!(label).to_string(), theme::regular(13.5), t.text).size().x + pad)
                .sum();
            let whole = ui.painter().layout_no_wrap(title(s), theme::bold(18.0), t.text).size().x;
            // The days shown, then the views' buttons against the end. In a window with no room
            // for both, the days alone (they ran on under the buttons): the views are on the
            // ribbon too.
            let both = whole <= ui.available_width() - buttons - 12.0;
            let shown = widgets::one_line(ui, &title(s), theme::bold(18.0), t.text, ui.available_width());
            ui.label(shown);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                for (mode, label, code) in modes.into_iter().filter(|_| both) {
                    if ui.selectable_label(s.cal_mode == mode, tl!(label)).clicked() {
                        acts.push(Action::Run("calendar.view", json!({"mode": code})));
                    }
                }
            });
        });
    });
    let (d0, d1) = s.cal_days();
    let (from, to) = (s.local_midnight(d0), s.local_midnight(d1));
    // The tasks due these days too, drawn from the tasks (they aren't events).
    let mut items = s.agenda(from, to);
    items.extend(s.task_items(from, to));
    match s.cal_mode {
        CalMode::Month => month(s, &t, ui, &items, d0, acts),
        CalMode::Day | CalMode::WorkWeek | CalMode::Week => days(s, st, ui, &items, s.cal_day_list(), acts),
        CalMode::Schedule => schedule(s, st, ui, &items, d0, acts),
    }
}

fn title_of(it: &AgendaItem) -> String {
    event_title(&it.summary, it.privacy, it.show_as)
}

/// Open an item (its event, or its task). A colleague's time taken has nothing to open.
fn open(s: &Session, item: &AgendaItem) -> Option<Action> {
    if item.busy_only {
        return None;
    }
    if item.task {
        return Dialog::open_task(s, &item.uid).map(Action::Dialog);
    }
    Dialog::open_event(s, &item.uid, item.occurrence).map(Action::Dialog)
}

/// An event's (or a task's) box: clicked, it opens; `movable`, it's dragged to another time or
/// day (issue #192).
fn chip(ui: &mut Ui, t: &Tokens, rect: Rect, item: &AgendaItem, text: &str, movable: bool) -> egui::Response {
    // The same meeting can be in two calendars (two accounts', or an account's and a shared
    // mailbox's) at the same time: told apart by whose it is, or the click went to neither and
    // started a new event behind them.
    let sense = if movable { Sense::click_and_drag() } else { Sense::click() };
    let resp = ui.interact(rect, ui.scope_id().with(("chip", &item.account, &item.calendar, &item.uid, item.start)), sense);
    // A category's colour, as Outlook draws a categorized appointment; else its calendar's.
    let c = item.category.map_or_else(|| color(item.color, t), |i| theme::category_color(i, t.dark));
    let tentative = item.needs_reply || item.status == "TENTATIVE" || matches!(item.show_as, ShowAs::Tentative | ShowAs::Free);
    // Stronger under the pointer, with its colour round it.
    let hovered = resp.hovered();
    let fill = match (tentative, hovered) {
        (true, false) => t.surface,
        (true, true) => t.hover,
        (false, h) => c.gamma_multiply(match (t.dark, h) {
            (true, false) => 0.35,
            (true, true) => 0.5,
            (false, false) => 0.18,
            (false, true) => 0.3,
        }),
    };
    ui.painter().rect(rect, 4.0, fill, egui::Stroke::new(if tentative || hovered { 1.0 } else { 0.0 }, c), egui::StrokeKind::Inside);
    time_strip(ui, Rect::from_min_size(rect.min, vec2(3.0, rect.height())), c, item.show_as);
    // A private one has a lock in its corner (issue #32), as Outlook marks it.
    let lock = item.privacy != Privacy::Public && rect.width() > 30.0;
    if lock {
        let r = Rect::from_min_size(pos2(rect.max.x - 15.0, rect.min.y + 3.0), vec2(11.0, 11.0));
        icons::paint(ui.painter(), r, Icon::Lock, t.dim);
    }
    // As many whole lines as the box holds, the last one ending in "…" when more is left (a line
    // cut in half by the box's edge read badly).
    let font = theme::regular(12.0);
    let row = ui.fonts_mut(|f| f.row_height(&font)).max(1.0);
    // A task: its box before its title, ticked (and the title faint) when it's done.
    let done = item.task && item.status == "COMPLETED";
    let lead = if item.task && rect.width() > 30.0 { 15.0 } else { 0.0 };
    if lead > 0.0 {
        let r = Rect::from_min_size(pos2(rect.min.x + 6.0, rect.min.y + 3.0), vec2(11.0, 11.0));
        icons::paint(ui.painter(), r, Icon::Task, if done { t.faint } else { t.dim });
    }
    let ink = if done { t.faint } else { t.text };
    let room = (rect.width() - lead - if lock { 22.0 } else { 8.0 }).max(4.0);
    // Too narrow for a word of its title (events side by side in a narrow day): the start of
    // the title on one line. Wrapped, it was a letter a line down the box.
    let title = text.lines().next().unwrap_or_default();
    let widest = title.split_whitespace().map(|w| ui.painter().layout_no_wrap(w.to_string(), font.clone(), ink).size().x).fold(0.0, f32::max);
    let narrow = widest > room;
    let mut job = egui::text::LayoutJob::simple(if narrow { title.to_string() } else { text.to_string() }, font, ink, room);
    job.wrap.max_rows = if narrow { 1 } else { ((rect.height() - 4.0) / row).floor().max(1.0) as usize };
    job.wrap.break_anywhere = narrow;
    job.wrap.overflow_character = Some('…');
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    ui.painter().with_clip_rect(rect.shrink(1.0)).galley(rect.min + vec2(6.0 + lead, 2.0), galley, ink);
    resp
}

/// The month view's rows at their shortest: a day's number over its "n more" link.
const MONTH_ROW_MIN: f32 = 42.0;

/// A calendar's name as shown: the local one's in the language shown.
fn calendar_name(id: &str, name: &str) -> String {
    if id == mailcraft_engine::model::pim::LOCAL { tl!("Calendar").to_string() } else { name.to_string() }
}

fn month(s: &Session, t: &Tokens, ui: &mut Ui, items: &[AgendaItem], d0: i64, acts: &mut Vec<Action>) {
    let (_, shown_month, _) = civil_from_days(s.cal_day());
    let now = today(s);
    let area = ui.available_rect_before_wrap().shrink2(vec2(10.0, 4.0));
    let head = 22.0;
    let cw = area.width() / 7.0;
    // Six weeks in the pane's height: a short pane's days show fewer events each, the rest
    // behind "n more" (as tall as a day's number and that link). Only `area` is allocated: rows
    // kept 60 points tall ran the last weeks past the pane's edge, where nothing scrolls.
    let rh = ((area.height() - head) / 6.0).max(MONTH_ROW_MIN);
    for (i, d) in dates::WEEKDAYS.iter().enumerate() {
        ui.painter().text(pos2(area.min.x + cw * (i as f32 + 0.5), area.min.y + 10.0), Align2::CENTER_CENTER, tl!(d), theme::regular(12.0), t.dim);
    }
    for w in 0..6 {
        for i in 0..7 {
            let d = d0 + w * 7 + i;
            let r = Rect::from_min_size(pos2(area.min.x + cw * i as f32, area.min.y + head + rh * w as f32), vec2(cw, rh));
            if ui.rect_contains_pointer(r) {
                ui.painter().rect_filled(r, 0.0, t.hover_row);
            }
            ui.painter().rect_stroke(r, 0.0, egui::Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
            let (_, m, dd) = civil_from_days(d);
            let num_pos = r.min + vec2(14.0, 12.0);
            if d == now {
                ui.painter().circle_filled(num_pos, 10.0, t.accent);
            }
            ui.painter().text(
                num_pos,
                Align2::CENTER_CENTER,
                dd.to_string(),
                theme::regular(12.0),
                if d == now {
                    t.on_accent
                } else if m == shown_month {
                    t.text
                } else {
                    t.faint
                },
            );
            let bg = ui.interact(r, ui.scope_id().with(("month-day", d)), Sense::click());
            let todays: Vec<&AgendaItem> =
                items.iter().filter(|it| day_of(it.start, s) <= d && day_of(it.end.max(it.start + 1) - 1, s) >= d).collect();
            let fit = ((rh - 26.0) / 19.0).floor().max(0.0) as usize;
            let shown = if todays.len() > fit { fit.saturating_sub(1) } else { todays.len() };
            for (k, it) in todays.iter().take(shown).enumerate() {
                let cr = Rect::from_min_size(r.min + vec2(3.0, 24.0 + 19.0 * k as f32), vec2(cw - 6.0, 17.0));
                let text = if it.all_day { title_of(it) } else { format!("{} {}", hm(it.start, s), title_of(it)) };
                let resp = chip(ui, t, cr, it, &text, s.movable(it));
                if resp.clicked()
                    && let Some(a) = open(s, it)
                {
                    acts.push(a);
                }
                // Dragged to another day (issue #192): the day under the pointer, its time of
                // day kept.
                if let Some(drag) = moving::dragged(ui, &resp, cr) {
                    let col = ((drag.pointer.x - area.min.x) / cw).floor().clamp(0.0, 6.0) as i64;
                    let row = ((drag.pointer.y - area.min.y - head) / rh).floor().clamp(0.0, 5.0) as i64;
                    let to = d0 + row * 7 + col;
                    let first = day_of(it.start, s);
                    let by = if it.all_day { (to - d) * 86_400 } else { s.local_midnight(first + to - d) - s.local_midnight(first) };
                    let cell = Rect::from_min_size(pos2(area.min.x + cw * col as f32, area.min.y + head + rh * row as f32), vec2(cw, rh));
                    let day = crate::i18n::date(&dates::day_short(first + to - d, &s.clock));
                    moving::ghost(ui, t, cell.shrink(2.0), &format!("{}\n{day}", title_of(it)));
                    if drag.done
                        && let Some(a) = moving::moved(it, by)
                    {
                        acts.push(a);
                    }
                }
            }
            if todays.len() > shown {
                let more = ui.put(
                    Rect::from_min_size(r.min + vec2(6.0, 24.0 + 19.0 * shown as f32), vec2(cw - 12.0, 17.0)),
                    egui::Link::new(RichText::new(tf!("{n} more", n = todays.len() - shown)).small()),
                );
                if more.clicked() {
                    acts.push(Action::Run("calendar.view", json!({"mode": "day"})));
                    acts.push(goto(d));
                }
            }
            if bg.double_clicked() {
                acts.push(Action::Dialog(Dialog::new_event(s, Some(s.local_midnight(d) + 9 * 3600))));
            }
        }
    }
    ui.allocate_rect(area, Sense::hover());
}
