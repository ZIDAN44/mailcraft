//! The Calendar: a month to pick days from and the calendars in the side pane; a day, a week
//! or a month in the main one. Click an empty hour to add an event there, an event to open it.

use egui::{Align2, Color32, Margin, Rect, RichText, Sense, Ui, pos2, vec2};
use mailcraft_engine::model::dates::{self, civil_from_days, days_from_civil};
use mailcraft_engine::{AgendaItem, CalMode, Session};
use serde_json::json;

use crate::icons::Icon;
use crate::theme::{self, Tokens};
use crate::{Action, Dialog, UiState, widgets};

/// Height of an hour in the day and week views.
const HOUR: f32 = 46.0;
const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const DAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/// A calendar's colour (the first one is the app's).
pub fn color(i: u8, t: &Tokens) -> Color32 {
    if i == 0 { t.accent } else { theme::category_color(i.saturating_sub(1), t.dark) }
}

fn today(s: &Session) -> i64 {
    s.clock.now.saturating_add(i64::from(s.clock.offset)).div_euclid(86_400)
}

/// `HH:MM`, local.
pub fn hm(t: i64, s: &Session) -> String {
    let c = dates::civil(t, &s.clock);
    format!("{:02}:{:02}", c.hour, c.minute)
}

/// The local day a time falls on.
fn day_of(t: i64, s: &Session) -> i64 {
    t.saturating_add(i64::from(s.clock.offset)).div_euclid(86_400)
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
        CalMode::Week => {
            let ((y0, m0, a), (y1, m1, b)) = (civil_from_days(d0), civil_from_days(d1 - 1));
            if (y0, m0) == (y1, m1) {
                tf!("{from} – {to} {month} {year}", from = a, to = b, month = month(m1), year = y1)
            } else {
                tf!("{from} {month_from} – {to} {month} {year}", from = a, month_from = month(m0), to = b, month = month(m1), year = y1)
            }
        }
        CalMode::Day => {
            let (y, m, d) = civil_from_days(d0);
            let wd = (d0 + 3).rem_euclid(7) as usize;
            let day_name = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"].get(wd).copied().unwrap_or("");
            tf!("{weekday}, {day} {month} {year}", weekday = tl!(day_name), day = d, month = month(m), year = y)
        }
    }
}

/// The side pane: a month to jump around in, then the calendars.
pub fn nav(s: &Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    let t = st.tokens;
    egui::ScrollArea::vertical().auto_shrink(false).id_salt("calendar-nav").show(ui, |ui| {
        if widgets::accent_button(ui, Icon::Plus, "New event", &t, !s.accounts.is_empty()).clicked() {
            acts.push(Action::Dialog(Dialog::new_event(s, None)));
        }
        ui.add_space(10.0);
        mini_month(s, st, ui, acts);
        ui.add_space(12.0);
        ui.label(RichText::new(tl!("Calendars")).font(theme::bold(12.0)).color(t.faint));
        for a in &s.accounts {
            let Some(p) = s.pim.get(&a.config.id) else { continue };
            for c in &p.calendars {
                let shared = c.id.strip_prefix("shared:").map(str::to_string);
                let row = ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
                    ui.painter().rect_filled(r, 3.0, color(c.color, &t));
                    let name = if c.id == mailcraft_engine::model::pim::LOCAL { tl!("Calendar").to_string() } else { c.name.clone() };
                    let tip = if shared.is_some() {
                        tf!("Shared with you, opened from {account}", account = a.config.email)
                    } else {
                        a.config.email.clone()
                    };
                    ui.label(RichText::new(name).color(t.text)).on_hover_text(tip);
                    if c.read_only {
                        ui.label(RichText::new(tl!("read-only")).color(t.faint).small());
                    }
                });
                // Someone else's calendar: right-click closes it.
                if let Some(email) = shared {
                    row.response.interact(Sense::click()).context_menu(|ui| {
                        if ui.button(tl!("Remove this calendar")).clicked() {
                            acts.push(Action::Run("calendar.closeShared", json!({"email": email})));
                            ui.close();
                        }
                    });
                }
            }
        }
        // Outlook's Open Calendar: someone's calendar shared with an Exchange account.
        if s.accounts.iter().any(|a| a.config.graph.is_some() || a.config.ews.is_some()) {
            ui.add_space(6.0);
            if ui.link(RichText::new(tl!("Open shared calendar…")).small()).clicked() {
                acts.push(Action::Dialog(Dialog::OpenSharedCalendar { email: String::new(), error: None }));
            }
        }
    });
}

/// A small month: click a day to show it.
fn mini_month(s: &Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    let t = st.tokens;
    let shown = st.mini_month.unwrap_or_else(|| {
        let (y, m, _) = civil_from_days(s.cal_day());
        y * 12 + i64::from(m) - 1
    });
    let (y, m) = (shown.div_euclid(12), u32::try_from(shown.rem_euclid(12) + 1).unwrap_or(1));
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(tf!("{month} {year}", month = crate::i18n::tc("month", MONTHS.get(m as usize - 1).copied().unwrap_or("")), year = y))
                .font(theme::bold(13.0))
                .color(t.text),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if widgets::tiny_icon_button(ui, Icon::ChevronRight, "Next month", &t).clicked() {
                st.mini_month = Some(shown + 1);
            }
            if widgets::tiny_icon_button(ui, Icon::ChevronLeft, "Previous month", &t).clicked() {
                st.mini_month = Some(shown - 1);
            }
        });
    });
    let w = ((ui.available_width() - 4.0) / 7.0).clamp(20.0, 34.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for d in DAYS {
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
    let first = days_from_civil(y, m, 1);
    let start = first - (first + 3).rem_euclid(7);
    let (sel0, sel1) = s.cal_days();
    let now = today(s);
    for week in 0..6 {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            for i in 0..7 {
                let d = start + week * 7 + i;
                let (r, resp) = ui.allocate_exact_size(vec2(w, 22.0), Sense::click());
                let (_, dm, dd) = civil_from_days(d);
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
            ui.label(RichText::new(title(s)).font(theme::bold(18.0)).color(t.text));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                for (mode, label, code) in [(CalMode::Month, "Month", "month"), (CalMode::Week, "Week", "week"), (CalMode::Day, "Day", "day")] {
                    if ui.selectable_label(s.cal_mode == mode, tl!(label)).clicked() {
                        acts.push(Action::Run("calendar.view", json!({"mode": code})));
                    }
                }
            });
        });
    });
    let (d0, d1) = s.cal_days();
    let items = s.agenda(s.local_midnight(d0), s.local_midnight(d1));
    match s.cal_mode {
        CalMode::Month => month(s, &t, ui, &items, d0, acts),
        CalMode::Day | CalMode::Week => days(s, st, ui, &items, d0, d1, acts),
    }
}

/// Open an item (its event).
fn open(s: &Session, item: &AgendaItem) -> Option<Action> {
    Dialog::open_event(s, &item.uid, item.occurrence).map(Action::Dialog)
}

fn chip(ui: &mut Ui, t: &Tokens, rect: Rect, item: &AgendaItem, text: &str) -> egui::Response {
    let resp = ui.interact(rect, ui.id().with(("chip", &item.uid, item.start)), Sense::click());
    let c = color(item.color, t);
    let tentative = item.needs_reply || item.status == "TENTATIVE";
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
    ui.painter().rect_filled(Rect::from_min_size(rect.min, vec2(3.0, rect.height())), 2.0, c);
    // As many whole lines as the box holds, the last one ending in "…" when more is left (a line
    // cut in half by the box's edge read badly).
    let font = theme::regular(12.0);
    let row = ui.fonts_mut(|f| f.row_height(&font)).max(1.0);
    let mut job = egui::text::LayoutJob::simple(text.to_string(), font, t.text, (rect.width() - 8.0).max(4.0));
    job.wrap.max_rows = ((rect.height() - 4.0) / row).floor().max(1.0) as usize;
    job.wrap.overflow_character = Some('…');
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    ui.painter().with_clip_rect(rect.shrink(1.0)).galley(rect.min + vec2(6.0, 2.0), galley, t.text);
    resp
}

fn month(s: &Session, t: &Tokens, ui: &mut Ui, items: &[AgendaItem], d0: i64, acts: &mut Vec<Action>) {
    let (_, shown_month, _) = civil_from_days(s.cal_day());
    let now = today(s);
    let area = ui.available_rect_before_wrap().shrink2(vec2(10.0, 4.0));
    let head = 22.0;
    let cw = area.width() / 7.0;
    let rh = ((area.height() - head) / 6.0).max(60.0);
    for (i, d) in DAYS.iter().enumerate() {
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
            let bg = ui.interact(r, ui.id().with(("month-day", d)), Sense::click());
            let todays: Vec<&AgendaItem> =
                items.iter().filter(|it| day_of(it.start, s) <= d && day_of(it.end.max(it.start + 1) - 1, s) >= d).collect();
            let fit = ((rh - 26.0) / 19.0).floor().max(0.0) as usize;
            let shown = if todays.len() > fit { fit.saturating_sub(1) } else { todays.len() };
            for (k, it) in todays.iter().take(shown).enumerate() {
                let cr = Rect::from_min_size(r.min + vec2(3.0, 24.0 + 19.0 * k as f32), vec2(cw - 6.0, 17.0));
                let text = if it.all_day { it.summary.clone() } else { format!("{} {}", hm(it.start, s), it.summary) };
                if chip(ui, t, cr, it, &text).clicked()
                    && let Some(a) = open(s, it)
                {
                    acts.push(a);
                }
            }
            if todays.len() > shown {
                let more = ui.put(
                    Rect::from_min_size(r.min + vec2(6.0, 24.0 + 19.0 * shown as f32), vec2(cw - 12.0, 17.0)),
                    egui::Link::new(RichText::new(tf!("{n} more", n = todays.len() - shown)).small()),
                );
                if more.clicked() {
                    let (y, mm, ddd) = civil_from_days(d);
                    acts.push(Action::Run("calendar.view", json!({"mode": "day"})));
                    acts.push(Action::Run("calendar.goto", json!({"date": format!("{y:04}-{mm:02}-{ddd:02}")})));
                }
            }
            if bg.double_clicked() {
                acts.push(Action::Dialog(Dialog::new_event(s, Some(s.local_midnight(d) + 9 * 3600))));
            }
        }
    }
    ui.allocate_rect(area, Sense::hover());
}

#[allow(clippy::too_many_arguments)]
fn days(s: &Session, st: &mut UiState, ui: &mut Ui, items: &[AgendaItem], d0: i64, d1: i64, acts: &mut Vec<Action>) {
    let t = st.tokens;
    let n = (d1 - d0).max(1);
    let gutter = 52.0;
    let now = today(s);
    let full = ui.available_rect_before_wrap();
    let cw = ((full.width() - gutter - 16.0) / n as f32).max(40.0);
    // Day headings.
    let (head, _) = ui.allocate_exact_size(vec2(full.width(), 40.0), Sense::hover());
    for i in 0..n {
        let d = d0 + i;
        let x = head.min.x + gutter + cw * i as f32;
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
    // All-day events.
    let all_day: Vec<&AgendaItem> = items.iter().filter(|i| i.all_day || i.end - i.start >= 86_400).collect();
    if !all_day.is_empty() {
        let mut lanes: Vec<Vec<(i64, i64)>> = Vec::new();
        let mut placed: Vec<(usize, &AgendaItem)> = Vec::new();
        for it in &all_day {
            let (a, b) = (day_of(it.start, s).max(d0), day_of(it.end.max(it.start + 1) - 1, s).min(d1 - 1));
            let lane = lanes.iter().position(|l| l.iter().all(|(x, y)| b < *x || a > *y)).unwrap_or(lanes.len());
            if lane == lanes.len() {
                lanes.push(Vec::new());
            }
            if let Some(l) = lanes.get_mut(lane) {
                l.push((a, b));
            }
            placed.push((lane, it));
        }
        let (strip, _) = ui.allocate_exact_size(vec2(full.width(), 20.0 * lanes.len().min(4) as f32 + 6.0), Sense::hover());
        ui.painter().text(pos2(strip.min.x + gutter - 6.0, strip.min.y + 10.0), Align2::RIGHT_CENTER, tl!("All day"), theme::regular(11.0), t.faint);
        for (lane, it) in placed.into_iter().filter(|(l, _)| *l < 4) {
            let (a, b) = (day_of(it.start, s).max(d0), day_of(it.end.max(it.start + 1) - 1, s).min(d1 - 1));
            let r = Rect::from_min_size(
                pos2(strip.min.x + gutter + cw * (a - d0) as f32 + 2.0, strip.min.y + 3.0 + 20.0 * lane as f32),
                vec2(cw * (b - a + 1) as f32 - 4.0, 18.0),
            );
            if chip(ui, &t, r, it, &it.summary).clicked()
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
        for h in 0..24 {
            let y = grid.min.y + HOUR * h as f32;
            ui.painter().line_segment([pos2(grid.min.x + gutter, y), pos2(grid.max.x, y)], egui::Stroke::new(1.0, t.border));
            if h > 0 {
                ui.painter().text(pos2(grid.min.x + gutter - 6.0, y), Align2::RIGHT_CENTER, format!("{h:02}:00"), theme::regular(11.0), t.faint);
            }
        }
        for i in 0..=n {
            let x = grid.min.x + gutter + cw * i as f32;
            ui.painter().line_segment([pos2(x, grid.min.y), pos2(x, grid.max.y)], egui::Stroke::new(1.0, t.border));
        }
        // Click an empty half hour to add an event there.
        for i in 0..n {
            let col = Rect::from_min_size(pos2(grid.min.x + gutter + cw * i as f32, grid.min.y), vec2(cw, grid.height()));
            let r = ui.interact(col, ui.id().with(("hours", d0 + i)), Sense::click());
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
                acts.push(Action::Dialog(Dialog::new_event(s, Some(s.local_midnight(d0 + i) + half * 1800))));
            }
        }
        // Events, side by side when they overlap.
        for i in 0..n {
            let d = d0 + i;
            let (day0, day1) = (s.local_midnight(d), s.local_midnight(d + 1));
            let mut todays: Vec<&AgendaItem> =
                items.iter().filter(|it| !it.all_day && it.end - it.start < 86_400 && it.start < day1 && it.end.max(it.start + 1) > day0).collect();
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
                let r = Rect::from_min_size(
                    pos2(grid.min.x + gutter + cw * i as f32 + 3.0 + width * lane as f32, grid.min.y + top + 1.0),
                    vec2(width - 2.0, (bottom - top - 2.0).max(18.0)),
                );
                // The place beside the time when both fit on one line (a short event still shows it),
                // else on a line of its own.
                let mut text = format!("{}\n{} – {}", it.summary, hm(it.start, s), hm(it.end, s));
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
        // Now.
        let n_day = today(s);
        if n_day >= d0 && n_day < d1 {
            let y = grid.min.y + ((s.clock.now - s.local_midnight(n_day)) as f32 / 3600.0) * HOUR;
            let x0 = grid.min.x + gutter + cw * (n_day - d0) as f32;
            ui.painter().line_segment([pos2(x0, y), pos2(x0 + cw, y)], egui::Stroke::new(2.0, t.danger));
            ui.painter().circle_filled(pos2(x0, y), 4.0, t.danger);
        }
    });
}
