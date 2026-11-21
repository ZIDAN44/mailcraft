//! File › Info: the account picker, then the account's cards: its settings, security, automatic
//! replies, its mailbox and data file, and its rules.

use mailcraft_engine::AccountState;

use super::*;

pub(super) fn info(s: &Session, t: &Tokens, has_files: bool, ui: &mut Ui, b: &mut Backstage, acts: &mut Vec<Action>) {
    heading(ui, t, "Account information");
    ui.set_max_width(ui.available_width().min(760.0));
    let Some(a) = b.account.as_deref().and_then(|id| s.account(id)).or_else(|| s.accounts.first()) else {
        ui.label(RichText::new(tl!("Add an email account")).color(t.dim));
        if ui.button(tl!("Add account…")).clicked() {
            acts.push(Action::Dialog(Dialog::add_account()));
        }
        return;
    };
    account_picker(s, t, ui, a, b, acts);
    ui.add_space(16.0);
    let id = &a.config.id;
    if !a.is_local() && b.asked.insert(id.clone()) {
        acts.push(Action::Run("account.vacation", json!({"account": id})));
    }
    account_settings(s, t, ui, a, acts);
    // S/MIME and OpenPGP, for accounts that send mail.
    if !a.is_data_file() {
        security::security(s, t, ui, a, acts);
        pgp::pgp(s, t, ui, a, acts);
        security::defaults(s, t, ui, a, acts);
    }
    automatic_replies(t, ui, a, acts);
    data_file::mailbox(s, t, has_files, ui, a, b, acts);
    rules(s, t, ui, acts);
}

/// The account picker. In a narrow window the list is as wide as the page and the button goes
/// under it (the two in a row pushed the page, and every card on it, past the window's edge).
fn account_picker(s: &Session, t: &Tokens, ui: &mut Ui, a: &AccountState, b: &mut Backstage, acts: &mut Vec<Action>) {
    let id = &a.config.id;
    ui.horizontal_wrapped(|ui| {
        let current = account_label(a);
        let width = (ui.available_width() - 2.0 * ui.spacing().button_padding.x).min(420.0);
        let r = egui::ComboBox::from_id_salt("backstage-account")
            .width(width)
            .selected_text(current)
            .truncate()
            .show_ui(ui, |ui| {
                for x in &s.accounts {
                    if ui.selectable_label(&x.config.id == id, account_label(x)).clicked() {
                        b.account = Some(x.config.id.clone());
                    }
                }
            })
            .response
            .accessible_name(tl!("Account"));
        let ids: Vec<&String> = s.accounts.iter().map(|x| &x.config.id).collect();
        let mut at = id;
        crate::drop_down::keys(ui, &r, &mut at, &ids);
        if at != id {
            b.account = Some(at.clone());
        }
        if widgets::pill_button(ui, Icon::Plus, "Add account…", t).clicked() {
            acts.push(Action::Dialog(Dialog::add_account()));
        }
    });
}

/// Where the account connects, in a line.
fn servers(a: &AccountState) -> String {
    if a.is_data_file() {
        tl!("Opened from an Outlook data file, kept on this computer only").to_string()
    } else if let Some(d) = a.config.dav.as_ref().filter(|_| a.config.is_calendar_only()) {
        let at = if d.caldav.is_empty() { &d.carddav } else { &d.caldav };
        tf!("Server: {url} (CalDAV and CardDAV)", url = readable_url(at))
    } else if a.is_local() {
        tl!("Sample mailbox, kept on this computer only").to_string()
    } else if let Some(u) = &a.config.graph {
        tf!("Server: {url} (Microsoft Graph)", url = u)
    } else if let Some(u) = &a.config.jmap {
        tf!("Server: {url} (JMAP)", url = u)
    } else if let Some(u) = &a.config.ews {
        tf!("Server: {url} (EWS)", url = u)
    } else if let Some(p) = &a.config.pop {
        tf!(
            "Servers: {pop} (POP {pop_port}), {smtp} (SMTP {smtp_port})",
            pop = p.host,
            pop_port = p.port,
            smtp = a.config.smtp.host,
            smtp_port = a.config.smtp.port
        )
    } else {
        tf!(
            "Servers: {imap} (IMAP {imap_port}), {smtp} (SMTP {smtp_port})",
            imap = a.config.imap.host,
            imap_port = a.config.imap.port,
            smtp = a.config.smtp.host,
            smtp_port = a.config.smtp.port
        )
    }
}

/// Account settings: where it connects, and every change to it, each with its own button.
fn account_settings(s: &Session, t: &Tokens, ui: &mut Ui, a: &AccountState, acts: &mut Vec<Action>) {
    let id = &a.config.id;
    let servers = servers(a);
    card(ui, t, tl!("Account settings"), Some(tl!("Change this account's name, servers and sign-in, or its signature.")), |ui| {
        setting_row(ui, t, true, name(t, &a.config.email), Some(note(t, &servers)), |ui| {
            if a.config.has_server() && ui.button(tl!(if a.config.oauth.is_some() { "Sign in again…" } else { "Password…" })).clicked() {
                acts.push(Action::SignInAgain(id.clone()));
            }
            if !a.is_local() && a.config.graph.is_none() && ui.button(tl!("Server settings…")).clicked() {
                acts.push(Action::Dialog(Dialog::edit_account(&a.config)));
            }
        });
        // Its other addresses (issue #81): the same mailbox's, to send from. Not a data file's,
        // which sends nothing.
        if !a.is_data_file() {
            other_addresses(t, ui, a, acts);
        }
        if a.config.ews.is_some() {
            shared_mailboxes(t, ui, a, acts);
        }
        // Calendars and contacts: where they were found, or the addresses given by hand (the
        // desktop app's: the browser doesn't sync calendars).
        if !cfg!(target_arch = "wasm32") && a.config.has_server() && a.config.graph.is_none() && a.config.ews.is_none() {
            calendars(s, t, ui, a, acts);
        }
        setting_row(ui, t, false, name(t, tl!("Signature")), None, |ui| {
            if ui.button(tl!("Signature…")).clicked() {
                acts.push(Action::EditSignature { account: id.clone(), insert: false });
            }
        });
        setting_row(ui, t, false, name(t, tl!("Remove account")), None, |ui| {
            if ui.button(RichText::new(tl!("Remove account…")).color(t.danger)).clicked() {
                acts.push(Action::Dialog(Dialog::RemoveAccount { account: id.clone(), email: a.config.email.clone() }));
            }
        });
    });
}

/// The same mailbox's other addresses, to send from: found on a JMAP server, added, edited,
/// removed.
fn other_addresses(t: &Tokens, ui: &mut Ui, a: &AccountState, acts: &mut Vec<Action>) {
    let id = &a.config.id;
    let others = &a.config.identities;
    let about = if others.is_empty() {
        tl!("Addresses of this same mailbox that you also send from.").to_string()
    } else {
        // The one new mail is written from says so (issue #88).
        let named = |i: &mailcraft_engine::model::Identity| if i.new_mail { tf!("{address} (new mail)", address = i.email) } else { i.email.clone() };
        others.iter().map(named).collect::<Vec<_>>().join(", ")
    };
    setting_row(ui, t, false, name(t, tl!("Other addresses")), Some(note(t, &about)), |ui| {
        // A JMAP server keeps them: asked for, they're added.
        if a.config.jmap.is_some() && ui.button(tl!("Find on the server")).clicked() {
            acts.push(Action::Run("account.findAddresses", json!({"account": id})));
        }
        if ui.button(tl!("Add…")).clicked() {
            acts.push(Action::Dialog(Dialog::Address(crate::dialogs::AddressForm::new(id))));
        }
        if !others.is_empty() {
            ui.menu_button(tl!("Edit"), |ui| {
                for i in others {
                    if ui.button(i.email.as_str()).clicked() {
                        acts.push(Action::Dialog(Dialog::Address(crate::dialogs::AddressForm::of(id, i))));
                        ui.close();
                    }
                }
            });
            ui.menu_button(tl!("Remove"), |ui| {
                for i in others {
                    if ui.button(i.email.as_str()).clicked() {
                        acts.push(Action::Run("account.removeAddress", json!({"account": id, "address": i.email})));
                        ui.close();
                    }
                }
            });
        }
    });
}

/// An Exchange account's shared mailboxes, opened by themselves: added, removed.
fn shared_mailboxes(t: &Tokens, ui: &mut Ui, a: &AccountState, acts: &mut Vec<Action>) {
    let id = &a.config.id;
    let added = &a.config.shared_mailboxes;
    let about = if added.is_empty() { tl!("Mailboxes you have full access to open by themselves.").to_string() } else { added.join(", ") };
    setting_row(ui, t, false, name(t, tl!("Shared mailboxes")), Some(note(t, &about)), |ui| {
        if ui.button(tl!("Add…")).clicked() {
            acts.push(Action::Dialog(Dialog::AddSharedMailbox { account: id.clone(), address: String::new(), error: None }));
        }
        if !added.is_empty() {
            ui.menu_button(tl!("Remove"), |ui| {
                for m in added {
                    if ui.button(m).clicked() {
                        acts.push(Action::Run("account.removeSharedMailbox", json!({"account": id, "address": m})));
                        ui.close();
                    }
                }
            });
        }
    });
}

/// Where the account's calendars and contacts were found, or that they weren't; the calendar
/// server's address given by hand.
fn calendars(s: &Session, t: &Tokens, ui: &mut Ui, a: &AccountState, acts: &mut Vec<Action>) {
    let pim = s.pim.get(&a.config.id);
    let found = pim.and_then(|p| p.caldav_home.clone());
    let about = match found {
        Some(h) if !h.is_empty() => tf!("Calendars at {url}", url = readable_url(&h)),
        Some(_) if a.config.dav.as_ref().is_some_and(|d| !d.caldav.is_empty()) => {
            tl!("No calendars were found at the address given: check it, and its sign-in").to_string()
        }
        Some(_) => tl!("Your provider's calendar server wasn't found: give its address").to_string(),
        None if a.config.oauth.is_some() && a.config.dav.is_none() => {
            tl!("Calendars come from a server of their own: give its address and sign-in").to_string()
        }
        None => tl!("Looked for when the account syncs").to_string(),
    };
    setting_row(ui, t, false, name(t, tl!("Calendars and contacts")), Some(note(t, &about)), |ui| {
        if ui.button(tl!("Calendar server…")).clicked() {
            acts.push(Action::Dialog(Dialog::Dav(crate::dialogs::DavForm::of(&a.config))));
        }
    });
}

/// Automatic replies: on or off, and a button to change them where the server has them.
fn automatic_replies(t: &Tokens, ui: &mut Ui, a: &AccountState, acts: &mut Vec<Action>) {
    let (color, status) = match &a.vacation {
        _ if a.is_data_file() => (t.faint, tl!("Not available: a data file has no server.").to_string()),
        _ if a.config.is_calendar_only() => (t.faint, tl!("Not available: this account has no mail server.").to_string()),
        _ if a.is_local() => (t.faint, tl!("Not available: this mailbox has no server.").to_string()),
        VacationState::Unknown | VacationState::Loading => (t.faint, tl!("Checking with the server…").to_string()),
        VacationState::Loaded(v) if v.enabled => (
            t.ok,
            match (&v.from, &v.until) {
                (Some(f), Some(u)) => tf!("On from {from} until {until}", from = f, until = u),
                (Some(f), None) => tf!("On from {from}", from = f),
                (None, Some(u)) => tf!("On until {until}", until = u),
                (None, None) => tl!("On").to_string(),
            },
        ),
        VacationState::Loaded(_) => (t.dim, tl!("Off").to_string()),
        VacationState::Failed(e) => (t.warn, tf!("Not available: {error}", error = tl!(e))),
    };
    let can_change = !a.is_local() && !matches!(a.vacation, VacationState::Failed(_));
    card(
        ui,
        t,
        tl!("Automatic replies"),
        Some(tl!("Tell people you're away: each sender gets one reply while they're on, even when this computer is off.")),
        |ui| {
            setting_row(ui, t, true, RichText::new(&status).color(color), None, |ui| {
                if ui.add_enabled(can_change, egui::Button::new(tl!("Change…"))).clicked() {
                    let current = match &a.vacation {
                        VacationState::Loaded(v) => v.clone(),
                        _ => Default::default(),
                    };
                    // Exchange keeps one reply for inside the organisation and one for outside.
                    let exchange = a.config.ews.is_some() || a.config.graph.is_some();
                    acts.push(Action::Dialog(Dialog::vacation(&a.config.id, &current, exchange)));
                }
                if matches!(a.vacation, VacationState::Loading) {
                    ui.spinner();
                }
            });
        },
    );
}

/// The rules: how many, and Manage rules.
fn rules(s: &Session, t: &Tokens, ui: &mut Ui, acts: &mut Vec<Action>) {
    let n = s.rules.len();
    card(ui, t, tl!("Rules"), Some(tl!("Sort mail as it arrives, or as you send it: move it, flag it, categorize it.")), |ui| {
        let count = if n == 0 { tl!("No rules yet.").to_string() } else { tf!("Rules: {n}", n = n) };
        setting_row(ui, t, true, name(t, &count), None, |ui| {
            if ui.button(tl!("Manage rules")).clicked() {
                acts.push(Action::Dialog(Dialog::Rules));
            }
        });
    });
}
