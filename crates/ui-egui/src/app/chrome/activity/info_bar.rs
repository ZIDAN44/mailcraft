//! The bar over the panes, for what needs you now (an account to sign in to again, working
//! offline, Send / Receive that failed): the most pressing one, what helps, how many more there
//! are (they're in the Activity panel), and a way to put it away until it says something else.
//! Work under way isn't said here: the Activity button's ring says it.

use super::*;

/// The bar's height.
const BAR_H: f32 = 36.0;

/// What a problem said, to put it away until it says something else.
fn said(p: &Problem) -> String {
    format!("{}\n{}", p.key, p.detail)
}

impl MailApp {
    /// The bar, while something needs you that wasn't put away.
    pub(crate) fn info_bar(&mut self, ui: &mut Ui) {
        let a = &self.ui.activity.current;
        let waiting: Vec<&Problem> =
            a.needs.iter().filter(|p| p.severity != Severity::Info && !self.ui.activity.dismissed.contains(&said(p))).collect();
        let Some(first) = waiting.first().map(|p| (*p).clone()) else { return };
        let more = waiting.len() - 1;
        let t = self.ui.tokens;
        let (icon_color, ground) = match first.severity {
            Severity::Error => (t.danger, motion::mix(t.surface, t.danger, 0.12)),
            _ => (t.warn, motion::mix(t.surface, t.warn, 0.14)),
        };
        let mut acts: Vec<Action> = Vec::new();
        let mut put_away = false;
        egui::Panel::top("info-bar")
            .exact_size(BAR_H)
            .frame(egui::Frame::new().fill(ground).inner_margin(Margin::symmetric(12, 0)).stroke(egui::Stroke::new(1.0, t.border)))
            .show(ui, |ui| {
                ui.with_layout(bidi::row_from_start(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let (ir, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                    icons::paint(ui.painter(), ir, first.icon, icon_color);
                    ui.label(RichText::new(&first.title).font(theme::semibold(crate::sizes::BODY)).color(t.text));
                    // The rest of the row for the words, the buttons kept clear at its end.
                    ui.with_layout(bidi::row_from_end(Align::Center), |ui| {
                        put_away = widgets::tiny_icon_button(ui, Icon::Close, "Dismiss", &t).clicked();
                        if more > 0 && ui.link(RichText::new(tf!("{n} more", n = more)).color(t.accent).small()).clicked() {
                            acts.push(Action::ActivityPanel);
                        }
                        if let Some((words, what)) = &first.action
                            && widgets::secondary(ui, words).clicked()
                        {
                            acts.push(what.action());
                        }
                        ui.with_layout(bidi::row_from_start(Align::Center), |ui| {
                            ui.add(egui::Label::new(RichText::new(&first.detail).color(t.dim)).truncate());
                        });
                    });
                });
            });
        if put_away {
            self.ui.activity.dismissed.insert(said(&first));
        }
        for act in acts {
            self.act(act);
        }
    }
}
