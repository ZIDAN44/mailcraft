//! A meeting's guests in its form (issue #253), as a message's To: the people known suggested as
//! a name is typed, Ctrl+K checking the names typed (Check Names, issue #211: one person put in,
//! several offered here to pick from, none said), and Invite… opening the address book. And who
//! the guests are, with their answers, and whom a guest handed their place to (issue #254).

use super::*;
use crate::look::people_box;
use mailcraft_engine::model::EmailAddress;

/// A name Check Names found several people for, to pick from.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NameAsk {
    pub typed: String,
    pub people: Vec<EmailAddress>,
}

/// The guests' row: their box, its address book button, and the names to pick people for.
pub(super) fn row(ui: &mut Ui, t: &Tokens, f: &mut EventForm, acts: &mut Vec<Action>) {
    form_row(ui, t, true, Some(Icon::People), "Invite", |ui| {
        ui.vertical(|ui| {
            ui.horizontal(|ui| {
                let book = tl!("Invite…");
                let w = (ui.available_width() - ui.spacing().item_spacing.x - book_width(ui, book)).max(120.0);
                let hint = tl!("Names <addresses>, separated by ; or ,");
                if people_box::in_dialog(ui, t, "guests", tl!("Invite"), &mut f.guests, hint, w, &mut f.guest_box) {
                    acts.push(Action::CheckGuestNames(false));
                }
                if ui
                    .add_sized(vec2(book_width(ui, book), FIELD_H), egui::Button::new(book))
                    .on_hover_text(tl!("Pick guests from the address book"))
                    .clicked()
                {
                    acts.push(Action::GuestsFromBook);
                }
            });
            let mut picked = None;
            for (i, ask) in f.name_asks.iter().enumerate() {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(tf!("Who is “{name}”?", name = ask.typed)).color(t.dim).small());
                    // Each by name (their address on hover), so several fit on a line.
                    for p in &ask.people {
                        let name = if p.name.trim().is_empty() { p.email.as_str() } else { p.name.as_str() };
                        if ui.small_button(name).on_hover_text(p.to_string()).clicked() {
                            picked = Some((i, p.clone()));
                        }
                    }
                });
            }
            if let Some((i, person)) = picked
                && let Some(ask) = f.name_asks.get(i).cloned()
            {
                if let Some(text) = mailcraft_engine::model::addr::check_names::pick(&f.guests, &ask.typed, &person) {
                    f.guests = text;
                }
                f.name_asks.retain(|a| a.typed != ask.typed);
            }
        });
    });
}

impl crate::Dialog {
    /// The suggestions of a dialog's boxes of people, made for what's typed in them (before it's
    /// drawn: drawing can't change the session); a meeting's Check Names asked again once the
    /// directory it waited on answered.
    pub(crate) fn refresh_people(&mut self, s: &mut Session, acts: &mut Vec<Action>) {
        match self {
            crate::Dialog::Event(f) => {
                f.guest_box.refresh(s, &f.guests);
                if f.names_pending.is_some_and(|rev| rev != s.directory.rev) {
                    f.names_pending = None;
                    acts.push(Action::CheckGuestNames(true));
                }
            }
            crate::Dialog::PassMeeting(f) => f.people.refresh(s, &f.to),
            _ => {}
        }
    }
}

/// How wide the address book button is: its label and a little room.
fn book_width(ui: &Ui, label: &str) -> f32 {
    ui.painter().layout_no_wrap(label.to_string(), theme::regular(13.5), Color32::WHITE).size().x + 20.0
}

/// The guests as the form lists them: each with their answer, or whom they handed their place to.
pub(super) fn listed(attendees: &[(String, String, String)]) -> String {
    attendees
        .iter()
        .map(|(who, answer, to)| match answer.as_str() {
            "DELEGATED" if !to.is_empty() => tf!("{name} (delegated to {delegate})", name = who, delegate = to),
            _ => format!("{who} ({})", tl!(partstat_label(answer))),
        })
        .collect::<Vec<_>>()
        .join(", ")
}
