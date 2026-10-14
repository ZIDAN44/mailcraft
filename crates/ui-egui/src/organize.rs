//! Menus and marks for organising mail: Categorize, Snooze, rules.

use egui::{RichText, Sense, Ui, vec2};
use mailcraft_engine::Session;
use mailcraft_engine::model::dates;
use mailcraft_engine::model::organize::SnoozeUntil;
use serde_json::json;

use crate::theme::{self, Tokens};
use crate::{Action, Dialog};

/// A coloured dot for a category.
pub fn dot(ui: &mut Ui, color: u8, dark: bool) {
    let (r, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
    ui.painter().circle_filled(r.center(), 5.0, theme::category_color(color, dark));
}

/// The Categorize menu: tick or untick a category on the selection, or make a new one.
pub fn categorize_menu(ui: &mut Ui, s: &Session, t: &Tokens, acts: &mut Vec<Action>) {
    let first = s.selected().into_iter().next();
    for c in &s.prefs.categories {
        let on = first.as_ref().is_some_and(|m| m.flags.has(&c.keyword));
        let clicked = ui
            .horizontal(|ui| {
                dot(ui, c.color, t.dark);
                ui.selectable_label(on, tl!(&c.name)).clicked()
            })
            .inner;
        if clicked {
            acts.push(Action::Run("mail.categorize", json!({"category": c.name, "on": !on})));
            ui.close();
        }
    }
    ui.separator();
    if ui.button(tl!("New category…")).clicked() {
        acts.push(Action::Dialog(Dialog::NewCategory {
            name: String::new(),
            color: u8::try_from(s.prefs.categories.len() % 8).unwrap_or(0),
            error: None,
        }));
        ui.close();
    }
}

/// The Snooze menu: the usual times, shown in local time.
pub fn snooze_menu(ui: &mut Ui, s: &Session, t: &Tokens, acts: &mut Vec<Action>) {
    for p in SnoozeUntil::ALL {
        let at = p.at(&s.clock);
        let when = dates::full_time(at, &s.clock);
        let clicked = ui
            .horizontal(|ui| {
                let r = ui.selectable_label(false, tl!(p.label()));
                ui.label(RichText::new(crate::i18n::date(&when)).color(t.faint).small());
                r.clicked()
            })
            .inner;
        if clicked {
            acts.push(Action::Run("mail.snooze", json!({"preset": p.code()})));
            ui.close();
        }
    }
}

/// Every organising item of the message context menu.
pub fn context_items(ui: &mut Ui, s: &Session, t: &Tokens, acts: &mut Vec<Action>, pinned: bool) {
    ui.menu_button(tl!("Categorize"), |ui| categorize_menu(ui, s, t, acts));
    if ui.button(if pinned { "Unpin" } else { "Pin" }).clicked() {
        acts.push(Action::Run("mail.pin", json!({"on": !pinned})));
        ui.close();
    }
    ui.menu_button(tl!("Snooze"), |ui| snooze_menu(ui, s, t, acts));
    if s.focus_split() {
        let (id, label) = if s.focus_other { ("mail.moveToFocused", "Always move to Focused") } else { ("mail.moveToOther", "Always move to Other") };
        if ui.button(label).clicked() {
            acts.push(Action::Run(id, json!({})));
            ui.close();
        }
    }
    if let Some(m) = s.selected().into_iter().next()
        && ui.button(tl!("Create rule…")).clicked()
    {
        acts.push(Action::Dialog(Dialog::rule_from(&m.from.email, &m.subject)));
        ui.close();
    }
}
