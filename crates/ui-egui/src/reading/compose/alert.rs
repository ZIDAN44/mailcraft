//! The message being written: why it couldn't be sent or saved, said over it until dismissed.

use super::*;

/// Why the message being written couldn't be sent or saved, said over it until dismissed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ComposeAlert {
    /// Sent to no one.
    NoRecipient,
    /// From an account that can't send.
    Blocked(SendBlock),
    /// Anything else, in the engine's words.
    Other(String),
}

impl ComposeAlert {
    /// What the alert says: the window's own sentence for what it knows, the engine's words as
    /// a sentence for the rest.
    pub(crate) fn text(&self) -> String {
        match self {
            ComposeAlert::NoRecipient => tl!("Add at least one name in To, Cc or Bcc before sending.").to_string(),
            ComposeAlert::Blocked(b) => send_block_text(*b),
            ComposeAlert::Other(msg) => {
                let mut text = msg.clone();
                if let Some(first) = text.get(..1) {
                    text = first.to_uppercase() + text.get(1..).unwrap_or_default();
                }
                if !text.ends_with('.') {
                    text.push('.');
                }
                text
            }
        }
    }
}

/// Why message `id` couldn't go, over it (in its own window when it has one), until OK; one
/// not sent for want of an account offers to add one while that's still why.
pub(super) fn alert(ui: &mut Ui, t: &Tokens, st: &mut UiState, id: u64, blocked: Option<SendBlock>, acts: &mut Vec<Action>) {
    let Some(alert) = st.compose_alert.clone() else { return };
    let modal = egui::Modal::new(egui::Id::unique(("compose-alert", id))).frame(crate::dialogs::dialog_frame(t)).show(ui.ctx(), |ui| {
        ui.set_width(380.0);
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::hover());
            icons::paint(ui.painter(), r, Icon::Warning, t.warn);
            ui.add_space(6.0);
            ui.vertical(|ui| {
                ui.label(RichText::new(tl!("MailCraft")).font(theme::bold(16.0)).color(t.text));
                ui.add(egui::Label::new(RichText::new(alert.text()).color(t.text)).wrap());
            });
        });
        ui.add_space(10.0);
        ui.with_layout(crate::bidi::row_from_end(egui::Align::Center), |ui| {
            // Enter is OK; not the Ctrl+Enter that tried to send and brought it up.
            let ok = ui.button(tl!("OK")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.command);
            // Not sent for want of an account (still): one can be added from here.
            let add = matches!(alert, ComposeAlert::Blocked(b) if b != SendBlock::NoOutgoingServer && blocked == Some(b))
                && ui.button(tl!("Add an account")).clicked();
            if add {
                acts.push(Action::Dialog(crate::Dialog::add_account()));
            }
            ok || add
        })
        .inner
    });
    if modal.inner || modal.should_close() {
        st.compose_alert = None;
    }
}
