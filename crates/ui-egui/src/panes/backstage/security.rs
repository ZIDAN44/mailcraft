//! File › Info › Security: the account's S/MIME certificate (from a `.p12` or `.pfx` file; its key
//! stays in the system's keychain) and whether new messages are signed and encrypted.

use super::*;

/// The button for the system's own certificates, where MailCraft can use them (Windows' store,
/// the macOS Keychain).
fn system_certificate(ui: &mut Ui, id: &str, acts: &mut Vec<Action>) {
    let label = if cfg!(windows) {
        Some(tl!("Use one from Windows…"))
    } else if cfg!(target_os = "macos") {
        Some(tl!("Use one from the Keychain…"))
    } else {
        None
    };
    if let Some(label) = label
        && ui.button(label).on_hover_text(tl!("A certificate this computer already has, its key staying where it is")).clicked()
    {
        acts.push(Action::UseSystemCertificate(id.to_string()));
    }
}

/// A row for a certificate this computer has (a smart card's, the system's own) instead of a file.
fn other_sources(ui: &mut Ui, t: &Tokens, id: &str, acts: &mut Vec<Action>) {
    if cfg!(target_arch = "wasm32") {
        return;
    }
    setting_row(
        ui,
        t,
        false,
        name(t, tl!("Or use one this computer has")),
        Some(note(t, tl!("On a smart card, or kept by the system: the key stays where it is."))),
        |ui| {
            system_certificate(ui, id, acts);
            if ui.button(tl!("Use a smart card…")).clicked() {
                acts.push(Action::UseSmartCard(id.to_string()));
            }
        },
    );
}

/// The day a certificate runs out, as the interface writes dates.
fn until(s: &Session, not_after: i64) -> String {
    crate::i18n::date(&mailcraft_engine::model::dates::day_label(not_after.saturating_add(i64::from(s.clock.offset)).div_euclid(86_400)))
}

/// The older certificates kept to read mail encrypted to them (after a renewal), each with
/// buttons to sign and encrypt with it again or to forget it.
fn older(s: &Session, t: &Tokens, ui: &mut Ui, id: &str, acts: &mut Vec<Action>) {
    for cert in s.older_identities(id) {
        let who = cert.emails.first().cloned().unwrap_or_else(|| cert.name.clone());
        let about = tf!("Older certificate, valid until {date}: kept to read mail encrypted to it", date = until(s, cert.not_after));
        setting_row(ui, t, false, name(t, &who), Some(note(t, &about)), |ui| {
            if ui
                .button(RichText::new(tl!("Remove")).color(t.danger))
                .on_hover_text(tl!("Forget this certificate: mail encrypted to it can't be read any more"))
                .clicked()
            {
                acts.push(Action::Run("security.forget", json!({"account": id, "older": cert.fingerprint})));
            }
            if cert.not_after >= s.clock.now
                && ui
                    .button(tl!("Use for sending"))
                    .on_hover_text(tl!("Sign and encrypt with this one again; the one in use is kept for reading"))
                    .clicked()
            {
                acts.push(Action::Run("security.useOlder", json!({"account": id, "older": cert.fingerprint})));
            }
        });
    }
}

pub(super) fn security(s: &Session, t: &Tokens, ui: &mut Ui, a: &AccountState, acts: &mut Vec<Action>) {
    let id = a.config.id.clone();
    let about = tl!("Sign your messages so people can tell they're from you, and read and send encrypted mail, with a certificate (S/MIME).");
    card(ui, t, tl!("Security"), Some(about), |ui| {
        let Some(cert) = s.identity(&id) else {
            setting_row(
                ui,
                t,
                true,
                name(t, tl!("No certificate")),
                Some(note(t, tl!("Import the .p12 or .pfx file your certificate came in."))),
                |ui| {
                    if ui.button(tl!("Import certificate…")).clicked() {
                        acts.push(Action::ImportCertificate(id.clone()));
                    }
                    // Older ones kept in the browser open with their password, to read old mail.
                    if cfg!(target_arch = "wasm32")
                        && !s.older_identities(&id).is_empty()
                        && ui
                            .button(tl!("Unlock…"))
                            .on_hover_text(tl!(
                                "Give its password again to sign and decrypt in this page (it opens every certificate it's the password of)"
                            ))
                            .clicked()
                    {
                        acts.push(Action::UnlockCertificate(id.clone()));
                    }
                },
            );
            older(s, t, ui, &id, acts);
            other_sources(ui, t, &id, acts);
            return;
        };
        let until = until(s, cert.not_after);
        let who = cert.emails.first().cloned().unwrap_or_else(|| cert.name.clone());
        let line = tf!("Issued by {issuer}, valid until {date}", issuer = cert.issuer, date = until);
        let expired = cert.not_after < s.clock.now;
        let mut about = RichText::new(format!("{line} · {}", cert.key)).size(12.0).color(if expired { t.danger } else { t.faint });
        if expired {
            about = RichText::new(tf!("It ran out on {date}: messages can't be signed with it any more.", date = until)).size(12.0).color(t.danger);
        }
        setting_row(ui, t, true, name(t, &who), Some(about), |ui| {
            if ui
                .button(RichText::new(tl!("Remove")).color(t.danger))
                .on_hover_text(tl!("Forget this certificate: its key goes from the keychain too"))
                .clicked()
            {
                acts.push(Action::Run("security.forget", json!({"account": id})));
            }
            if ui
                .button(tl!("Add another…"))
                .on_hover_text(tl!("A renewed certificate takes over signing and encrypting; an older one is kept to read old mail"))
                .clicked()
            {
                acts.push(Action::ImportCertificate(id.clone()));
            }
            if cfg!(target_arch = "wasm32")
                && ui
                    .button(tl!("Unlock…"))
                    .on_hover_text(tl!("Give its password again to sign and decrypt in this page (it opens every certificate it's the password of)"))
                    .clicked()
            {
                acts.push(Action::UnlockCertificate(id.clone()));
            }
        });
        older(s, t, ui, &id, acts);
        other_sources(ui, t, &id, acts);
        let settings = s.smime_settings(&id);
        setting_row(ui, t, false, name(t, tl!("Sign new messages")), None, |ui| {
            let mut on = settings.sign;
            if widgets::switch(ui, t, &mut on, tl!("Sign new messages")).changed() {
                acts.push(Action::Run("security.defaults", json!({"account": id, "sign": on})));
            }
        });
        setting_row(
            ui,
            t,
            false,
            name(t, tl!("Encrypt new messages")),
            Some(note(t, tl!("Each recipient needs to have sent you a signed message first; replies to encrypted mail are always encrypted."))),
            |ui| {
                let mut on = settings.encrypt;
                if widgets::switch(ui, t, &mut on, tl!("Encrypt new messages")).changed() {
                    acts.push(Action::Run("security.defaults", json!({"account": id, "encrypt": on})));
                }
            },
        );
        setting_row(
            ui,
            t,
            false,
            name(t, tl!("Hide the subject of encrypted mail")),
            Some(note(t, tl!("On the way it reads [...]; the real one is inside, for its recipients (header protection)."))),
            |ui| {
                let mut on = !settings.keep_subject;
                if widgets::switch(ui, t, &mut on, tl!("Hide the subject of encrypted mail")).changed() {
                    acts.push(Action::Run("security.defaults", json!({"account": id, "keepSubject": !on})));
                }
            },
        );
        // The browser (gap 7): certificates opened again on the next visit, their password sealed
        // with a key that never leaves this browser.
        if s.jmap_only {
            setting_row(
                ui,
                t,
                false,
                name(t, tl!("Keep unlocked in this browser")),
                Some(note(
                    t,
                    tl!(
                        "The password is kept sealed with a key this browser makes and never lets out: anyone using this browser profile can sign and read your encrypted mail, nobody can take the key elsewhere."
                    ),
                )),
                |ui| {
                    let mut on = settings.keep_unlocked;
                    if widgets::switch(ui, t, &mut on, tl!("Keep unlocked in this browser")).changed() {
                        acts.push(Action::Run("security.keepUnlocked", json!({"account": id, "on": on})));
                    }
                },
            );
        }
        let known = settings.known.len();
        if known > 0 {
            setting_row(ui, t, false, name(t, &tf!("Certificates from signed mail: {n}", n = known)), None, |_| {});
        }
    });
}
