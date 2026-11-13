//! An account's other addresses (issue #81): another address of the same mailbox to send from,
//! with the name shown on mail from it, where replies to it go and the signature it gets.

use super::*;

/// What the dialog holds while it's open.
#[derive(Clone, Debug, Default)]
pub struct AddressForm {
    pub account: String,
    pub address: String,
    /// One the account has already, being changed: its address stays.
    pub changing: bool,
    pub name: String,
    pub reply_to: String,
    /// One of the account's signatures; `None`: the account's own for new mail and replies.
    pub signature: Option<String>,
    pub error: Option<String>,
}

impl AddressForm {
    /// For a new address of `account`.
    pub fn new(account: &str) -> AddressForm {
        AddressForm { account: account.to_string(), ..Default::default() }
    }

    /// For one of the account's addresses, as it is.
    pub fn of(account: &str, i: &mailcraft_engine::model::Identity) -> AddressForm {
        AddressForm {
            account: account.to_string(),
            address: i.email.clone(),
            changing: true,
            name: i.name.clone(),
            reply_to: i.reply_to.clone(),
            signature: i.signature.clone(),
            error: None,
        }
    }

    fn params(&self) -> serde_json::Value {
        json!({
            "account": self.account,
            "address": self.address.trim(),
            "name": self.name.trim(),
            "replyTo": self.reply_to.trim(),
            "signature": self.signature,
        })
    }
}

pub(super) fn draw(ui: &mut Ui, f: &mut AddressForm, s: &Session, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    ui.set_width(460.0);
    title(ui, t, if f.changing { "Change an address" } else { "Add an address" });
    let about = tl!(
        "Another address of this same mailbox, such as an alias, that mail can be sent from. Pick it in From when you write a message. Your mail server has to allow sending from it."
    );
    ui.add(egui::Label::new(RichText::new(about).color(t.dim)).wrap());
    ui.add_space(10.0);
    let config = s.account(&f.account).map(|a| &a.config);
    let size = vec2(290.0, 26.0);
    let mut first = None;
    egui::Grid::new("account-address").num_columns(2).spacing(vec2(10.0, 8.0)).show(ui, |ui| {
        ui.label(tl!("Email address"));
        if f.changing {
            ui.label(RichText::new(f.address.as_str()).color(t.dim));
        } else {
            first = Some(ui.add(egui::TextEdit::singleline(&mut f.address).hint_text(tl!("you@example.com")).min_size(size).desired_width(size.x)));
        }
        ui.end_row();
        ui.label(tl!("Name on mail from it"));
        let own = config.map(|c| c.name.clone()).unwrap_or_default();
        ui.add(egui::TextEdit::singleline(&mut f.name).hint_text(own).min_size(size).desired_width(size.x));
        ui.end_row();
        ui.label(tl!("Replies go to"));
        ui.add(egui::TextEdit::singleline(&mut f.reply_to).hint_text(f.address.trim().to_string()).min_size(size).desired_width(size.x));
        ui.end_row();
        ui.label(tl!("Signature"));
        let own = tl!("(the account's own)").to_string();
        let names: Vec<String> = config.map(|c| c.signatures.iter().map(|g| g.name.clone()).collect()).unwrap_or_default();
        egui::ComboBox::from_id_salt("account-address-signature")
            .width(size.x)
            .selected_text(f.signature.clone().unwrap_or_else(|| own.clone()))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut f.signature, None, own.as_str());
                for n in &names {
                    ui.selectable_value(&mut f.signature, Some(n.clone()), n.as_str());
                }
            });
        ui.end_row();
    });
    if let Some(r) = first
        && ui.memory(|m| m.focused().is_none())
    {
        r.request_focus();
    }
    if let Some(e) = &f.error {
        ui.add_space(6.0);
        ui.label(RichText::new(e.as_str()).color(t.danger).small());
    }
    let ready = f.address.contains('@');
    let (ok, cancel) = buttons(ui, t, if f.changing { "Save" } else { "Add" }, ready);
    if ok && ready {
        f.error = None;
        acts.push(Action::Submit("account.setAddress", f.params()));
    }
    !cancel
}
