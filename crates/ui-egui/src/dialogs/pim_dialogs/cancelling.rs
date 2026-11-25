//! Deleting a meeting the user organizes, with guests (issue #235): as Outlook, the guests' cancellation is
//! asked about first, with a box for the user's own words, or the meeting is deleted without
//! telling anyone (a cancellation went at once, in English, with nothing to add).

use super::*;

/// The box asking how a meeting the user organizes is cancelled.
#[derive(Clone, Debug)]
pub struct CancelForm {
    pub uid: String,
    pub summary: String,
    /// The occurrence of a series that's deleted alone.
    pub occurrence: Option<i64>,
    /// The user's words to the guests.
    pub message: String,
}

impl CancelForm {
    pub fn new(uid: &str, summary: &str, occurrence: Option<i64>) -> Box<CancelForm> {
        Box::new(CancelForm { uid: uid.to_string(), summary: summary.to_string(), occurrence, message: String::new() })
    }

    fn params(&self, notify: bool) -> serde_json::Value {
        let mut p = json!({"uid": self.uid, "notify": notify});
        if let Some(at) = self.occurrence {
            p["occurrence"] = json!(at);
        }
        if notify && !self.message.trim().is_empty() {
            p["message"] = json!(self.message.trim());
        }
        p
    }
}

/// The box; whether it stays open.
pub fn cancel(ui: &mut Ui, f: &mut CancelForm, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    ui.set_width(440.0);
    if f.occurrence.is_some() {
        crate::dialogs::title(ui, t, "Cancel this occurrence");
    } else {
        crate::dialogs::title(ui, t, "Cancel meeting");
    }
    ui.add(egui::Label::new(RichText::new(f.summary.as_str()).color(t.dim)).truncate());
    ui.add(
        egui::Label::new(RichText::new(tl!("Your guests are sent a cancellation. Add a message to them, if you like.")).color(t.dim).small()).wrap(),
    );
    ui.add_space(6.0);
    ui.add(
        egui::TextEdit::multiline(&mut f.message)
            .hint_text(tl!("Message to your guests"))
            .desired_rows(4)
            .desired_width(f32::INFINITY)
            .margin(Margin::symmetric(8, 6)),
    );
    ui.add_space(10.0);
    crate::dialogs::button_row(ui, |ui| {
        let mut keep = true;
        if widgets::primary_button(ui, "Send cancellation", t, true).clicked() {
            acts.push(Action::Submit("event.delete", f.params(true)));
        }
        if ui.button(tl!("Delete without sending")).clicked() {
            acts.push(Action::Submit("event.delete", f.params(false)));
        }
        if ui.button(tl!("Cancel")).clicked() {
            keep = false;
        }
        keep
    })
}
