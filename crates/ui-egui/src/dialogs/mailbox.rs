//! Dialogs: an account's automatic replies.

use super::*;

pub(super) fn draw(ui: &mut Ui, d: &mut Dialog, s: &Session, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    match d {
        Dialog::Signature(f) => super::signatures::draw(ui, f, s, t, acts),
        Dialog::Vacation(f) => vacation(ui, f, t, acts),
        // Every other kind is shown by its own group (`body`).
        _ => true,
    }
}

/// Automatic replies being edited.
#[derive(Clone, Debug, Default)]
pub struct VacationForm {
    pub account: String,
    pub enabled: bool,
    pub subject: String,
    pub text: String,
    /// Only between two days.
    pub dated: bool,
    pub from: String,
    pub until: String,
    /// Exchange keeps a reply for people inside the organisation and one for those outside.
    pub exchange: bool,
    /// Outside gets a reply of its own.
    pub split: bool,
    pub external: String,
    /// Who outside gets one.
    pub outside: Outside,
    /// IMAP (issue #243): the replies shown are another program's filters' (that script's),
    /// changed there; and another program's filters run in place of MailCraft's.
    pub sent_by: Option<String>,
    pub instead: Option<String>,
    pub error: Option<String>,
}

/// Who outside the organisation gets an automatic reply (Exchange).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Outside {
    #[default]
    Anyone,
    /// The user's contacts only.
    Known,
    NoOne,
}

impl Outside {
    /// As the account's replies say it (`Vacation::outside`): `known`, `none`, else (empty or
    /// `all`) anyone.
    pub fn from_code(code: &str) -> Outside {
        match code {
            "known" => Outside::Known,
            "none" => Outside::NoOne,
            _ => Outside::Anyone,
        }
    }

    /// As `account.setVacation` takes it.
    pub fn code(self) -> &'static str {
        match self {
            Outside::Anyone => "all",
            Outside::Known => "known",
            Outside::NoOne => "none",
        }
    }
}

impl VacationForm {
    pub(crate) fn params(&self) -> serde_json::Value {
        let day = |d: &str| if self.dated && !d.trim().is_empty() { json!(d.trim()) } else { serde_json::Value::Null };
        let mut p = json!({
            "account": self.account,
            "enabled": self.enabled,
            "subject": self.subject.trim(),
            "text": self.text,
            "from": day(&self.from),
            "until": day(&self.until),
        });
        if self.exchange {
            p["outside"] = json!(self.outside.code());
            if self.split && self.outside != Outside::NoOne {
                p["external"] = json!(self.external);
            }
        }
        p
    }
}

pub(super) fn vacation(ui: &mut Ui, f: &mut VacationForm, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    ui.set_width(520.0);
    title(ui, t, "Automatic replies");
    ui.label(
        RichText::new(tl!(
            "Each sender gets one reply a day while they're on. They're kept on your mail server, so they work while this computer is off."
        ))
        .color(t.dim)
        .small(),
    );
    // Another program's filters on the server (issue #243: SOGo's took over, and its replies
    // showed as off here).
    if let Some(name) = &f.instead {
        ui.add_space(6.0);
        ui.add(
            egui::Label::new(
                RichText::new(tf!(
                    "MailCraft's automatic replies and rules on the server aren't running: another program made its own filters (“{name}”) the ones that run.",
                    name = name
                ))
                .color(t.warn)
                .small(),
            )
            .wrap(),
        );
    }
    let theirs = f.sent_by.clone();
    if let Some(name) = &theirs {
        ui.add_space(6.0);
        ui.add(
            egui::Label::new(
                RichText::new(tf!(
                    "These are the replies your other filters on the server (“{name}”) send: change them in the program that made them. MailCraft's can be on only while those are off: the server sends one reply.",
                    name = name
                ))
                .color(t.dim)
                .small(),
            )
            .wrap(),
        );
    }
    ui.add_space(8.0);
    ui.add_enabled_ui(theirs.is_none(), |ui| form(ui, f));
    if let Some(e) = &f.error {
        ui.add_space(6.0);
        ui.label(RichText::new(e).color(t.danger));
    }
    let (ok, cancel) = buttons(ui, t, "Save", theirs.is_none() && (!f.enabled || !f.text.trim().is_empty()));
    if ok {
        f.error = None;
        acts.push(Action::Submit("account.setVacation", f.params()));
    }
    !cancel
}

/// The replies' fields.
fn form(ui: &mut Ui, f: &mut VacationForm) {
    ui.radio_value(&mut f.enabled, false, tl!("Don't send automatic replies"));
    ui.radio_value(&mut f.enabled, true, tl!("Send automatic replies"));
    ui.add_enabled_ui(f.enabled, |ui| {
        ui.add_space(4.0);
        ui.checkbox(&mut f.dated, tl!("Only between these days"));
        if f.dated {
            ui.horizontal(|ui| {
                ui.label(tl!("From"));
                ui.add_sized([110.0, 26.0], egui::TextEdit::singleline(&mut f.from).hint_text("YYYY-MM-DD"));
                ui.label(tl!("Until"));
                ui.add_sized([110.0, 26.0], egui::TextEdit::singleline(&mut f.until).hint_text("YYYY-MM-DD"));
            });
        }
        ui.add_space(4.0);
        ui.add_sized([ui.available_width(), 26.0], egui::TextEdit::singleline(&mut f.subject).hint_text(tl!("Subject (optional)")));
        ui.add_space(4.0);
        let rows = if f.exchange { 4 } else { 6 };
        if f.exchange {
            ui.label(RichText::new(tl!("Inside my organization")).strong().small());
        }
        ui.add(egui::TextEdit::multiline(&mut f.text).hint_text(tl!("I'm away until…")).desired_rows(rows).desired_width(f32::INFINITY));
        // Exchange: who outside the organisation gets a reply, and which (Outlook's two tabs).
        if f.exchange {
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(tl!("Outside my organization")).strong().small());
                for (who, label) in [(Outside::Anyone, "Anyone"), (Outside::Known, "My contacts only"), (Outside::NoOne, "No one")] {
                    ui.radio_value(&mut f.outside, who, tl!(label));
                }
            });
            if f.outside != Outside::NoOne {
                ui.checkbox(&mut f.split, tl!("A different reply for them"));
                if f.split {
                    ui.add(
                        egui::TextEdit::multiline(&mut f.external).hint_text(tl!("I'm away until…")).desired_rows(rows).desired_width(f32::INFINITY),
                    );
                }
            }
        }
    });
}
