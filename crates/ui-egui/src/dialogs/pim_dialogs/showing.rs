//! The event form's Show as and Private (issue #32): how the event shows its time to those
//! planning around it, and whether others see what it is. Outlook's Private is the lock; "Show
//! time only" is the third choice SOGo and Thunderbird have (`CLASS:CONFIDENTIAL`). And the
//! calendar it's in, picked for a new one, or moved to for one of a CalDAV account (issue #279).

use egui::{RichText, Ui, vec2};
use mailcraft_engine::Session;
use mailcraft_engine::model::ical::{Privacy, ShowAs};

use super::{CHOICE_W, EventForm, FIELD_H, form_row};
use crate::icons::Icon;
use crate::panes::calendar::{AWAY, show_as_label};
use crate::theme::Tokens;

/// Show as, in Outlook's order, and the lock beside it.
pub(super) fn show_as_row(ui: &mut Ui, t: &Tokens, f: &mut EventForm) {
    form_row(ui, t, true, Some(Icon::Clock), "Show as", |ui| {
        ui.allocate_ui_with_layout(vec2(CHOICE_W, FIELD_H), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            let r = egui::ComboBox::from_id_salt("event-show-as")
                .width(CHOICE_W - 6.0)
                .selected_text(show_as_label(f.show_as))
                .show_ui(ui, |ui| {
                    for s in ShowAs::ALL {
                        let label = RichText::new(show_as_label(s));
                        ui.selectable_value(&mut f.show_as, s, if s == ShowAs::OutOfOffice { label.color(AWAY) } else { label });
                    }
                })
                .response
                .accessible_name(tl!("Show as"));
            crate::drop_down::keys(ui, &r, &mut f.show_as, &ShowAs::ALL);
        });
        ui.add_space(6.0);
        // Wrapped under it when a language's words don't fit beside it.
        ui.allocate_ui_with_layout(
            vec2(ui.available_width(), FIELD_H),
            egui::Layout::left_to_right(egui::Align::Center).with_main_wrap(true),
            |ui| {
                // A checkbox, which says it's a choice (a lock and the word, it was, with
                // nothing showing it could be clicked).
                let mut private = f.privacy != Privacy::Public;
                if ui.checkbox(&mut private, tl!("Private")).changed() {
                    f.privacy = if private { Privacy::Private } else { Privacy::Public };
                }
                if f.privacy != Privacy::Public {
                    let mut time_only = f.privacy == Privacy::Confidential;
                    if ui.checkbox(&mut time_only, tl!("Show time only")).changed() {
                        f.privacy = if time_only { Privacy::Confidential } else { Privacy::Private };
                    }
                }
            },
        );
    });
}

pub(super) fn writable_calendars(s: &Session) -> Vec<(String, String, String)> {
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

/// The calendar a new event goes in, when there's more than one; for an event opened, the
/// calendars of its account it can move to (issue #279): a CalDAV account's (Exchange's aren't
/// moved between yet), and not this computer's for a meeting on the server, whose guests would
/// be told it's off.
pub(super) fn calendar_row(ui: &mut Ui, s: &Session, t: &Tokens, f: &mut EventForm) {
    let options: Vec<usize> = match &f.calendar_was {
        None => (0..f.calendars.len()).collect(),
        Some(was) => {
            let Some(account) = f.calendars.iter().find(|c| &c.1 == was).map(|c| c.0.clone()) else { return };
            if !s.pim.get(&account).is_some_and(|p| p.caldav_home.as_deref().is_some_and(|h| !h.is_empty())) {
                return;
            }
            let local = mailcraft_engine::model::pim::LOCAL;
            let meeting = !f.guests.trim().is_empty() && was != local;
            (0..f.calendars.len()).filter(|i| f.calendars.get(*i).is_some_and(|c| c.0 == account && !(meeting && c.1 == local))).collect()
        }
    };
    if options.len() > 1 {
        let name = f.calendars.get(f.calendar).map(|c| c.2.clone()).unwrap_or_default();
        form_row(ui, t, true, Some(Icon::Calendar), "Calendar", |ui| {
            ui.allocate_ui_with_layout(vec2(260.0, FIELD_H), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                let r = egui::ComboBox::from_id_salt("event-calendar")
                    .width(254.0)
                    .selected_text(name)
                    .show_ui(ui, |ui| {
                        for i in &options {
                            if let Some(c) = f.calendars.get(*i) {
                                ui.selectable_value(&mut f.calendar, *i, &c.2);
                            }
                        }
                    })
                    .response
                    .accessible_name(tl!("Calendar"));
                crate::drop_down::keys(ui, &r, &mut f.calendar, &options);
            });
        });
    }
}
