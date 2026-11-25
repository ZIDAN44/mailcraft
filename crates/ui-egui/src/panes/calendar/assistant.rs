//! A meeting's Scheduling Assistant, in its form (issue #277), as Outlook has one: over the
//! meeting's day, a row of everyone together, one for the user and one for each guest, their
//! times taken drawn as the Schedule view draws them (#141), the hours they don't work shaded
//! (#206), and the meeting's own time across them all. A click moves the meeting there, its
//! length kept. The guests' times come from the account's server (#242), asked for by the form.

use mailcraft_engine::GuestTimes;

use super::showing::AWAY;
use super::*;

/// A row's height, the hours' heading's, and the names' column's width.
const ROW_H: f32 = 26.0;
const HEAD: f32 = 20.0;
const NAMES_W: f32 = 140.0;

/// The meeting being written, as the assistant needs it.
pub(crate) struct Meeting<'a> {
    /// The account whose server is asked for the guests' times.
    pub account: &'a str,
    /// Its guests: (name shown, address).
    pub guests: &'a [(String, String)],
    pub start: i64,
    pub end: i64,
    /// The meeting being moved, its own time not taken.
    pub uid: Option<&'a str>,
}

/// What a row shows: its name, its times taken, the hours worked (`None`: not known), and why
/// its times aren't there (still coming, or can't be had).
struct Row {
    name: String,
    taken: Vec<(i64, i64, ShowAs)>,
    work: Option<Vec<(i64, i64)>>,
    missing: Option<String>,
}

/// The rows: the user's, then each guest's, over `day0..day1`.
fn rows(s: &Session, m: &Meeting<'_>, (day0, day1): (i64, i64)) -> Vec<Row> {
    let mine = s
        .agenda(day0, day1)
        .into_iter()
        .filter(|it| !it.busy_only && it.show_as != ShowAs::Free && it.status != "CANCELLED" && Some(it.uid.as_str()) != m.uid)
        .map(|it| (it.start, it.end.max(it.start + 60), it.show_as))
        .collect();
    let mut out = vec![Row { name: tl!("You").to_string(), taken: mine, work: Some(s.work_spans(day0, day1)), missing: None }];
    for (name, email) in m.guests {
        let name = if name.is_empty() { email.clone() } else { name.clone() };
        out.push(match s.guest_times(m.account, email) {
            GuestTimes::Known { busy, work, .. } => Row {
                name,
                taken: busy
                    .iter()
                    .filter(|b| b.start < day1 && b.end > day0 && b.show_as != ShowAs::Free)
                    .map(|b| (b.start, b.end, b.show_as))
                    .collect(),
                work: work.map(|w| w.spans(day0, day1)),
                missing: None,
            },
            GuestTimes::Asking => Row { name, taken: Vec::new(), work: None, missing: Some(tl!("Asking their server…").to_string()) },
            GuestTimes::Unknown(why) => Row { name, taken: Vec::new(), work: None, missing: Some(tf!("Their times can't be had: {why}", why = why)) },
        });
    }
    out
}

/// How many of the user and the guests are free for the meeting's time, and how many there are
/// (those whose times aren't known counted as free, as Next free time counts them).
pub(crate) fn free_count(s: &Session, m: &Meeting<'_>) -> (usize, usize) {
    let all = rows(s, m, (m.start, m.end.max(m.start + 60)));
    let free = all.iter().filter(|r| !r.taken.iter().any(|(a, b, _)| *a < m.end && *b > m.start)).count();
    (free, all.len())
}

/// The assistant: its rows over the meeting's day. Returns a new start chosen by a click.
pub(crate) fn draw(ui: &mut Ui, s: &Session, t: &Tokens, m: &Meeting<'_>) -> Option<i64> {
    let day = s.clock.local_day(m.start);
    let (midnight, next) = (s.clock.midnight(day), s.clock.midnight(day + 1));
    // Office hours, widened to the meeting's own.
    let hour = |t: i64| (t - midnight).div_euclid(3600);
    let first = hour(m.start).clamp(0, 23).min(8);
    let last = (hour(m.end.max(m.start + 1) - 1) + 1).clamp(1, 24).max(18);
    let (day0, day1) = (midnight + first * 3600, (midnight + last * 3600).min(next.max(midnight + 3600)));
    let all = rows(s, m, (day0, day1));
    let width = ui.available_width().max(NAMES_W + 120.0);
    let height = HEAD + ROW_H * (all.len() + 1) as f32;
    let (rect, resp) = ui.allocate_exact_size(vec2(width, height), Sense::click());
    let grid = Rect::from_min_max(pos2(rect.min.x + NAMES_W, rect.min.y), rect.max);
    let span = (day1 - day0).max(3600) as f32;
    let x = |t: i64| grid.min.x + (t.clamp(day0, day1) - day0) as f32 / span * grid.width();
    let row_y = |k: usize| grid.min.y + HEAD + ROW_H * k as f32;
    let p = ui.painter_at(rect);
    // The hours' names and lines.
    for h in first..last {
        let at = x(midnight + h * 3600);
        p.line_segment([pos2(at, grid.min.y + HEAD), pos2(at, grid.max.y)], egui::Stroke::new(1.0, t.border));
        let label = crate::i18n::date(&dates::hour_label(
            u32::try_from(s.clock.local(midnight + h * 3600).rem_euclid(86_400) / 3600).unwrap_or(0),
            s.clock.style,
        ));
        p.text(pos2(at + 3.0, grid.min.y + HEAD / 2.0), Align2::LEFT_CENTER, label, theme::regular(10.0), t.faint);
    }
    // Everyone together first, then each row.
    let names: Vec<String> = std::iter::once(tl!("All together").to_string()).chain(all.iter().map(|r| r.name.clone())).collect();
    for (k, name) in names.iter().enumerate() {
        let y = row_y(k);
        p.line_segment([pos2(rect.min.x, y), pos2(grid.max.x, y)], egui::Stroke::new(1.0, t.border));
        let text = Rect::from_min_max(pos2(rect.min.x + 4.0, y), pos2(rect.min.x + NAMES_W - 6.0, y + ROW_H));
        let font = if k == 0 { theme::bold(12.0) } else { theme::regular(12.0) };
        p.with_clip_rect(text).text(pos2(text.min.x, text.center().y), Align2::LEFT_CENTER, name, font, t.text);
    }
    for (k, row) in all.iter().enumerate() {
        let (y0, y1) = (row_y(k + 1), row_y(k + 2));
        if let Some(work) = &row.work {
            for (a, b) in outside(work, day0, day1) {
                p.rect_filled(Rect::from_min_max(pos2(x(a), y0), pos2(x(b), y1)), 0.0, off_hours(t));
            }
        }
        if let Some(why) = &row.missing {
            let r = Rect::from_min_max(pos2(grid.min.x, y0), pos2(grid.max.x, y1));
            p.text(r.left_center() + vec2(6.0, 0.0), Align2::LEFT_CENTER, why, theme::regular(11.0), t.faint);
            ui.interact(r, egui::Id::unique(("assist-missing", k)), Sense::hover()).on_hover_text(why);
        }
        for (a, b, show_as) in &row.taken {
            bar(&p, t, (x(*a), x(*b)), (y0, y1), *show_as);
        }
    }
    // All together: everyone's time taken.
    let (y0, y1) = (row_y(0), row_y(1));
    for (a, b, show_as) in all.iter().flat_map(|r| r.taken.iter()) {
        bar(&p, t, (x(*a), x(*b)), (y0, y1), *show_as);
    }
    // The meeting, across every row.
    let band = Rect::from_min_max(pos2(x(m.start), row_y(0)), pos2(x(m.end).max(x(m.start) + 2.0), grid.max.y));
    p.rect_filled(band, 2.0, t.accent.gamma_multiply(0.18));
    p.rect_stroke(band, 2.0, egui::Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
    p.line_segment([pos2(rect.min.x, grid.max.y), pos2(grid.max.x, grid.max.y)], egui::Stroke::new(1.0, t.border));
    // A click on the hours: the meeting moves there, to the half hour.
    let resp = resp.on_hover_text(tl!("Click a time to move the meeting there"));
    let at = resp.interact_pointer_pos().filter(|_| resp.clicked()).filter(|p| p.x >= grid.min.x && p.y >= grid.min.y + HEAD)?;
    let t0 = day0 + (((at.x - grid.min.x) / grid.width() * span) as i64);
    Some(t0 - (s.clock.local(t0)).rem_euclid(1800))
}

/// A time taken, in a row between `y0` and `y1`: darker where busy, purple where away, faint
/// where only tentative.
fn bar(p: &egui::Painter, t: &Tokens, (x0, x1): (f32, f32), (y0, y1): (f32, f32), show_as: ShowAs) {
    let c = match show_as {
        ShowAs::OutOfOffice => AWAY.gamma_multiply(0.75),
        ShowAs::Busy => t.accent.gamma_multiply(0.75),
        _ => t.accent.gamma_multiply(0.3),
    };
    p.rect_filled(Rect::from_min_max(pos2(x0, y0 + 5.0), pos2(x1.max(x0 + 2.0), y1 - 5.0)), 2.0, c);
}
