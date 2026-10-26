//! Dialogs: previews, calendars and people.

use super::*;

pub(super) fn draw(ui: &mut Ui, d: &mut Dialog, s: &Session, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    match d {
        Dialog::ConfirmDeleteContact { uid, name } => {
            ui.set_width(380.0);
            if name.trim().is_empty() {
                title(ui, t, "Delete this contact?");
            } else {
                title(ui, t, &tf!("Delete “{name}”?", name = name));
            }
            ui.label(RichText::new(tl!("This contact will be deleted, along with everything kept about them. This can't be undone.")).color(t.dim));
            let (ok, cancel) = buttons(ui, t, "Delete", true);
            if ok {
                acts.push(Action::Run("contact.delete", json!({"uid": uid})));
                return false;
            }
            !cancel
        }
        Dialog::PicturePreview { key, index, name } => {
            let w = (ui.ctx().content_rect().width() * 0.75).clamp(360.0, 1200.0);
            let h = (ui.ctx().content_rect().height() * 0.7).max(240.0);
            ui.set_width(w);
            title(ui, t, name);
            match crate::reading::big_picture(s, ui.ctx(), key, *index) {
                Some(tex) => {
                    let sz = tex.size_vec2();
                    let scale = (w / sz.x).min(h / sz.y).min(1.0);
                    ui.vertical_centered(|ui| ui.add(egui::Image::new(&tex).fit_to_exact_size(sz * scale)));
                }
                None => {
                    ui.label(RichText::new(tl!("This picture can't be shown.")).color(t.dim));
                }
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button(tl!("Open")).clicked() {
                    acts.push(Action::InWindow(key.clone(), Box::new(Action::OpenAttachment(*index))));
                }
                close_row(ui)
            })
            .inner
        }
        Dialog::Preview { title: name, header, text, warning } => {
            let w = (ui.ctx().content_rect().width() * 0.7).clamp(360.0, 900.0);
            ui.set_width(w);
            title(ui, t, name);
            if let Some(warning) = warning {
                crate::reading::sender_warning(t, ui, warning);
            }
            for (k, v) in header.iter() {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(k.as_str()).color(t.dim).small());
                    ui.label(RichText::new(v.as_str()).small());
                });
            }
            ui.add_space(6.0);
            let h = (ui.ctx().content_rect().height() * 0.6).max(200.0);
            egui::ScrollArea::both().max_height(h).auto_shrink([false, true]).show(ui, |ui| {
                let mut shown = text.as_str();
                ui.add(
                    egui::TextEdit::multiline(&mut shown).font(egui::FontId::monospace(13.0)).desired_width(f32::INFINITY).frame(egui::Frame::NONE),
                );
            });
            ui.add_space(6.0);
            close_row(ui)
        }
        Dialog::Certificate { account, path, data, password } => {
            ui.set_width(420.0);
            let unlock = path.is_empty() && data.is_none();
            title(ui, t, if unlock { "Unlock certificate" } else { "Import certificate" });
            let file = path.rsplit(['/', '\\']).next().unwrap_or(path.as_str()).to_string();
            // Kept unlocked between visits (gap 7): the password is kept, sealed, after all.
            let kept = s.jmap_only && s.smime_settings(account).keep_unlocked;
            let ask = if unlock && kept {
                tl!("Your certificate's password, to keep it unlocked in this browser:").to_string()
            } else if unlock {
                tl!("Your certificate's password, to use it in this browser until the page is closed:").to_string()
            } else {
                tf!("The password {file} was saved with:", file = file)
            };
            ui.add(egui::Label::new(RichText::new(ask).color(t.dim)).wrap());
            ui.add_space(8.0);
            let r = ui.add(egui::TextEdit::singleline(password).password(true).desired_width(f32::INFINITY));
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if ui.memory(|m| m.focused().is_none()) {
                r.request_focus();
            }
            ui.add_space(4.0);
            ui.add(
                egui::Label::new(
                    RichText::new(if cfg!(target_arch = "wasm32") && kept {
                        tl!(
                            "The browser keeps the file, locked with its password, and the password sealed with a key that never leaves this browser."
                        )
                    } else if cfg!(target_arch = "wasm32") {
                        tl!("The browser keeps the file, locked with its password; the password itself isn't kept.")
                    } else {
                        tl!("Its private key is kept in the system's keychain, not in MailCraft's files.")
                    })
                    .color(t.faint)
                    .small(),
                )
                .wrap(),
            );
            let (ok, cancel) = buttons(ui, t, if unlock { "Unlock" } else { "Import" }, true);
            if ok || enter {
                let params = match data {
                    Some(d) => json!({"account": account, "data": d, "password": password}),
                    None if unlock => json!({"account": account, "kept": true, "password": password}),
                    None => json!({"account": account, "path": path, "password": password}),
                };
                acts.push(Action::Run("security.import", params));
                return false;
            }
            !cancel
        }
        Dialog::SmartCard { account, module, pin, system } => smart_card(ui, s, t, acts, account, module, pin, *system),
        Dialog::OpenSharedCalendar { email, error } => {
            ui.set_width(420.0);
            title(ui, t, "Open a shared calendar");
            ui.add(
                egui::Label::new(
                    RichText::new(tl!("Whose calendar? It opens read-only beside yours, when they've shared it with you in Exchange.")).color(t.dim),
                )
                .wrap(),
            );
            ui.add_space(8.0);
            let r = ui.add(egui::TextEdit::singleline(email).hint_text("name@example.com").desired_width(f32::INFINITY));
            // Before the focus request below, which would hide the Enter that ended input.
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if ui.memory(|m| m.focused().is_none()) {
                r.request_focus();
            }
            if let Some(e) = error {
                ui.add_space(4.0);
                ui.label(RichText::new(e.as_str()).color(t.danger).small());
            }
            let ready = email.contains('@');
            let (ok, cancel) = buttons(ui, t, "Open", ready);
            if (ok || enter) && ready {
                acts.push(Action::Submit("calendar.openShared", json!({"email": email.trim()})));
            }
            !cancel
        }
        Dialog::Event(f) => crate::pim_dialogs::event(ui, f, t, acts),
        Dialog::Contact(f) => crate::pim_dialogs::contact(ui, f, t, acts),
        // Every other kind is shown by its own group (`body`).
        _ => true,
    }
}

/// A certificate on a smart card: the card's module and PIN, the certificates it has, one chosen.
fn smart_card(
    ui: &mut Ui,
    s: &Session,
    t: &Tokens,
    acts: &mut Vec<Action>,
    account: &str,
    module: &mut String,
    pin: &mut String,
    system: bool,
) -> bool {
    ui.set_width(460.0);
    if system {
        title(ui, t, if cfg!(windows) { "Use a certificate from Windows" } else { "Use a certificate from the Keychain" });
        ui.add(
            egui::Label::new(
                RichText::new(if cfg!(windows) {
                    tl!("The certificates for email in your Windows certificate store (Personal). The key stays in Windows; it may ask before it's used.")
                } else {
                    tl!("The certificates for email in your Keychain. The key stays there; macOS may ask before it's used.")
                })
                .color(t.dim),
            )
            .wrap(),
        );
        return held_keys(ui, s, t, acts, account);
    }
    title(ui, t, "Use a smart card");
    ui.add(egui::Label::new(RichText::new(tl!("The module your card came with (OpenSC's reads most cards):")).color(t.dim)).wrap());
    ui.add_space(4.0);
    ui.add(egui::TextEdit::singleline(module).hint_text("opensc-pkcs11").desired_width(f32::INFINITY));
    ui.add_space(8.0);
    ui.label(RichText::new(tl!("The card's PIN")).color(t.dim));
    let r = ui.add(egui::TextEdit::singleline(pin).password(true).desired_width(f32::INFINITY));
    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    ui.add_space(8.0);
    let search = s.card_search();
    ui.horizontal(|ui| {
        let ready = !module.trim().is_empty() && !search.searching;
        if ui.add_enabled(ready, egui::Button::new(tl!("Find certificates"))).clicked() || (enter && ready) {
            acts.push(Action::Run("security.findKeys", json!({"module": module.trim(), "pin": pin})));
        }
        if cfg!(target_os = "macos") && ui.add_enabled(!search.searching, egui::Button::new(tl!("Look in the Keychain"))).clicked() {
            acts.push(Action::Run("security.findKeys", json!({"keychain": true})));
        }
        if cfg!(windows) && ui.add_enabled(!search.searching, egui::Button::new(tl!("Look in Windows"))).clicked() {
            acts.push(Action::Run("security.findKeys", json!({"keychain": true})));
        }
    });
    held_keys(ui, s, t, acts, account)
}

/// What the last look found, each with Use; and where the key stays.
fn held_keys(ui: &mut Ui, s: &Session, t: &Tokens, acts: &mut Vec<Action>, account: &str) -> bool {
    let search = s.card_search();
    if search.searching {
        ui.add_space(6.0);
        ui.label(RichText::new(tl!("Looking…")).color(t.faint).small());
    }
    if let Some(e) = &search.error {
        ui.add_space(6.0);
        ui.add(egui::Label::new(RichText::new(tl!(e)).color(t.danger).small()).wrap());
    }
    let mut chosen = false;
    for (i, k) in search.keys.iter().enumerate() {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let info = mailcraft_engine::smime::cert::info(&k.cert).ok();
            ui.vertical(|ui| {
                ui.label(RichText::new(&k.label).color(t.text));
                if let Some(info) = &info {
                    let who = info.emails.first().cloned().unwrap_or_else(|| info.name.clone());
                    ui.label(RichText::new(format!("{who} · {} · {}", info.issuer, info.key)).color(t.faint).small());
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(tl!("Use")).clicked() {
                    acts.push(Action::Run("security.useKey", json!({"account": account, "index": i})));
                    chosen = true;
                }
            });
        });
    }
    ui.add_space(4.0);
    if search.pin_given() {
        ui.add(
            egui::Label::new(RichText::new(tl!("The key stays on the card; its PIN is kept in the system's keychain.")).color(t.faint).small())
                .wrap(),
        );
    }
    !chosen && close_row(ui)
}
