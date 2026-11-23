//! A mail server's certificate no authority vouches for, asked about (issues #77 and #105).
//! MailCraft refuses it, says which certificate it was, and asks here whether to trust that one
//! for the account. On this computer a bridge to a mail service makes its own; elsewhere a
//! company's or a home server may, and so could someone in between: what the certificate says
//! of itself is shown, and only whoever runs the server can confirm its fingerprint.

use super::*;
use mailcraft_engine::model::UntrustedCertificate;

/// A fingerprint as it's read out and compared: in fours, on two lines.
pub(crate) fn in_fours(fingerprint: &str) -> String {
    let upper = fingerprint.to_ascii_uppercase();
    let groups: Vec<&str> = upper.as_bytes().chunks(4).filter_map(|c| std::str::from_utf8(c).ok()).collect();
    let half = groups.len().div_ceil(2);
    groups.chunks(half.max(1)).map(|line| line.join(" ")).collect::<Vec<_>>().join("\n")
}

/// The day of `at`, as the interface writes dates.
fn day(s: &Session, at: i64) -> String {
    crate::i18n::date(&mailcraft_engine::model::dates::day_text(at.saturating_add(i64::from(s.clock.offset)).div_euclid(86_400), &s.clock))
}

/// What a certificate elsewhere says of itself: for whom (and which servers), by whom, when.
fn what_it_says(ui: &mut Ui, s: &Session, t: &Tokens, shown: &UntrustedCertificate) {
    // A row it has nothing for (a certificate that couldn't be read) is left out.
    let row = |ui: &mut Ui, label: String, value: String| {
        if value.trim().is_empty() {
            return;
        }
        ui.label(RichText::new(label).color(t.faint).small());
        ui.add(egui::Label::new(RichText::new(value).color(t.text)).wrap());
        ui.end_row();
    };
    egui::Grid::new("untrusted-certificate").num_columns(2).spacing([12.0, 4.0]).show(ui, |ui| {
        let names = if shown.names.is_empty() { shown.subject.clone() } else { shown.names.join(", ") };
        row(ui, tl!("Issued to").to_string(), names);
        row(ui, tl!("Issued by").to_string(), shown.issuer.clone());
        if shown.not_after > 0 {
            row(ui, tl!("Valid").to_string(), tf!("{from} to {until}", from = day(s, shown.not_before), until = day(s, shown.not_after)));
        }
    });
}

/// Whether the dialog stays open.
pub(crate) fn draw(ui: &mut Ui, d: &Dialog, s: &Session, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    let Dialog::TrustCertificate { account, shown } = d else { return false };
    ui.set_width(460.0);
    if shown.remote {
        title(ui, t, "Trust this mail server's certificate?");
    } else {
        title(ui, t, "Trust this computer's mail server?");
    }
    if let Some(a) = s.account(account) {
        ui.label(RichText::new(a.config.email.as_str()).strong());
        ui.add_space(4.0);
    }
    let about = if shown.remote {
        tf!(
            "No authority this computer trusts vouches for the certificate {host} shows. A company's or a home mail server may make its own, but someone between you and the server could show one too. Trust it only if whoever runs the server confirms this fingerprint.",
            host = shown.host
        )
    } else {
        tf!(
            "{host} is on this computer and shows a certificate it made itself, which no authority vouches for. A program that connects a mail service to mail apps does this. Trust it only if you set that program up.",
            host = shown.host
        )
    };
    ui.add(egui::Label::new(RichText::new(about).color(t.dim)).wrap());
    ui.add_space(10.0);
    if shown.remote {
        what_it_says(ui, s, t, shown);
        ui.add_space(8.0);
    }
    ui.label(RichText::new(tl!("Certificate fingerprint (SHA-256)")).color(t.faint).small());
    ui.add(egui::Label::new(RichText::new(in_fours(&shown.fingerprint)).monospace().color(t.text)).selectable(true));
    ui.add_space(8.0);
    ui.add(egui::Label::new(RichText::new(tl!("MailCraft asks again if the server shows another certificate.")).color(t.faint).small()).wrap());
    if shown.remote {
        let file = tl!("Whoever runs it may give you its certificate as a file instead: Server settings › Trust a certificate file…");
        ui.add(egui::Label::new(RichText::new(file).color(t.faint).small()).wrap());
    }
    // Not where the organisation doesn't allow trusting a certificate by hand (issue #245).
    let allowed = !s.org.no_certificate_exceptions;
    if !allowed {
        ui.add(egui::Label::new(RichText::new(tl!("Your organisation doesn't allow trusting a certificate by hand.")).color(t.warn).small()).wrap());
    }
    let (ok, cancel) = buttons(ui, t, "Trust this certificate", allowed);
    if ok {
        acts.push(Action::Run("account.trustCertificate", json!({"account": account, "fingerprint": shown.fingerprint})));
        return false;
    }
    !cancel
}

/// The certificates an account trusts besides the usual authorities, each with Stop trusting,
/// and Trust a certificate file… (issue #105): in Server settings, for an account with mail
/// servers of its own.
pub(crate) fn trusted_rows(ui: &mut Ui, s: &Session, t: &Tokens, account: &str, acts: &mut Vec<Action>) {
    ui.add_space(6.0);
    ui.label(RichText::new(tl!("Certificates")).color(t.dim));
    for c in s.trusted_certificates(account) {
        // Each told apart by whom it's for, or one trusted when asked by its fingerprint's start;
        // the whole fingerprint on hover. Kept to one short line, as the dialog is narrow.
        let short: String = in_fours(&c.fingerprint).split_whitespace().take(4).collect::<Vec<_>>().join(" ");
        let (what, by) = match &c.name {
            // An authority issues its own certificate: said once.
            Some((whom, issuer)) if !whom.is_empty() && whom == issuer => (whom.clone(), None),
            Some((whom, issuer)) if !whom.is_empty() => (whom.clone(), Some(tf!("Issued by {issuer}", issuer = issuer))),
            Some((_, issuer)) => (tf!("Issued by {issuer}", issuer = issuer), None),
            None => (tl!("Trusted when asked").to_string(), Some(format!("{short} …"))),
        };
        ui.horizontal(|ui| {
            ui.label(RichText::new(what).color(t.text)).on_hover_text(in_fours(&c.fingerprint));
            if let Some(by) = by {
                ui.label(RichText::new(by).color(t.faint).small());
            }
            if ui.small_button(tl!("Stop trusting")).clicked() {
                acts.push(Action::Run("account.forgetCertificate", json!({"account": account, "fingerprint": c.fingerprint})));
            }
        });
    }
    let hint = tl!("An authority your organisation runs, or your mail server's own certificate (.pem, .crt, .cer)");
    // Not where the organisation doesn't allow it (issue #245).
    let allowed = !s.org.no_certificate_exceptions;
    if ui.add_enabled(allowed, egui::Button::new(tl!("Trust a certificate file…"))).on_hover_text(hint).clicked() {
        acts.push(Action::TrustCertificateFile(account.to_string()));
    }
}

#[cfg(test)]
#[test]
fn a_fingerprint_reads_in_fours() {
    let shown = in_fours(&"0a1b2c3d".repeat(8));
    assert_eq!(shown, "0A1B 2C3D 0A1B 2C3D 0A1B 2C3D 0A1B 2C3D\n0A1B 2C3D 0A1B 2C3D 0A1B 2C3D 0A1B 2C3D");
    assert_eq!(in_fours(""), "");
    assert_eq!(in_fours("abc"), "ABC");
}
