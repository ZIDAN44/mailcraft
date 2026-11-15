//! The event, task and contact forms.

use egui::{Color32, Margin, RichText, Sense, Ui, vec2};
use mailcraft_engine::Session;
use mailcraft_engine::model::dates;
use serde_json::{Value, json};

use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};
use crate::{Action, widgets};

mod contact;
mod online_meeting;
mod repeat;
mod showing;
mod task;
pub use contact::{ContactForm, contact};
pub use online_meeting::MeetingState;
#[cfg(test)]
pub(crate) use online_meeting::{add as online_meeting_add, remove as online_meeting_remove};
pub use repeat::Repeat;
use repeat::repeat_row;
use task::category_chips;
pub use task::{TaskForm, status_label, task};

/// A new event's reminder (Outlook's default for appointments).
pub const DEFAULT_REMINDER: i64 = 15;
/// The reminders offered, in minutes before the start (Outlook's list).
pub const REMINDER_CHOICES: [i64; 18] = [0, 5, 10, 15, 30, 60, 120, 180, 240, 480, 720, 1080, 1440, 2880, 4320, 5760, 10080, 20160];

/// A reminder as the list shows it: `None`, `15 minutes`, `2 hours`, `1 day`, `2 weeks`.
pub fn reminder_label(m: Option<i64>) -> String {
    match m {
        None => tl!("None").to_string(),
        Some(m) if m >= 10080 && m % 10080 == 0 => {
            if m == 10080 {
                tl!("1 week").to_string()
            } else {
                tf!("{n} weeks", n = m / 10080)
            }
        }
        Some(m) if m >= 1440 && m % 1440 == 0 => {
            if m == 1440 {
                tl!("1 day").to_string()
            } else {
                tf!("{n} days", n = m / 1440)
            }
        }
        Some(m) if m >= 60 && m % 60 == 0 => {
            if m == 60 {
                tl!("1 hour").to_string()
            } else {
                tf!("{n} hours", n = m / 60)
            }
        }
        Some(m) => {
            if m == 1 {
                tl!("1 minute").to_string()
            } else {
                tf!("{n} minutes", n = m)
            }
        }
    }
}

/// An event being added or edited.
#[derive(Clone, Debug, Default)]
pub struct EventForm {
    /// `None`: a new event.
    pub uid: Option<String>,
    /// The occurrence opened (its start), for deleting just that one.
    pub occurrence: Option<i64>,
    pub recurring: bool,
    /// The calendars that can take it: (account, calendar id, name).
    pub calendars: Vec<(String, String, String)>,
    pub calendar: usize,
    pub summary: String,
    pub location: String,
    pub description: String,
    /// Local `YYYY-MM-DD`, and times as typed (shown on the clock File › Options sets).
    pub date: String,
    pub start: String,
    pub end_date: String,
    pub end: String,
    pub all_day: bool,
    /// Its times are shown on a 12-hour clock (File › Options).
    pub hours12: bool,
    pub repeat: Repeat,
    /// Its reminder, in minutes before it starts (`None`: none).
    pub reminder: Option<i64>,
    /// Who sees what of it, and how it shows its time (issue #32).
    pub privacy: mailcraft_engine::model::ical::Privacy,
    pub show_as: mailcraft_engine::model::ical::ShowAs,
    pub organizer: Option<String>,
    /// (who, their answer).
    pub attendees: Vec<(String, String)>,
    /// The guests, as typed (`Name <address>, …`): editable on the user's own events.
    pub guests: String,
    /// The user can change the guests (a new event, or one they organize).
    pub can_invite: bool,
    /// The user was invited and hasn't answered.
    pub needs_reply: bool,
    pub read_only: bool,
    /// Its categories, and every category there is (name, colour) to pick from.
    pub categories: Vec<String>,
    pub all_categories: Vec<(String, u8)>,
    /// The web addresses in its place and notes, and the one that joins it online (Teams, Zoom…):
    /// shown as links to open, as the notes box can't open them.
    pub links: Vec<String>,
    pub join: Option<String>,
    /// The notes shown to read, their links to open, rather than in the box that edits them:
    /// an invitation's (issue #1), or any with links.
    pub reading_notes: bool,
    /// Its online meeting (issue #25): the rooms to add, and what Save asks.
    pub meeting: MeetingState,
    pub error: Option<String>,
}

fn local_date(t: i64, s: &Session) -> String {
    dates::day_string(s.clock.local_day(t))
}

/// A local time as its box shows it, on the clock File › Options sets (gap 9).
fn local_time(t: i64, s: &Session) -> String {
    let c = dates::civil(t, &s.clock);
    crate::i18n::time_box(c.hour, c.minute, s.clock.style)
}

/// A time as typed, as the commands take it (`HH:MM`); one that doesn't read is passed on as
/// typed, for the command to refuse.
pub(crate) fn time_param(typed: &str) -> String {
    crate::i18n::read_time(typed).map_or_else(|| typed.trim().to_string(), |(h, m)| format!("{h:02}:{m:02}"))
}

/// A box to type a time in, `hour` o'clock shown in it while it's empty: wide enough for
/// `12:30 PM` on a 12-hour clock.
fn time_field(ui: &mut Ui, value: &mut String, hour: u32, hours12: bool) -> egui::Response {
    let hint = crate::i18n::time_box(hour, 0, dates::DateStyle { hours12, ..dates::DateStyle::default() });
    field(ui, value, &hint, if hours12 { 96.0 } else { 70.0 })
}

fn writable_calendars(s: &Session) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    for a in &s.accounts {
        if let Some(p) = s.pim.get(&a.config.id) {
            for c in p.calendars.iter().filter(|c| !c.read_only) {
                let name = if c.id == mailcraft_engine::model::pim::LOCAL { tl!("Calendar").to_string() } else { c.name.clone() };
                out.push((a.config.id.clone(), c.id.clone(), format!("{name} · {}", a.config.email)));
            }
        }
    }
    out
}

impl EventForm {
    /// A new event at `at` (an hour long), or at the next full hour.
    pub fn new(s: &Session, at: Option<i64>) -> EventForm {
        let start = at.unwrap_or_else(|| (s.clock.now / 3600 + 1) * 3600);
        let calendars = writable_calendars(s);
        let current = s.current_account().map(|a| a.config.id.clone()).unwrap_or_default();
        // The account's default calendar (issue #30: the one chosen, else its own rather than a
        // colleague's listed first), as an event made without the form goes to.
        let default = s.pim.get(&current).and_then(|p| p.default_calendar()).map(|c| c.id.clone());
        EventForm {
            calendar: calendars
                .iter()
                .position(|c| c.0 == current && default.as_ref() == Some(&c.1))
                .or_else(|| calendars.iter().position(|c| c.0 == current))
                .unwrap_or(0),
            calendars,
            date: local_date(start, s),
            start: local_time(start, s),
            end_date: local_date(start + 3600, s),
            end: local_time(start + 3600, s),
            hours12: s.clock.style.hours12,
            can_invite: true,
            all_categories: all_categories(s, &[]),
            // As Outlook sets a new appointment's reminder.
            reminder: Some(DEFAULT_REMINDER),
            meeting: MeetingState::new(s, None),
            ..Default::default()
        }
    }

    /// An existing event, opened at one of its occurrences.
    pub fn open(s: &Session, uid: &str, occurrence: i64) -> Option<EventForm> {
        let (account, stored) = s.find_event(uid)?;
        let e = &stored.event;
        let calendars = writable_calendars(s);
        let read_only = s.pim.get(account).is_some_and(|p| p.calendars.iter().any(|c| c.id == stored.calendar && c.read_only));
        // A recurring event is edited as a series: its first occurrence's dates.
        let (start, end) = if e.all_day { (s.clock.utc(e.start), s.clock.utc(e.end - 86_400)) } else { (e.start, e.end) };
        let mine: Vec<String> = s.accounts.iter().map(|a| a.config.email.to_ascii_lowercase()).collect();
        // As an address is written in a recipient box (a name with a comma quoted), so that the
        // guests read back as they were.
        let written = |a: &mailcraft_engine::model::ical::Attendee| mailcraft_engine::model::EmailAddress::new(&a.name, &a.email).to_string();
        let links = e.links();
        Some(EventForm {
            uid: Some(uid.to_string()),
            occurrence: Some(occurrence),
            recurring: e.rrule.is_some(),
            calendar: calendars.iter().position(|c| c.0 == account && c.1 == stored.calendar).unwrap_or(0),
            calendars,
            summary: e.summary.clone(),
            location: e.location.clone(),
            description: e.description.clone(),
            date: local_date(start, s),
            start: local_time(start, s),
            end_date: local_date(end, s),
            end: local_time(end, s),
            all_day: e.all_day,
            hours12: s.clock.style.hours12,
            reminder: mailcraft_engine::model::ical::reminder_minutes(e),
            privacy: mailcraft_engine::model::ical::privacy(e),
            show_as: mailcraft_engine::model::ical::show_as(e),
            repeat: Repeat::from_rrule(e.rrule.as_deref()),
            organizer: e.organizer.as_ref().map(written),
            attendees: e.attendees.iter().map(|a| (if a.name.is_empty() { a.email.clone() } else { a.name.clone() }, a.partstat.clone())).collect(),
            needs_reply: e.attendees.iter().any(|a| mine.contains(&a.email.to_ascii_lowercase()) && a.partstat == "NEEDS-ACTION"),
            guests: e.attendees.iter().map(written).collect::<Vec<_>>().join(", "),
            can_invite: e.organizer.as_ref().is_none_or(|o| mine.contains(&o.email.to_ascii_lowercase())),
            reading_notes: read_only || !links.is_empty(),
            links,
            join: s.join_link(e),
            meeting: MeetingState::new(s, Some(e)),
            read_only,
            categories: e.categories.clone(),
            all_categories: all_categories(s, &e.categories),
            error: None,
        })
    }

    pub(crate) fn params(&self) -> Value {
        let (start, end) = if self.all_day {
            (self.date.trim().to_string(), self.end_date.trim().to_string())
        } else {
            (format!("{}T{}", self.date.trim(), time_param(&self.start)), format!("{}T{}", self.end_date.trim(), time_param(&self.end)))
        };
        let mut p = json!({
            "summary": self.summary.trim(),
            "location": self.location.trim(),
            "description": self.description,
            "start": start,
            "end": end,
            "allDay": self.all_day,
            "repeat": self.repeat.param(),
            "reminder": self.reminder.map_or_else(|| json!("none"), |m| json!(m)),
            "categories": self.categories,
            "privacy": self.privacy.code(),
            "showAs": self.show_as.code(),
        });
        if self.can_invite {
            p["attendees"] = json!(self.guests.trim());
        }
        online_meeting::param(self, &mut p);
        match &self.uid {
            Some(u) => p["uid"] = json!(u),
            None => {
                if let Some((a, c, _)) = self.calendars.get(self.calendar) {
                    p["account"] = json!(a);
                    p["calendar"] = json!(c);
                }
            }
        }
        p
    }
}

/// The categories to pick from (name, colour): the ones made in MailCraft, and any an event
/// already has from elsewhere.
fn all_categories(s: &Session, has: &[String]) -> Vec<(String, u8)> {
    let mut out: Vec<(String, u8)> = s.prefs.categories.iter().map(|c| (c.name.clone(), c.color)).collect();
    for c in has {
        if !out.iter().any(|(n, _)| n.eq_ignore_ascii_case(c)) {
            out.push((c.clone(), s.category_color_of(c)));
        }
    }
    out
}

fn partstat_label(p: &str) -> &'static str {
    match p {
        "ACCEPTED" => "Accepted",
        "DECLINED" => "Declined",
        "TENTATIVE" => "Tentative",
        _ => "No answer yet",
    }
}

/// Width of the forms' label column.
const LABEL_W: f32 = 104.0;
/// Height of a one-line field.
const FIELD_H: f32 = 30.0;

/// A form's heading: a round badge (an icon, or a person's initials) and the title.
fn form_header(ui: &mut Ui, t: &Tokens, badge: Result<Icon, String>, title: &str) {
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::hover());
        ui.painter().circle_filled(r.center(), 20.0, t.accent_soft);
        match badge {
            Ok(icon) => icons::paint(ui.painter(), icons::centered(r, 20.0), icon, t.accent),
            Err(initials) => {
                ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, initials, theme::bold(15.0), t.accent);
            }
        }
        ui.add_space(6.0);
        ui.label(RichText::new(tl!(title)).font(theme::bold(19.0)).color(t.text));
    });
    ui.add_space(12.0);
}

/// A labelled row of a form: an optional icon and the label in a column of their own, the
/// field (or fields) after, lined up with the field's first line.
/// `icons_used`: the form puts icons before labels, so a row without one is indented to match.
fn form_row(ui: &mut Ui, t: &Tokens, icons_used: bool, icon: Option<Icon>, label: &str, add: impl FnOnce(&mut Ui)) {
    ui.horizontal_top(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(LABEL_W, FIELD_H), Sense::hover());
        let mut x = r.left();
        if let Some(icon) = icon {
            icons::paint(ui.painter(), egui::Rect::from_center_size(egui::pos2(x + 8.0, r.center().y), vec2(16.0, 16.0)), icon, t.dim);
        }
        if icons_used {
            x += 24.0;
        }
        ui.painter().text(egui::pos2(x, r.center().y), egui::Align2::LEFT_CENTER, tl!(label), theme::regular(13.0), t.dim);
        add(ui);
    });
    ui.add_space(8.0);
}

/// A one-line field of a form, `width` wide (the rest of the row when infinite).
fn field(ui: &mut Ui, value: &mut String, hint: &str, width: f32) -> egui::Response {
    let w = if width.is_finite() { width } else { ui.available_width() };
    ui.add(
        egui::TextEdit::singleline(value)
            .hint_text(hint)
            .desired_width(w)
            .min_size(vec2(w, FIELD_H))
            .margin(Margin::symmetric(8, 6))
            .align(egui::Align2::LEFT_CENTER),
    )
}

/// A form's title, in large letters across the form, and the space below it.
fn title_field(ui: &mut Ui, value: &mut String) {
    let w = ui.available_width();
    ui.add(
        egui::TextEdit::singleline(value)
            .hint_text(tl!("Add a title"))
            .font(theme::bold(16.0))
            .desired_width(w)
            .min_size(vec2(w, 36.0))
            .margin(Margin::symmetric(10, 8))
            .align(egui::Align2::LEFT_CENTER),
    );
    ui.add_space(12.0);
}

/// A field of several lines, the rest of the row wide.
fn area(ui: &mut Ui, value: &mut String, hint: &str, rows: usize) -> egui::Response {
    let w = ui.available_width();
    ui.add(egui::TextEdit::multiline(value).hint_text(hint).desired_width(w).desired_rows(rows).margin(Margin::symmetric(8, 6)))
}

/// The tallest the notes are shown before they scroll.
const NOTES_MAX_H: f32 = 320.0;

/// An event's notes to read (issue #1): an invitation reads as it does in Outlook, each link
/// under its words and opened from there, long addresses shortened, the whole scrolling when
/// it's tall.
fn notes(ui: &mut Ui, t: &Tokens, text: &str, acts: &mut Vec<Action>) {
    egui::Frame::new().stroke(egui::Stroke::new(1.0, t.border)).corner_radius(6.0).inner_margin(Margin::symmetric(10, 8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        egui::ScrollArea::vertical().id_salt("event-notes-read").max_height(NOTES_MAX_H).show(ui, |ui| {
            ui.spacing_mut().item_spacing = vec2(0.0, 2.0);
            let mut blank = 0;
            for line in mailcraft_engine::model::ical::note_lines(text) {
                // Runs of blank lines as one gap.
                if line.iter().all(|p| matches!(p, mailcraft_engine::model::ical::NotePart::Text(s) if s.trim().is_empty())) {
                    blank += 1;
                    if blank == 1 {
                        ui.add_space(6.0);
                    }
                    continue;
                }
                blank = 0;
                ui.horizontal_wrapped(|ui| {
                    for part in line {
                        match part {
                            mailcraft_engine::model::ical::NotePart::Text(s) => {
                                ui.label(RichText::new(s).color(t.text));
                            }
                            mailcraft_engine::model::ical::NotePart::Link { label, url } => {
                                let shown = if label == url { link_label(&url, 56) } else { label };
                                if ui.link(shown).on_hover_text(&url).clicked() {
                                    acts.push(Action::OpenUrl(url));
                                }
                            }
                        }
                    }
                });
            }
        });
    });
}

/// A web address as a link shows it: without its scheme (`meet.example.org/abc`), cut to `max`
/// characters. The notes box's links get the box's width; those on a row after a label, less.
fn link_label(url: &str, max: usize) -> String {
    widgets::shorten(url.split_once("://").map_or(url, |(_, r)| r), max)
}

/// The line above a form's buttons.
fn footer_rule(ui: &mut Ui, t: &Tokens) {
    ui.add_space(6.0);
    let r = ui.available_rect_before_wrap();
    ui.painter().line_segment([r.left_top(), r.right_top()], egui::Stroke::new(1.0, t.border));
    ui.add_space(10.0);
}

pub fn event(ui: &mut Ui, f: &mut EventForm, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    ui.set_width(540.0);
    form_header(ui, t, Ok(Icon::Calendar), if f.uid.is_some() { "Event" } else { "New event" });
    // An online meeting (Teams, Zoom…): Join, as Outlook has it, read-only calendar or not.
    if let Some(url) = f.join.clone() {
        if ui.add(egui::Button::new(RichText::new(tl!("Join meeting")).color(t.on_accent).strong()).fill(t.accent)).on_hover_text(&url).clicked() {
            acts.push(Action::OpenUrl(url));
        }
        ui.add_space(8.0);
    }
    online_meeting::row(ui, f, t, acts);
    ui.add_enabled_ui(!f.read_only, |ui| {
        title_field(ui, &mut f.summary);
        form_row(ui, t, true, Some(Icon::Clock), "Starts", |ui| {
            field(ui, &mut f.date, "YYYY-MM-DD", 120.0);
            if !f.all_day {
                time_field(ui, &mut f.start, 9, f.hours12);
            }
        });
        form_row(ui, t, true, None, "Ends", |ui| {
            field(ui, &mut f.end_date, "YYYY-MM-DD", 120.0);
            if !f.all_day {
                time_field(ui, &mut f.end, 10, f.hours12);
            }
            ui.add_space(6.0);
            ui.allocate_ui_with_layout(vec2(ui.available_width(), FIELD_H), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.checkbox(&mut f.all_day, tl!("All day"));
            });
        });
        repeat_row(ui, t, "event-repeat", &mut f.repeat);
        form_row(ui, t, true, Some(Icon::Clock), "Reminder", |ui| {
            ui.allocate_ui_with_layout(vec2(200.0, FIELD_H), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                let mut choices: Vec<Option<i64>> = std::iter::once(None).chain(REMINDER_CHOICES.iter().map(|m| Some(*m))).collect();
                // One set elsewhere (another app, a server) stays offered as it is.
                if !choices.contains(&f.reminder) {
                    choices.push(f.reminder);
                }
                egui::ComboBox::from_id_salt("event-reminder")
                    .width(194.0)
                    .selected_text(reminder_label(f.reminder))
                    .show_ui(ui, |ui| {
                        for c in choices {
                            ui.selectable_value(&mut f.reminder, c, reminder_label(c));
                        }
                    })
                    .response
                    .accessible_name(tl!("Reminder"));
            });
        });
        showing::show_as_row(ui, t, f);
        form_row(ui, t, true, Some(Icon::Location), "Location", |ui| {
            field(ui, &mut f.location, tl!("Add a place"), f32::INFINITY);
        });
        // Categories colour it in the calendar (Outlook's Categorize on an appointment).
        if !f.all_categories.is_empty() {
            form_row(ui, t, true, Some(Icon::Tag), "Categories", |ui| {
                category_chips(ui, t, &f.all_categories, &mut f.categories);
            });
        }
        if f.can_invite {
            form_row(ui, t, true, Some(Icon::People), "Invite", |ui| {
                field(ui, &mut f.guests, tl!("Names <addresses>, separated by ; or ,"), f32::INFINITY);
            });
        }
        if f.uid.is_none() && f.calendars.len() > 1 {
            let name = f.calendars.get(f.calendar).map(|c| c.2.clone()).unwrap_or_default();
            form_row(ui, t, true, Some(Icon::Calendar), "Calendar", |ui| {
                ui.allocate_ui_with_layout(vec2(260.0, FIELD_H), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    egui::ComboBox::from_id_salt("event-calendar")
                        .width(254.0)
                        .selected_text(name)
                        .show_ui(ui, |ui| {
                            for (i, c) in f.calendars.iter().enumerate() {
                                ui.selectable_value(&mut f.calendar, i, &c.2);
                            }
                        })
                        .response
                        .accessible_name(tl!("Calendar"));
                });
            });
        }
        if !f.reading_notes {
            // Tall notes scroll in their box: the dialog keeps its buttons on screen.
            egui::ScrollArea::vertical().id_salt("event-notes").max_height(NOTES_MAX_H).show(ui, |ui| {
                area(ui, &mut f.description, tl!("Notes"), 4);
            });
        }
    });
    if f.reading_notes {
        ui.add_space(4.0);
        notes(ui, t, &f.description, acts);
        if !f.read_only && ui.small_button(tl!("Edit notes")).clicked() {
            f.reading_notes = false;
        }
    }
    // The notes' links, to open (the box above only edits them): a Teams invitation's "Join the
    // meeting now", a document, a dial-in page.
    if !f.links.is_empty() && !f.reading_notes {
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(tl!("Links:")).color(t.dim).small());
            for url in &f.links {
                if ui.link(RichText::new(link_label(url, 48)).small()).on_hover_text(url).clicked() {
                    acts.push(Action::OpenUrl(url.clone()));
                }
            }
        });
    }
    if f.recurring {
        ui.add_space(4.0);
        ui.label(RichText::new(tl!("Changes apply to every occurrence.")).color(t.faint).small());
    }
    if let Some(o) = &f.organizer {
        ui.add_space(4.0);
        ui.label(RichText::new(tf!("Organizer: {name}", name = o)).color(t.dim).small());
    }
    if !f.attendees.is_empty() {
        let list: Vec<String> = f.attendees.iter().map(|(who, p)| format!("{who} ({})", tl!(partstat_label(p)))).collect();
        ui.label(RichText::new(tf!("Attendees: {names}", names = list.join(", "))).color(t.dim).small());
    }
    if f.needs_reply
        && let Some(uid) = &f.uid
    {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(tl!("Your answer:")).color(t.text));
            for (label, code) in [("Accept", "accepted"), ("Tentative", "tentative"), ("Decline", "declined")] {
                if ui.button(tl!(label)).clicked() {
                    acts.push(Action::Submit("event.respond", json!({"uid": uid, "answer": code})));
                }
            }
        });
    }
    if let Some(e) = &f.error {
        ui.add_space(6.0);
        ui.label(RichText::new(e).color(t.danger));
    }
    footer_rule(ui, t);
    let mut keep = true;
    ui.horizontal(|ui| {
        if let Some(uid) = f.uid.clone().filter(|_| !f.read_only) {
            if f.recurring {
                if ui.button(tl!("Delete this occurrence")).clicked() {
                    acts.push(Action::Submit("event.delete", json!({"uid": uid, "occurrence": f.occurrence})));
                }
                if ui.button(tl!("Delete the series")).clicked() {
                    acts.push(Action::Submit("event.delete", json!({"uid": uid})));
                }
            } else if ui.button(tl!("Delete")).clicked() {
                acts.push(Action::Submit("event.delete", json!({"uid": uid})));
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if !f.read_only && widgets::accent_button(ui, crate::icons::Icon::Calendar, "Save", t, !f.summary.trim().is_empty()).clicked() {
                f.error = None;
                acts.push(Action::Submit(if f.uid.is_some() { "event.update" } else { "event.create" }, f.params()));
            }
            if ui.button(tl!(if f.read_only { "Close" } else { "Cancel" })).clicked() {
                keep = false;
            }
        });
    });
    keep
}
