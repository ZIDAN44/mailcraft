//! The event, task and contact forms.

use crate::bidi::ComboBoxExt as _;
use crate::dialogs::set_dialog_width;
use egui::{Color32, Margin, RichText, Sense, Ui, vec2};
use mailcraft_engine::Session;
use mailcraft_engine::model::dates;
use serde_json::{Value, json};

use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};
use crate::{Action, widgets};

mod attendees;
mod event;
mod event_location;
mod online_meeting;
mod showing;
mod task;
pub use event::ContactForm;
pub use event::event;
use event::{ADDRESSES, PHONES};
pub use online_meeting::MeetingState;
#[cfg(test)]
pub(crate) use online_meeting::{add as online_meeting_add, remove as online_meeting_remove};
use task::category_chips;
pub use task::{TaskForm, status_label, task};

/// A new event's reminder (Outlook's default for appointments).
pub const DEFAULT_REMINDER: i64 = 15;
/// The form has no editor for explicit recurrence dates; this sentinel keeps them on save.
const RDATE_ONLY_REPEAT: &str = "<rdate-only>";
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
    /// The original event exported for Android's system share sheet; new events have none.
    pub share_ical: Option<(String, String)>,
    /// A simple event or recurring series that can be prefilled in the Android Calendar app.
    pub system_calendar_event: Option<mailcraft_engine::model::ical::CalEvent>,
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
    /// `""` (never), `daily`, `weekly`, `monthly`, `yearly`, a kept rule, or an RDATE-only marker.
    pub repeat: String,
    /// Its reminder, in minutes before it starts (`None`: none).
    pub reminder: Option<i64>,
    /// Who sees what of it, and how it shows its time (issue #32).
    pub privacy: mailcraft_engine::model::ical::Privacy,
    pub show_as: mailcraft_engine::model::ical::ShowAs,
    pub organizer: Option<String>,
    pub organizer_email: Option<String>,
    /// (name, email address, their answer).
    pub attendees: Vec<(String, String, String)>,
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
    let c = dates::civil(t, &s.clock);
    format!("{:04}-{:02}-{:02}", c.year, c.month, c.day)
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
        let account = s.find_event(uid)?.0.to_string();
        Self::open_in_account(s, &account, uid, occurrence)
    }

    /// An existing event, opened from a notification for this account.
    pub fn open_in_account(s: &Session, account: &str, uid: &str, occurrence: i64) -> Option<EventForm> {
        let stored = s.pim.get(account)?.events.iter().find(|stored| stored.event.uid == uid && stored.event.recurrence_id.is_none())?;
        let e = &stored.event;
        let calendars = writable_calendars(s);
        let read_only = s.pim.get(account).is_some_and(|p| p.calendars.iter().any(|c| c.id == stored.calendar && c.read_only));
        // A recurring event is edited as a series: its first occurrence's dates.
        let (start, end) = if e.all_day { (s.clock.utc(e.start), s.clock.utc(e.end - 86_400)) } else { (e.start, e.end) };
        let mine: Vec<String> = s.accounts.iter().map(|a| a.config.email.to_ascii_lowercase()).collect();
        Some(EventForm {
            uid: Some(uid.to_string()),
            occurrence: Some(occurrence),
            recurring: e.is_recurring(),
            share_ical: Some((
                if e.summary.trim().is_empty() { tl!("Event").to_string() } else { e.summary.clone() },
                mailcraft_engine::model::ical::write(std::slice::from_ref(e), None, s.clock.now),
            )),
            system_calendar_event: (e.recurrence_id.is_none()
                && e.rdates.len().saturating_add(e.exdates.len()) <= 512
                && e.rrule.as_ref().is_none_or(|rule| !rule.trim().is_empty() && !rule.bytes().any(|byte| byte.is_ascii_control())))
            .then(|| e.clone()),
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
            repeat: if e.rrule.is_none() && !e.rdates.is_empty() {
                RDATE_ONLY_REPEAT.into()
            } else {
                match e.rrule.as_deref() {
                    None => String::new(),
                    Some(r) => match r.to_ascii_uppercase().as_str() {
                        "FREQ=DAILY" => "daily".into(),
                        "FREQ=WEEKLY" => "weekly".into(),
                        "FREQ=MONTHLY" => "monthly".into(),
                        "FREQ=YEARLY" => "yearly".into(),
                        _ => r.to_string(),
                    },
                }
            },
            organizer: e.organizer.as_ref().map(|o| if o.name.is_empty() { o.email.clone() } else { format!("{} <{}>", o.name, o.email) }),
            organizer_email: e.organizer.as_ref().map(|o| o.email.clone()),
            attendees: attendees::from_event(&e.attendees),
            needs_reply: e.attendees.iter().any(|a| mine.contains(&a.email.to_ascii_lowercase()) && a.partstat == "NEEDS-ACTION"),
            guests: e
                .attendees
                .iter()
                .map(|a| if a.name.is_empty() { a.email.clone() } else { format!("{} <{}>", a.name, a.email) })
                .collect::<Vec<_>>()
                .join(", "),
            can_invite: e.organizer.as_ref().is_none_or(|o| mine.contains(&o.email.to_ascii_lowercase())),
            links: e.links(),
            join: s.join_link(e),
            meeting: MeetingState::new(s, Some(e)),
            reading_notes: read_only || !e.links().is_empty(),
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
            "reminder": self.reminder.map_or_else(|| json!("none"), |m| json!(m)),
            "categories": self.categories,
            "privacy": self.privacy.code(),
            "showAs": self.show_as.code(),
        });
        if self.repeat != RDATE_ONLY_REPEAT {
            p["repeat"] = json!(if self.repeat.is_empty() { "none" } else { self.repeat.as_str() });
        }
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

pub(super) fn field_height(ui: &Ui) -> f32 {
    if ui.ctx().input(|input| input.has_touch_screen()) { 44.0 } else { FIELD_H }
}

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
    if ui.available_width() < 480.0 {
        ui.horizontal(|ui| {
            if let Some(icon) = icon {
                let (r, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                icons::paint(ui.painter(), icons::centered(r, 16.0), icon, t.dim);
            } else if icons_used {
                ui.add_space(18.0);
            }
            ui.label(RichText::new(tl!(label)).small().color(t.dim));
        });
        ui.horizontal_wrapped(|ui| add(ui));
    } else {
        ui.horizontal_top(|ui| {
            let height = field_height(ui);
            let (r, _) = ui.allocate_exact_size(vec2(LABEL_W, height), Sense::hover());
            let rtl = crate::i18n::current().rtl();
            let mut x = if rtl { r.right() } else { r.left() };
            if let Some(icon) = icon {
                let offset = if rtl { -8.0 } else { 8.0 };
                icons::paint(ui.painter(), egui::Rect::from_center_size(egui::pos2(x + offset, r.center().y), vec2(16.0, 16.0)), icon, t.dim);
            }
            if icons_used {
                x += if rtl { -24.0 } else { 24.0 };
            }
            let align = if rtl { egui::Align2::RIGHT_CENTER } else { egui::Align2::LEFT_CENTER };
            ui.painter().text(egui::pos2(x, r.center().y), align, tl!(label), theme::regular(13.0), t.dim);
            add(ui);
        });
    }
    ui.add_space(8.0);
}

fn field_is_rtl(value: &str) -> bool {
    if value.is_empty() { crate::i18n::current().rtl() } else { crate::bidi::paragraph_rtl(value) }
}

fn field_alignment(value: &str) -> egui::Align2 {
    if field_is_rtl(value) { egui::Align2::RIGHT_CENTER } else { egui::Align2::LEFT_CENTER }
}

/// A one-line field of a form, `width` wide (the rest of the row when infinite).
fn field(ui: &mut Ui, value: &mut String, hint: &str, width: f32) -> egui::Response {
    let w = if width.is_finite() { width } else { ui.available_width() };
    let align = field_alignment(value);
    let height = field_height(ui);
    ui.add(egui::TextEdit::singleline(value).hint_text(hint).desired_width(w).min_size(vec2(w, height)).margin(Margin::symmetric(8, 6)).align(align))
}

/// A field of several lines, the rest of the row wide.
fn area(ui: &mut Ui, value: &mut String, hint: &str, rows: usize) -> egui::Response {
    let w = ui.available_width();
    let align = if field_is_rtl(value) { egui::Align2::RIGHT_TOP } else { egui::Align2::LEFT_TOP };
    ui.add(egui::TextEdit::multiline(value).hint_text(hint).desired_width(w).desired_rows(rows).margin(Margin::symmetric(8, 6)).align(align))
}

/// The line above a form's buttons.
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
                                let shown =
                                    if label == url { widgets::shorten(url.split_once("://").map_or(url.as_str(), |(_, r)| r), 56) } else { label };
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

fn footer_rule(ui: &mut Ui, t: &Tokens) {
    ui.add_space(6.0);
    let r = ui.available_rect_before_wrap();
    ui.painter().line_segment([r.left_top(), r.right_top()], egui::Stroke::new(1.0, t.border));
    ui.add_space(10.0);
}

impl ContactForm {
    /// A new contact with a name and address filled in (Add to contacts on someone in a message).
    pub fn from_address(a: &mailcraft_engine::model::EmailAddress) -> ContactForm {
        let mut f = ContactForm { uid: None, name: a.name.trim().to_string(), ..Default::default() };
        f.emails[0] = a.email.trim().to_string();
        f
    }

    pub fn edit(c: &mailcraft_engine::model::vcard::Card) -> ContactForm {
        let mut f = ContactForm {
            uid: Some(c.uid.clone()),
            name: c.display(),
            org: c.org.clone(),
            title: c.title.clone(),
            note: c.note.clone(),
            birthday: c.birthday.clone(),
            ..Default::default()
        };
        for (i, e) in c.emails.iter().enumerate() {
            match f.emails.get_mut(i) {
                Some(slot) => *slot = e.value.clone(),
                None => f.more_emails.push(e.value.clone()),
            }
        }
        for ph in &c.phones {
            let kind = match ph.kind.as_str() {
                "mobile" => "cell",
                k if PHONES.iter().any(|(_, x)| *x == k) => k,
                _ => "",
            };
            let slot = PHONES.iter().position(|(_, k)| *k == kind).and_then(|i| f.phones.get_mut(i)).filter(|s| s.is_empty());
            match slot {
                Some(slot) => *slot = ph.value.clone(),
                None => f.more_phones.push(ph.clone()),
            }
        }
        for a in &c.addresses {
            let i = ADDRESSES.iter().position(|(_, k)| *k == a.kind).unwrap_or(2);
            if let Some(slot) = f.addresses.get_mut(i).filter(|s| s.is_empty()) {
                *slot = a.clone();
            }
        }
        f.address_shown = f.addresses.iter().position(|a| !a.is_empty()).unwrap_or(0);
        f
    }

    pub(crate) fn params(&self) -> Value {
        let emails: Vec<&str> =
            self.emails.iter().map(|e| e.trim()).chain(self.more_emails.iter().map(String::as_str)).filter(|e| !e.is_empty()).collect();
        let mut phones: Vec<Value> = PHONES
            .iter()
            .zip(&self.phones)
            .filter(|(_, v)| !v.trim().is_empty())
            .map(|((_, kind), v)| json!({"value": v.trim(), "kind": kind}))
            .collect();
        phones.extend(self.more_phones.iter().map(|x| json!({"value": x.value, "kind": x.kind})));
        let addresses: Vec<Value> = ADDRESSES
            .iter()
            .zip(&self.addresses)
            .filter(|(_, a)| !a.is_empty())
            .map(|((_, kind), a)| {
                json!({"kind": kind, "street": a.street, "city": a.city, "region": a.region, "postcode": a.postcode, "country": a.country})
            })
            .collect();
        let mut p = json!({
            "name": self.name.trim(),
            "emails": emails,
            "phones": phones,
            "addresses": addresses,
            "birthday": self.birthday.trim(),
            "org": self.org.trim(),
            "title": self.title.trim(),
            "note": self.note,
        });
        if let Some(u) = &self.uid {
            p["uid"] = json!(u);
        }
        p
    }
}

pub fn contact(ui: &mut Ui, f: &mut ContactForm, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    set_dialog_width(ui, 560.0);
    // The person's initials once a name is typed, as the people list shows them.
    let initials: String = f.name.split_whitespace().filter_map(|w| w.chars().next()).take(2).collect::<String>().to_uppercase();
    let badge = if initials.is_empty() { Ok(Icon::Person) } else { Err(initials) };
    form_header(ui, t, badge, if f.uid.is_some() { "Edit contact" } else { "New contact" });
    let tall = ui.ctx().content_rect().height() * 0.62;
    egui::ScrollArea::vertical().max_height(tall.max(260.0)).auto_shrink([false, true]).show(ui, |ui| {
        // Room on the right for the scroll bar, so it doesn't run over the fields.
        ui.set_max_width((ui.available_width() - 14.0).max(120.0));
        form_row(ui, t, false, None, "Name", |ui| {
            field(ui, &mut f.name, tl!("First and last name"), f32::INFINITY);
        });
        form_row(ui, t, false, None, "Company", |ui| {
            field(ui, &mut f.org, "", f32::INFINITY);
        });
        form_row(ui, t, false, None, "Job title", |ui| {
            field(ui, &mut f.title, "", f32::INFINITY);
        });
        for (i, label) in ["Email", "Email 2", "Email 3"].into_iter().enumerate() {
            if let Some(v) = f.emails.get_mut(i) {
                form_row(ui, t, false, None, label, |ui| {
                    field(ui, v, "", f32::INFINITY);
                });
            }
        }
        for (i, (label, _)) in PHONES.iter().enumerate() {
            if let Some(v) = f.phones.get_mut(i) {
                let label = match *label {
                    "Business" => "Business phone",
                    "Home" => "Home phone",
                    "Mobile" => "Mobile phone",
                    "Fax" => "Business fax",
                    _ => "Other phone",
                };
                form_row(ui, t, false, None, label, |ui| {
                    field(ui, v, "", f32::INFINITY);
                });
            }
        }
        // One address at a time, as Outlook's form: choose which.
        form_row(ui, t, false, None, "Address", |ui| {
            let shown = f.address_shown.min(2);
            let name = |i: usize| match i {
                0 => tl!("Business"),
                1 => tl!("Home"),
                _ => tl!("Other"),
            };
            egui::ComboBox::from_id_salt("contact-address").selected_text(name(shown)).show_ui_rtl(ui, |ui| {
                for i in 0..3 {
                    let filled = f.addresses.get(i).is_some_and(|a| !a.is_empty());
                    let label = if filled { format!("{} ✓", name(i)) } else { name(i).to_string() };
                    if ui.selectable_label(shown == i, label).clicked() {
                        f.address_shown = i;
                    }
                }
            });
        });
        let shown = f.address_shown.min(2);
        if let Some(a) = f.addresses.get_mut(shown) {
            form_row(ui, t, false, None, "Street", |ui| {
                area(ui, &mut a.street, "", 2);
            });
            form_row(ui, t, false, None, "Postal code", |ui| {
                field(ui, &mut a.postcode, "", 110.0);
                ui.label(RichText::new(tl!("City")).color(t.dim));
                field(ui, &mut a.city, "", f32::INFINITY);
            });
            form_row(ui, t, false, None, "State/Province", |ui| {
                field(ui, &mut a.region, "", f32::INFINITY);
            });
            form_row(ui, t, false, None, "Country/Region", |ui| {
                field(ui, &mut a.country, "", f32::INFINITY);
            });
        }
        form_row(ui, t, false, None, "Birthday", |ui| {
            field(ui, &mut f.birthday, tl!("YYYY-MM-DD"), 140.0);
        });
        form_row(ui, t, false, None, "Notes", |ui| {
            area(ui, &mut f.note, "", 3);
        });
    });
    if let Some(e) = &f.error {
        ui.label(RichText::new(e).color(t.danger));
    }
    footer_rule(ui, t);
    let mut keep = true;
    let any_email = f.emails.iter().any(|e| !e.trim().is_empty());
    ui.with_layout(crate::bidi::row_from_end(egui::Align::Center), |ui| {
        if widgets::accent_button(ui, crate::icons::Icon::Person, "Save", t, !f.name.trim().is_empty() || any_email).clicked() {
            f.error = None;
            acts.push(Action::Submit(if f.uid.is_some() { "contact.update" } else { "contact.create" }, f.params()));
        }
        if ui.button(tl!("Cancel")).clicked() {
            keep = false;
        }
    });
    keep
}
