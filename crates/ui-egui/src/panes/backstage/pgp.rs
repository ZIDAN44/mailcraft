//! File › Info › Security: the account's OpenPGP keys (from a secret key file; the secret part
//! stays in the system's keychain), whether its key goes along with its mail (Autocrypt), and the
//! keys MailCraft has for the people it writes to.

use super::*;

/// How long a key is valid, in words.
fn validity(s: &Session, k: &mailcraft_engine::pgp::KeyInfo) -> String {
    let day =
        |at: i64| crate::i18n::date(&mailcraft_engine::model::dates::day_label(at.saturating_add(i64::from(s.clock.offset)).div_euclid(86_400)));
    match (k.revoked, k.expires) {
        (Some(at), _) => tf!("Withdrawn on {date}", date = day(at)),
        (None, Some(at)) if at <= s.clock.now => tf!("It ran out on {date}", date = day(at)),
        (None, Some(at)) => tf!("Valid until {date}", date = day(at)),
        (None, None) => tl!("No end date").to_string(),
    }
}

pub(super) fn pgp(s: &Session, t: &Tokens, ui: &mut Ui, a: &AccountState, acts: &mut Vec<Action>) {
    let id = a.config.id.clone();
    let about =
        tl!("Sign and encrypt with OpenPGP, the way many people and mail programs do: with your own key, and the keys of the people you write to.");
    card(ui, t, tl!("OpenPGP"), Some(about), |ui| {
        // The browser keeps no secret keys yet (docs/gaps.md).
        if cfg!(target_arch = "wasm32") {
            setting_row(ui, t, true, name(t, tl!("OpenPGP keys work in the desktop app")), None, |_| {});
            return;
        }
        match s.pgp_key(&id) {
            None => setting_row(
                ui,
                t,
                true,
                name(t, tl!("No OpenPGP key")),
                Some(note(t, tl!("Import your secret key file (.asc or .gpg), with its passphrase."))),
                |ui| {
                    if ui.button(tl!("Import key…")).clicked() {
                        acts.push(Action::ImportPgpKey(id.clone()));
                    }
                },
            ),
            Some(k) => {
                let who = k.emails.first().cloned().unwrap_or_else(|| k.name.clone());
                let bad = k.revoked.is_some() || k.expires.is_some_and(|at| at <= s.clock.now);
                let about = RichText::new(format!("{} · {} · {}", k.spaced(), k.algorithm, validity(s, &k))).size(12.0).color(if bad {
                    t.danger
                } else {
                    t.faint
                });
                setting_row(ui, t, true, name(t, &who), Some(about), |ui| {
                    if ui
                        .button(RichText::new(tl!("Remove")).color(t.danger))
                        .on_hover_text(tl!("Forget this key: its secret part goes from the keychain too"))
                        .clicked()
                    {
                        acts.push(Action::Run("pgp.forget", json!({"account": id, "fingerprint": k.fingerprint})));
                    }
                    if ui
                        .button(tl!("Add another…"))
                        .on_hover_text(tl!("A new key takes over signing and decrypting; the one in use is kept to decrypt old mail"))
                        .clicked()
                    {
                        acts.push(Action::ImportPgpKey(id.clone()));
                    }
                });
            }
        }
        for k in s.pgp_older_keys(&id) {
            let who = k.emails.first().cloned().unwrap_or_else(|| k.name.clone());
            let about = tf!("Older key {key}: kept to decrypt mail sent to it", key = k.key_id);
            setting_row(ui, t, false, name(t, &who), Some(note(t, &about)), |ui| {
                if ui
                    .button(RichText::new(tl!("Remove")).color(t.danger))
                    .on_hover_text(tl!("Forget this key: mail encrypted to it can't be read any more"))
                    .clicked()
                {
                    acts.push(Action::Run("pgp.forget", json!({"account": id, "fingerprint": k.fingerprint})));
                }
                if ui
                    .button(tl!("Use for sending"))
                    .on_hover_text(tl!("Sign and decrypt with this key again; the one in use is kept to decrypt"))
                    .clicked()
                {
                    acts.push(Action::Run("pgp.use", json!({"account": id, "fingerprint": k.fingerprint})));
                }
            });
        }
        let settings = s.pgp_settings(&id);
        if s.pgp_key(&id).is_some() {
            setting_row(
                ui,
                t,
                false,
                name(t, tl!("Send your public key with your mail")),
                Some(note(t, tl!("Their mail programs can then encrypt their answers (Autocrypt)."))),
                |ui| {
                    let mut on = !settings.no_autocrypt;
                    if widgets::switch(ui, t, &mut on, tl!("Send your public key with your mail")).changed() {
                        acts.push(Action::Run("pgp.options", json!({"account": id, "autocrypt": on})));
                    }
                },
            );
        }
        let label = tf!("Keys of the people you write to: {n}", n = settings.known.len());
        setting_row(ui, t, false, name(t, &label), Some(note(t, tl!("They come with their mail, or from a key file they gave you."))), |ui| {
            if ui.button(tl!("Import someone's key…")).clicked() {
                acts.push(Action::ImportPublicKey(id.clone()));
            }
        });
    });
}
