//! Reactions under a message (issue #39): the emoji others sent in answer to it, each with how
//! many and who, and a button to send one, as Outlook offers its reactions beside Reply.

use egui::{Sense, Ui, vec2};
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
            ui.spacing_mut().item_spacing.x = 2.0;
            for emoji in mailcraft_engine::mime::REACTIONS {
                let (rect, resp) = ui.allocate_exact_size(vec2(32.0, 32.0), Sense::click());
                if resp.hovered() {
                    ui.painter().rect_filled(rect, 6.0, t.hover);
                }
                let r = rect.shrink(5.0);
                if !crate::faces::paint(ui.painter(), r, emoji) {
                    ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, emoji, crate::theme::regular(18.0), t.text);
                }
                widgets::named(ui, &resp, emoji, true, None);
                if resp.clicked() {
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
            // The emoji drawn when it's one of the six, else as text; then how many.
            let drawn = crate::faces::drawn(&r.emoji);
            let text = if drawn { r.people.len().to_string() } else { format!("{} {}", r.emoji, r.people.len()) };
            let galley = ui.painter().layout_no_wrap(text, crate::theme::regular(14.0), t.text);
            let art = if drawn { 18.0 } else { 0.0 };
            let (rect, resp) = ui.allocate_exact_size(galley.size() + vec2(16.0 + art, 8.0), Sense::click());
            let fill = if r.mine {
                t.accent.gamma_multiply(0.18)
            } else if resp.hovered() {
                t.hover
            } else {
                t.surface
            };
            let stroke = egui::Stroke::new(1.0, if r.mine { t.accent } else { t.border });
            ui.painter().rect(rect, 12.0, fill, stroke, egui::StrokeKind::Inside);
            if drawn {
                let face = egui::Rect::from_min_size(egui::pos2(rect.min.x + 7.0, rect.center().y - 8.0), vec2(16.0, 16.0));
                crate::faces::paint(ui.painter(), face, &r.emoji);
            }
            ui.painter().galley(egui::pos2(rect.min.x + 8.0 + art, rect.center().y - galley.size().y / 2.0), galley, t.text);
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
