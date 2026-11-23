//! File › Info's card for an organisation's profile (issue #245): who set MailCraft up here and
//! who to ask, where the profile was read, what it locks and holds to, what of it couldn't be
//! taken; and the person's details its signature takes, for the account shown.

use mailcraft_engine::AccountState;

use super::*;

/// The card, when there's a profile, or one that couldn't be taken (none: nothing drawn).
pub(super) fn show(s: &Session, t: &Tokens, a: &AccountState, ui: &mut Ui, acts: &mut Vec<Action>) {
    if s.organisation.is_empty() && s.organisation.ignored.is_empty() {
        return;
    }
    let o = &s.org;
    let title = o.organisation.as_deref().map_or_else(|| tl!("Your organisation").to_string(), str::to_string);
    let about = tl!("Your organisation set up MailCraft on this computer. What it locked, you can't change.");
    card(ui, t, &title, Some(about), |ui| {
        if let Some(c) = &o.contact {
            line(ui, t, tl!("Questions"), c);
        }
        if !s.organisation.sources.is_empty() {
            line(ui, t, tl!("Read from"), &s.organisation.sources.join("\n"));
        }
        // What it locks, by the names the settings have in MailCraft.
        let registry = s.registry();
        let mut locked: Vec<String> = o
            .settings
            .iter()
            .filter(|(_, g)| g.locked)
            .map(|(k, _)| {
                let by = registry.all().iter().find(|c| c.setting.split(',').any(|f| mailcraft_engine::setting_key(f.trim()) == *k));
                by.map_or_else(|| k.clone(), |c| tl!(c.label).to_string())
            })
            .collect();
        if o.theme.as_ref().is_some_and(|g| g.locked) {
            locked.push(tl!("Theme").to_string());
        }
        if o.language.as_ref().is_some_and(|g| g.locked) {
            locked.push(tl!("Language").to_string());
        }
        locked.sort();
        locked.dedup();
        if !locked.is_empty() {
            line(ui, t, tl!("Locked"), &locked.join(", "));
        }
        let mut held: Vec<String> = Vec::new();
        if o.require_encryption {
            held.push(tl!("Encrypted connections only").to_string());
        }
        if o.no_certificate_exceptions {
            held.push(tl!("No certificate trusted by hand").to_string());
        }
        if let Some(d) = &o.allowed_domains {
            held.push(tf!("Accounts of {domains} only", domains = d.join(", ")));
        }
        if s.organisation.disclaimer_for(&a.config.email).is_some() {
            held.push(tl!("A notice under every message sent").to_string());
        }
        if !held.is_empty() {
            line(ui, t, tl!("Rules"), &held.join("\n"));
        }
        let unread: Vec<&String> = s.organisation.ignored.iter().chain(&o.notes).collect();
        if !unread.is_empty() {
            let text = unread.iter().take(20).map(|u| u.as_str()).collect::<Vec<_>>().join("\n");
            line(ui, t, tl!("Not taken"), &text);
        }
        if s.organisation.signature_for(&a.config.email).is_some() {
            details(t, a, ui, acts);
        }
    });
}

/// A labelled line of the card.
fn line(ui: &mut Ui, t: &Tokens, label: &str, text: &str) {
    ui.add_space(4.0);
    ui.horizontal_top(|ui| {
        ui.add_sized(vec2(140.0, 18.0), egui::Label::new(RichText::new(label).color(t.dim)));
        ui.add(egui::Label::new(RichText::new(text).color(t.text)).wrap());
    });
}

/// The person's details for the organisation's signature, for account `a`: each edited in place,
/// kept with Save.
fn details(t: &Tokens, a: &AccountState, ui: &mut Ui, acts: &mut Vec<Action>) {
    ui.add_space(10.0);
    ui.label(RichText::new(tl!("Your details for its signature")).strong().color(t.text));
    let labels = [tl!("Name"), tl!("Job title"), tl!("Department"), tl!("Phone"), tl!("Mobile"), tl!("Office"), tl!("Company")];
    let id = egui::Id::unique(("org-details", a.config.id.as_str()));
    let mut values: Vec<String> = ui
        .data_mut(|d| d.get_temp::<Vec<String>>(id))
        .unwrap_or_else(|| mailcraft_engine::DETAILS.iter().map(|k| a.config.details.get(*k).cloned().unwrap_or_default()).collect());
    values.resize(mailcraft_engine::DETAILS.len(), String::new());
    egui::Grid::new(("org-details-grid", a.config.id.as_str())).num_columns(2).spacing(vec2(12.0, 6.0)).show(ui, |ui| {
        for (label, value) in labels.iter().zip(values.iter_mut()) {
            ui.label(RichText::new(*label).color(t.dim));
            ui.add(egui::TextEdit::singleline(value).desired_width(280.0).char_limit(200)).accessible_name(*label);
            ui.end_row();
        }
    });
    let saved: Vec<String> = mailcraft_engine::DETAILS.iter().map(|k| a.config.details.get(*k).cloned().unwrap_or_default()).collect();
    ui.add_space(4.0);
    if ui.add_enabled(values != saved, egui::Button::new(tl!("Save"))).clicked() {
        let mut params = json!({"account": a.config.id});
        for (k, v) in mailcraft_engine::DETAILS.iter().zip(&values) {
            params[*k] = json!(v);
        }
        acts.push(Action::Run("account.details", params));
        ui.data_mut(|d| d.remove::<Vec<String>>(id));
    } else {
        ui.data_mut(|d| d.insert_temp(id, values));
    }
}
