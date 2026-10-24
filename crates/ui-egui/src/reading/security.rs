//! The reading pane: how a message was protected (S/MIME), said above it in MailCraft's own
//! colours, outside anything the message itself draws, so a message can't pass for signed.

use mailcraft_engine::{Lock, Seal, Security, smime};

use super::*;

/// An encrypted message not decrypted (yet, or at all): nothing of it is shown but the bar.
pub(super) fn still_locked(sec: &Security) -> bool {
    matches!(sec.lock, Some(Lock::Opening | Lock::Failed(_)))
}

fn when(s: &Session, at: i64) -> String {
    crate::i18n::date(&dates::day_label(at.saturating_add(i64::from(s.clock.offset)).div_euclid(86_400)))
}

/// Why a certificate isn't trusted for a message, each reason in words.
fn problem(s: &Session, p: &smime::Problem) -> String {
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
    let who = signed.cert.as_ref().map(|c| match c.emails.first() {
        Some(e) if !c.name.eq_ignore_ascii_case(e) => format!("{} <{e}>", c.name),
        _ => c.name.clone(),
    });
    let hover = signed.cert.as_ref().map(|c| {
        let mut h = tf!("Issued by {issuer}, valid until {date}", issuer = c.issuer, date = when(s, c.not_after));
        h.push_str(&format!("\n{}\n{}", c.key, c.fingerprint));
        if let Some(at) = signed.at {
            h.push_str(&format!("\n{}", tf!("Signed {date}", date = crate::i18n::date(&dates::full_time(at, &s.clock)))));
        }
        h
    });
    let weak = signed.weak.then(|| tl!("Signed with SHA-1, which no longer proves much.").to_string());
    match &signed.seal {
        Seal::Trusted => {
            let issuer = signed.cert.as_ref().map(|c| c.issuer.clone()).unwrap_or_default();
            line(
                ui,
                t,
                Icon::Seal,
                t.ok,
                tf!("Signed by {signer}: the signature holds (certificate from {issuer}).", signer = who.unwrap_or_default(), issuer = issuer),
                weak,
                hover,
            );
        }
        Seal::Untrusted(problems) => {
            let why: Vec<String> = problems.iter().map(|p| problem(s, p)).collect();
            line(
                ui,
                t,
                Icon::Seal,
                t.warn,
                tf!("Signed by {signer}, but {why}.", signer = who.unwrap_or_default(), why = why.join("; ")),
                weak,
                hover,
            );
        }
        Seal::Broken(why) => line(ui, t, Icon::Seal, t.danger, tf!("The signature doesn't hold: {why}.", why = tl!(why)), None, hover),
    }
}
