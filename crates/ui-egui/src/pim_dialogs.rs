//! The event and contact forms.

use egui::{Color32, Margin, RichText, Sense, Ui, vec2};
use mailcraft_engine::Session;
use mailcraft_engine::model::dates;
use serde_json::{Value, json};

use crate::icons::{self, Icon};
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
    /// Its categories, and every category there is (name, colour) to pick from.
    pub categories: Vec<String>,
    pub all_categories: Vec<(String, u8)>,
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
            all_categories: all_categories(s, &[]),
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
            categories: e.categories.clone(),
            all_categories: all_categories(s, &e.categories),
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
            "categories": self.categories,
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
            .vertical_align(egui::Align::Center),
    )
}

/// A field of several lines, the rest of the row wide.
fn area(ui: &mut Ui, value: &mut String, hint: &str, rows: usize) -> egui::Response {
    let w = ui.available_width();
    ui.add(egui::TextEdit::multiline(value).hint_text(hint).desired_width(w).desired_rows(rows).margin(Margin::symmetric(8, 6)))
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
    ui.add_enabled_ui(!f.read_only, |ui| {
        let w = ui.available_width();
        ui.add(
            egui::TextEdit::singleline(&mut f.summary)
                .hint_text(tl!("Add a title"))
                .font(theme::bold(16.0))
                .desired_width(w)
                .min_size(vec2(w, 36.0))
                .margin(Margin::symmetric(10, 8))
                .vertical_align(egui::Align::Center),
        );
        ui.add_space(12.0);
        form_row(ui, t, true, Some(Icon::Clock), "Starts", |ui| {
            field(ui, &mut f.date, "YYYY-MM-DD", 120.0);
            if !f.all_day {
                field(ui, &mut f.start, "09:00", 70.0);
            }
        });
        form_row(ui, t, true, None, "Ends", |ui| {
            field(ui, &mut f.end_date, "YYYY-MM-DD", 120.0);
            if !f.all_day {
                field(ui, &mut f.end, "10:00", 70.0);
            }
            ui.add_space(6.0);
            ui.allocate_ui_with_layout(vec2(ui.available_width(), FIELD_H), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.checkbox(&mut f.all_day, tl!("All day"));
            });
        });
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
        form_row(ui, t, true, Some(Icon::Sync), "Repeat", |ui| {
            ui.allocate_ui_with_layout(vec2(200.0, FIELD_H), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                egui::ComboBox::from_id_salt("event-repeat").width(194.0).selected_text(label(&f.repeat)).show_ui(ui, |ui| {
                    for r in ["", "daily", "weekly", "monthly", "yearly"] {
                        ui.selectable_value(&mut f.repeat, r.to_string(), label(r));
                    }
                    if custom {
                        let keep = f.repeat.clone();
                        ui.selectable_value(&mut f.repeat, keep, tl!("Custom"));
                    }
                });
            });
        });
        form_row(ui, t, true, Some(Icon::Location), "Location", |ui| {
            field(ui, &mut f.location, tl!("Add a place"), f32::INFINITY);
        });
        // Categories colour it in the calendar (Outlook's Categorize on an appointment).
        if !f.all_categories.is_empty() {
            form_row(ui, t, true, Some(Icon::Tag), "Categories", |ui| {
                ui.horizontal_wrapped(|ui| {
                    for (name, color) in f.all_categories.clone() {
                        let on = f.categories.iter().any(|c| c.eq_ignore_ascii_case(&name));
                        let c = theme::category_color(color, t.dark);
                        let text = RichText::new(format!("● {}", name)).color(if on { Color32::WHITE } else { c });
                        let r = ui.add(
                            egui::Button::new(text)
                                .fill(if on { c } else { Color32::TRANSPARENT })
                                .stroke(egui::Stroke::new(1.0, c))
                                .corner_radius(10),
                        );
                        if r.clicked() {
                            if on {
                                f.categories.retain(|x| !x.eq_ignore_ascii_case(&name));
                            } else {
                                f.categories.push(name.clone());
                            }
                        }
                    }
                });
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
                    egui::ComboBox::from_id_salt("event-calendar").width(254.0).selected_text(name).show_ui(ui, |ui| {
                        for (i, c) in f.calendars.iter().enumerate() {
                            ui.selectable_value(&mut f.calendar, i, &c.2);
                        }
                    });
                });
            });
        }
        area(ui, &mut f.description, tl!("Notes"), 4);
    });
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
    ui.set_width(500.0);
    // The person's initials once a name is typed, as the people list shows them.
    let initials: String = f.name.split_whitespace().filter_map(|w| w.chars().next()).take(2).collect::<String>().to_uppercase();
    let badge = if initials.is_empty() { Ok(Icon::Person) } else { Err(initials) };
    form_header(ui, t, badge, if f.uid.is_some() { "Edit contact" } else { "New contact" });
    form_row(ui, t, false, None, "Name", |ui| {
        field(ui, &mut f.name, tl!("First and last name"), f32::INFINITY);
    });
    form_row(ui, t, false, None, "Company", |ui| {
        field(ui, &mut f.org, "", f32::INFINITY);
    });
    form_row(ui, t, false, None, "Job title", |ui| {
        field(ui, &mut f.title, "", f32::INFINITY);
    });
    form_row(ui, t, false, None, "Email", |ui| {
        area(ui, &mut f.emails, tl!("One address per line"), 1);
    });
    form_row(ui, t, false, None, "Phone", |ui| {
        area(ui, &mut f.phones, tl!("One number per line"), 1);
    });
    form_row(ui, t, false, None, "Notes", |ui| {
        area(ui, &mut f.note, "", 3);
    });
    if let Some(e) = &f.error {
        ui.label(RichText::new(e).color(t.danger));
    }
    footer_rule(ui, t);
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
