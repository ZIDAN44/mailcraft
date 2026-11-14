//! An account's server settings, changed: its name and user name, then its kind's own rows.

use super::*;

pub(super) fn edit_account(ui: &mut Ui, f: &mut EditForm, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    ui.set_width(480.0);
    title(ui, t, "Server settings");
    ui.label(RichText::new(&f.email).color(t.dim));
    ui.add_space(8.0);
    let w = ui.available_width();
    field_label(ui, "Your name");
    ui.add_sized([w, 26.0], egui::TextEdit::singleline(&mut f.name).hint_text(tl!("Shown on mail you send")));
    ui.add_space(4.0);
    field_label(ui, "User name");
    ui.add_sized([w, 26.0], egui::TextEdit::singleline(&mut f.username).hint_text(tl!("Usually the email address")));
    ui.add_space(6.0);
    if let Some(url) = f.ews.as_mut() {
        // An Exchange server's address is all EWS needs.
        field_label(ui, "Exchange server");
        ui.add_sized([w, 26.0], egui::TextEdit::singleline(url).hint_text("mail.example.com"));
        user_agent_field(ui, t, w, &mut f.user_agent);
    } else {
        if let Some((host, port, security)) = f.pop.as_mut() {
            server_row(ui, w, Server::Pop, "edit-", host, port, security);
            f.pop_keep.ui(ui);
        } else {
            ui.horizontal(|ui| {
                field_label(ui, "Mail server");
                ui.radio_value(&mut f.jmap, false, tl!("IMAP and SMTP"));
                ui.radio_value(&mut f.jmap, true, tl!("JMAP"));
            });
            if f.jmap {
                field_label(ui, "JMAP session address");
                ui.add_sized([w, 26.0], egui::TextEdit::singleline(&mut f.jmap_url).hint_text("https://api.example.com/jmap/session"));
            } else {
                server_row(ui, w, Server::Imap, "edit-", &mut f.imap_host, &mut f.imap_port, &mut f.imap_security);
            }
        }
        // POP and IMAP send over SMTP.
        if f.pop.is_some() || !f.jmap {
            ui.add_space(4.0);
            server_row(ui, w, Server::Smtp, "edit-", &mut f.smtp_host, &mut f.smtp_port, &mut f.smtp_security);
            f.smtp_sign_in.ui(ui, w);
            let incoming = f.pop.as_ref().map_or(f.imap_security, |(_, _, security)| *security);
            clear_text_warning(ui, t, incoming, f.smtp_security);
        }
    }
    if f.pop.is_none() {
        online_only(ui, t, &mut f.online_only);
    }
    if let Some(e) = &f.error {
        ui.add_space(6.0);
        ui.label(RichText::new(e).color(t.danger));
    }
    let (ok, cancel) = buttons(ui, t, "Save", true);
    if ok {
        f.error = None;
        acts.push(Action::Submit("account.update", f.params()));
    }
    !cancel
}
