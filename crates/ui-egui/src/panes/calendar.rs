//! The Calendar: a month to pick days from and the calendars in the side pane; a day, a week
//! or a month in the main one. Click an empty hour to add an event there, an event to open it.

use egui::{Align2, Color32, Label, Margin, Rect, RichText, Sense, Ui, pos2, vec2};
use mailcraft_engine::model::dates::{self, civil_from_days, days_from_civil};
use mailcraft_engine::model::ical::{Privacy, ShowAs};
use mailcraft_engine::{AgendaItem, CalMode, Session};
use serde_json::json;

mod found;
mod showing;
#[cfg(test)]
pub(crate) use found::open_hit;
pub(crate) use found::{CalSearch, found};
pub use showing::*;

use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};
use crate::{Action, Dialog, UiState, widgets};

mod side;
pub use side::nav;

/// Height of an hour in the day and week views.
const HOUR: f32 = 46.0;
pub(crate) const MONTHS: [&str; 12] =
    ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const DAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

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
    let month = |m: u32| crate::i18n::tc("month", MONTHS.get(m as usize - 1).copied().unwrap_or("")).to_string();
    match s.cal_mode {
        CalMode::Month => {
            let (y, m, _) = civil_from_days(s.cal_day());
            tf!("{month} {year}", month = month(m), year = y)
        }
        CalMode::WorkWeek | CalMode::Week => {
            let ((y0, m0, a), (y1, m1, b)) = (civil_from_days(d0), civil_from_days(d1 - 1));
            if (y0, m0) == (y1, m1) {
                tf!("{from} – {to} {month} {year}", from = a, to = b, month = month(m1), year = y1)
            } else {
                tf!("{from} {month_from} – {to} {month} {year}", from = a, month_from = month(m0), to = b, month = month(m1), year = y1)
            }
        }
        CalMode::Day => long_date(d0),
    }
}

/// A day written out: `Thursday, 22 October 2026`, in the interface's language.
pub fn long_date(day: i64) -> String {
    let (y, m, d) = civil_from_days(day);
    let month = crate::i18n::tc("month", MONTHS.get(m as usize - 1).copied().unwrap_or("")).to_string();
    let wd = (day + 3).rem_euclid(7) as usize;
    let day_name = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"].get(wd).copied().unwrap_or("");
    tf!("{weekday}, {day} {month} {year}", weekday = tl!(day_name), day = d, month = month, year = y)
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
    for k in 0..i64::from(count) {
        if k > 0 {
            ui.add_space(8.0);
        }
        mini_month(s, st, ui, acts, shown, shown + k, k == 0);
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

/// A small month (`month`: months since year 0): click a day to show it. The first of them has
/// the arrows that move them all (`shown` is the first one's month).
fn mini_month(s: &Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>, shown: i64, month: i64, arrows: bool) {
    let t = st.tokens;
    let (y, m) = (month.div_euclid(12), u32::try_from(month.rem_euclid(12) + 1).unwrap_or(1));
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(tf!("{month} {year}", month = crate::i18n::tc("month", MONTHS.get(m as usize - 1).copied().unwrap_or("")), year = y))
                .font(theme::bold(13.0))
                .color(t.text),
        );
        if !arrows {
            return;
        }
        ui.with_layout(crate::bidi::row_from_end(egui::Align::Center), |ui| {
            let next = if crate::i18n::current().rtl() { Icon::ChevronLeft } else { Icon::ChevronRight };
            if widgets::tiny_icon_button(ui, next, "Next month", &t).clicked() {
                st.mini_month = Some(shown + 1);
            }
            let previous = if crate::i18n::current().rtl() { Icon::ChevronRight } else { Icon::ChevronLeft };
            if widgets::tiny_icon_button(ui, previous, "Previous month", &t).clicked() {
                st.mini_month = Some(shown - 1);
            }
        });
    });
    let touch = ui.ctx().input(|input| input.has_touch_screen());
    let w = ((ui.available_width() - 4.0) / 7.0).clamp(20.0, if touch { 44.0 } else { 34.0 });
    let weekday_height = if touch { 24.0 } else { 18.0 };
    let day_height = if touch { 44.0 } else { 22.0 };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for d in DAYS {
            let (r, _) = ui.allocate_exact_size(vec2(w, weekday_height), Sense::hover());
            ui.painter().text(
                r.center(),
                Align2::CENTER_CENTER,
                tl!(d).chars().next().map(String::from).unwrap_or_default(),
                theme::regular(11.0),
                t.faint,
            );
        }
    });
    let first = days_from_civil(y, m, 1);
    let start = first - (first + 3).rem_euclid(7);
    let (sel0, sel1) = s.cal_days();
    let now = today(s);
    for week in 0..6 {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            for i in 0..7 {
                let d = start + week * 7 + i;
                let (r, resp) = ui.allocate_exact_size(vec2(w, day_height), Sense::click());
                let (_, dm, dd) = civil_from_days(d);
                widgets::named(ui, &resp, &long_date(d), true, Some(s.cal_mode != CalMode::Month && d >= sel0 && d < sel1));
                if d >= sel0 && d < sel1 && s.cal_mode != CalMode::Month {
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
                ui.painter().text(r.center(), Align2::CENTER_CENTER, dd.to_string(), theme::regular(12.0), c);
                if resp.clicked() {
                    let (yy, mm, ddd) = civil_from_days(d);
                    acts.push(Action::Run("calendar.goto", json!({"date": format!("{yy:04}-{mm:02}-{ddd:02}")})));
                    st.mini_month = None;
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
        let rtl = crate::i18n::current().rtl();
        ui.with_layout(crate::bidi::row_from_start(egui::Align::Center), |ui| {
            let today = if st.phone_mode { ui.add(egui::Button::new(tl!("Today")).min_size(vec2(52.0, 44.0))) } else { ui.button(tl!("Today")) };
            if today.clicked() {
                acts.push(Action::Run("calendar.goto", json!({"date": "today"})));
            }
            let previous = if rtl { Icon::ChevronRight } else { Icon::ChevronLeft };
            let next = if rtl { Icon::ChevronLeft } else { Icon::ChevronRight };
            if widgets::tiny_icon_button(ui, previous, "Previous", &t).clicked() {
                acts.push(Action::Run("calendar.goto", json!({"step": -1})));
            }
            if widgets::tiny_icon_button(ui, next, "Next", &t).clicked() {
                acts.push(Action::Run("calendar.goto", json!({"step": 1})));
            }
            ui.add_space(6.0);
            if st.phone_mode {
                ui.add(Label::new(RichText::new(title(s)).font(theme::bold(16.0)).color(t.text)).truncate());
                let current = match s.cal_mode {
                    CalMode::Month => "Month",
                    CalMode::Week => "Week",
                    CalMode::WorkWeek => "Work week",
                    CalMode::Day => "Day",
                };
                let view = ui.add(egui::Button::new(tl!(current)).min_size(vec2(0.0, 44.0)));
                crate::bidi::popup_menu(&view).show(|ui| {
                    for (mode, label, code) in [
                        (CalMode::Month, "Month", "month"),
                        (CalMode::Week, "Week", "week"),
                        (CalMode::WorkWeek, "Work week", "workWeek"),
                        (CalMode::Day, "Day", "day"),
                    ] {
                        if ui.selectable_label(s.cal_mode == mode, tl!(label)).clicked() {
                            acts.push(Action::Run("calendar.view", json!({"mode": code})));
                            ui.close();
                        }
                    }
                });
            } else {
                ui.label(RichText::new(title(s)).font(theme::bold(18.0)).color(t.text));
                ui.with_layout(crate::bidi::row_from_end(egui::Align::Center), |ui| {
                    for (mode, label, code) in [
                        (CalMode::Month, "Month", "month"),
                        (CalMode::Week, "Week", "week"),
                        (CalMode::WorkWeek, "Work week", "workWeek"),
                        (CalMode::Day, "Day", "day"),
                    ] {
                        if ui.selectable_label(s.cal_mode == mode, tl!(label)).clicked() {
                            acts.push(Action::Run("calendar.view", json!({"mode": code})));
                        }
                    }
                });
            }
        });
    });
    let (d0, d1) = s.cal_days();
    let (from, to) = (s.local_midnight(d0), s.local_midnight(d1));
    // The tasks due these days too, drawn from the tasks (they aren't events).
    let mut items = s.agenda(from, to);
    items.extend(s.task_items(from, to));
    match s.cal_mode {
        CalMode::Month => month(s, &t, ui, &items, d0, acts),
        CalMode::Day | CalMode::WorkWeek | CalMode::Week => days(s, st, ui, &items, d0, d1, acts),
    }
}

fn title_of(it: &AgendaItem) -> String {
    event_title(&it.summary, it.privacy, it.show_as)
}

/// Open an item (its event, or its task).
fn open(s: &Session, item: &AgendaItem) -> Option<Action> {
    if item.task {
        return Dialog::open_task(s, &item.uid).map(Action::Dialog);
    }
    Dialog::open_event(s, &item.uid, item.occurrence).map(Action::Dialog)
}

fn chip(ui: &mut Ui, t: &Tokens, rect: Rect, item: &AgendaItem, text: &str) -> egui::Response {
    // The same meeting can be in two calendars (two accounts', or an account's and a shared
    // mailbox's) at the same time: told apart by whose it is, or the click went to neither and
    // started a new event behind them.
    let resp = ui.interact(rect, ui.scope_id().with(("chip", &item.account, &item.calendar, &item.uid, item.start)), Sense::click());
    crate::widgets::named(ui, &resp, text, true, None);
    let mirror = crate::bidi::Mirror::new(rect);
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
    time_strip(ui, mirror.rect(Rect::from_min_size(rect.min, vec2(3.0, rect.height()))), c, item.show_as);
    // A private one has a lock in its corner (issue #32), as Outlook marks it.
    let lock = item.privacy != Privacy::Public && rect.width() > 30.0;
    if lock {
        let r = Rect::from_min_size(mirror.pos(pos2(rect.max.x - 15.0, rect.min.y + 3.0)), vec2(11.0, 11.0));
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
        let r = Rect::from_min_size(mirror.pos(pos2(rect.min.x + 6.0, rect.min.y + 3.0)), vec2(11.0, 11.0));
        icons::paint(ui.painter(), r, Icon::Task, if done { t.faint } else { t.dim });
    }
    let ink = if done { t.faint } else { t.text };
    let mut job = egui::text::LayoutJob::simple(text.to_string(), font, ink, (rect.width() - lead - if lock { 22.0 } else { 8.0 }).max(4.0));
    let rtl_text = crate::bidi::has_rtl(&job.text) && crate::bidi::paragraph_rtl(&job.text);
    job.halign = if rtl_text { egui::Align::Max } else { egui::Align::Min };
    job.wrap.max_rows = ((rect.height() - 4.0) / row).floor().max(1.0) as usize;
    job.wrap.overflow_character = Some('…');
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    let text_x = mirror.start(rect.min.x + 6.0 + lead, galley.size().x);
    ui.painter().with_clip_rect(rect.shrink(1.0)).galley(pos2(text_x, rect.min.y + 2.0), galley, ink);
    resp
}

fn month(s: &Session, t: &Tokens, ui: &mut Ui, items: &[AgendaItem], d0: i64, acts: &mut Vec<Action>) {
    let (_, shown_month, _) = civil_from_days(s.cal_day());
    let now = today(s);
    let touch = ui.ctx().input(|input| input.has_touch_screen());
    let area = ui.available_rect_before_wrap().shrink2(vec2(10.0, 4.0));
    let head = 22.0;
    let touch_input = ui.input(|input| input.events.iter().any(|event| matches!(event, egui::Event::Touch { .. })));
    let cw = area.width() / 7.0;
    let rh = ((area.height() - head) / 6.0).max(if touch { 44.0 } else { 60.0 });
    let compact_touch = touch && rh < 60.0;
    let mirror = crate::bidi::Mirror::new(area);
    for (i, d) in DAYS.iter().enumerate() {
        ui.painter().text(
            mirror.pos(pos2(area.min.x + cw * (i as f32 + 0.5), area.min.y + 10.0)),
            Align2::CENTER_CENTER,
            tl!(d),
            theme::regular(12.0),
            t.dim,
        );
    }
    for w in 0..6 {
        for i in 0..7 {
            let d = d0 + w * 7 + i;
            let r = mirror.rect(Rect::from_min_size(pos2(area.min.x + cw * i as f32, area.min.y + head + rh * w as f32), vec2(cw, rh)));
            if ui.rect_contains_pointer(r) {
                ui.painter().rect_filled(r, 0.0, t.hover_row);
            }
            ui.painter().rect_stroke(r, 0.0, egui::Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
            let (_, m, dd) = civil_from_days(d);
            let num_pos = crate::bidi::Mirror::new(r).pos(r.min + vec2(14.0, 12.0));
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
            widgets::named(ui, &bg, &long_date(d), true, None);
            let todays: Vec<&AgendaItem> =
                items.iter().filter(|it| day_of(it.start, s) <= d && day_of(it.end.max(it.start + 1) - 1, s) >= d).collect();
            if compact_touch {
                if let Some(first) = todays.first() {
                    let marker_y = r.bottom() - 7.0;
                    if todays.len() == 1 {
                        ui.painter().circle_filled(pos2(r.center().x, marker_y), 3.0, color(first.color, t));
                    } else {
                        let count = todays.len().to_string();
                        ui.painter().text(pos2(r.center().x, marker_y), Align2::CENTER_CENTER, count, theme::bold(9.0), color(first.color, t));
                    }
                }
            } else {
                let event_top = if touch { 28.0 } else { 24.0 };
                let event_step = if touch { 30.0 } else { 19.0 };
                let event_height = if touch { 28.0 } else { 17.0 };
                let fit = ((rh - event_top - 2.0) / event_step).floor().max(0.0) as usize;
                let shown = if todays.len() > fit { fit.saturating_sub(1) } else { todays.len() };
                for (k, it) in todays.iter().take(shown).enumerate() {
                    let cr = crate::bidi::Mirror::new(r)
                        .rect(Rect::from_min_size(r.min + vec2(3.0, event_top + event_step * k as f32), vec2(cw - 6.0, event_height)));
                    let text = if it.all_day { title_of(it) } else { format!("{} {}", hm(it.start, s), title_of(it)) };
                    if chip(ui, t, cr, it, &text).clicked()
                        && let Some(a) = open(s, it)
                    {
                        acts.push(a);
                    }
                }
                if todays.len() > shown {
                    let more = ui.put(
                        crate::bidi::Mirror::new(r)
                            .rect(Rect::from_min_size(r.min + vec2(6.0, event_top + event_step * shown as f32), vec2(cw - 12.0, event_height))),
                        egui::Link::new(RichText::new(tf!("{n} more", n = todays.len() - shown)).small()),
                    );
                    if more.clicked() {
                        let (y, mm, ddd) = civil_from_days(d);
                        acts.push(Action::Run("calendar.view", json!({"mode": "day"})));
                        acts.push(Action::Run("calendar.goto", json!({"date": format!("{y:04}-{mm:02}-{ddd:02}")})));
                    }
                }
            }
            if touch_input && bg.clicked() {
                let (y, mm, ddd) = civil_from_days(d);
                acts.push(Action::Run("calendar.view", json!({"mode": "day"})));
                acts.push(Action::Run("calendar.goto", json!({"date": format!("{y:04}-{mm:02}-{ddd:02}")})));
            } else if bg.double_clicked() {
                acts.push(Action::Dialog(Dialog::new_event(s, Some(s.local_midnight(d) + 9 * 3600))));
            }
        }
    }
    ui.allocate_rect(area, Sense::hover());
}

/// A new event at `at`, in calendar `cal` (account, calendar id) when it can take one.
fn new_event_in(s: &Session, at: i64, cal: Option<&(String, String, String, u8)>) -> Action {
    let mut d = Dialog::new_event(s, Some(at));
    if let (Dialog::Event(f), Some((account, id, _, _))) = (&mut d, cal)
        && let Some(i) = f.calendars.iter().position(|(a, c, _)| a == account && c == id)
    {
        f.calendar = i;
    }
    Action::Dialog(d)
}

/// The space between calendars shown side by side.
const PANE_GAP: f32 = 10.0;

#[allow(clippy::too_many_arguments)]
fn days(s: &Session, st: &mut UiState, ui: &mut Ui, items: &[AgendaItem], d0: i64, d1: i64, acts: &mut Vec<Action>) {
    let t = st.tokens;
    let n = (d1 - d0).max(1);
    if st.phone_mode && n > 1 {
        phone_agenda(s, &t, ui, items, d0, n, acts);
        return;
    }
    let gutter = 52.0;
    let now = today(s);
    let full = ui.available_rect_before_wrap();
    let mirror = crate::bidi::Mirror::new(full);
    // Side by side (Outlook's): each calendar shown a pane of its own, its days in it, under a
    // heading in its colour.
    let cals: Vec<(String, String, String, u8)> = if s.prefs.calendars_side_by_side { s.shown_calendars() } else { Vec::new() };
    let k = if cals.len() > 1 { cals.len() } else { 1 };
    let gaps = PANE_GAP * (k - 1) as f32;
    let pane = ((full.width() - gutter - 16.0 - gaps) / k as f32).max(40.0 * n as f32);
    let cw = pane / n as f32;
    let col_of = |it: &AgendaItem| if k > 1 { cals.iter().position(|(a, c, _, _)| *a == it.account && *c == it.calendar).unwrap_or(0) } else { 0 };
    // Where calendar `j`'s pane starts, and day `i`'s column in it (from the left edge).
    // Keep Monday as day zero, but put it at the visual end in a right-to-left calendar.
    let pane_x = |j: usize| mirror.x(full.min.x + gutter + (pane + PANE_GAP) * j as f32 + pane) - full.min.x;
    let day_x = |i: i64, j: usize| mirror.x(full.min.x + gutter + (pane + PANE_GAP) * j as f32 + cw * (i + 1) as f32) - full.min.x;
    let day_boundary = |i: i64, j: usize| mirror.x(full.min.x + gutter + (pane + PANE_GAP) * j as f32 + cw * i as f32) - full.min.x;
    let tint = |j: usize| cals.get(j).map(|(_, _, _, c)| color(*c, &t));
    // The calendars' headings: the name on its colour, × to hide it.
    if k > 1 {
        let (names, _) = ui.allocate_exact_size(vec2(full.width(), 24.0), Sense::hover());
        for (j, (account, id, name, c)) in cals.iter().enumerate() {
            let r = Rect::from_min_size(pos2(names.min.x + pane_x(j), names.min.y), vec2(pane, 24.0)).shrink2(vec2(0.0, 1.0));
            ui.painter().rect_filled(r, 3.0, color(*c, &t).gamma_multiply(if t.dark { 0.45 } else { 0.3 }));
            let mut label = if id == mailcraft_engine::model::pim::LOCAL { tl!("Calendar").to_string() } else { name.clone() };
            // Two accounts' "Calendar": which is whose.
            if cals.iter().filter(|(_, _, n, _)| n == name).count() > 1
                && let Some(email) = s.account(account).map(|a| a.config.email.clone())
            {
                label = format!("{label} · {email}");
            }
            let r_mirror = crate::bidi::Mirror::new(r);
            let label_clip = r_mirror.rect(r.shrink2(vec2(6.0, 0.0)).with_max_x(r.max.x - 24.0));
            ui.painter().with_clip_rect(label_clip).text(
                r_mirror.pos(pos2(r.min.x + 8.0, r.center().y)),
                r_mirror.align(Align2::LEFT_CENTER),
                label.clone(),
                theme::bold(12.0),
                t.text,
            );
            let x = Rect::from_center_size(r_mirror.pos(pos2(r.max.x - 12.0, r.center().y)), vec2(18.0, 18.0));
            let resp = ui.interact(x, ui.scope_id().with(("hide-calendar", account, id)), Sense::click());
            crate::widgets::named(ui, &resp, &format!("{label}: {}", tl!("Close")), true, None);
            if resp.hovered() {
                ui.painter().rect_filled(x, 3.0, t.hover);
            }
            icons::paint(ui.painter(), x.shrink(4.0), Icon::Close, if resp.hovered() { t.text } else { t.dim });
            if resp.on_hover_text(tf!("Hide {calendar}", calendar = label)).clicked() {
                acts.push(Action::Run("calendar.show", json!({"account": account, "calendar": id, "shown": false})));
            }
        }
    }
    // Day headings, in every pane.
    let (head, _) = ui.allocate_exact_size(vec2(full.width(), 40.0), Sense::hover());
    for j in 0..k {
        for i in 0..n {
            let d = d0 + i;
            let x = head.min.x + day_x(i, j);
            let (_, _, dd) = civil_from_days(d);
            let wd = DAYS.get(((d + 3).rem_euclid(7)) as usize).copied().unwrap_or("");
            ui.painter().text(
                pos2(x + cw / 2.0, head.min.y + 10.0),
                Align2::CENTER_CENTER,
                tl!(wd),
                theme::regular(12.0),
                if d == now { t.accent } else { t.dim },
            );
            let c = pos2(x + cw / 2.0, head.min.y + 28.0);
            if d == now {
                ui.painter().circle_filled(c, 12.0, t.accent);
            }
            ui.painter().text(c, Align2::CENTER_CENTER, dd.to_string(), theme::bold(15.0), if d == now { t.on_accent } else { t.text });
        }
    }
    // All-day events, across their days (in their calendar's pane).
    let all_day: Vec<&AgendaItem> = items.iter().filter(|i| i.all_day || i.end - i.start >= 86_400).collect();
    if !all_day.is_empty() {
        let mut lanes: Vec<Vec<(usize, i64, i64)>> = Vec::new();
        let mut placed: Vec<(usize, &AgendaItem)> = Vec::new();
        for it in &all_day {
            let (a, b) = (day_of(it.start, s).max(d0), day_of(it.end.max(it.start + 1) - 1, s).min(d1 - 1));
            let j = col_of(it);
            let lane = lanes.iter().position(|l| l.iter().all(|(pj, x, y)| *pj != j || b < *x || a > *y)).unwrap_or(lanes.len());
            if lane == lanes.len() {
                lanes.push(Vec::new());
            }
            if let Some(l) = lanes.get_mut(lane) {
                l.push((j, a, b));
            }
            placed.push((lane, it));
        }
        let (strip, _) = ui.allocate_exact_size(vec2(full.width(), 20.0 * lanes.len().min(4) as f32 + 6.0), Sense::hover());
        ui.painter().text(
            mirror.pos(pos2(strip.min.x + gutter - 6.0, strip.min.y + 10.0)),
            mirror.align(Align2::RIGHT_CENTER),
            tl!("All day"),
            theme::regular(11.0),
            t.faint,
        );
        for (lane, it) in placed.into_iter().filter(|(l, _)| *l < 4) {
            let (a, b) = (day_of(it.start, s).max(d0), day_of(it.end.max(it.start + 1) - 1, s).min(d1 - 1));
            let r = Rect::from_min_size(
                pos2(strip.min.x + day_x(b - d0, col_of(it)) + 2.0, strip.min.y + 3.0 + 20.0 * lane as f32),
                vec2(cw * (b - a + 1) as f32 - 4.0, 18.0),
            );
            if chip(ui, &t, r, it, &title_of(it)).clicked()
                && let Some(act) = open(s, it)
            {
                acts.push(act);
            }
        }
    }
    let line = ui.available_rect_before_wrap();
    ui.painter().line_segment([line.left_top(), line.right_top()], egui::Stroke::new(1.0, t.border));
    // The hours.
    let mut scroll = egui::ScrollArea::vertical().auto_shrink(false).id_salt(("calendar-hours", s.cal_mode == CalMode::Day));
    // Each view (day, week) opens on the working day the first time it's shown.
    let view = usize::from(s.cal_mode == CalMode::Day);
    if !st.cal_scrolled[view] {
        scroll = scroll.vertical_scroll_offset(HOUR * 7.5);
        st.cal_scrolled[view] = true;
    }
    scroll.show(ui, |ui| {
        let (grid, _) = ui.allocate_exact_size(vec2(ui.available_width(), HOUR * 24.0), Sense::hover());
        // Each calendar's pane faintly in its colour.
        for j in 0..k {
            if let Some(c) = tint(j).filter(|_| k > 1) {
                let r = Rect::from_min_size(pos2(grid.min.x + pane_x(j), grid.min.y), vec2(pane, grid.height()));
                ui.painter().rect_filled(r, 0.0, c.gamma_multiply(if t.dark { 0.1 } else { 0.07 }));
            }
        }
        for h in 0..24 {
            let y = grid.min.y + HOUR * h as f32;
            for j in 0..k {
                let x0 = grid.min.x + pane_x(j);
                ui.painter().line_segment([pos2(x0, y), pos2(x0 + pane, y)], egui::Stroke::new(1.0, t.border));
            }
            if h > 0 {
                // As the clock is set (issue #46): `13:00`, or `1 PM`.
                let label = crate::i18n::date(&mailcraft_engine::model::dates::hour_label(h, s.clock.style));
                ui.painter().text(
                    mirror.pos(pos2(grid.min.x + gutter - 6.0, y)),
                    mirror.align(Align2::RIGHT_CENTER),
                    label,
                    theme::regular(11.0),
                    t.faint,
                );
            }
        }
        for j in 0..k {
            for i in 0..=n {
                let x = grid.min.x + day_boundary(i, j);
                ui.painter().line_segment([pos2(x, grid.min.y), pos2(x, grid.max.y)], egui::Stroke::new(1.0, t.border));
            }
        }
        // Click an empty half hour to add an event there (in that pane's calendar).
        for j in 0..k {
            for i in 0..n {
                let col = Rect::from_min_size(pos2(grid.min.x + day_x(i, j), grid.min.y), vec2(cw, grid.height()));
                let r = ui.interact(col, ui.scope_id().with(("hours", d0 + i, j)), Sense::click());
                // The half hour under the pointer lights up: a click adds an event there.
                if let Some(p) = r.hover_pos() {
                    let half = ((p.y - grid.min.y) / (HOUR / 2.0)).floor().clamp(0.0, 47.0);
                    let slot = Rect::from_min_size(pos2(col.min.x, grid.min.y + half * HOUR / 2.0), vec2(cw, HOUR / 2.0)).shrink(1.0);
                    ui.painter().rect_filled(slot, 2.0, t.hover_row);
                }
                if r.clicked()
                    && let Some(p) = r.interact_pointer_pos()
                {
                    let half = ((p.y - grid.min.y) / (HOUR / 2.0)).floor().clamp(0.0, 47.0) as i64;
                    acts.push(new_event_in(s, s.local_midnight(d0 + i) + half * 1800, cals.get(j).filter(|_| k > 1)));
                }
            }
        }
        // Events, side by side when they overlap (each calendar in its own pane, side by side).
        for (i, j) in (0..n).flat_map(|i| (0..k).map(move |j| (i, j))) {
            let d = d0 + i;
            let (day0, day1) = (s.local_midnight(d), s.local_midnight(d + 1));
            let mut todays: Vec<&AgendaItem> = items
                .iter()
                .filter(|it| !it.all_day && it.end - it.start < 86_400 && it.start < day1 && it.end.max(it.start + 1) > day0 && col_of(it) == j)
                .collect();
            todays.sort_by_key(|it| (it.start, -(it.end - it.start)));
            let mut lane_end: Vec<i64> = Vec::new();
            let mut lanes: Vec<usize> = Vec::new();
            for it in &todays {
                let lane = lane_end.iter().position(|e| *e <= it.start).unwrap_or(lane_end.len());
                if lane == lane_end.len() {
                    lane_end.push(it.end);
                } else if let Some(e) = lane_end.get_mut(lane) {
                    *e = it.end;
                }
                lanes.push(lane);
            }
            let width = (cw - 6.0) / lane_end.len().max(1) as f32;
            for (it, lane) in todays.iter().zip(lanes) {
                let top = ((it.start.max(day0) - day0) as f32 / 3600.0) * HOUR;
                let bottom = ((it.end.min(day1) - day0) as f32 / 3600.0) * HOUR;
                let cell = Rect::from_min_size(pos2(grid.min.x + day_x(i, j), grid.min.y + top + 1.0), vec2(cw, (bottom - top - 2.0).max(18.0)));
                let r = crate::bidi::Mirror::new(cell)
                    .rect(Rect::from_min_size(pos2(cell.min.x + 3.0 + width * lane as f32, cell.min.y), vec2(width - 2.0, cell.height())));
                // The place beside the time when both fit on one line (a short event still shows it),
                // else on a line of its own.
                let mut text = format!("{}\n{} – {}", title_of(it), hm(it.start, s), hm(it.end, s));
                if !it.location.is_empty() {
                    let both = format!("{} – {} · {}", hm(it.start, s), hm(it.end, s), it.location);
                    let w = ui.fonts_mut(|f| f.layout_no_wrap(both, theme::regular(12.0), t.text).size().x);
                    text.push_str(if w <= r.width() - 8.0 { " · " } else { "\n" });
                    text.push_str(&it.location);
                }
                if chip(ui, &t, r, it, &text).clicked()
                    && let Some(a) = open(s, it)
                {
                    acts.push(a);
                }
            }
        }
        // Now, in every pane.
        let n_day = today(s);
        if n_day >= d0 && n_day < d1 {
            let y = grid.min.y + ((s.clock.now - s.local_midnight(n_day)) as f32 / 3600.0) * HOUR;
            for j in 0..k {
                let cell = Rect::from_min_size(pos2(grid.min.x + day_x(n_day - d0, j), grid.min.y), vec2(cw, grid.height()));
                let x0 = crate::bidi::Mirror::new(cell).pos(cell.left_center()).x;
                ui.painter().line_segment([pos2(cell.left(), y), pos2(cell.right(), y)], egui::Stroke::new(2.0, t.danger));
                ui.painter().circle_filled(pos2(x0, y), 4.0, t.danger);
            }
        }
    });
}

/// Week views on a phone read as a list of days instead of squeezing seven time grids into a
/// narrow screen. The day view keeps its timeline so the user can still place events by time.
fn phone_agenda(s: &Session, t: &Tokens, ui: &mut Ui, items: &[AgendaItem], first: i64, days: i64, acts: &mut Vec<Action>) {
    egui::ScrollArea::vertical().auto_shrink(false).id_salt(("phone-calendar-agenda", first, days)).show(ui, |ui| {
        for offset in 0..days {
            let day = first.saturating_add(offset);
            let (start, end) = (s.local_midnight(day), s.local_midnight(day.saturating_add(1)));
            ui.add_space(6.0);
            egui::Frame::new()
                .fill(t.surface)
                .stroke(egui::Stroke::new(1.0, t.border))
                .corner_radius(10)
                .inner_margin(Margin::symmetric(10, 8))
                .show(ui, |ui| {
                    ui.with_layout(crate::bidi::row_from_start(egui::Align::Center), |ui| {
                        let date_label = long_date(day);
                        let heading = RichText::new(&date_label).font(theme::bold(15.0)).color(if day == today(s) { t.accent } else { t.text });
                        let width = (ui.available_width() - 48.0).max(0.0);
                        let date_button = ui.add_sized(vec2(width, 44.0), egui::Button::new(heading).frame_when_inactive(false));
                        widgets::named(ui, &date_button, &date_label, true, None);
                        if date_button.clicked() {
                            let (year, month, date) = civil_from_days(day);
                            acts.push(Action::Run("calendar.goto", json!({"date": format!("{year:04}-{month:02}-{date:02}")})));
                            acts.push(Action::Run("calendar.view", json!({"mode": "day"})));
                        }
                        ui.with_layout(crate::bidi::row_from_end(egui::Align::Center), |ui| {
                            let add = ui.add_sized(vec2(44.0, 44.0), egui::Button::new(RichText::new("+").size(22.0)));
                            widgets::named(ui, &add, tl!("New event"), true, None);
                            if add.clicked() {
                                acts.push(Action::Dialog(Dialog::new_event(s, Some(start + 9 * 3600))));
                            }
                        });
                    });
                    let mut day_items: Vec<&AgendaItem> =
                        items.iter().filter(|item| item.start < end && item.end.max(item.start.saturating_add(1)) > start).collect();
                    day_items.sort_by_key(|item| (!(item.all_day || item.end.saturating_sub(item.start) >= 86_400), item.start, item.task));
                    for item in day_items {
                        let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 56.0), Sense::click());
                        let event_color = color(item.color, t);
                        ui.painter().rect(
                            rect,
                            7.0,
                            event_color.gamma_multiply(if t.dark { 0.28 } else { 0.12 }),
                            egui::Stroke::new(1.0, event_color.gamma_multiply(0.55)),
                            egui::StrokeKind::Inside,
                        );
                        let mirror = crate::bidi::Mirror::new(rect);
                        let all_day = item.all_day || item.end.saturating_sub(item.start) >= 86_400;
                        let time = if all_day { tl!("All day").to_string() } else { format!("{} – {}", hm(item.start, s), hm(item.end, s)) };
                        widgets::named(ui, &response, &title_of(item), true, None);
                        let x = mirror.x(rect.left() + 10.0);
                        ui.painter().with_clip_rect(rect.shrink(8.0)).text(
                            pos2(x, rect.center().y - 8.0),
                            mirror.align(Align2::LEFT_CENTER),
                            time,
                            theme::regular(11.0),
                            t.dim,
                        );
                        ui.painter().with_clip_rect(rect.shrink(8.0)).text(
                            pos2(x, rect.center().y + 10.0),
                            mirror.align(Align2::LEFT_CENTER),
                            title_of(item),
                            theme::bold(13.0),
                            t.text,
                        );
                        if response.clicked()
                            && let Some(action) = open(s, item)
                        {
                            acts.push(action);
                        }
                    }
                });
        }
    });
}
