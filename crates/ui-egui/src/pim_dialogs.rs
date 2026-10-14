//! The event and contact forms.

use egui::{RichText, Ui};
use mailcraft_engine::Session;
use mailcraft_engine::model::dates;
use serde_json::{Value, json};

use crate::theme::{self, Tokens};
use crate::{Action, widgets};

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
    /// Local `YYYY-MM-DD` and `HH:MM`.
    pub date: String,
    pub start: String,
    pub end_date: String,
    pub end: String,
    pub all_day: bool,
    /// `""` (never), `daily`, `weekly`, `monthly`, `yearly`, or a rule kept as it was.
    pub repeat: String,
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
    pub error: Option<String>,
}

fn local_date(t: i64, s: &Session) -> String {
    let c = dates::civil(t, &s.clock);
    format!("{:04}-{:02}-{:02}", c.year, c.month, c.day)
}

fn local_time(t: i64, s: &Session) -> String {
    let c = dates::civil(t, &s.clock);
    format!("{:02}:{:02}", c.hour, c.minute)
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
        EventForm {
            calendar: calendars.iter().position(|c| c.0 == current).unwrap_or(0),
            calendars,
            date: local_date(start, s),
            start: local_time(start, s),
            end_date: local_date(start + 3600, s),
            end: local_time(start + 3600, s),
            can_invite: true,
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
        let (start, end) = if e.all_day {
            let off = i64::from(s.clock.offset);
            (e.start - off, e.end - off - 86_400)
        } else {
            (e.start, e.end)
        };
        let mine: Vec<String> = s.accounts.iter().map(|a| a.config.email.to_ascii_lowercase()).collect();
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
            repeat: match e.rrule.as_deref() {
                None => String::new(),
                Some(r) => match r.to_ascii_uppercase().as_str() {
                    "FREQ=DAILY" => "daily".into(),
                    "FREQ=WEEKLY" => "weekly".into(),
                    "FREQ=MONTHLY" => "monthly".into(),
                    "FREQ=YEARLY" => "yearly".into(),
                    _ => r.to_string(),
                },
            },
            organizer: e.organizer.as_ref().map(|o| if o.name.is_empty() { o.email.clone() } else { format!("{} <{}>", o.name, o.email) }),
            attendees: e.attendees.iter().map(|a| (if a.name.is_empty() { a.email.clone() } else { a.name.clone() }, a.partstat.clone())).collect(),
            needs_reply: e.attendees.iter().any(|a| mine.contains(&a.email.to_ascii_lowercase()) && a.partstat == "NEEDS-ACTION"),
            guests: e
                .attendees
                .iter()
                .map(|a| if a.name.is_empty() { a.email.clone() } else { format!("{} <{}>", a.name, a.email) })
                .collect::<Vec<_>>()
                .join(", "),
            can_invite: e.organizer.as_ref().is_none_or(|o| mine.contains(&o.email.to_ascii_lowercase())),
            read_only,
            error: None,
        })
    }

    pub(crate) fn params(&self) -> Value {
        let (start, end) = if self.all_day {
            (self.date.trim().to_string(), self.end_date.trim().to_string())
        } else {
            (format!("{}T{}", self.date.trim(), self.start.trim()), format!("{}T{}", self.end_date.trim(), self.end.trim()))
        };
        let mut p = json!({
            "summary": self.summary.trim(),
            "location": self.location.trim(),
            "description": self.description,
            "start": start,
            "end": end,
            "allDay": self.all_day,
            "repeat": if self.repeat.is_empty() { "none" } else { self.repeat.as_str() },
        });
        if self.can_invite {
            p["attendees"] = json!(self.guests.trim());
        }
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

fn partstat_label(p: &str) -> &'static str {
    match p {
        "ACCEPTED" => "Accepted",
        "DECLINED" => "Declined",
        "TENTATIVE" => "Tentative",
        _ => "No answer yet",
    }
}

pub fn event(ui: &mut Ui, f: &mut EventForm, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    ui.set_width(500.0);
    ui.label(RichText::new(tl!(if f.uid.is_some() { "Event" } else { "New event" })).font(theme::bold(18.0)).color(t.text));
    ui.add_space(6.0);
    let w = ui.available_width();
    ui.add_enabled_ui(!f.read_only, |ui| {
        ui.add_sized([w, 28.0], egui::TextEdit::singleline(&mut f.summary).hint_text(tl!("Add a title")).font(theme::regular(15.0)));
        ui.add_space(6.0);
        egui::Grid::new("event-form").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
            ui.label(tl!("Starts"));
            ui.horizontal(|ui| {
                ui.add_sized([104.0, 24.0], egui::TextEdit::singleline(&mut f.date).hint_text("YYYY-MM-DD"));
                if !f.all_day {
                    ui.add_sized([60.0, 24.0], egui::TextEdit::singleline(&mut f.start).hint_text("09:00"));
                }
            });
            ui.end_row();
            ui.label(tl!("Ends"));
            ui.horizontal(|ui| {
                ui.add_sized([104.0, 24.0], egui::TextEdit::singleline(&mut f.end_date).hint_text("YYYY-MM-DD"));
                if !f.all_day {
                    ui.add_sized([60.0, 24.0], egui::TextEdit::singleline(&mut f.end).hint_text("10:00"));
                }
                ui.checkbox(&mut f.all_day, tl!("All day"));
            });
            ui.end_row();
            ui.label(tl!("Repeat"));
            let custom = !matches!(f.repeat.as_str(), "" | "daily" | "weekly" | "monthly" | "yearly");
            let label = |r: &str| -> String {
                match r {
                    "" => tl!("Never").to_string(),
                    "daily" => tl!("Every day").to_string(),
                    "weekly" => tl!("Every week").to_string(),
                    "monthly" => tl!("Every month").to_string(),
                    "yearly" => tl!("Every year").to_string(),
                    _ => tl!("Custom").to_string(),
                }
            };
            egui::ComboBox::from_id_salt("event-repeat").selected_text(label(&f.repeat)).show_ui(ui, |ui| {
                for r in ["", "daily", "weekly", "monthly", "yearly"] {
                    ui.selectable_value(&mut f.repeat, r.to_string(), label(r));
                }
                if custom {
                    let keep = f.repeat.clone();
                    ui.selectable_value(&mut f.repeat, keep, tl!("Custom"));
                }
            });
            ui.end_row();
            ui.label(tl!("Location"));
            ui.add_sized([w - 90.0, 24.0], egui::TextEdit::singleline(&mut f.location));
            ui.end_row();
            if f.can_invite {
                ui.label(tl!("Invite"));
                ui.add_sized([w - 90.0, 24.0], egui::TextEdit::singleline(&mut f.guests).hint_text(tl!("Names <addresses>, separated by ; or ,")));
                ui.end_row();
            }
            if f.uid.is_none() && f.calendars.len() > 1 {
                ui.label(tl!("Calendar"));
                let name = f.calendars.get(f.calendar).map(|c| c.2.clone()).unwrap_or_default();
                egui::ComboBox::from_id_salt("event-calendar").selected_text(name).show_ui(ui, |ui| {
                    for (i, c) in f.calendars.iter().enumerate() {
                        ui.selectable_value(&mut f.calendar, i, &c.2);
                    }
                });
                ui.end_row();
            }
        });
        ui.add_space(4.0);
        ui.add(egui::TextEdit::multiline(&mut f.description).hint_text(tl!("Notes")).desired_rows(3).desired_width(f32::INFINITY));
    });
    if f.recurring {
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
    ui.add_space(10.0);
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

/// A person being added or edited.
#[derive(Clone, Debug, Default)]
pub struct ContactForm {
    pub uid: Option<String>,
    pub name: String,
    /// One per line.
    pub emails: String,
    pub phones: String,
    pub org: String,
    pub title: String,
    pub note: String,
    pub error: Option<String>,
}

impl ContactForm {
    pub fn edit(c: &mailcraft_engine::model::vcard::Card) -> ContactForm {
        ContactForm {
            uid: Some(c.uid.clone()),
            name: c.display(),
            emails: c.emails.iter().map(|e| e.value.clone()).collect::<Vec<_>>().join("\n"),
            phones: c.phones.iter().map(|e| e.value.clone()).collect::<Vec<_>>().join("\n"),
            org: c.org.clone(),
            title: c.title.clone(),
            note: c.note.clone(),
            error: None,
        }
    }

    pub(crate) fn params(&self) -> Value {
        let mut p = json!({
            "name": self.name.trim(),
            "emails": self.emails,
            "phones": self.phones,
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
    ui.set_width(440.0);
    ui.label(RichText::new(tl!(if f.uid.is_some() { "Edit contact" } else { "New contact" })).font(theme::bold(18.0)).color(t.text));
    ui.add_space(6.0);
    egui::Grid::new("contact-form").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
        let w = 320.0;
        ui.label(tl!("Name"));
        ui.add_sized([w, 24.0], egui::TextEdit::singleline(&mut f.name));
        ui.end_row();
        ui.label(tl!("Email"));
        ui.add_sized([w, 44.0], egui::TextEdit::multiline(&mut f.emails).hint_text(tl!("One address per line")).desired_rows(2));
        ui.end_row();
        ui.label(tl!("Phone"));
        ui.add_sized([w, 44.0], egui::TextEdit::multiline(&mut f.phones).desired_rows(2));
        ui.end_row();
        ui.label(tl!("Company"));
        ui.add_sized([w, 24.0], egui::TextEdit::singleline(&mut f.org));
        ui.end_row();
        ui.label(tl!("Job title"));
        ui.add_sized([w, 24.0], egui::TextEdit::singleline(&mut f.title));
        ui.end_row();
        ui.label(tl!("Notes"));
        ui.add_sized([w, 60.0], egui::TextEdit::multiline(&mut f.note).desired_rows(3));
        ui.end_row();
    });
    if let Some(e) = &f.error {
        ui.add_space(6.0);
        ui.label(RichText::new(e).color(t.danger));
    }
    ui.add_space(10.0);
    let mut keep = true;
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if widgets::accent_button(ui, crate::icons::Icon::Person, "Save", t, !f.name.trim().is_empty() || !f.emails.trim().is_empty()).clicked() {
            f.error = None;
            acts.push(Action::Submit(if f.uid.is_some() { "contact.update" } else { "contact.create" }, f.params()));
        }
        if ui.button(tl!("Cancel")).clicked() {
            keep = false;
        }
    });
    keep
}
