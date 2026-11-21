//! The Schedule view (Outlook's, issue #141): a day's hours across, a row for each calendar
//! shown under all of them together, to see when several people are free at once, each row's
//! hours not worked shaded (issue #185; on all together's, when not everyone works). A time
//! chosen on a colleague's row (clicked, or dragged across) makes a meeting with them; on the
//! row of all together, with everyone shown; on one's own calendar, an event in it.

use mailcraft_engine::model::pim::shared_calendar_owner;

use super::*;

/// The width of an hour, and the height of a row and of the hours' heading.
const HOUR_W: f32 = 60.0;
const ROW_H: f32 = 34.0;
const HEAD: f32 = 24.0;
/// Events side by side in a row at most: the rest share the last lane.
const LANES: usize = 3;

/// A calendar shown: (account, id, name, colour).
type Cal = (String, String, String, u8);

/// A row: a calendar's, or (`None`) all of them together.
type Row = Option<Cal>;

pub(super) fn schedule(s: &Session, st: &mut UiState, ui: &mut Ui, items: &[AgendaItem], d0: i64, acts: &mut Vec<Action>) {
    let t = st.tokens;
    let cals = s.shown_calendars();
    if cals.is_empty() {
        ui.add_space(20.0);
        ui.vertical_centered(|ui| ui.label(RichText::new(tl!("Tick a calendar to show it here")).color(t.dim)));
        return;
    }
    let mut rows: Vec<Row> = Vec::new();
    if cals.len() > 1 {
        rows.push(None);
    }
    rows.extend(cals.iter().cloned().map(Some));
    let mut only = s.prefs.schedule_work_hours;
    if ui.checkbox(&mut only, tl!("Only work hours")).changed() {
        acts.push(Action::Run("calendar.scheduleWorkHours", json!({ "on": only })));
    }
    let (day0, day1) = (s.local_midnight(d0), s.local_midnight(d0 + 1));
    let (day0, day1) = if s.prefs.schedule_work_hours { work_window(s, &cals, day0, day1) } else { (day0, day1) };
    // The day's own length: a change of clocks makes it 23 or 25 hours.
    let hours = ((day1 - day0) as f32 / 3600.0).max(1.0);
    let items: Vec<&AgendaItem> = items.iter().filter(|it| !it.task && it.start < day1 && it.end.max(it.start + 1) > day0).collect();
    let label_w = (ui.available_width() * 0.28).clamp(90.0, 200.0);
    let height = HEAD + ROW_H * rows.len() as f32;
    egui::ScrollArea::vertical().auto_shrink(false).id_salt("schedule-rows").show(ui, |ui| {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let (names, _) = ui.allocate_exact_size(vec2(label_w, height), Sense::hover());
            row_names(s, &t, ui, names, &rows);
            let mut scroll = egui::ScrollArea::horizontal().id_salt("schedule-hours").auto_shrink([false, true]);
            // Opened on the working day the first time it's shown (the whole day's).
            if !st.cal_scrolled[2] && !s.prefs.schedule_work_hours {
                scroll = scroll.horizontal_scroll_offset(HOUR_W * 7.5);
                st.cal_scrolled[2] = true;
            }
            scroll.show(ui, |ui| {
                let (grid, _) = ui.allocate_exact_size(vec2(HOUR_W * hours, height), Sense::hover());
                let day = Day { day0, day1, grid };
                off_hours_rows(s, &t, ui, &day, &rows);
                hour_lines(s, &t, ui, &day, rows.len());
                for (k, row) in rows.iter().enumerate() {
                    let r = Rect::from_min_size(pos2(grid.min.x, grid.min.y + HEAD + ROW_H * k as f32), vec2(grid.width(), ROW_H));
                    choose_time(s, &t, ui, &day, r, row, &cals, acts);
                    match row {
                        None => together(&t, ui, &day, r, &items),
                        Some((account, id, ..)) => {
                            let mine: Vec<&AgendaItem> = items.iter().copied().filter(|it| it.account == *account && it.calendar == *id).collect();
                            row_items(s, &t, ui, &day, r, &mine, acts);
                        }
                    }
                }
                // The time found free for everyone (issue #186), across every row.
                if let Some((from, to)) = s.free_slot().filter(|(a, b)| *a < day.day1 && *b > day.day0) {
                    let r = Rect::from_min_max(pos2(day.x(from), grid.min.y + HEAD), pos2(day.x(to), grid.max.y)).shrink2(vec2(1.0, 2.0));
                    if let Some(slot) = marked_slot(s, &t, ui, r, ("free-slot", 0, 0)) {
                        acts.push(slot);
                    }
                }
                now_line(s, &t, ui, &day);
            });
        });
    });
}

/// The hours anyone in `cals` works between `day0` and `day1` (issue #206), in whole hours: from
/// the earliest start to the latest end; the whole day when no one works then.
fn work_window(s: &Session, cals: &[Cal], day0: i64, day1: i64) -> (i64, i64) {
    let spans: Vec<(i64, i64)> = cals.iter().flat_map(|(account, id, ..)| s.calendar_work_spans(account, id, day0, day1)).collect();
    let (Some(from), Some(to)) = (spans.iter().map(|(a, _)| *a).min(), spans.iter().map(|(_, b)| *b).max()) else { return (day0, day1) };
    let from = day0 + (from.max(day0) - day0) / 3600 * 3600;
    let to = (day0 + (to.min(day1) - day0 + 3599) / 3600 * 3600).min(day1);
    if to > from { (from, to) } else { (day0, day1) }
}

/// The day on screen: where it starts and ends, and the hours' area.
struct Day {
    day0: i64,
    day1: i64,
    grid: Rect,
}

impl Day {
    fn x(&self, t: i64) -> f32 {
        self.grid.min.x + (t.clamp(self.day0, self.day1) - self.day0) as f32 / 3600.0 * HOUR_W
    }

    /// The half hour at `x`, counted from the day's start.
    fn half(&self, x: f32) -> i64 {
        let last = ((self.day1 - self.day0) / 1800 - 1).max(0);
        (((x - self.grid.min.x) / (HOUR_W / 2.0)).floor() as i64).clamp(0, last)
    }
}

/// The rows' names: all together, then each calendar in its colour (whose, when two share a
/// name).
fn row_names(s: &Session, t: &Tokens, ui: &mut Ui, names: Rect, rows: &[Row]) {
    for (k, row) in rows.iter().enumerate() {
        let r = Rect::from_min_size(pos2(names.min.x, names.min.y + HEAD + ROW_H * k as f32), vec2(names.width(), ROW_H));
        ui.painter().line_segment([r.left_bottom(), r.right_bottom()], egui::Stroke::new(1.0, t.border));
        let (label, c) = match row {
            None => (tl!("All together").to_string(), t.dim),
            Some((account, id, name, c)) => {
                let mut label = calendar_name(id, name);
                if rows.iter().flatten().filter(|(_, _, n, _)| n == name).count() > 1
                    && let Some(a) = s.account(account)
                {
                    label = format!("{label} · {}", a.config.email);
                }
                (label, color(*c, t))
            }
        };
        ui.painter().circle_filled(pos2(r.min.x + 10.0, r.center().y), 4.0, c);
        let text = r.with_min_x(r.min.x + 20.0).with_max_x(r.max.x - 6.0);
        let font = if row.is_none() { theme::bold(12.0) } else { theme::regular(12.0) };
        ui.painter().with_clip_rect(text).text(pos2(text.min.x, text.center().y), Align2::LEFT_CENTER, label, font, t.text);
    }
}

/// Each row's hours not worked, shaded: a calendar's owner's (a colleague's as their server gave
/// them, else the user's own); all together's, where not everyone works.
fn off_hours_rows(s: &Session, t: &Tokens, ui: &mut Ui, day: &Day, rows: &[Row]) {
    let of = |(account, id, ..): &Cal| s.calendar_work_spans(account, id, day.day0, day.day1);
    let everyone = rows.iter().flatten().map(of).reduce(|a, b| overlap(&a, &b)).unwrap_or_default();
    for (k, row) in rows.iter().enumerate() {
        let work = match row {
            Some(cal) => of(cal),
            None => everyone.clone(),
        };
        let y = day.grid.min.y + HEAD + ROW_H * k as f32;
        for (a, b) in outside(&work, day.day0, day.day1) {
            ui.painter().rect_filled(Rect::from_min_max(pos2(day.x(a), y), pos2(day.x(b), y + ROW_H)), 0.0, off_hours(t));
        }
    }
}

/// The hours' names on top, their lines down every row, the rows' lines across.
fn hour_lines(s: &Session, t: &Tokens, ui: &mut Ui, day: &Day, rows: usize) {
    let g = day.grid;
    let hours = ((day.day1 - day.day0) / 3600).max(1);
    for h in 0..=hours {
        let x = g.min.x + HOUR_W * h as f32;
        ui.painter().line_segment([pos2(x, g.min.y + HEAD), pos2(x, g.max.y)], egui::Stroke::new(1.0, t.border));
        if h < hours {
            // The local hour the line stands at: the day may start later (Only work hours).
            let hour = s.clock.local(day.day0.saturating_add(h.saturating_mul(3600))).rem_euclid(86_400) / 3600;
            let at = crate::i18n::date(&mailcraft_engine::model::dates::hour_label(u32::try_from(hour).unwrap_or(0), s.clock.style));
            ui.painter().text(pos2(x + 4.0, g.min.y + HEAD / 2.0), Align2::LEFT_CENTER, at, theme::regular(11.0), t.faint);
        }
    }
    for k in 0..=rows {
        let y = g.min.y + HEAD + ROW_H * k as f32;
        ui.painter().line_segment([pos2(g.min.x, y), pos2(g.max.x, y)], egui::Stroke::new(1.0, t.border));
    }
}

/// A row's empty time: the half hour under the pointer lights up; clicked, or dragged across,
/// it's the time of a new meeting (or event).
#[expect(clippy::too_many_arguments, reason = "the row, its place and the calendars a meeting invites, each its own")]
fn choose_time(s: &Session, t: &Tokens, ui: &mut Ui, day: &Day, r: Rect, row: &Row, cals: &[Cal], acts: &mut Vec<Action>) {
    let id = ui.scope_id().with(("schedule-row", row.as_ref().map(|(a, c, ..)| (a.clone(), c.clone()))));
    let resp = ui.interact(r, id, Sense::click_and_drag());
    let slot = |a: i64, b: i64| {
        Rect::from_min_max(pos2(day.grid.min.x + a as f32 * HOUR_W / 2.0, r.min.y), pos2(day.grid.min.x + (b + 1) as f32 * HOUR_W / 2.0, r.max.y))
    };
    // Where a drag began, kept while it lasts: the pointer's own record of it is gone by the
    // frame it's let go in.
    if resp.drag_started()
        && let Some(p) = ui.input(|i| i.pointer.press_origin()).or(resp.interact_pointer_pos())
    {
        ui.data_mut(|d| d.insert_temp(id, day.half(p.x)));
    }
    // The half hours from where it was pressed to where the pointer is, in order.
    let range = || {
        let at = day.half(resp.interact_pointer_pos()?.x);
        let from = if resp.clicked() { at } else { ui.data(|d| d.get_temp::<i64>(id)).unwrap_or(at) };
        Some((from.min(at), from.max(at)))
    };
    if resp.dragged()
        && let Some((a, b)) = range()
    {
        ui.painter().rect_filled(slot(a, b).shrink(1.0), 2.0, t.accent.gamma_multiply(0.25));
    } else if let Some(p) = resp.hover_pos() {
        let h = day.half(p.x);
        ui.painter().rect_filled(slot(h, h).shrink(1.0), 2.0, t.hover_row);
    }
    let chosen = if resp.drag_stopped() || resp.clicked() { range() } else { None };
    if let Some((a, b)) = chosen {
        let (from, to) = (day.day0 + a * 1800, day.day0 + (b + 1) * 1800);
        acts.push(new_meeting(s, (from, to), row, cals));
    }
}

/// A new event over `when` from a row: on a colleague's row, a meeting with them; on the row of
/// all together, with everyone whose calendar is shown; on the user's own calendar, an event
/// in it.
pub(super) fn new_meeting(s: &Session, when: (i64, i64), row: &Row, cals: &[Cal]) -> Action {
    let mine: Vec<String> = s.accounts.iter().map(|a| a.config.email.to_ascii_lowercase()).collect();
    let people = |list: &mut dyn Iterator<Item = &Cal>| -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for (_, id, ..) in list {
            if let Some(e) = shared_calendar_owner(id).filter(|e| !mine.contains(&e.to_string()))
                && !out.iter().any(|o| o == e)
            {
                out.push(e.to_string());
            }
        }
        out
    };
    let (calendar, guests) = match row {
        None => (None, people(&mut cals.iter())),
        Some(cal) => {
            let guests = people(&mut std::iter::once(cal));
            // A colleague's calendar is theirs: the meeting goes in the account's own.
            let id = if guests.is_empty() { cal.1.as_str() } else { "" };
            (Some((cal.0.as_str(), id)), guests)
        }
    };
    Action::Dialog(Dialog::Event(Box::new(crate::pim_dialogs::EventForm::between(s, when, calendar, &guests))))
}

/// The row of all together: every calendar's time taken, darker where it's busy, purple where
/// someone is away, faint where it's only tentative; free time left clear.
fn together(t: &Tokens, ui: &mut Ui, day: &Day, r: Rect, items: &[&AgendaItem]) {
    let mut taken: Vec<&&AgendaItem> = items.iter().filter(|it| it.show_as != ShowAs::Free && it.status != "CANCELLED").collect();
    // Tentative under busy under away, so the strongest shows where they overlap.
    let weight = |it: &AgendaItem| match it.show_as {
        ShowAs::OutOfOffice => 3,
        ShowAs::Busy => 2,
        _ => 1,
    };
    taken.sort_by_key(|it| weight(it));
    for it in taken {
        let (x0, x1) = (day.x(it.start), day.x(it.end.max(it.start + 60)));
        let bar = Rect::from_min_max(pos2(x0, r.min.y + 6.0), pos2(x1.max(x0 + 2.0), r.max.y - 6.0));
        let c = match it.show_as {
            ShowAs::OutOfOffice => AWAY,
            ShowAs::Busy => t.accent,
            _ => t.accent.gamma_multiply(0.4),
        };
        let tentative = it.show_as == ShowAs::Tentative || it.status == "TENTATIVE" || it.needs_reply;
        ui.painter().rect_filled(bar, 2.0, if tentative { c.gamma_multiply(0.35) } else { c.gamma_multiply(0.75) });
    }
}

/// A calendar's events on its row, in lanes where they overlap; a click opens one (a
/// colleague's time taken opens nothing).
fn row_items(s: &Session, t: &Tokens, ui: &mut Ui, day: &Day, r: Rect, items: &[&AgendaItem], acts: &mut Vec<Action>) {
    let mut sorted: Vec<&AgendaItem> = items.to_vec();
    sorted.sort_by_key(|it| (it.start, -(it.end - it.start)));
    let mut lane_end: Vec<i64> = Vec::new();
    let mut lanes = Vec::with_capacity(sorted.len());
    for it in &sorted {
        let lane = lane_end.iter().position(|e| *e <= it.start).unwrap_or(lane_end.len()).min(LANES - 1);
        match lane_end.get_mut(lane) {
            Some(e) => *e = (*e).max(it.end),
            None => lane_end.push(it.end),
        }
        lanes.push(lane);
    }
    let h = (ROW_H - 4.0) / lane_end.len().clamp(1, LANES) as f32;
    for (it, lane) in sorted.into_iter().zip(lanes) {
        let (x0, x1) = (day.x(it.start), day.x(it.end.max(it.start + 60)));
        let cr = Rect::from_min_size(pos2(x0 + 1.0, r.min.y + 2.0 + h * lane as f32), vec2((x1 - x0 - 2.0).max(6.0), h - 1.0));
        // Opened, not dragged: a row's time is the person's, the day's across the hours.
        let resp = chip(ui, t, cr, it, &title_of(it), false);
        let when = if it.all_day { tl!("All day").to_string() } else { format!("{} – {}", hm(it.start, s), hm(it.end, s)) };
        let tip = if it.location.is_empty() { format!("{}\n{when}", title_of(it)) } else { format!("{}\n{when} · {}", title_of(it), it.location) };
        if resp.on_hover_text(tip).clicked()
            && let Some(a) = open(s, it)
        {
            acts.push(a);
        }
    }
}

/// Now, down every row, when the day shown is today.
fn now_line(s: &Session, t: &Tokens, ui: &mut Ui, day: &Day) {
    let now = s.clock.now;
    if now >= day.day0 && now < day.day1 {
        let x = day.x(now);
        ui.painter().line_segment([pos2(x, day.grid.min.y + HEAD), pos2(x, day.grid.max.y)], egui::Stroke::new(2.0, t.danger));
        ui.painter().circle_filled(pos2(x, day.grid.min.y + HEAD), 4.0, t.danger);
    }
}
