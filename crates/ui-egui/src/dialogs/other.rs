//! Dialogs: previews, calendars and people.

use super::*;

pub(super) fn draw(ui: &mut Ui, d: &mut Dialog, s: &Session, pics: &mut crate::reading::Pictures, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    match d {
        Dialog::ConfirmDeleteContact { uid, name } => {
            ui.set_width(crate::sizes::DIALOG_S);
            if name.trim().is_empty() {
                title(ui, t, "Delete this contact?");
            } else {
                title(ui, t, &tf!("Delete “{name}”?", name = name));
            }
            ui.label(RichText::new(tl!("This contact will be deleted, along with everything kept about them. This can't be undone.")).color(t.dim));
            let (ok, cancel) = danger_buttons(ui, t, "Delete", true);
            if ok {
                acts.push(Action::Run("contact.delete", json!({"uid": uid})));
                return false;
            }
            !cancel
        }
        Dialog::PicturePreview { key, index, name } => {
            let w = (ui.ctx().content_rect().width() * 0.75).clamp(360.0, 1200.0);
            let h = (ui.ctx().content_rect().height() * 0.7).max(240.0);
            ui.set_width(w);
            title(ui, t, name);
            match pics.get(key, *index, crate::reading::Want::Big) {
                crate::reading::Got::Ready(tex) => {
                    let sz = tex.size_vec2();
                    let scale = (w / sz.x).min(h / sz.y).min(1.0);
                    ui.vertical_centered(|ui| ui.add(egui::Image::new(&tex).fit_to_exact_size(sz * scale)));
                }
                crate::reading::Got::Pending => {
                    ui.vertical_centered(|ui| ui.spinner());
                }
                crate::reading::Got::Failed => {
                    ui.label(RichText::new(tl!("This picture can't be shown.")).color(t.dim));
                }
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button(tl!("Open")).clicked() {
                    acts.push(Action::InWindow(key.clone(), Box::new(Action::OpenAttachment(*index))));
                }
                close_row(ui)
            })
            .inner
        }
        Dialog::Preview { title: name, header, text, warning, open } => {
            // A viewer, as wide as it reads well (900 was wider than any dialog).
            let w = (ui.ctx().content_rect().width() * 0.7).clamp(360.0, crate::sizes::VIEWER_MAX);
            ui.set_width(w);
            title(ui, t, name);
            if let Some(warning) = warning {
                crate::reading::sender_warning(t, ui, warning);
            }
            // A mail to hundreds of people scrolls: Close stays on the window.
            let tall = ui.ctx().content_rect().height();
            egui::ScrollArea::vertical().id_salt("preview-header").max_height((tall * 0.2).max(90.0)).auto_shrink([false, true]).show(ui, |ui| {
                for (k, v) in header.iter() {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(k.as_str()).color(t.dim).small());
                        ui.label(RichText::new(v.as_str()).small());
                    });
                }
            });
            ui.add_space(6.0);
            let h = (tall * 0.55).max(200.0);
            // An attached message reads in the body's font; a file (code, a table) keeps its
            // columns in monospace.
            let font = if header.is_empty() { egui::FontId::monospace(13.0) } else { theme::regular(crate::sizes::BODY) };
            egui::ScrollArea::both().max_height(h).auto_shrink([false, true]).show(ui, |ui| {
                let mut shown = text.as_str();
                ui.add(egui::TextEdit::multiline(&mut shown).font(font).desired_width(f32::INFINITY).frame(egui::Frame::NONE));
            });
            ui.add_space(6.0);
            match open {
                Some((key, index)) => {
                    ui.horizontal(|ui| {
                        if ui.button(tl!("Open")).clicked() {
                            acts.push(Action::InWindow(key.clone(), Box::new(Action::OpenAttachment(*index))));
                        }
                        close_row(ui)
                    })
                    .inner
                }
                None => close_row(ui),
            }
        }
        Dialog::Certificate { account, path, data, password } => {
            ui.set_width(crate::sizes::DIALOG_M);
            let unlock = path.is_empty() && data.is_none();
            title(ui, t, if unlock { "Unlock certificate" } else { "Import certificate" });
            let file = mailcraft_engine::model::file_name(path);
            // Kept unlocked between visits (gap 7): the password is kept, sealed, after all.
            let kept = s.jmap_only && s.smime_settings(account).keep_unlocked;
            let ask = if unlock && kept {
                tl!("Your certificate's password, to keep it unlocked in this browser:").to_string()
            } else if unlock {
                tl!("Your certificate's password, to use it in this browser until the page is closed:").to_string()
            } else {
                tf!("The password {file} was saved with:", file = file)
            };
            ui.add(egui::Label::new(RichText::new(ask).color(t.dim)).wrap());
            ui.add_space(8.0);
            let r = widgets::password_field(ui, password, "");
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if ui.memory(|m| m.focused().is_none()) {
                r.request_focus();
            }
            ui.add_space(4.0);
            ui.add(
                egui::Label::new(
                    RichText::new(if cfg!(target_arch = "wasm32") && kept {
                        tl!(
                            "The browser keeps the file, locked with its password, and the password sealed with a key that never leaves this browser."
                        )
                    } else if cfg!(target_arch = "wasm32") {
                        tl!("The browser keeps the file, locked with its password; the password itself isn't kept.")
                    } else {
                        tl!("Its private key is kept in the system's keychain, not in MailCraft's files.")
                    })
                    .color(t.faint)
                    .small(),
                )
                .wrap(),
            );
            let (ok, cancel) = buttons(ui, t, if unlock { "Unlock" } else { "Import" }, true);
            if ok || enter {
                let params = match data {
                    Some(d) => json!({"account": account, "data": d, "password": password}),
                    None if unlock => json!({"account": account, "kept": true, "password": password}),
                    None => json!({"account": account, "path": path, "password": password}),
                };
                acts.push(Action::Run("security.import", params));
                return false;
            }
            !cancel
        }
        Dialog::PgpKey { account, path, passphrase } => {
            ui.set_width(crate::sizes::DIALOG_M);
            title(ui, t, "Import OpenPGP key");
            let file = mailcraft_engine::model::file_name(path);
            label_over(ui, t, "Passphrase");
            let r = widgets::password_field(ui, passphrase, "");
            let help = tf!("Leave it empty if {file} has none.", file = file);
            ui.add(egui::Label::new(RichText::new(help).color(t.faint).small()).wrap());
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if ui.memory(|m| m.focused().is_none()) {
                r.request_focus();
            }
            ui.add_space(4.0);
            ui.add(
                egui::Label::new(
                    RichText::new(tl!("Its secret part is kept in the system's keychain, not in MailCraft's files; the passphrase isn't kept."))
                        .color(t.faint)
                        .small(),
                )
                .wrap(),
            );
            let (ok, cancel) = buttons(ui, t, "Import", true);
            if ok || enter {
                acts.push(Action::Run("pgp.import", json!({"account": account, "path": path, "passphrase": passphrase})));
                return false;
            }
            !cancel
        }
        Dialog::SmartCard { account, module, pin, system } => smart_card(ui, s, t, acts, account, module, pin, *system),
        Dialog::OpenSharedCalendar { email, error } => {
            ui.set_width(crate::sizes::DIALOG_M);
            title(ui, t, "Open a shared calendar");
            ui.add(
                egui::Label::new(
                    RichText::new(tl!("Whose calendar? It opens read-only beside yours, when they've shared it with you in Exchange.")).color(t.dim),
                )
                .wrap(),
            );
            ui.add_space(8.0);
            let r = ui.add(egui::TextEdit::singleline(email).hint_text("name@example.com").desired_width(f32::INFINITY));
            // Before the focus request below, which would hide the Enter that ended input.
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if ui.memory(|m| m.focused().is_none()) {
                r.request_focus();
            }
            if let Some(e) = error {
                ui.add_space(4.0);
                field_error(ui, t, e);
            }
            // Outlook's From Address Book: picked from the address book, the organisation's
            // directory among them (issue #140).
            if ui.link(RichText::new(tl!("Pick from the address book…")).small()).clicked() {
                acts.push(Action::Dialog(Dialog::names_for(NamesFor::Calendars, email)));
            }
            let ready = email.contains('@');
            let (ok, cancel) = buttons(ui, t, "Open", ready);
            if (ok || enter) && ready {
                acts.push(Action::Submit("calendar.openShared", json!({"email": email.trim()})));
            }
            !cancel
        }
        Dialog::SubscribeCalendar { url, name, error } => {
            ui.set_width(crate::sizes::DIALOG_M);
            title(ui, t, "Subscribe to a calendar");
            ui.add(
                egui::Label::new(
                    RichText::new(tl!(
                        "Paste the calendar's link (iCal or webcal://). It shows beside yours, read-only, and is read again every few hours."
                    ))
                    .color(t.dim),
                )
                .wrap(),
            );
            ui.add_space(8.0);
            let r = ui.add(egui::TextEdit::singleline(url).hint_text("webcal://example.com/calendar.ics").desired_width(f32::INFINITY));
            // Before the focus request below, which would hide the Enter that ended input.
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if ui.memory(|m| m.focused().is_none()) {
                r.request_focus();
            }
            ui.add_space(6.0);
            ui.label(RichText::new(tl!("Name (optional)")).color(t.dim).small());
            ui.add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY));
            if let Some(e) = error {
                ui.add_space(4.0);
                field_error(ui, t, e);
            }
            let ready = mailcraft_engine::model::pim::calendar_link(url).is_some();
            let (ok, cancel) = buttons(ui, t, "Subscribe", ready);
            if (ok || enter) && ready {
                acts.push(Action::Submit("calendar.subscribe", json!({"url": url.trim(), "name": name.trim()})));
            }
            !cancel
        }
        Dialog::AddRoom { name, url, code, details, error } => {
            ui.set_width(crate::sizes::DIALOG_M);
            title(ui, t, "Add a meeting room");
            label_over(ui, t, "Link");
            let r = ui.add(egui::TextEdit::singleline(url).hint_text("https://meet.example.org/rooms/abc-def-ghi").desired_width(f32::INFINITY));
            if ui.memory(|m| m.focused().is_none()) {
                r.request_focus();
            }
            ui.add_space(6.0);
            label_over(ui, t, "Name");
            ui.add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY));
            ui.add_space(6.0);
            label_over(ui, t, "Access code (optional)");
            ui.add(egui::TextEdit::singleline(code).desired_width(200.0));
            ui.add_space(6.0);
            label_over(ui, t, "Dial-in numbers and other lines for the invitation (optional)");
            ui.add(egui::TextEdit::multiline(details).desired_rows(3).desired_width(f32::INFINITY));
            // What's wrong with the link, under it as it's typed (it was a notice in the corner).
            let link = mailcraft_engine::model::ical::meeting_url(url);
            if !url.trim().is_empty() && link.is_none() {
                field_error(ui, t, tl!("That isn't a meeting link: paste the room's https:// address."));
            }
            if let Some(e) = error {
                field_error(ui, t, e);
            }
            let (ok, cancel) = buttons(ui, t, "Add room", link.is_some());
            if ok {
                acts.push(Action::Submit(
                    "meetings.addRoom",
                    json!({"name": name.trim(), "url": url.trim(), "code": code.trim(), "details": details.trim()}),
                ));
            }
            !cancel
        }
        Dialog::Event(f) => crate::pim_dialogs::event(ui, f, s, t, acts),
        Dialog::Task(f) => crate::pim_dialogs::task(ui, f, t, acts),
        Dialog::NewTaskList { name, error } => {
            ui.set_width(crate::sizes::DIALOG_S);
            title(ui, t, "New task list");
            label_over(ui, t, "Task list name");
            let r = ui.add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY));
            if ui.memory(|m| m.focused().is_none()) {
                r.request_focus();
            }
            // One field: Enter means OK wherever the focus is.
            let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
            if let Some(e) = error {
                field_error(ui, t, e);
            }
            let ready = !name.trim().is_empty();
            let (ok, cancel) = buttons(ui, t, "Create", ready);
            if (ok || enter) && ready {
                *error = None;
                acts.push(Action::Submit("task.newList", json!({"name": name.trim()})));
            }
            !cancel
        }
        Dialog::FollowUp(f) => crate::pim_dialogs::follow_up(ui, f, t, acts),
        Dialog::Snooze(f) => crate::pim_dialogs::snooze(ui, f, s, t, acts),
        Dialog::Propose(f) => crate::pim_dialogs::propose(ui, f, t, acts),
        Dialog::WaitingInvites(f) => crate::pim_dialogs::waiting(ui, f, s, t, acts),
        // An occurrence of a repeating event (issue #241): it alone, or the series.
        Dialog::OpenRecurring { uid, occurrence, summary } => {
            ui.set_width(crate::sizes::DIALOG_M);
            title(ui, t, "Open a repeating event");
            ui.add(egui::Label::new(RichText::new(summary.as_str()).color(t.dim)).truncate());
            ui.add_space(4.0);
            ui.label(RichText::new(crate::i18n::date(&mailcraft_engine::model::dates::when_text(*occurrence, &s.clock))).color(t.dim));
            ui.add_space(12.0);
            let mut keep = true;
            ui.horizontal(|ui| {
                if widgets::accent_button(ui, crate::icons::Icon::Calendar, "Open this occurrence", t, true).clicked() {
                    if let Some(d) = Dialog::open_occurrence(s, uid, *occurrence) {
                        acts.push(Action::Dialog(d));
                    }
                    keep = false;
                }
                if ui.button(tl!("Open the series")).clicked() {
                    if let Some(d) = Dialog::open_event(s, uid, *occurrence) {
                        acts.push(Action::Dialog(d));
                    }
                    keep = false;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::secondary(ui, "Cancel").clicked() {
                        keep = false;
                    }
                });
            });
            keep
        }
        // Forward or delegate a meeting (issues #234, #254): to whom.
        Dialog::PassMeeting(f) => crate::pim_dialogs::pass(ui, f, t, acts),
        Dialog::CancelMeeting(f) => crate::pim_dialogs::cancel(ui, f, t, acts),
        Dialog::Contact(f) => crate::pim_dialogs::contact(ui, f, t, acts),
        // Every other kind is shown by its own group (`body`).
        _ => true,
    }
}

/// A certificate on a smart card: the card's module and PIN, the certificates it has, one chosen.
fn smart_card(
    ui: &mut Ui,
    s: &Session,
    t: &Tokens,
    acts: &mut Vec<Action>,
    account: &str,
    module: &mut String,
    pin: &mut String,
    system: bool,
) -> bool {
    ui.set_width(crate::sizes::DIALOG_M);
    if system {
        title(ui, t, if cfg!(windows) { "Use a certificate from Windows" } else { "Use a certificate from the Keychain" });
        ui.add(
            egui::Label::new(
                RichText::new(if cfg!(windows) {
                    tl!("The certificates for email in your Windows certificate store (Personal). The key stays in Windows; it may ask before it's used.")
                } else {
                    tl!("The certificates for email in your Keychain. The key stays there; macOS may ask before it's used.")
                })
                .color(t.dim),
            )
            .wrap(),
        );
        let chosen = held_keys(ui, s, t, acts, account, true);
        return !chosen && close_row(ui);
    }
    // Each label over its box, one style (a sentence ending in a colon, then a grey line), the
    // help under them; Find certificates the dialog's main button (it had none).
    title(ui, t, "Use a smart card");
    label_over(ui, t, "Card module");
    ui.add(egui::TextEdit::singleline(module).hint_text("opensc-pkcs11").desired_width(f32::INFINITY));
    ui.add(egui::Label::new(RichText::new(tl!("The one your card came with. OpenSC's works with most cards.")).color(t.faint).small()).wrap());
    ui.add_space(8.0);
    label_over(ui, t, "Card PIN");
    let r = widgets::password_field(ui, pin, "");
    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    let search = s.card_search();
    // The system's own store: another place to look.
    let elsewhere = if cfg!(target_os = "macos") {
        Some(tl!("Look in the Keychain"))
    } else if cfg!(windows) {
        Some(tl!("Look in Windows"))
    } else {
        None
    };
    if let Some(label) = elsewhere {
        ui.add_space(6.0);
        if ui.add_enabled(!search.searching, egui::Button::new(label)).clicked() {
            acts.push(Action::Run("security.findKeys", json!({"keychain": true})));
        }
    }
    let chosen = held_keys(ui, s, t, acts, account, false);
    let ready = !module.trim().is_empty() && !search.searching;
    let (find, cancel) = buttons(ui, t, "Find certificates", ready);
    if find || (enter && ready) {
        acts.push(Action::Run("security.findKeys", json!({"module": module.trim(), "pin": pin})));
    }
    !chosen && !cancel
}

/// What the last look found, each with Use (whether one was chosen); and where the key stays.
/// Nothing found is said as such, with what to do instead (the system's store: `system`), not as
/// an error.
fn held_keys(ui: &mut Ui, s: &Session, t: &Tokens, acts: &mut Vec<Action>, account: &str, system: bool) -> bool {
    let search = s.card_search();
    if search.searching {
        ui.add_space(6.0);
        ui.label(RichText::new(tl!("Looking…")).color(t.faint).small());
    }
    if search.error.as_deref() == Some(mailcraft_engine::NO_HELD_KEYS) {
        ui.add_space(10.0);
        let (none, next) = match (system, cfg!(windows)) {
            (true, true) => (
                tl!("No email certificates were found in Windows' Personal store."),
                tl!(
                    "Install the one your organisation or certificate authority gave you in Windows, or import its file under Security in File › Options."
                ),
            ),
            (true, false) => (
                tl!("No email certificates were found in the Keychain."),
                tl!(
                    "Add the one your organisation or certificate authority gave you to the Keychain, or import its file under Security in File › Options."
                ),
            ),
            (false, _) => (
                tl!("No certificate with its key was found on the card."),
                tl!("Check the card is in its reader and the module is the one it came with."),
            ),
        };
        ui.label(RichText::new(none).color(t.text));
        ui.add(egui::Label::new(RichText::new(next).color(t.dim).small()).wrap());
    } else if let Some(e) = &search.error {
        ui.add_space(6.0);
        ui.add(egui::Label::new(RichText::new(tl!(e)).color(t.danger).small()).wrap());
    }
    let mut chosen = false;
    for (i, k) in search.keys.iter().enumerate() {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new(&k.label).color(t.text));
                if let Some(info) = search.info(i) {
                    let who = info.emails.first().unwrap_or(&info.name);
                    ui.label(RichText::new(format!("{who} · {} · {}", info.issuer, info.key)).color(t.faint).small());
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(tl!("Use")).clicked() {
                    acts.push(Action::Run("security.useKey", json!({"account": account, "index": i})));
                    chosen = true;
                }
            });
        });
    }
    ui.add_space(4.0);
    if search.pin_given() {
        ui.add(
            egui::Label::new(RichText::new(tl!("The key stays on the card; its PIN is kept in the system's keychain.")).color(t.faint).small())
                .wrap(),
        );
    }
    chosen
}
