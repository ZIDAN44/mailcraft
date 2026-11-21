//! The day and week views: the calendars' headings when they're side by side, the days'
//! headings, the all-day strip, and the hours with their events and the line at now; the hours
//! not worked shaded (issue #185).

use super::*;

/// The space between calendars shown side by side.
const PANE_GAP: f32 = 10.0;

/// How the day and week views are laid out across: the hours' gutter, then a pane per calendar
/// shown side by side (one otherwise), a column per day in each.
struct Columns {
    /// The days, in order: the work week's may skip some (issue #185).
    days: Vec<i64>,
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
    /// Column `i`'s day.
    fn day(&self, i: i64) -> i64 {
        usize::try_from(i).ok().and_then(|i| self.days.get(i)).copied().unwrap_or_default()
    }

    /// The columns from the first on or after day `a` to the last on or before day `b`.
    fn columns(&self, a: i64, b: i64) -> Option<(i64, i64)> {
        let first = self.days.iter().position(|d| *d >= a)?;
        let last = self.days.iter().rposition(|d| *d <= b)?;
        (first <= last).then(|| (i64::try_from(first).unwrap_or_default(), i64::try_from(last).unwrap_or_default()))
    }

    /// Where calendar `j`'s pane starts (from the left edge).
    fn pane_x(&self, j: usize) -> f32 {
        self.gutter + (self.pane + PANE_GAP) * j as f32
    }

    /// Where day `i`'s column starts in calendar `j`'s pane.
    fn day_x(&self, i: i64, j: usize) -> f32 {
        self.pane_x(j) + self.cw * i as f32
    }

    /// The day's column at `x` (from the left edge), in whichever calendar's pane it is: the
    /// first or last past the ends. An event dragged into another calendar's pane goes to that
    /// day there, still in its own calendar.
    fn column_at(&self, x: f32) -> i64 {
        let pane = (((x - self.gutter) / (self.pane + PANE_GAP).max(1.0)).floor() as i64).clamp(0, self.k.saturating_sub(1) as i64);
        let j = usize::try_from(pane).unwrap_or(0);
        (((x - self.pane_x(j)) / self.cw.max(1.0)).floor() as i64).clamp(0, (self.n - 1).max(0))
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

pub(super) fn days(s: &Session, st: &mut UiState, ui: &mut Ui, items: &[AgendaItem], days: Vec<i64>, acts: &mut Vec<Action>) {
    let t = st.tokens;
    let n = i64::try_from(days.len()).unwrap_or(1).max(1);
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
    let cols = Columns { days, n, k, gutter, pane, cw, cals };
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
        free_slot(s, &t, ui, &cols, grid, acts);
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
            let d = cols.day(i);
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
    // Each across the columns of its days (none when they're all days the work week skips).
    let spread = |it: &AgendaItem| cols.columns(day_of(it.start, s), day_of(it.end.max(it.start + 1) - 1, s));
    let all_day: Vec<(&AgendaItem, (i64, i64))> =
        items.iter().filter(|i| i.all_day || i.end - i.start >= 86_400).filter_map(|i| Some((i, spread(i)?))).collect();
    if all_day.is_empty() {
        return;
    }
    let mut lanes: Vec<Vec<(usize, i64, i64)>> = Vec::new();
    let mut placed: Vec<(usize, &AgendaItem, (i64, i64))> = Vec::new();
    for (it, (a, b)) in all_day {
        let j = cols.col_of(it);
        let lane = lanes.iter().position(|l| l.iter().all(|(pj, x, y)| *pj != j || b < *x || a > *y)).unwrap_or(lanes.len());
        if lane == lanes.len() {
            lanes.push(Vec::new());
        }
        if let Some(l) = lanes.get_mut(lane) {
            l.push((j, a, b));
        }
        placed.push((lane, it, (a, b)));
    }
    let (strip, _) = ui.allocate_exact_size(vec2(width, 20.0 * lanes.len().min(4) as f32 + 6.0), Sense::hover());
    ui.painter().text(pos2(strip.min.x + cols.gutter - 6.0, strip.min.y + 10.0), Align2::RIGHT_CENTER, tl!("All day"), theme::regular(11.0), t.faint);
    for (lane, it, (a, b)) in placed.into_iter().filter(|(l, ..)| *l < 4) {
        let j = cols.col_of(it);
        let r = Rect::from_min_size(
            pos2(strip.min.x + cols.day_x(a, j) + 2.0, strip.min.y + 3.0 + 20.0 * lane as f32),
            vec2(cols.cw * (b - a + 1) as f32 - 4.0, 18.0),
        );
        let resp = chip(ui, t, r, it, &title_of(it), s.movable(it));
        if resp.clicked()
            && let Some(act) = open(s, it)
        {
            acts.push(act);
        }
        // Dragged along the days (issue #192): by as many days as its first column went.
        if let Some(drag) = moving::dragged(ui, &resp, r) {
            let to = cols.column_at(drag.top_left.x + cols.cw / 2.0 - strip.min.x);
            let by_days = cols.day(to) - cols.day(a);
            let by = if it.all_day { by_days * 86_400 } else { s.local_midnight(cols.day(to)) - s.local_midnight(cols.day(a)) };
            let ghost = r.translate(vec2(cols.cw * (to - a) as f32, 0.0));
            let day = crate::i18n::date(&dates::day_short(day_of(it.start, s) + by_days, &s.clock));
            moving::ghost(ui, t, ghost, &format!("{} · {day}", title_of(it)));
            if drag.done
                && let Some(act) = moving::moved(it, by)
            {
                acts.push(act);
            }
        }
    }
}

/// Each calendar's pane faintly in its colour, the hours not worked shaded, the hours' lines and
/// names, the days' lines.
fn hour_lines(s: &Session, t: &Tokens, ui: &mut Ui, cols: &Columns, grid: Rect) {
    let k = cols.k;
    // Each calendar's pane faintly in its colour.
    for j in 0..k {
        if let Some(c) = cols.tint(j, t).filter(|_| k > 1) {
            let r = Rect::from_min_size(pos2(grid.min.x + cols.pane_x(j), grid.min.y), vec2(cols.pane, grid.height()));
            ui.painter().rect_filled(r, 0.0, c.gamma_multiply(if t.dark { 0.1 } else { 0.07 }));
        }
    }
    // The hours not worked (issue #185): the user's own, or in a colleague's pane theirs, as
    // their server gave them.
    for (i, j) in (0..cols.n).flat_map(|i| (0..k).map(move |j| (i, j))) {
        let d = cols.day(i);
        let (day0, day1) = (s.local_midnight(d), s.local_midnight(d + 1));
        let work = match cols.cals.get(j).filter(|_| k > 1) {
            Some((account, id, ..)) => s.calendar_work_spans(account, id, day0, day1),
            None => s.work_spans(day0, day1),
        };
        let x0 = grid.min.x + cols.day_x(i, j);
        let y = |at: i64| grid.min.y + ((at - day0) as f32 / 3600.0) * HOUR;
        for (a, b) in outside(&work, day0, day1) {
            ui.painter().rect_filled(Rect::from_min_max(pos2(x0, y(a)), pos2(x0 + cols.cw, y(b))), 0.0, off_hours(t));
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
            let r = ui.interact(col, ui.scope_id().with(("hours", cols.day(i), j)), Sense::click_and_drag());
            let half_at = |y: f32| ((y - grid.min.y) / (HOUR / 2.0)).floor().clamp(0.0, 47.0);
            let cal = cols.cals.get(j).filter(|_| k > 1);
            let day0 = s.local_midnight(cols.day(i));
            // Dragged over (issue #212): the half hours from where it was pressed to where it is,
            // lit up; let go, an event that long, as Outlook's selection made one. (A click made
            // half an hour only, and a longer one needed its end changed in the form.)
            // Where it was pressed, kept from the drag's start: the pointer forgets it once let go.
            if r.drag_started()
                && let Some(from) = ui.input(|inp| inp.pointer.press_origin())
            {
                ui.data_mut(|d| d.insert_temp(r.id.with("from"), from));
            }
            let from = (r.dragged() || r.drag_stopped()).then(|| ui.data(|d| d.get_temp::<egui::Pos2>(r.id.with("from")))).flatten();
            let held = from.zip(ui.ctx().pointer_latest_pos());
            if let Some((from, to)) = held {
                let (a, b) = (half_at(from.y), half_at(to.y));
                let (lo, hi) = (a.min(b), a.max(b));
                let span = Rect::from_min_max(pos2(col.min.x, grid.min.y + lo * HOUR / 2.0), pos2(col.max.x, grid.min.y + (hi + 1.0) * HOUR / 2.0));
                ui.painter().rect_filled(span.shrink(1.0), 2.0, t.accent_soft);
                if r.drag_stopped() {
                    acts.push(new_event_between(s, day0 + lo as i64 * 1800, day0 + (hi as i64 + 1) * 1800, cal));
                }
            } else if let Some(p) = r.hover_pos() {
                // The half hour under the pointer lights up: a click adds an event there.
                let slot = Rect::from_min_size(pos2(col.min.x, grid.min.y + half_at(p.y) * HOUR / 2.0), vec2(cw, HOUR / 2.0)).shrink(1.0);
                ui.painter().rect_filled(slot, 2.0, t.hover_row);
            }
            if r.clicked()
                && let Some(p) = r.interact_pointer_pos()
            {
                acts.push(new_event_in(s, day0 + half_at(p.y) as i64 * 1800, cal));
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

/// A new event from `start` to `end` (dragged over, issue #212), in calendar `cal` as
/// [`new_event_in`].
fn new_event_between(s: &Session, start: i64, end: i64, cal: Option<&(String, String, String, u8)>) -> Action {
    let cal = cal.map(|(account, id, _, _)| (account.as_str(), id.as_str()));
    Action::Dialog(Dialog::Event(Box::new(crate::pim_dialogs::EventForm::between(s, (start, end), cal, &[]))))
}

/// Events, side by side when they overlap (each calendar in its own pane, side by side).
fn timed_events(s: &Session, t: &Tokens, ui: &mut Ui, items: &[AgendaItem], cols: &Columns, grid: Rect, acts: &mut Vec<Action>) {
    let cw = cols.cw;
    for (i, j) in (0..cols.n).flat_map(|i| (0..cols.k).map(move |j| (i, j))) {
        let d = cols.day(i);
        let (day0, day1) = (s.local_midnight(d), s.local_midnight(d + 1));
        let mut todays: Vec<&AgendaItem> = items
            .iter()
            .filter(|it| !it.all_day && it.end - it.start < 86_400 && it.start < day1 && it.end.max(it.start + 1) > day0 && cols.col_of(it) == j)
            .collect();
        todays.sort_by_key(|it| (it.start, -(it.end - it.start)));
        for (it, lane, lanes) in side_by_side(&todays) {
            let width = (cw - 6.0) / lanes.max(1) as f32;
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
            let resp = chip(ui, t, r, it, &text, s.movable(it));
            if resp.clicked()
                && let Some(a) = open(s, it)
            {
                acts.push(a);
            }
            // Dragged to another time, or day (issue #192): the day under the pointer (shown in
            // its own calendar's pane), its time as far as its top went, in quarter hours.
            if let Some(drag) = moving::dragged(ui, &resp, r) {
                let to = cols.column_at(drag.pointer.x - grid.min.x);
                let later = moving::quarters((drag.top_left.y - r.min.y) / HOUR * 3600.0);
                let by = s.local_midnight(cols.day(to)) - day0 + later;
                let x = grid.min.x + cols.day_x(to, j) + 3.0;
                let ghost = Rect::from_min_size(pos2(x, r.min.y + later as f32 / 3600.0 * HOUR), vec2(cw - 6.0, r.height()));
                moving::ghost(ui, t, ghost, &format!("{}\n{} – {}", title_of(it), hm(it.start + by, s), hm(it.end + by, s)));
                if drag.done
                    && let Some(a) = moving::moved(it, by)
                {
                    acts.push(a);
                }
            }
        }
    }
}

/// The day's events (in order of start), each with its lane and how many lanes its group has: a
/// group is the events that overlap one another, directly or through others, and shares the
/// column among itself alone, as Outlook draws them (issue #155: every event of a day took the
/// width of the most that overlapped anywhere in it, a morning meeting a third of the column for
/// three in the afternoon). An event is taken as long as it's drawn (a short one is drawn taller).
fn side_by_side<'a>(todays: &[&'a AgendaItem]) -> Vec<(&'a AgendaItem, usize, usize)> {
    let least = (18.0 / HOUR * 3600.0) as i64;
    let mut out: Vec<(&AgendaItem, usize, usize)> = Vec::with_capacity(todays.len());
    // The group being laid out: where it starts in `out`, each lane's end, and its own end.
    let (mut first, mut lane_end, mut group_end): (usize, Vec<i64>, i64) = (0, Vec::new(), i64::MIN);
    let close = |out: &mut Vec<(&AgendaItem, usize, usize)>, first: usize, lanes: usize| {
        for o in out.iter_mut().skip(first) {
            o.2 = lanes;
        }
    };
    for it in todays {
        if !lane_end.is_empty() && it.start >= group_end {
            close(&mut out, first, lane_end.len());
            first = out.len();
            lane_end.clear();
            group_end = i64::MIN;
        }
        let end = it.end.max(it.start.saturating_add(least));
        let lane = lane_end.iter().position(|e| *e <= it.start).unwrap_or(lane_end.len());
        match lane_end.get_mut(lane) {
            Some(e) => *e = end,
            None => lane_end.push(end),
        }
        group_end = group_end.max(end);
        out.push((*it, lane, 0));
    }
    close(&mut out, first, lane_end.len());
    out
}

/// The time found free for everyone (issue #186), outlined in each pane; clicked, a meeting
/// then with everyone shown.
fn free_slot(s: &Session, t: &Tokens, ui: &mut Ui, cols: &Columns, grid: Rect, acts: &mut Vec<Action>) {
    let Some((from, to)) = s.free_slot() else { return };
    for (i, j) in (0..cols.n).flat_map(|i| (0..cols.k).map(move |j| (i, j))) {
        let d = cols.day(i);
        let (day0, day1) = (s.local_midnight(d), s.local_midnight(d + 1));
        if from >= day1 || to <= day0 {
            continue;
        }
        let y = |at: i64| grid.min.y + ((at.clamp(day0, day1) - day0) as f32 / 3600.0) * HOUR;
        let x0 = grid.min.x + cols.day_x(i, j);
        let r = Rect::from_min_max(pos2(x0 + 2.0, y(from)), pos2(x0 + cols.cw - 2.0, y(to)));
        if let Some(slot) = marked_slot(s, t, ui, r, ("free-slot", i, j)) {
            acts.push(slot);
        }
    }
}

/// Now, in every pane: a line across today's column.
fn now_line(s: &Session, t: &Tokens, ui: &mut Ui, cols: &Columns, grid: Rect) {
    let n_day = today(s);
    if let Some((i, _)) = cols.columns(n_day, n_day) {
        let y = grid.min.y + ((s.clock.now - s.local_midnight(n_day)) as f32 / 3600.0) * HOUR;
        for j in 0..cols.k {
            let x0 = grid.min.x + cols.day_x(i, j);
            ui.painter().line_segment([pos2(x0, y), pos2(x0 + cols.cw, y)], egui::Stroke::new(2.0, t.danger));
            ui.painter().circle_filled(pos2(x0, y), 4.0, t.danger);
        }
    }
}
