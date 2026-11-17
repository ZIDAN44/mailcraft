use super::*;

pub fn event(
    ui: &mut Ui,
    f: &mut EventForm,
    t: &Tokens,
    can_open_map: bool,
    can_share_calendar_events: bool,
    can_export_calendar_events: bool,
    can_add_to_system_calendar: bool,
    acts: &mut Vec<Action>,
) -> bool {
    set_dialog_width(ui, 540.0);
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
        let w = ui.available_width();
        let title_alignment = field_alignment(&f.summary);
        let title_height = field_height(ui);
        ui.add(
            egui::TextEdit::singleline(&mut f.summary)
                .hint_text(tl!("Add a title"))
                .font(theme::bold(16.0))
                .desired_width(w)
                .min_size(vec2(w, title_height))
                .margin(Margin::symmetric(10, 8))
                .align(title_alignment),
        );
        ui.add_space(12.0);
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
            let height = field_height(ui);
            ui.allocate_ui_with_layout(vec2(ui.available_width(), height), crate::bidi::row_from_start(egui::Align::Center), |ui| {
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
            let height = field_height(ui);
            ui.allocate_ui_with_layout(vec2(200.0, height), crate::bidi::row_from_start(egui::Align::Center), |ui| {
                egui::ComboBox::from_id_salt("event-repeat").width(194.0).selected_text(label(&f.repeat)).show_ui_rtl(ui, |ui| {
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
        form_row(ui, t, true, Some(Icon::Clock), "Reminder", |ui| {
            let height = field_height(ui);
            ui.allocate_ui_with_layout(vec2(200.0, height), crate::bidi::row_from_start(egui::Align::Center), |ui| {
                let mut choices: Vec<Option<i64>> = std::iter::once(None).chain(REMINDER_CHOICES.iter().map(|m| Some(*m))).collect();
                // One set elsewhere (another app, a server) stays offered as it is.
                if !choices.contains(&f.reminder) {
                    choices.push(f.reminder);
                }
                egui::ComboBox::from_id_salt("event-reminder").width(194.0).selected_text(reminder_label(f.reminder)).show_ui_rtl(ui, |ui| {
                    for c in choices {
                        ui.selectable_value(&mut f.reminder, c, reminder_label(c));
                    }
                });
            });
        });
        showing::show_as_row(ui, t, f);
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
                let height = field_height(ui);
                ui.allocate_ui_with_layout(vec2(260.0, height), crate::bidi::row_from_start(egui::Align::Center), |ui| {
                    egui::ComboBox::from_id_salt("event-calendar").width(254.0).selected_text(name).show_ui_rtl(ui, |ui| {
                        for (i, c) in f.calendars.iter().enumerate() {
                            ui.selectable_value(&mut f.calendar, i, &c.2);
                        }
                    });
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
    event_location::show(ui, f, t, can_open_map, acts);
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
                let shown = url.split_once("://").map_or(url.as_str(), |(_, r)| r);
                if ui.link(RichText::new(crate::widgets::shorten(shown, 48)).small()).on_hover_text(url).clicked() {
                    acts.push(Action::OpenUrl(url.clone()));
                }
            }
        });
    }
    if f.recurring {
        ui.add_space(4.0);
        ui.label(RichText::new(tl!("Changes apply to every occurrence.")).color(t.faint).small());
    }
    attendees::organizer(ui, t, f.organizer.as_deref(), f.organizer_email.as_deref(), acts);
    if !f.attendees.is_empty() {
        attendees::show(ui, t, &f.attendees, acts);
    }
    if f.needs_reply
        && let Some(uid) = &f.uid
    {
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
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
    ui.horizontal_wrapped(|ui| {
        if can_share_calendar_events
            && let Some((title, ical)) = &f.share_ical
            && ui.button(tl!("Share event")).clicked()
        {
            acts.push(Action::ShareCalendarEvent { title: title.clone(), ical: ical.clone() });
        }
        if can_export_calendar_events
            && let Some((name, ical)) = &f.share_ical
            && ui.button(tl!("Save event")).clicked()
        {
            acts.push(Action::ExportCalendarEvent { name: name.clone(), ical: ical.clone() });
        }
        if can_add_to_system_calendar
            && let Some(event) = &f.system_calendar_event
            && ui.button(tl!("Add to Android calendar")).clicked()
        {
            acts.push(Action::AddToSystemCalendar {
                title: if event.summary.trim().is_empty() { tl!("Event").to_string() } else { event.summary.clone() },
                location: event.location.clone(),
                description: if event.join_url.is_empty() {
                    event.description.clone()
                } else if event.description.trim().is_empty() {
                    event.join_url.clone()
                } else {
                    format!("{}\n{}", event.description.trim_end(), event.join_url)
                },
                start: event.start,
                end: event.end,
                all_day: event.all_day,
                timezone: if event.all_day { Some("UTC".into()) } else { (!event.tzid.is_empty()).then(|| event.tzid.clone()) },
                rrule: event.rrule.clone(),
                rdates: event.rdates.clone(),
                exdates: event.exdates.clone(),
                attendees: event.attendees.iter().map(|attendee| attendee.email.clone()).collect(),
            });
        }
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
        ui.with_layout(crate::bidi::row_from_end(egui::Align::Center), |ui| {
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

/// Outlook's phone fields, and the kind each keeps.
pub(super) const PHONES: [(&str, &str); 5] = [("Business", "work"), ("Home", "home"), ("Mobile", "cell"), ("Fax", "fax"), ("Other", "")];
/// Outlook's addresses, and the kind each keeps.
pub(super) const ADDRESSES: [(&str, &str); 3] = [("Business", "work"), ("Home", "home"), ("Other", "")];

/// A person being added or edited: Outlook's contact form (its emails, phones by kind, business,
/// home and other address, birthday).
#[derive(Clone, Debug, Default)]
pub struct ContactForm {
    pub uid: Option<String>,
    pub name: String,
    /// Email, Email 2 and Email 3.
    pub emails: [String; 3],
    /// More than three, kept as they were.
    pub more_emails: Vec<String>,
    /// Business, home, mobile, fax and other (as [`PHONES`]).
    pub phones: [String; 5],
    /// Numbers beyond one of each kind, kept as they were.
    pub more_phones: Vec<mailcraft_engine::model::vcard::Typed>,
    /// Business, home and other (as [`ADDRESSES`]).
    pub addresses: [mailcraft_engine::model::vcard::PostalAddress; 3],
    /// The address shown in the form.
    pub address_shown: usize,
    /// `YYYY-MM-DD`.
    pub birthday: String,
    pub org: String,
    pub title: String,
    pub note: String,
    pub error: Option<String>,
}
