//! The reading pane: the people in a message's header, their menus and contact cards.

use super::*;

/// Most people named in To or Cc above a message (the rest say how many more).
pub(super) const MAX_PEOPLE_SHOWN: usize = 50;

/// The contact someone is saved as, if they are (by address).
pub(super) fn known_contact<'a>(s: &'a Session, email: &str) -> Option<&'a mailcraft_engine::model::vcard::Card> {
    if email.trim().is_empty() {
        return None;
    }
    s.people(email).into_iter().find(|x| x.card.emails.iter().any(|e| e.value.eq_ignore_ascii_case(email))).map(|x| x.card)
}

/// Someone's name above a message, as a link: lit and underlined under the pointer.
pub(super) fn person_link(ui: &mut Ui, t: &Tokens, text: RichText) -> egui::Response {
    let r = ui.add(egui::Label::new(text).sense(Sense::click()).selectable(false));
    if r.hovered() {
        ui.painter().line_segment([r.rect.left_bottom(), r.rect.right_bottom()], egui::Stroke::new(1.0, t.dim));
    }
    r.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// What a name above a message opens: a contact card on a click (or double-click), as
/// Outlook's, and the same things on a right-click.
pub(super) fn person_menus(s: &Session, t: &Tokens, r: &egui::Response, a: &EmailAddress, acts: &mut Vec<Action>) {
    if a.email.trim().is_empty() {
        return;
    }
    let known = known_contact(s, &a.email);
    egui::Popup::menu(r).id(r.id.with("card")).show(|ui| contact_card(ui, t, a, known, acts));
    r.context_menu(|ui| {
        if ui.button(tl!("Send email")).clicked() {
            acts.push(Action::Run("mail.new", json!({"to": a.to_string()})));
            ui.close();
        }
        match &known {
            Some(card) => {
                if ui.button(tl!("Edit contact…")).clicked() {
                    acts.push(Action::Dialog(crate::Dialog::edit_contact(card)));
                    ui.close();
                }
            }
            None => {
                if ui.button(tl!("Add to contacts…")).clicked() {
                    acts.push(Action::Dialog(crate::Dialog::contact_from(a)));
                    ui.close();
                }
            }
        }
        ui.separator();
        if ui.button(tl!("Copy address")).clicked() {
            ui.ctx().copy_text(a.email.clone());
            ui.close();
        }
    });
}

/// A small card for someone (Outlook's contact card): who they are, their address (a click
/// writes to them), and what can be done.
pub(super) fn contact_card(ui: &mut Ui, t: &Tokens, a: &EmailAddress, known: Option<&mailcraft_engine::model::vcard::Card>, acts: &mut Vec<Action>) {
    ui.set_min_width(260.0);
    ui.horizontal(|ui| {
        let (ar, _) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::hover());
        widgets::avatar(ui, ar.center(), 19.0, &a.initials(), theme::avatar_color(&a.email, t.dark));
        ui.add_space(4.0);
        ui.vertical(|ui| {
            let name = known.map(|c| c.display()).filter(|n| !n.is_empty()).unwrap_or_else(|| a.display().to_string());
            ui.label(RichText::new(&name).font(theme::bold(15.0)).color(t.text));
            // Their job and company, when saved (not the company again when it's the name).
            if let Some(c) = known {
                let what = [c.title.as_str(), c.org.as_str()]
                    .into_iter()
                    .filter(|x| !x.is_empty() && !x.eq_ignore_ascii_case(&name))
                    .collect::<Vec<_>>()
                    .join(" · ");
                if !what.is_empty() {
                    ui.label(RichText::new(what).color(t.dim).small());
                }
            }
            let mail = ui.add(egui::Label::new(RichText::new(&a.email).color(t.accent).underline()).sense(Sense::click()));
            if mail.on_hover_text(tl!("Send email")).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                acts.push(Action::Run("mail.new", json!({"to": a.to_string()})));
                ui.close();
            }
        });
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if widgets::pill_button(ui, Icon::NewMail, "Send email", t).clicked() {
            acts.push(Action::Run("mail.new", json!({"to": a.to_string()})));
            ui.close();
        }
        let (label, dialog) = match known {
            Some(card) => ("Edit contact…", crate::Dialog::edit_contact(card)),
            None => ("Add to contacts…", crate::Dialog::contact_from(a)),
        };
        if widgets::pill_button(ui, Icon::Person, label, t).clicked() {
            acts.push(Action::Dialog(dialog));
            ui.close();
        }
        if ui.button(tl!("Copy address")).clicked() {
            ui.ctx().copy_text(a.email.clone());
            ui.close();
        }
    });
}

/// The subject over a message: at most a few lines of it, "…" after them and all of it under
/// the pointer (a subject of two thousand characters filled the pane, the message far below).
pub(super) fn subject_heading(ui: &mut Ui, subject: &str, t: &Tokens) -> egui::Response {
    const LINES: usize = 4;
    let mut job = egui::text::LayoutJob::simple(subject.to_string(), theme::bold(21.0), t.text, ui.available_width());
    job.wrap.max_rows = LINES;
    job.wrap.overflow_character = Some('…');
    let resp = crate::bidi::add_label(ui, job);
    if subject.chars().count() > 160 { resp.on_hover_text(subject.chars().take(1200).collect::<String>()) } else { resp }
}
