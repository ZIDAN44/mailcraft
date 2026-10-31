//! The event form's Show as and Private (issue #32): how the event shows its time to those
//! planning around it, and whether others see what it is. Outlook's Private is the lock; "Show
//! time only" is the third choice SOGo and Thunderbird have (`CLASS:CONFIDENTIAL`).

use egui::{RichText, Ui, vec2};
use mailcraft_engine::model::ical::{Privacy, ShowAs};

use super::{EventForm, FIELD_H, form_row};
use crate::icons::Icon;
use crate::panes::calendar::{AWAY, show_as_label};
use crate::theme::Tokens;

/// Show as, in Outlook's order, and the lock beside it.
pub(super) fn show_as_row(ui: &mut Ui, t: &Tokens, f: &mut EventForm) {
    form_row(ui, t, true, Some(Icon::Clock), "Show as", |ui| {
        ui.allocate_ui_with_layout(vec2(170.0, FIELD_H), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            egui::ComboBox::from_id_salt("event-show-as").width(164.0).selected_text(show_as_label(f.show_as)).show_ui(ui, |ui| {
                for s in ShowAs::ALL {
                    let label = RichText::new(show_as_label(s));
                    ui.selectable_value(&mut f.show_as, s, if s == ShowAs::OutOfOffice { label.color(AWAY) } else { label });
                }
            });
        });
        ui.add_space(6.0);
        // Wrapped under it when a language's words don't fit beside it.
        ui.allocate_ui_with_layout(
            vec2(ui.available_width(), FIELD_H),
            egui::Layout::left_to_right(egui::Align::Center).with_main_wrap(true),
            |ui| {
                let private = f.privacy != Privacy::Public;
                // A toggle with the lock the calendar marks private events with.
                let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), egui::Sense::hover());
                crate::icons::paint(ui.painter(), r, Icon::Lock, if private { t.accent } else { t.dim });
                if ui.selectable_label(private, tl!("Private")).clicked() {
                    f.privacy = if private { Privacy::Public } else { Privacy::Private };
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
