//! The day and week views: the calendars' headings when they're side by side, the days'
//! headings, the all-day strip, and the hours with their events and the line at now.

use super::*;

/// The space between calendars shown side by side.
const PANE_GAP: f32 = 10.0;

/// How the day and week views are laid out across: the hours' gutter, then a pane per calendar
/// shown side by side (one otherwise), a column per day in each.
struct Columns {
    /// The first day and the day after the last.
    d0: i64,
    d1: i64,
    /// Days, and panes.
    n: i64,
    k: usize,
    gutter: f32,
    pane: f32,
    /// A day's column's width.
    cw: f32,
    /// The calendars side by side (account, id, name, colour), when there are several.
    cals: Vec<(String, String, String, u8)>,
}

impl Columns {
    /// Where calendar `j`'s pane starts (from the left edge).
    fn pane_x(&self, j: usize) -> f32 {
        self.gutter + (self.pane + PANE_GAP) * j as f32
    }

    /// Where day `i`'s column starts in calendar `j`'s pane.
    fn day_x(&self, i: i64, j: usize) -> f32 {
        self.pane_x(j) + self.cw * i as f32
    }

    /// The pane an event goes in.
    fn col_of(&self, it: &AgendaItem) -> usize {
        if self.k > 1 { self.cals.iter().position(|(a, c, _, _)| *a == it.account && *c == it.calendar).unwrap_or(0) } else { 0 }
    }

    /// Calendar `j`'s colour.
    fn tint(&self, j: usize, t: &Tokens) -> Option<Color32> {
        self.cals.get(j).map(|(_, _, _, c)| color(*c, t))
    }
}

pub(super) fn days(s: &Session, st: &mut UiState, ui: &mut Ui, items: &[AgendaItem], d0: i64, d1: i64, acts: &mut Vec<Action>) {
    let t = st.tokens;
    let n = (d1 - d0).max(1);
    // Wide enough for "All day" in the language shown ("Toute la journée" ran out of it, under
    // the folder pane's edge).
    let all_day = ui.painter().layout_no_wrap(tl!("All day").to_string(), theme::regular(11.0), t.faint).size().x;
    let gutter = (all_day + 14.0).clamp(52.0, 110.0);
    let now = today(s);
    let full = ui.available_rect_before_wrap();
    // Side by side (Outlook's): each calendar shown a pane of its own, its days in it, under a
    // heading in its colour.
    let cals: Vec<(String, String, String, u8)> = if s.prefs.calendars_side_by_side { s.shown_calendars() } else { Vec::new() };
    let k = if cals.len() > 1 { cals.len() } else { 1 };
    let gaps = PANE_GAP * (k - 1) as f32;
    let pane = ((full.width() - gutter - 16.0 - gaps) / k as f32).max(40.0 * n as f32);
    let cw = pane / n as f32;
    let cols = Columns { d0, d1, n, k, gutter, pane, cw, cals };
    if k > 1 {
        calendar_headings(s, &t, ui, &cols, full.width(), acts);
    }
    day_headings(&t, ui, &cols, full.width(), now);
    all_day_strip(s, &t, ui, items, &cols, full.width(), acts);
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
        hour_lines(s, &t, ui, &cols, grid);
        new_event_slots(s, &t, ui, &cols, grid, acts);
        timed_events(s, &t, ui, items, &cols, grid, acts);
        now_line(s, &t, ui, &cols, grid);
    });
}

/// The calendars' headings, side by side: the name on its colour, × to hide it.
fn calendar_headings(s: &Session, t: &Tokens, ui: &mut Ui, cols: &Columns, width: f32, acts: &mut Vec<Action>) {
    let (names, _) = ui.allocate_exact_size(vec2(width, 24.0), Sense::hover());
    for (j, (account, id, name, c)) in cols.cals.iter().enumerate() {
        let r = Rect::from_min_size(pos2(names.min.x + cols.pane_x(j), names.min.y), vec2(cols.pane, 24.0)).shrink2(vec2(0.0, 1.0));
        ui.painter().rect_filled(r, 3.0, color(*c, t).gamma_multiply(if t.dark { 0.45 } else { 0.3 }));
        let mut label = calendar_name(id, name);
        // Two accounts' "Calendar": which is whose.
        if cols.cals.iter().filter(|(_, _, n, _)| n == name).count() > 1
            && let Some(email) = s.account(account).map(|a| a.config.email.clone())
        {
            label = format!("{label} · {email}");
        }
        ui.painter().with_clip_rect(r.shrink2(vec2(6.0, 0.0)).with_max_x(r.max.x - 24.0)).text(
            pos2(r.min.x + 8.0, r.center().y),
            Align2::LEFT_CENTER,
            label.clone(),
            theme::bold(12.0),
            t.text,
        );
        let x = Rect::from_center_size(pos2(r.max.x - 12.0, r.center().y), vec2(18.0, 18.0));
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

/// Day headings, in every pane: the weekday and the day, today's on the accent.
fn day_headings(t: &Tokens, ui: &mut Ui, cols: &Columns, width: f32, now: i64) {
    let (head, _) = ui.allocate_exact_size(vec2(width, 40.0), Sense::hover());
    for j in 0..cols.k {
        for i in 0..cols.n {
            let d = cols.d0 + i;
            let x = head.min.x + cols.day_x(i, j);
            let (_, _, dd) = civil_from_days(d);
            let wd = dates::weekday_short(d);
            ui.painter().text(
                pos2(x + cols.cw / 2.0, head.min.y + 10.0),
                Align2::CENTER_CENTER,
                tl!(wd),
                theme::regular(12.0),
                if d == now { t.accent } else { t.dim },
            );
            let c = pos2(x + cols.cw / 2.0, head.min.y + 28.0);
            if d == now {
                ui.painter().circle_filled(c, 12.0, t.accent);
            }
            ui.painter().text(c, Align2::CENTER_CENTER, dd.to_string(), theme::bold(15.0), if d == now { t.on_accent } else { t.text });
        }
    }
}

/// All-day events, across their days (in their calendar's pane), four lanes at most.
fn all_day_strip(s: &Session, t: &Tokens, ui: &mut Ui, items: &[AgendaItem], cols: &Columns, width: f32, acts: &mut Vec<Action>) {
    let (d0, d1) = (cols.d0, cols.d1);
    let all_day: Vec<&AgendaItem> = items.iter().filter(|i| i.all_day || i.end - i.start >= 86_400).collect();
    if all_day.is_empty() {
        return;
    }
    let mut lanes: Vec<Vec<(usize, i64, i64)>> = Vec::new();
    let mut placed: Vec<(usize, &AgendaItem)> = Vec::new();
    for it in &all_day {
        let (a, b) = (day_of(it.start, s).max(d0), day_of(it.end.max(it.start + 1) - 1, s).min(d1 - 1));
        let j = cols.col_of(it);
        let lane = lanes.iter().position(|l| l.iter().all(|(pj, x, y)| *pj != j || b < *x || a > *y)).unwrap_or(lanes.len());
        if lane == lanes.len() {
            lanes.push(Vec::new());
        }
        if let Some(l) = lanes.get_mut(lane) {
            l.push((j, a, b));
        }
        placed.push((lane, it));
    }
    let (strip, _) = ui.allocate_exact_size(vec2(width, 20.0 * lanes.len().min(4) as f32 + 6.0), Sense::hover());
    ui.painter().text(pos2(strip.min.x + cols.gutter - 6.0, strip.min.y + 10.0), Align2::RIGHT_CENTER, tl!("All day"), theme::regular(11.0), t.faint);
    for (lane, it) in placed.into_iter().filter(|(l, _)| *l < 4) {
        let (a, b) = (day_of(it.start, s).max(d0), day_of(it.end.max(it.start + 1) - 1, s).min(d1 - 1));
        let r = Rect::from_min_size(
            pos2(strip.min.x + cols.day_x(a - d0, cols.col_of(it)) + 2.0, strip.min.y + 3.0 + 20.0 * lane as f32),
            vec2(cols.cw * (b - a + 1) as f32 - 4.0, 18.0),
        );
        if chip(ui, t, r, it, &title_of(it)).clicked()
            && let Some(act) = open(s, it)
        {
            acts.push(act);
        }
    }
}

/// Each calendar's pane faintly in its colour, the hours' lines and names, the days' lines.
fn hour_lines(s: &Session, t: &Tokens, ui: &mut Ui, cols: &Columns, grid: Rect) {
    let k = cols.k;
    // Each calendar's pane faintly in its colour.
    for j in 0..k {
        if let Some(c) = cols.tint(j, t).filter(|_| k > 1) {
            let r = Rect::from_min_size(pos2(grid.min.x + cols.pane_x(j), grid.min.y), vec2(cols.pane, grid.height()));
            ui.painter().rect_filled(r, 0.0, c.gamma_multiply(if t.dark { 0.1 } else { 0.07 }));
        }
    }
    for h in 0..24 {
        let y = grid.min.y + HOUR * h as f32;
        for j in 0..k {
            let x0 = grid.min.x + cols.pane_x(j);
            ui.painter().line_segment([pos2(x0, y), pos2(x0 + cols.pane, y)], egui::Stroke::new(1.0, t.border));
        }
        if h > 0 {
            // As the clock is set (issue #46): `13:00`, or `1 PM`.
            let label = crate::i18n::date(&mailcraft_engine::model::dates::hour_label(h, s.clock.style));
            ui.painter().text(pos2(grid.min.x + cols.gutter - 6.0, y), Align2::RIGHT_CENTER, label, theme::regular(11.0), t.faint);
        }
    }
    for j in 0..k {
        for i in 0..=cols.n {
            let x = grid.min.x + cols.day_x(i, j);
            ui.painter().line_segment([pos2(x, grid.min.y), pos2(x, grid.max.y)], egui::Stroke::new(1.0, t.border));
        }
    }
}

/// Click an empty half hour to add an event there (in that pane's calendar).
fn new_event_slots(s: &Session, t: &Tokens, ui: &mut Ui, cols: &Columns, grid: Rect, acts: &mut Vec<Action>) {
    let (k, cw) = (cols.k, cols.cw);
    for j in 0..k {
        for i in 0..cols.n {
            let col = Rect::from_min_size(pos2(grid.min.x + cols.day_x(i, j), grid.min.y), vec2(cw, grid.height()));
            let r = ui.interact(col, ui.scope_id().with(("hours", cols.d0 + i, j)), Sense::click());
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
                acts.push(new_event_in(s, s.local_midnight(cols.d0 + i) + half * 1800, cols.cals.get(j).filter(|_| k > 1)));
            }
        }
    }
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

/// Events, side by side when they overlap (each calendar in its own pane, side by side).
fn timed_events(s: &Session, t: &Tokens, ui: &mut Ui, items: &[AgendaItem], cols: &Columns, grid: Rect, acts: &mut Vec<Action>) {
    let cw = cols.cw;
    for (i, j) in (0..cols.n).flat_map(|i| (0..cols.k).map(move |j| (i, j))) {
        let d = cols.d0 + i;
        let (day0, day1) = (s.local_midnight(d), s.local_midnight(d + 1));
        let mut todays: Vec<&AgendaItem> = items
            .iter()
            .filter(|it| !it.all_day && it.end - it.start < 86_400 && it.start < day1 && it.end.max(it.start + 1) > day0 && cols.col_of(it) == j)
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
            let r = Rect::from_min_size(
                pos2(grid.min.x + cols.day_x(i, j) + 3.0 + width * lane as f32, grid.min.y + top + 1.0),
                vec2(width - 2.0, (bottom - top - 2.0).max(18.0)),
            );
            // The place beside the time when both fit on one line (a short event still shows it),
            // else on a line of its own.
            let mut text = format!("{}\n{} – {}", title_of(it), hm(it.start, s), hm(it.end, s));
            if !it.location.is_empty() {
                let both = format!("{} – {} · {}", hm(it.start, s), hm(it.end, s), it.location);
                let w = ui.fonts_mut(|f| f.layout_no_wrap(both, theme::regular(12.0), t.text).size().x);
                text.push_str(if w <= r.width() - 8.0 { " · " } else { "\n" });
                text.push_str(&it.location);
            }
            if chip(ui, t, r, it, &text).clicked()
                && let Some(a) = open(s, it)
            {
                acts.push(a);
            }
        }
    }
}

/// Now, in every pane: a line across today's column.
fn now_line(s: &Session, t: &Tokens, ui: &mut Ui, cols: &Columns, grid: Rect) {
    let n_day = today(s);
    if n_day >= cols.d0 && n_day < cols.d1 {
        let y = grid.min.y + ((s.clock.now - s.local_midnight(n_day)) as f32 / 3600.0) * HOUR;
        for j in 0..cols.k {
            let x0 = grid.min.x + cols.day_x(n_day - cols.d0, j);
            ui.painter().line_segment([pos2(x0, y), pos2(x0 + cols.cw, y)], egui::Stroke::new(2.0, t.danger));
            ui.painter().circle_filled(pos2(x0, y), 4.0, t.danger);
        }
    }
}
