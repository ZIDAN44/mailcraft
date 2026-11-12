//! A mail server on this computer with a certificate of its own (issue #77): a bridge to a mail
//! service makes one, which no authority vouches for. MailCraft refuses it, says which certificate
//! it was, and asks here whether to trust that one for the account. Only for a server on this
//! computer: anywhere else, a certificate nobody vouches for stays refused.

use super::*;

/// A fingerprint as it's read out and compared: in fours, on two lines.
fn in_fours(fingerprint: &str) -> String {
    let upper = fingerprint.to_ascii_uppercase();
    let groups: Vec<&str> = upper.as_bytes().chunks(4).filter_map(|c| std::str::from_utf8(c).ok()).collect();
    let half = groups.len().div_ceil(2);
    groups.chunks(half.max(1)).map(|line| line.join(" ")).collect::<Vec<_>>().join("\n")
}

/// Whether the dialog stays open.
pub(super) fn draw(ui: &mut Ui, d: &Dialog, s: &Session, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    let Dialog::TrustCertificate { account, host, fingerprint } = d else { return false };
    ui.set_width(460.0);
    title(ui, t, "Trust this computer's mail server?");
    if let Some(a) = s.account(account) {
        ui.label(RichText::new(a.config.email.as_str()).strong());
        ui.add_space(4.0);
    }
    let about = tf!(
        "{host} is on this computer and shows a certificate it made itself, which no authority vouches for. A program that connects a mail service to mail apps does this. Trust it only if you set that program up.",
        host = host
    );
    ui.add(egui::Label::new(RichText::new(about).color(t.dim)).wrap());
    ui.add_space(10.0);
    ui.label(RichText::new(tl!("Certificate fingerprint (SHA-256)")).color(t.faint).small());
    ui.add(egui::Label::new(RichText::new(in_fours(fingerprint)).monospace().color(t.text)).selectable(true));
    ui.add_space(8.0);
    ui.add(egui::Label::new(RichText::new(tl!("MailCraft asks again if the server shows another certificate.")).color(t.faint).small()).wrap());
    let (ok, cancel) = buttons(ui, t, "Trust this certificate", true);
    if ok {
        acts.push(Action::Run("account.trustCertificate", json!({"account": account, "fingerprint": fingerprint})));
        return false;
    }
    !cancel
}

#[cfg(test)]
#[test]
fn a_fingerprint_reads_in_fours() {
    let shown = in_fours(&"0a1b2c3d".repeat(8));
    assert_eq!(shown, "0A1B 2C3D 0A1B 2C3D 0A1B 2C3D 0A1B 2C3D\n0A1B 2C3D 0A1B 2C3D 0A1B 2C3D 0A1B 2C3D");
    assert_eq!(in_fours(""), "");
    assert_eq!(in_fours("abc"), "ABC");
}
