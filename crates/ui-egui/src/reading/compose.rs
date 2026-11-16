//! The message being written: its form (from, to, cc, bcc, subject), its ribbon and its body.
//! By topic: the alert over it (`alert`), the options under the ribbon (`options`), Send and
//! the address rows (`addresses`), the attachments (`chips`); here, what they share and the
//! form put together.

use mailcraft_engine::SendBlock;

use super::*;

mod addresses;
mod alert;
mod chips;
mod options;
#[cfg(test)]
mod tests;

pub(crate) use addresses::Suggested;
pub use alert::ComposeAlert;
#[cfg(test)]
pub(crate) use options::parse_local;

/// Height of an attachment chip.
pub(super) const CHIP_H: f32 = 42.0;

/// Width of the labels and label buttons left of the message's fields.
const LABEL_W: f32 = 76.0;

/// Width of the Send button beside the address rows, and the gap after it.
const SEND_W: f32 = 66.0;
const SEND_GAP: f32 = 10.0;
/// How far in the rows under Send start (Replies to, Subject, the attachments): past it.
const PAST_SEND: f32 = SEND_W + SEND_GAP;

const ROW_H: f32 = 30.0;

/// Why a message can't be sent from its account, as the window says it.
pub(crate) fn send_block_text(b: SendBlock) -> String {
    match b {
        SendBlock::NoAccount => tl!("Add an account to send mail."),
        SendBlock::DataFile => tl!("Mail can't be sent from an Outlook data file. Add an account to send mail."),
        SendBlock::NoServer => tl!("Mail can't be sent from a mailbox kept only on this computer. Add an account to send mail."),
        SendBlock::NoOutgoingServer => tl!("This account has no outgoing (SMTP) server. Add one in its account settings to send mail."),
    }
    .to_string()
}

/// Why the message can't be sent, over the form, with a way to add an account when one would
/// send it.
fn send_block_bar(ui: &mut Ui, t: &Tokens, b: SendBlock, acts: &mut Vec<Action>) {
    egui::Frame::new().fill(t.warn.gamma_multiply(0.14)).corner_radius(6).inner_margin(Margin::symmetric(10, 6)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
            icons::paint(ui.painter(), r, Icon::Warning, t.warn);
            ui.add(egui::Label::new(RichText::new(send_block_text(b)).color(t.text)).wrap());
            if b != SendBlock::NoOutgoingServer && ui.button(tl!("Add an account")).clicked() {
                acts.push(Action::Dialog(crate::Dialog::add_account()));
            }
        });
    });
}

/// What the form needs from the session, read before the message is taken out of it.
struct Form {
    /// Where it's drawn: in a window of its own, or in the reading pane.
    in_window: bool,
    /// From's choices: the account, the address ("" its own), and who that is.
    accounts: Vec<(String, String, String)>,
    /// The people suggested for the recipient box being typed in.
    suggestions: Vec<Contact>,
    clock: dates::Clock,
    /// Delay Delivery holds mail in the Outbox, which only a server account's sends from.
    can_delay: bool,
    /// Whether it can be signed and encrypted, by certificate or OpenPGP key (issue #6).
    has_cert: bool,
    has_pgp: bool,
    /// Why it can't be sent from its account (none set up, or a data file, can't), if it can't.
    blocked: Option<SendBlock>,
    blocked_text: Option<String>,
    /// Whether everyone in it can receive it encrypted.
    reach: Option<mailcraft_engine::Reach>,
    /// The names of its account's signatures.
    signatures: Vec<String>,
}

/// What the form needs from the session (computed before the form borrows the message).
fn form(s: &mut Session, st: &mut UiState, in_window: bool) -> Form {
    let accounts = addresses::from_choices(s);
    let suggestions = addresses::suggestions(s, st);
    let clock = s.clock;
    let can_delay = s.compose.as_ref().and_then(|c| s.account(&c.account)).is_some_and(|a| !a.is_local());
    // Sign and Encrypt, for an account with a certificate or an OpenPGP key; which of the two,
    // when it has both; whether everyone in it can receive it encrypted (issue #6).
    // By the address it's from (issue #81); one with neither of its own can still encrypt
    // with the account's.
    let has_cert = s.compose.as_ref().is_some_and(|c| s.identity_for(&c.account, &c.from).or_else(|| s.identity(&c.account)).is_some());
    // Mail goes only from an account that can send it (none set up, or a data file, can't).
    let blocked = s.compose_send_block();
    let blocked_text = blocked.map(send_block_text);
    let has_pgp = s.compose.as_ref().is_some_and(|c| s.pgp_key_from(&c.account, &c.from).or_else(|| s.pgp_key(&c.account)).is_some());
    let reach = s.encryption_reach();
    let signatures: Vec<String> = s
        .compose
        .as_ref()
        .and_then(|c| s.account(&c.account))
        .map(|a| a.config.signatures.iter().map(|g| g.name.clone()).collect())
        .unwrap_or_default();
    Form { in_window, accounts, suggestions, clock, can_delay, has_cert, has_pgp, blocked, blocked_text, reach, signatures }
}

/// The message being written, in a window of its own.
pub fn compose_window(s: &mut Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    compose(s, st, ui, acts, true);
}

pub(super) fn compose(s: &mut Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>, in_window: bool) {
    let t = st.tokens;
    if !in_window {
        st.compose_area = Some((ui.ctx().cumulative_pass_nr(), ui.max_rect()));
    }
    attach_dropped(ui, &t, acts);
    // Where there are no windows of their own (the browser), the other messages being written
    // are a click away above this one.
    if !in_window && !s.composes.is_empty() && ui.ctx().embed_viewports() {
        also_being_written(ui, &t, s, acts);
    }
    let f = form(s, st, in_window);
    // Out of the session while its form is drawn, so the original under a reply can be drawn
    // from the session as the reading pane draws a message; put back after.
    let Some(mut taken) = s.compose.take() else { return };
    // The pictures from the web the reader was shown, for the ones this message quotes.
    rich_edit::web_loaded(ui.ctx(), &s.pictures);
    // The editor's state belongs to one message.
    if st.editor.as_ref().is_none_or(|e| e.id != taken.id) {
        st.editor = Some(rich_edit::Editor::new(taken.id));
    }
    let Some(mut ed) = st.editor.take() else {
        s.compose = Some(taken);
        return;
    };
    let c = &mut taken;
    let mut quote_eds = std::mem::take(&mut st.quote_editors);
    // The block of the quoted original with the cursor, while there is one: the ribbon works on it.
    let quote_cell = st.quote_focus.filter(|(id, _)| *id == c.id && c.quote.is_some()).map(|(_, n)| n);
    // Why it couldn't go: over the message, in its own window when it has one.
    alert::alert(ui, &t, st, c.id, f.blocked, acts);
    // A newly opened message: start in To (new mail) or the body (replies).
    let fresh = st.compose_shown.insert(c.id);
    let focus_to = fresh && c.to.trim().is_empty();
    let focus_body = fresh && !focus_to;
    shortcuts(ui, c, &f, s.prefs.ctrl_enter, acts);
    super::quote::select_all(st, ui, c, &mut ed, &mut quote_eds);
    egui::Frame::new().inner_margin(Margin { left: 16, right: 16, top: 8, bottom: 12 }).show(ui, |ui| {
        // From the right in a right-to-left interface, its rows too (the message's own text goes
        // the way it's written).
        ui.with_layout(crate::bidi::column(), |ui| {
            // The Message ribbon.
            egui::Frame::new().fill(t.chrome).corner_radius(6).inner_margin(Margin::symmetric(8, 4)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                let on_quote = quote_cell.and_then(|n| {
                    super::quote::with_cell(c, &mut quote_eds, n, |c, ed| rich_edit::ribbon(ui, &t, ed, c, &f.signatures, &f.clock, acts))
                });
                if on_quote.is_none() {
                    rich_edit::ribbon(ui, &t, &mut ed, c, &f.signatures, &f.clock, acts);
                }
            });
            ui.add_space(4.0);
            options::options_row(ui, &t, c, &f, acts);
            if let Some(b) = f.blocked {
                ui.add_space(4.0);
                send_block_bar(ui, &t, b, acts);
            }
            if let Some(e) = &c.error {
                ui.add_space(4.0);
                egui::Frame::new().fill(t.danger.gamma_multiply(0.12)).corner_radius(6).inner_margin(Margin::symmetric(10, 6)).show(ui, |ui| {
                    ui.label(RichText::new(tf!("Not sent: {error}", error = e)).color(t.danger));
                });
            }
            ui.add_space(6.0);
            ui.add_enabled_ui(!c.sending, |ui| {
                ui.spacing_mut().item_spacing.y = 6.0;
                addresses::address_rows(ui, st, c, &f, focus_to, acts);
                if !c.attachments.is_empty() {
                    chips::attachment_chips(ui, &t, s.prefs.keep_photo_location, c, acts);
                }
                ui.add_space(4.0);
                let sep = ui.available_rect_before_wrap();
                ui.painter().line_segment([sep.left_top(), sep.right_top()], egui::Stroke::new(1.0, t.border));
                ui.add_space(8.0);
                paste_picture(ui, c, acts);
                egui::ScrollArea::vertical().auto_shrink(false).id_salt("compose-body").show(ui, |ui| {
                    // Replies start above the quoted text.
                    // A signature written as HTML code is drawn laid out, where its text stands.
                    super::signature_card::place(st, ui, c, &mut ed);
                    rich_edit::body(ui, &t, &mut ed, c, focus_body.then_some(0), acts);
                    super::signature_card::show(s, st, ui, c, &mut ed, acts);
                    if ui.memory(|m| m.has_focus(rich_edit::body_id(c))) {
                        st.quote_focus = None;
                    }
                    // The original, as it came, under the text: changed where it stands.
                    super::quote::show(s, st, ui, c, &mut quote_eds, acts);
                });
            });
        });
    });
    st.editor = Some(ed);
    // Those of messages no longer being written go.
    quote_eds.retain(|(id, _), _| *id == taken.id || s.composes.iter().any(|c| c.id == *id));
    st.quote_editors = quote_eds;
    st.signature_cards.retain(|id, _| *id == taken.id || s.composes.iter().any(|c| c.id == *id));
    s.compose = Some(taken);
}

/// The other messages being written, each a click away.
fn also_being_written(ui: &mut Ui, t: &Tokens, s: &Session, acts: &mut Vec<Action>) {
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(tl!("Also being written:")).color(t.dim).small());
        for c in &s.composes {
            let subject = if c.subject.trim().is_empty() { tl!("Untitled").to_string() } else { c.subject.trim().to_string() };
            if ui.small_button(subject).clicked() {
                acts.push(Action::Run("compose.activate", json!({"id": c.id})));
            }
        }
    });
}

/// Send, or first ask about an empty subject. From an account that can't send, Send says why
/// (not first ask about the subject).
fn send(c: &mailcraft_engine::Compose, blocked: Option<SendBlock>, acts: &mut Vec<Action>) {
    if c.subject.trim().is_empty() && blocked.is_none() {
        acts.push(Action::Dialog(crate::Dialog::EmptySubject));
    } else {
        acts.push(Action::Run("compose.send", json!({})));
    }
}

/// Alt+S sends, as in Outlook; in a window of its own the main window's shortcuts don't reach
/// it, so Ctrl+Enter and Ctrl+S are the message's own there.
fn shortcuts(ui: &mut Ui, c: &mailcraft_engine::Compose, f: &Form, ctrl_enter: mailcraft_engine::CtrlEnter, acts: &mut Vec<Action>) {
    if c.sending {
        return;
    }
    let (alt_s, send_key, save_key) = ui.input_mut(|i| {
        (
            i.consume_key(egui::Modifiers::ALT, egui::Key::S),
            f.in_window && i.consume_key(egui::Modifiers::COMMAND, egui::Key::Enter),
            f.in_window && i.consume_key(egui::Modifiers::COMMAND, egui::Key::S),
        )
    });
    // Ctrl+Enter asks first, or does nothing, as set (issue #115); Alt+S sends as ever.
    if alt_s || (send_key && ctrl_enter == mailcraft_engine::CtrlEnter::Send) {
        send(c, f.blocked, acts);
    } else if send_key && ctrl_enter == mailcraft_engine::CtrlEnter::Ask {
        acts.push(Action::Dialog(crate::Dialog::CtrlEnter { remember: false }));
    }
    if save_key {
        acts.push(Action::Run("compose.saveDraft", json!({})));
    }
}

/// A picture on the clipboard is pasted as a picture. On the desktop egui takes Ctrl+V as a
/// paste and never passes the key on: a picture alone on the clipboard comes as `PasteImage`
/// (so Ctrl+V pasted nothing); in the browser the key itself comes.
fn paste_picture(ui: &mut Ui, c: &mailcraft_engine::Compose, acts: &mut Vec<Action>) {
    let body_id = rich_edit::body_id(c);
    if ui.memory(|m| m.has_focus(body_id))
        && ui.input(|i| {
            i.events.iter().any(|e| {
                matches!(e, egui::Event::PasteImage(_))
                    || matches!(e, egui::Event::Key { key: egui::Key::V, pressed: true, modifiers, .. } if modifiers.command)
            })
        })
    {
        acts.push(Action::PastePicture);
    }
}

/// Files dragged from the desktop onto the message are attached (read from where they are, as
/// Attach File reads them); while they're held over it, its edge says it takes them. In the
/// browser a dropped file has no path to read from, and nothing is attached.
fn attach_dropped(ui: &mut Ui, t: &Tokens, acts: &mut Vec<Action>) {
    let held_over = ui.ctx().input(|state| !state.raw.hovered_files.is_empty());
    let dropped = ui.ctx().input(|state| state.raw.dropped_files.clone());
    if held_over {
        ui.painter().rect_stroke(ui.max_rect().shrink(2.0), 6.0, egui::Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
    }
    for f in dropped.iter().take(100).filter(|f| f.path().is_absolute()) {
        acts.push(Action::Run("compose.attach", json!({"path": f.path().to_string_lossy()})));
    }
}
