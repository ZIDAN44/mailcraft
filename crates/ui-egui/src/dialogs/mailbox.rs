//! Dialogs: an account's automatic replies and signature.

use super::*;

pub(super) fn draw(ui: &mut Ui, d: &mut Dialog, _s: &Session, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    match d {
        Dialog::Signature(f) => {
            ui.set_width(480.0);
            title(ui, t, "Signature");
            ui.label(RichText::new(tl!("Added to new messages, replies and forwards.")).color(t.dim).small());
            ui.label(
                RichText::new(tl!("Paste one from Outlook or Word and it keeps its look; Ctrl+B, Ctrl+I and Ctrl+U format it.")).color(t.dim).small(),
            );
            ui.add_space(4.0);
            egui::Frame::new().stroke(egui::Stroke::new(1.0, t.border)).inner_margin(egui::Margin::same(6)).show(ui, |ui| {
                egui::ScrollArea::vertical().id_salt("signature-text").max_height(220.0).auto_shrink([false, false]).show(ui, |ui| {
                    ui.set_min_height(220.0);
                    let SignatureForm { compose, editor, .. } = &mut **f;
                    crate::rich_edit::body(ui, t, &mut editor.0, compose, None, acts);
                    crate::rich_edit::link_dialog(ui, t, &mut editor.0, compose);
                });
            });
            let (ok, cancel) = buttons(ui, t, if f.insert { "Save and insert" } else { "Save" }, true);
            if ok {
                let text = f.compose.body.trim_end().to_string();
                let mut styles = f.compose.styles.clone();
                styles.fit(&text);
                let html = if styles.is_plain() { None } else { Some(mailcraft_engine::mime::rich::to_html(&text, &styles)) };
                acts.push(Action::Run("account.setSignature", json!({"account": f.account, "signature": text, "html": html})));
                if f.insert && !text.trim().is_empty() {
                    acts.push(Action::InsertSignature);
                }
                return false;
            }
            !cancel
        }
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
    /// Who outside gets one: `all`, `known` or `none`.
    pub outside: String,
    pub error: Option<String>,
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
            p["outside"] = json!(self.outside);
            if self.split && self.outside != "none" {
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
    ui.add_space(8.0);
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
                for (code, label) in [("all", "Anyone"), ("known", "My contacts only"), ("none", "No one")] {
                    ui.radio_value(&mut f.outside, code.to_string(), tl!(label));
                }
            });
            if f.outside != "none" {
                ui.checkbox(&mut f.split, tl!("A different reply for them"));
                if f.split {
                    ui.add(
                        egui::TextEdit::multiline(&mut f.external).hint_text(tl!("I'm away until…")).desired_rows(rows).desired_width(f32::INFINITY),
                    );
                }
            }
        }
    });
    if let Some(e) = &f.error {
        ui.add_space(6.0);
        ui.label(RichText::new(e).color(t.danger));
    }
    let (ok, cancel) = buttons(ui, t, "Save", !f.enabled || !f.text.trim().is_empty());
    if ok {
        f.error = None;
        acts.push(Action::Submit("account.setVacation", f.params()));
    }
    !cancel
}

/// The signature editor: the account's signature in a message editor of its own, so it can be
/// formatted, or pasted from Outlook with its look.
#[derive(Clone, Debug)]
pub struct SignatureForm {
    pub account: String,
    pub compose: Compose,
    /// Opened from a message being written: shown over its window, and put into it.
    pub insert: bool,
    editor: SignatureEditor,
}

impl SignatureForm {
    pub fn new(s: &Session, account: &str, insert: bool) -> SignatureForm {
        let (body, styles) = s.signature_rich(account);
        SignatureForm {
            account: account.to_string(),
            compose: Compose { id: crate::rich_edit::SIGNATURE_ID, account: account.to_string(), body, styles, ..Default::default() },
            insert,
            editor: SignatureEditor(crate::rich_edit::Editor::new(crate::rich_edit::SIGNATURE_ID)),
        }
    }
}

/// The editor's state (undo, the selection); a copy starts afresh.
pub(crate) struct SignatureEditor(pub crate::rich_edit::Editor);

impl Clone for SignatureEditor {
    fn clone(&self) -> SignatureEditor {
        SignatureEditor(crate::rich_edit::Editor::new(self.0.id))
    }
}

impl std::fmt::Debug for SignatureEditor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SignatureEditor")
    }
}
