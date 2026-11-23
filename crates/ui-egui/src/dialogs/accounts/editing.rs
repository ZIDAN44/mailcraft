//! An account's server settings, changed: its name and user name, then its kind's own rows,
//! laid out as when it was added: each server, then how it's signed in to.

use super::*;

pub(super) fn edit_account(ui: &mut Ui, f: &mut EditForm, s: &Session, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    ui.set_width(480.0);
    title(ui, t, "Server settings");
    let w = ui.available_width();
    if f.address_editable() {
        // Outlook's Email Address: mistyped, every message was turned away by the server.
        ui.add_space(4.0);
        field_label(ui, "Email address");
        ui.add_sized([w, 26.0], egui::TextEdit::singleline(&mut f.email).hint_text("you@example.com"));
        ui.add_space(6.0);
    } else {
        ui.label(RichText::new(&f.email).color(t.dim));
        ui.add_space(8.0);
    }
    field_label(ui, "Your name");
    ui.add_sized([w, 26.0], egui::TextEdit::singleline(&mut f.name).hint_text(tl!("Shown on mail you send")));
    ui.add_space(6.0);
    field_label(ui, "User name");
    ui.add_sized([w, 26.0], egui::TextEdit::singleline(&mut f.username).hint_text(tl!("Usually the email address")));
    ui.add_space(10.0);
    if let Some(url) = f.ews.as_mut() {
        // An Exchange server's address is all EWS needs.
        field_label(ui, "Exchange server");
        ui.add_sized([w, 26.0], egui::TextEdit::singleline(url).hint_text("mail.example.com"));
        user_agent_field(ui, t, w, &mut f.user_agent);
    } else {
        if let Some((host, port, security)) = f.pop.as_mut() {
            server_row(ui, w, Server::Pop, "edit-", host, port, security, !s.org.require_encryption);
            if !f.oauth {
                f.sign_in.incoming_ui(ui);
            }
        } else {
            ui.horizontal(|ui| {
                field_label(ui, "Mail server");
                ui.radio_value(&mut f.jmap, false, tl!("IMAP and SMTP"));
                ui.radio_value(&mut f.jmap, true, tl!("JMAP"));
            });
            ui.add_space(4.0);
            if f.jmap {
                field_label(ui, "JMAP session address");
                ui.add_sized([w, 26.0], egui::TextEdit::singleline(&mut f.jmap_url).hint_text("https://api.example.com/jmap/session"));
            } else {
                server_row(ui, w, Server::Imap, "edit-", &mut f.imap_host, &mut f.imap_port, &mut f.imap_security, !s.org.require_encryption);
                if !f.oauth {
                    f.sign_in.incoming_ui(ui);
                }
            }
        }
        // POP and IMAP send over SMTP.
        if f.pop.is_some() || !f.jmap {
            ui.add_space(10.0);
            server_row(ui, w, Server::Smtp, "edit-", &mut f.smtp_host, &mut f.smtp_port, &mut f.smtp_security, !s.org.require_encryption);
            // Signed in through the browser: no password to sign in with, nor a user name of its
            // own for the outgoing server.
            if !f.oauth {
                f.sign_in.outgoing_ui(ui);
                if f.sign_in.outgoing != mailcraft_engine::model::SignInMethod::None {
                    f.smtp_sign_in.ui(ui, w);
                }
            }
            let incoming = f.pop.as_ref().map_or(f.imap_security, |(_, _, security)| *security);
            clear_text_warning(ui, t, incoming, f.smtp_security);
        }
        // As when it was added: in a part of its own, closed, saying what it's set to.
        if f.pop.is_some() {
            ui.add_space(6.0);
            f.pop_keep.ui(ui, t);
        }
    }
    if f.pop.is_none() {
        online_only(ui, t, &mut f.online_only);
    }
    // Certificates trusted besides the usual authorities: for its own mail servers alone.
    if f.ews.is_none() && (f.pop.is_some() || !f.jmap) {
        super::super::trust::trusted_rows(ui, s, t, &f.account, acts);
    }
    if let Some(e) = &f.error {
        ui.add_space(6.0);
        ui.add(egui::Label::new(RichText::new(e).color(t.danger)).wrap());
    }
    let (ok, cancel) = buttons(ui, t, "Save", true);
    if ok {
        f.error = None;
        acts.push(Action::Submit("account.update", f.params()));
    }
    !cancel
}
