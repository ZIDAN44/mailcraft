//! The reading pane: how a message was protected (S/MIME), said above it in MailCraft's own
//! colours, outside anything the message itself draws, so a message can't pass for signed.

use mailcraft_engine::{Lock, Reach, RevocationCheck, Scheme, Seal, Security, smime};

use super::*;

/// An encrypted message not decrypted (yet, or at all): nothing of it is shown but the bar.
pub(super) fn still_locked(sec: &Security) -> bool {
    matches!(sec.lock, Some(Lock::Opening | Lock::Failed(_)))
}

fn when(s: &Session, at: i64) -> String {
    crate::i18n::date(&dates::day_label(at.saturating_add(i64::from(s.clock.offset)).div_euclid(86_400)))
}

/// Why a certificate (or, `pgp`, an OpenPGP key) isn't trusted for a message, each reason in words.
fn problem(s: &Session, p: &smime::Problem, pgp: bool) -> String {
    if pgp {
        match p {
            smime::Problem::Expired(at) => return tf!("its key ran out on {date}", date = when(s, *at)),
            smime::Problem::NotYetValid(at) => return tf!("its key isn't valid until {date}", date = when(s, *at)),
            smime::Problem::OtherAddress(emails) => {
                let addresses = if emails.is_empty() { "-".to_string() } else { emails.join(", ") };
                return tf!("its key is for {addresses}, not for the sender", addresses = addresses);
            }
            smime::Problem::Revoked(at) => return tf!("its owner withdrew the key on {date}", date = when(s, *at)),
            smime::Problem::UnknownIssuer | smime::Problem::NotForEmail => {}
        }
    }
    match p {
        smime::Problem::UnknownIssuer => tl!("its certificate isn't from an authority this computer trusts").to_string(),
        smime::Problem::Expired(at) => tf!("its certificate ran out on {date}", date = when(s, *at)),
        smime::Problem::NotYetValid(at) => tf!("its certificate isn't valid until {date}", date = when(s, *at)),
        smime::Problem::OtherAddress(emails) => {
            tf!(
                "its certificate is for {addresses}, not for the sender",
                addresses = if emails.is_empty() { "-".to_string() } else { emails.join(", ") }
            )
        }
        smime::Problem::NotForEmail => tl!("its certificate isn't for signing email").to_string(),
        smime::Problem::Revoked(at) => tf!("its authority withdrew its certificate on {date}", date = when(s, *at)),
    }
}

fn line(ui: &mut Ui, t: &Tokens, icon: Icon, color: Color32, text: String, note: Option<String>, hover: Option<String>) {
    egui::Frame::new().fill(color.gamma_multiply(0.12)).corner_radius(6).inner_margin(Margin::symmetric(10, 7)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        let r = ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
            icons::paint(ui.painter(), r, icon, color);
            ui.vertical(|ui| {
                ui.add(egui::Label::new(RichText::new(text).color(t.text)).wrap());
                if let Some(n) = note {
                    ui.add(egui::Label::new(RichText::new(n).color(t.dim).small()).wrap());
                }
            });
        });
        if let Some(h) = hover {
            r.response.on_hover_text(h);
        }
    });
    ui.add_space(6.0);
}

/// Under the recipients of the message being written: whether it can go encrypted to everyone in
/// it, and who can't receive it encrypted (issue #6). Nothing while the account has no
/// certificate or key, or the message no recipient.
pub(super) fn encryption_line(ui: &mut Ui, t: &Tokens, reach: &Reach, encrypting: bool) {
    let (color, text, hover) = match reach {
        Reach::NotSetUp | Reach::NoOne => return,
        Reach::Everyone(Scheme::Smime) => (t.ok, tl!("Encryption available: every recipient can receive it (S/MIME)").to_string(), None),
        Reach::Everyone(Scheme::Pgp) => (t.ok, tl!("Encryption available: every recipient can receive it (OpenPGP)").to_string(), None),
        Reach::Missing { who, .. } => {
            let names = who.join(", ");
            let text = tf!("Encryption not available for {names}: there's no certificate or OpenPGP key for them", names = names);
            (if encrypting { t.danger } else { t.dim }, text.clone(), Some(text))
        }
    };
    let r = ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
        icons::paint(ui.painter(), r, Icon::Lock, color);
        ui.add(egui::Label::new(RichText::new(text).color(color).small()).truncate());
    });
    if let Some(h) = hover {
        r.response.on_hover_text(h);
    }
}

/// The message being written: which protection it gets, when the account has both a certificate
/// and an OpenPGP key.
pub(super) fn scheme_menu(ui: &mut Ui, c: &mut mailcraft_engine::Compose) {
    let name = |s: Option<Scheme>| match s {
        None => tl!("Automatic"),
        Some(Scheme::Smime) => tl!("S/MIME"),
        Some(Scheme::Pgp) => tl!("OpenPGP"),
    };
    egui::ComboBox::from_id_salt(("compose-scheme", c.id))
        .selected_text(RichText::new(name(c.scheme)).small())
        .width(96.0)
        .show_ui(ui, |ui| {
            for s in [None, Some(Scheme::Smime), Some(Scheme::Pgp)] {
                ui.selectable_value(&mut c.scheme, s, name(s));
            }
        })
        .response
        .on_hover_text(tl!("Sign and encrypt with S/MIME or OpenPGP; Automatic takes the one every recipient can receive"));
}

/// The bars for an encrypted and/or signed message.
pub(super) fn security_bar(s: &Session, t: &Tokens, ui: &mut Ui, sec: &Security) {
    match &sec.lock {
        Some(Lock::Opening) => line(ui, t, Icon::Lock, t.dim, tl!("Encrypted. Decrypting it…").to_string(), None, None),
        Some(Lock::Failed(why)) => {
            line(ui, t, Icon::Lock, t.danger, tf!("This message is encrypted, and it couldn't be decrypted: {error}", error = tl!(why)), None, None)
        }
        Some(Lock::Open { .. }) => {
            let note = sec
                .unauthenticated()
                .then(|| tl!("It isn't signed, so it could have been changed on the way: be careful with what it asks.").to_string());
            line(
                ui,
                t,
                Icon::Lock,
                if note.is_some() { t.warn } else { t.ok },
                tl!("Encrypted: only you and its other recipients can read it.").to_string(),
                note,
                None,
            );
        }
        None => {}
    }
    if sec.headers {
        ui.label(RichText::new(tl!("Its subject and addresses were protected inside it too.")).color(t.faint).small());
        ui.add_space(4.0);
    }
    let Some(signed) = &sec.signed else { return };
    let named = |name: &str, emails: &[String]| match emails.first() {
        Some(e) if !name.eq_ignore_ascii_case(e) && !name.is_empty() => format!("{name} <{e}>"),
        Some(e) => e.clone(),
        None => name.to_string(),
    };
    let signed_at = signed.at.map(|at| format!("\n{}", tf!("Signed {date}", date = crate::i18n::date(&dates::full_time(at, &s.clock)))));
    let who = signed.cert.as_ref().map(|c| named(&c.name, &c.emails)).or_else(|| signed.key.as_ref().map(|k| named(&k.name, &k.emails)));
    let hover = signed
        .cert
        .as_ref()
        .map(|c| {
            let mut h = tf!("Issued by {issuer}, valid until {date}", issuer = c.issuer, date = when(s, c.not_after));
            h.push_str(&format!("\n{}\n{}", c.key, c.fingerprint));
            h
        })
        .or_else(|| signed.key.as_ref().map(|k| format!("{}\n{}", tf!("OpenPGP key {fingerprint}", fingerprint = k.spaced()), k.algorithm)))
        .map(|h| h + signed_at.as_deref().unwrap_or_default());
    let checked = match &signed.revocation {
        Some(RevocationCheck::Asking) => Some(tl!("Asking its authority whether the certificate was withdrawn…").to_string()),
        Some(RevocationCheck::Good) => Some(tl!("Its authority says the certificate wasn't withdrawn.").to_string()),
        Some(RevocationCheck::Unknown(why)) => Some(tf!("Whether the certificate was withdrawn couldn't be checked: {why}.", why = tl!(why))),
        None => None,
    };
    let weak = signed.weak.then(|| tl!("Signed with SHA-1, which no longer proves much.").to_string());
    let weak = match (weak, checked) {
        (Some(w), Some(c)) => Some(format!("{w}\n{c}")),
        (w, c) => w.or(c),
    };
    match &signed.seal {
        Seal::Trusted => {
            let text = match &signed.key {
                Some(k) => tf!("Signed by {signer}: the signature holds (OpenPGP key {key}).", signer = who.unwrap_or_default(), key = k.key_id),
                None => {
                    let issuer = signed.cert.as_ref().map(|c| c.issuer.clone()).unwrap_or_default();
                    tf!("Signed by {signer}: the signature holds (certificate from {issuer}).", signer = who.unwrap_or_default(), issuer = issuer)
                }
            };
            line(ui, t, Icon::Seal, t.ok, text, weak, hover);
        }
        Seal::UnknownKey(key) => line(
            ui,
            t,
            Icon::Seal,
            t.warn,
            tf!("Signed with the OpenPGP key {key}, which MailCraft doesn't have, so the signature can't be checked.", key = key),
            Some(tl!("Their key comes with their mail (Autocrypt), or can be imported in File › Info › Security.").to_string()),
            None,
        ),
        Seal::Untrusted(problems) => {
            let why: Vec<String> = problems.iter().map(|p| problem(s, p, signed.key.is_some())).collect();
            let revoked = problems.iter().any(|p| matches!(p, smime::Problem::Revoked(_)));
            line(
                ui,
                t,
                Icon::Seal,
                if revoked { t.danger } else { t.warn },
                tf!("Signed by {signer}, but {why}.", signer = who.unwrap_or_default(), why = why.join("; ")),
                weak,
                hover,
            );
        }
        Seal::Broken(why) => line(ui, t, Icon::Seal, t.danger, tf!("The signature doesn't hold: {why}.", why = tl!(why)), None, hover),
    }
}
