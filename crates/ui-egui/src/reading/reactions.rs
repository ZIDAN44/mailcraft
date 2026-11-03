//! Reactions under a message (issue #39): the emoji others sent in answer to it, each with how
//! many and who, and a button to send one, as Outlook offers its reactions beside Reply.

use egui::{RichText, Sense, Ui, vec2};
use mailcraft_engine::Session;
use mailcraft_engine::model::MessageKey;
use serde_json::json;

use crate::icons::{self, Icon};
use crate::theme::Tokens;
use crate::{Action, widgets};

/// The React button, beside Forward: a menu of the six reactions.
pub(crate) fn react_button(ui: &mut Ui, t: &Tokens, acts: &mut Vec<Action>) {
    let b = widgets::pill_button(ui, Icon::Smile, "React", t).on_hover_text(tl!("Send a reaction to everyone it went to"));
    egui::Popup::menu(&b).show(|ui| {
        ui.horizontal(|ui| {
            for emoji in mailcraft_engine::mime::REACTIONS {
                if ui.add(egui::Button::new(RichText::new(emoji).size(20.0)).frame(false)).clicked() {
                    acts.push(Action::Run("mail.react", json!({"emoji": emoji})));
                    ui.close();
                }
            }
        });
    });
}

/// The reactions answering the message: one chip per emoji, with how many sent it; who, when
/// pointed at. A chip of the user's own is drawn marked; clicking one sends that reaction too.
pub(crate) fn reactions_row(s: &Session, t: &Tokens, ui: &mut Ui, key: &MessageKey, acts: &mut Vec<Action>) {
    let all = s.reactions(key);
    if all.is_empty() {
        return;
    }
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
        for r in all.iter().take(12) {
            let text = format!("{} {}", r.emoji, r.people.len());
            let galley = ui.painter().layout_no_wrap(text.clone(), crate::theme::regular(14.0), t.text);
            let (rect, resp) = ui.allocate_exact_size(galley.size() + vec2(16.0, 8.0), Sense::click());
            let fill = if r.mine {
                t.accent.gamma_multiply(0.18)
            } else if resp.hovered() {
                t.hover
            } else {
                t.surface
            };
            let stroke = egui::Stroke::new(1.0, if r.mine { t.accent } else { t.border });
            ui.painter().rect(rect, 12.0, fill, stroke, egui::StrokeKind::Inside);
            ui.painter().galley(rect.center() - galley.size() / 2.0, galley, t.text);
            let who = r.people.join(", ");
            let said = tf!("Reacted with {emoji}: {people}", emoji = r.emoji, people = who);
            widgets::named(ui, &resp, &said, true, None);
            let resp = resp.on_hover_text(said);
            if resp.clicked() && !r.mine {
                acts.push(Action::Run("mail.react", json!({"emoji": r.emoji})));
            }
        }
        let (ir, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
        icons::paint(ui.painter(), ir, Icon::Smile, t.dim);
    });
    ui.add_space(6.0);
}
