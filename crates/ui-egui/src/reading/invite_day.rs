//! How a meeting invitation fits the user's calendar (issue #194), on its card, as Outlook shows
//! a meeting request: a line saying whether it clashes with something already there or is right
//! next to it, and a strip of the hours around it with the user's own events and the invitation
//! outlined among them. Their events are opened with a click.

use egui::{Align2, Rect, RichText, Sense, Ui, pos2, vec2};
use mailcraft_engine::model::ical::CalEvent;
use mailcraft_engine::{AgendaItem, Fit, InviteDay, Session};

use crate::theme::{self, Tokens};
use crate::{Action, Dialog};

/// The strip's height: the hours' names, then the events.
const STRIP: f32 = 50.0;
/// The hours' names' height, above the events.
const HOURS: f32 = 15.0;
/// Lanes of events overlapping one another, at most (the rest under the last).
const LANES: usize = 3;

/// An event as the line names it: its title and times.
fn named(s: &Session, it: &AgendaItem) -> String {
    let title = crate::calendar::event_title(&it.summary, it.privacy, it.show_as);
    format!("{title} ({} – {})", crate::calendar::hm(it.start, s), crate::calendar::hm(it.end, s))
}

/// The line and the strip, for the invitation `e` and the user's calendar around it.
pub(super) fn show(s: &Session, t: &Tokens, ui: &mut Ui, e: &CalEvent, day: &InviteDay, acts: &mut Vec<Action>) {
    let clashes: Vec<&AgendaItem> = day.clashes().collect();
    let (text, color) = match (clashes.as_slice(), day.beside()) {
        ([one], _) => (tf!("Clashes with {event} on your calendar", event = named(s, one)), t.danger),
        ([_, ..], _) => (tf!("Clashes with {n} events on your calendar", n = clashes.len()), t.danger),
        // Theirs ends as the invitation starts: the invitation is right after it.
        ([], Some((it, Fit::Before))) => (tf!("Right after {event} on your calendar", event = named(s, it)), t.text),
        ([], Some((it, _))) => (tf!("Right before {event} on your calendar", event = named(s, it)), t.text),
        ([], None) => (tl!("Nothing else on your calendar then").to_string(), t.dim),
    };
    ui.label(RichText::new(text).color(color));
    strip(s, t, ui, e, day, acts);
}

/// The hours around the invitation across, the user's events in them (in lanes where they
/// overlap), and the invitation outlined over them.
fn strip(s: &Session, t: &Tokens, ui: &mut Ui, e: &CalEvent, day: &InviteDay, acts: &mut Vec<Action>) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), STRIP), Sense::hover());
    let span = (day.to - day.from).max(3600) as f32;
    let x = |at: i64| rect.min.x + ((at.clamp(day.from, day.to) - day.from) as f32 / span) * rect.width();
    let p = ui.painter().with_clip_rect(rect);
    p.rect(rect, 6.0, t.surface, egui::Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
    let band = Rect::from_min_max(pos2(rect.min.x, rect.min.y + HOURS), pos2(rect.max.x, rect.max.y - 3.0));
    // The hours: a line each, named where there's room for it.
    let hours = (day.to - day.from) / 3600;
    let every = if rect.width() / (hours.max(1) as f32) < 40.0 { 2 } else { 1 };
    for (n, h) in (day.from..=day.to).step_by(3600).enumerate() {
        let hx = x(h);
        p.line_segment([pos2(hx, band.min.y), pos2(hx, band.max.y)], egui::Stroke::new(1.0, t.border));
        if n % every == 0 && hx + 30.0 < rect.max.x {
            p.text(pos2(hx + 3.0, rect.min.y + 2.0), Align2::LEFT_TOP, crate::calendar::hm(h, s), theme::regular(10.5), t.faint);
        }
    }
    // The user's events, a lane each where they overlap.
    let mut lane_end: Vec<i64> = Vec::new();
    let mut placed = Vec::new();
    for (it, fit) in &day.items {
        let lane = lane_end.iter().position(|end| *end <= it.start).unwrap_or(lane_end.len());
        match lane_end.get_mut(lane) {
            Some(end) => *end = it.end,
            None => lane_end.push(it.end),
        }
        placed.push((it, *fit, lane.min(LANES - 1)));
    }
    let lanes = lane_end.len().clamp(1, LANES);
    let lane_h = band.height() / lanes as f32;
    for (it, fit, lane) in placed {
        let top = band.min.y + lane_h * lane as f32;
        let r = Rect::from_min_max(pos2(x(it.start) + 1.0, top + 1.0), pos2((x(it.end) - 1.0).max(x(it.start) + 4.0), top + lane_h - 1.0));
        let c = crate::calendar::color(it.color, t);
        let resp = ui.interact(r, ui.scope_id().with(("invite-day", &it.uid, it.start)), Sense::click());
        let fill = c.gamma_multiply(match (fit, resp.hovered()) {
            (Fit::Apart, false) => 0.15,
            (_, false) => 0.35,
            (_, true) => 0.5,
        });
        let edge = if fit == Fit::Clash { egui::Stroke::new(1.5, t.danger) } else { egui::Stroke::NONE };
        p.rect(r, 3.0, fill, edge, egui::StrokeKind::Inside);
        let title = crate::calendar::event_title(&it.summary, it.privacy, it.show_as);
        p.with_clip_rect(r.shrink(2.0)).text(r.left_center() + vec2(4.0, 0.0), Align2::LEFT_CENTER, title, theme::regular(11.0), t.text);
        crate::widgets::named(ui, &resp, &named(s, it), true, None);
        if resp.on_hover_text(named(s, it)).clicked()
            && let Some(d) = Dialog::open_event(s, &it.uid, it.occurrence)
        {
            acts.push(Action::Dialog(d));
        }
    }
    // The invitation, over them.
    let r = Rect::from_min_max(pos2(x(e.start), band.min.y), pos2(x(e.end).max(x(e.start) + 4.0), band.max.y));
    p.rect(r, 3.0, t.accent.gamma_multiply(0.18), egui::Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
    p.with_clip_rect(r.shrink(2.0)).text(r.left_top() + vec2(5.0, 2.0), Align2::LEFT_TOP, &e.summary, theme::bold(11.0), t.text);
}
