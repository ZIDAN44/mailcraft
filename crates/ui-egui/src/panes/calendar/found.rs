//! The calendar's search (issue #33): what's typed in the search box in the Calendar lists the
//! events it finds in place of the days, as Outlook's calendar search does: each with its date
//! and time, subject, place and calendar's colour; upcoming ones first, then past ones, or only
//! either. Clicking one goes to its day and opens it.

use egui::{Align2, Margin, Rect, RichText, Sense, Ui, pos2, vec2};
use mailcraft_engine::model::ical::Privacy;
use mailcraft_engine::{EventHit, EventWhen, Session};
use serde_json::json;

use super::{color, event_title, hm, long_date, time_strip};
use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};
use crate::{Action, Dialog, UiState};

/// What the calendar's search box holds, and what it found (kept while nothing changes).
#[derive(Clone, Debug, Default)]
pub struct CalSearch {
    pub query: String,
    pub when: EventWhen,
    /// One calendar (account, calendar id) to look in; `None`: every one shown.
    pub scope: Option<(String, String)>,
    found: Option<(Key, Vec<EventHit>)>,
}

/// What a search's results depend on: the query, the switches, the calendars' state, the minute.
type Key = (String, EventWhen, Option<(String, String)>, u64, i64);

/// The most results listed.
const SHOWN: usize = 500;
/// A result's height.
const ROW: f32 = 46.0;

impl CalSearch {
    /// Searching: the box has something in it.
    pub fn active(&self) -> bool {
        !self.query.trim().is_empty()
    }

    pub(crate) fn results(&mut self, s: &Session) -> &[EventHit] {
        let key: Key = (self.query.clone(), self.when, self.scope.clone(), s.pim_revision, s.clock.now.div_euclid(60));
        if self.found.as_ref().is_none_or(|(k, _)| *k != key) {
            let (account, calendar) = match &self.scope {
                Some((a, c)) => (Some(a.as_str()), Some(c.as_str())),
                None => (None, None),
            };
            let hits = s.find_events(&self.query, self.when, account, calendar, SHOWN);
            self.found = Some((key, hits));
        }
        self.found.as_ref().map_or(&[], |(_, h)| h.as_slice())
    }
}

/// Every calendar there is (account, id, name), for the scope menu.
fn calendars(s: &Session) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    for a in &s.accounts {
        if let Some(p) = s.pim.get(&a.config.id) {
            for c in &p.calendars {
                let name = if c.id == mailcraft_engine::model::pim::LOCAL { tl!("Calendar").to_string() } else { c.name.clone() };
                out.push((a.config.id.clone(), c.id.clone(), format!("{name} · {}", a.config.email)));
            }
        }
    }
    out
}

/// The results, in place of the days.
pub(crate) fn found(s: &Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    let t = st.tokens;
    egui::Frame::new().inner_margin(Margin::symmetric(14, 10)).show(ui, |ui| {
        ui.horizontal(|ui| {
            for (when, label) in [(EventWhen::All, tl!("All")), (EventWhen::Upcoming, tl!("Upcoming")), (EventWhen::Past, tl!("Past"))] {
                if ui.selectable_label(st.cal_search.when == when, label).clicked() {
                    st.cal_search.when = when;
                }
            }
            ui.add_space(12.0);
            // Where to look, as the mail search box's menu: every calendar shown, or one.
            let all = calendars(s);
            let shown = match &st.cal_search.scope {
                Some((a, c)) => all.iter().find(|(x, y, _)| x == a && y == c).map_or_else(|| tl!("All calendars").to_string(), |x| x.2.clone()),
                None => tl!("All calendars").to_string(),
            };
            egui::ComboBox::from_id_salt("calendar-search-scope").selected_text(shown).show_ui(ui, |ui| {
                if ui.selectable_label(st.cal_search.scope.is_none(), tl!("All calendars")).clicked() {
                    st.cal_search.scope = None;
                }
                for (a, c, name) in &all {
                    let on = st.cal_search.scope.as_ref().is_some_and(|(x, y)| x == a && y == c);
                    if ui.selectable_label(on, name).clicked() {
                        st.cal_search.scope = Some((a.clone(), c.clone()));
                    }
                }
            });
        });
    });
    let hits = st.cal_search.results(s).to_vec();
    if hits.is_empty() {
        ui.add_space(40.0);
        ui.vertical_centered(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(48.0, 48.0), Sense::hover());
            icons::paint(ui.painter(), r, Icon::Calendar, t.border);
            ui.add_space(8.0);
            ui.label(RichText::new(tl!("No events match.")).color(t.faint));
        });
        return;
    }
    egui::ScrollArea::vertical().auto_shrink(false).id_salt("calendar-found").show(ui, |ui| {
        egui::Frame::new().inner_margin(Margin::symmetric(14, 0)).show(ui, |ui| {
            let mut heading: Option<bool> = None;
            for h in &hits {
                // Upcoming ones, then past ones, each under its heading.
                if st.cal_search.when == EventWhen::All && heading != Some(h.upcoming) {
                    heading = Some(h.upcoming);
                    ui.add_space(6.0);
                    ui.label(RichText::new(if h.upcoming { tl!("Upcoming") } else { tl!("Past") }).font(theme::bold(13.0)).color(t.dim));
                    ui.add_space(2.0);
                }
                if row(s, &t, ui, h).clicked() {
                    open_hit(s, st, h, acts);
                }
            }
            ui.add_space(10.0);
        });
    });
}

/// A result clicked: its day in the view the calendar was in (the search box emptied), and the
/// event open at that occurrence.
pub(crate) fn open_hit(s: &Session, st: &mut UiState, h: &EventHit, acts: &mut Vec<Action>) {
    let day = s.clock.local(h.start).div_euclid(86_400);
    let (y, m, d) = mailcraft_engine::model::dates::civil_from_days(day);
    acts.push(Action::Run("calendar.goto", json!({"date": format!("{y:04}-{m:02}-{d:02}")})));
    if let Some(dialog) = Dialog::open_event(s, &h.uid, h.occurrence) {
        acts.push(Action::Dialog(dialog));
    }
    st.cal_search.query.clear();
}

/// One result: its calendar's colour, its date and time, its subject (a lock when private, a
/// mark when it repeats), its place and calendar.
fn row(s: &Session, t: &Tokens, ui: &mut Ui, h: &EventHit) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 4.0, t.hover_row);
    }
    let c = h.category.map_or_else(|| color(h.color, t), |i| theme::category_color(i, t.dark));
    time_strip(ui, Rect::from_min_size(rect.min + vec2(2.0, 6.0), vec2(4.0, ROW - 12.0)), c, h.show_as);
    let p = ui.painter().with_clip_rect(rect);
    let date_x = rect.min.x + 16.0;
    let day = s.clock.local(h.start).div_euclid(86_400);
    p.text(pos2(date_x, rect.min.y + 14.0), Align2::LEFT_CENTER, long_date(day), theme::regular(12.5), t.text);
    let time = if h.all_day { tl!("All day").to_string() } else { format!("{} – {}", hm(h.start, s), hm(h.end, s)) };
    p.text(pos2(date_x, rect.min.y + 32.0), Align2::LEFT_CENTER, time, theme::regular(12.0), t.dim);
    // The subject and what's under it, after the date's column (narrower on a narrow pane).
    let x = date_x + (rect.width() * 0.3).clamp(150.0, 240.0);
    let mut sx = x;
    if h.privacy != Privacy::Public {
        icons::paint(&p, Rect::from_min_size(pos2(sx, rect.min.y + 8.0), vec2(12.0, 12.0)), Icon::Lock, t.dim);
        sx += 17.0;
    }
    if h.recurring {
        icons::paint(&p, Rect::from_min_size(pos2(sx, rect.min.y + 8.0), vec2(12.0, 12.0)), Icon::Sync, t.dim);
        sx += 17.0;
    }
    let title = event_title(&h.summary, h.privacy, h.show_as);
    p.with_clip_rect(rect.with_min_x(sx).shrink2(vec2(0.0, 1.0)).with_max_x(rect.max.x - 8.0)).text(
        pos2(sx, rect.min.y + 14.0),
        Align2::LEFT_CENTER,
        title,
        theme::bold(13.0),
        t.text,
    );
    let calendar = s
        .pim
        .get(&h.account)
        .and_then(|p| p.calendars.iter().find(|c| c.id == h.calendar))
        .map(|c| if c.id == mailcraft_engine::model::pim::LOCAL { tl!("Calendar").to_string() } else { c.name.clone() })
        .unwrap_or_default();
    let under: Vec<String> = [h.location.clone(), h.categories.join(", "), calendar].into_iter().filter(|x| !x.trim().is_empty()).collect();
    p.with_clip_rect(rect.with_min_x(x).with_max_x(rect.max.x - 8.0)).text(
        pos2(x, rect.min.y + 32.0),
        Align2::LEFT_CENTER,
        under.join(" · "),
        theme::regular(12.0),
        t.dim,
    );
    ui.painter().line_segment([rect.left_bottom(), rect.right_bottom()], egui::Stroke::new(1.0, t.border));
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    crate::widgets::named(ui, &resp, &format!("{} {}", long_date(day), event_title(&h.summary, h.privacy, h.show_as)), true, None);
    resp
}
