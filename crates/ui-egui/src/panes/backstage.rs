//! The File page: everything about accounts and the app that isn't about one message. It
//! replaces the ribbon and panes until the user goes back: Info (the account's settings, its
//! automatic replies, the space it uses and its rules), Open & Export (Outlook data files),
//! Options and About.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use egui::{Margin, RichText, Sense, Ui, vec2};
use mailcraft_engine::{AccountState, ReadingPane, Session, VacationState};
use serde_json::json;

use crate::icons::{self, Icon};
use crate::theme::{self, Tokens};
use crate::{Action, Dialog, UiState, UpdateState, widgets};

mod backup;
mod data_file;
#[cfg(test)]
pub(crate) use data_file::report_line;
mod meetings;
mod options;
mod pgp;
mod security;
#[cfg(test)]
mod tests;
use options::options;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Info,
    Open,
    Options,
    About,
}

#[derive(Clone, Debug)]
pub struct Backstage {
    pub page: Page,
    /// The account Info shows (the first one until another is picked).
    pub account: Option<String>,
    /// Accounts whose automatic replies were asked for since the page opened.
    asked: HashSet<String>,
    /// Bytes cached per account, measured once per visit (and again once a compaction ends).
    cached: HashMap<String, (u64, Option<data_file::Stamp>)>,
    /// Accounts whose data file was measured since the page opened.
    measured: HashSet<String>,
    /// The account Export writes (the one being looked at until another is picked).
    export: Option<String>,
}

impl Backstage {
    pub fn new(page: Page) -> Backstage {
        Backstage { page, account: None, asked: HashSet::new(), cached: HashMap::new(), measured: HashSet::new(), export: None }
    }
}

/// What kind of account it is, for the picker.
fn kind(a: &AccountState) -> &'static str {
    if a.is_data_file() {
        "Outlook data file"
    } else if a.config.is_calendar_only() {
        "Calendars and contacts"
    } else if a.is_local() {
        "Sample mailbox"
    } else if a.config.graph.is_some() {
        "Exchange (Microsoft Graph)"
    } else if a.config.ews.is_some() {
        "Exchange server (EWS)"
    } else if a.config.jmap.is_some() {
        "JMAP"
    } else if a.config.pop.is_some() {
        "POP and SMTP"
    } else {
        match a.config.oauth.as_deref() {
            Some("google") => "Google (IMAP)",
            Some("microsoft") => "Microsoft 365 (IMAP)",
            _ => "IMAP and SMTP",
        }
    }
}

pub fn show(s: &Session, st: &mut UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    let t = st.tokens;
    let Some(mut b) = st.backstage.take() else { return };
    let mut close = false;
    // As wide as its longest name in the language shown (bold, as the page open is), within
    // reason: "Enregistrer les pièces jointes" was cut at the rail's edge.
    let longest = ["Back", "Info", "Open & Export", "Save As", "Save Attachments", "Print", "Options", "About", "Exit"]
        .iter()
        .map(|l| ui.painter().layout_no_wrap(tl!(*l).to_string(), theme::bold(14.0), t.text).size().x)
        .fold(0.0, f32::max);
    let rail = (longest + 46.0 + 14.0).clamp(200.0, 280.0);
    egui::Panel::left("backstage-rail").exact_size(rail).frame(egui::Frame::new().fill(t.chrome).inner_margin(Margin::symmetric(0, 10))).show(
        ui,
        |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            let r = rail_item(ui, &t, Icon::Back, "Back", false);
            if r.clicked() {
                close = true;
            }
            ui.add_space(8.0);
            for (page, icon, label) in [(Page::Info, Icon::Info, "Info"), (Page::Open, Icon::Folder, "Open & Export")] {
                if rail_item(ui, &t, icon, label, b.page == page).clicked() {
                    b.page = page;
                }
            }
            // The open message, as Outlook's File › Save As, Save Attachments and Print.
            ui.add_space(8.0);
            let open = s.reading.as_ref().and_then(|r| r.parsed.as_ref());
            let attachments = open.is_some_and(|p| !p.attachments.is_empty());
            for (icon, label, on, action) in [
                (Icon::File, "Save As", open.is_some(), Action::SaveMessage),
                (Icon::Attach, "Save Attachments", attachments, Action::SaveAll),
                (Icon::Print, "Print", open.is_some(), Action::Print),
            ] {
                let r = ui.add_enabled_ui(on, |ui| rail_item(ui, &t, icon, label, false)).inner;
                if on && r.clicked() {
                    acts.push(action);
                    close = true;
                }
            }
            ui.add_space(8.0);
            for (page, icon, label) in [(Page::Options, Icon::Gear, "Options"), (Page::About, Icon::Star, "About")] {
                if rail_item(ui, &t, icon, label, b.page == page).clicked() {
                    b.page = page;
                }
            }
            // Close MailCraft (the desktop app; a browser tab is closed by the browser).
            if st.has_files && rail_item(ui, &t, Icon::Close, "Exit", false).clicked() {
                // Exit ends MailCraft, where closing the window only hides it (issue #67).
                st.quit_wanted = true;
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
        },
    );
    egui::CentralPanel::default().frame(egui::Frame::new().fill(t.bg)).show(ui, |ui| {
        egui::ScrollArea::vertical().auto_shrink(false).id_salt("backstage").show(ui, |ui| {
            egui::Frame::new().inner_margin(Margin { left: 36, right: 36, top: 24, bottom: 32 }).show(ui, |ui| {
                // At most this wide, and never wider than the window (`set_max_width` alone widens
                // the page past a narrow window's edge, where it was cut off rather than wrapped).
                ui.set_max_width(ui.available_width().min(820.0));
                match b.page {
                    Page::Info => info(s, &t, st.has_files, ui, &mut b, acts),
                    Page::Open => open(s, &t, ui, &mut b, acts),
                    Page::Options => options(s, st, ui, acts),
                    Page::About => about(s, &t, &st.update, ui, acts),
                }
            });
        });
    });
    if !close {
        st.backstage = Some(b);
    }
}

fn rail_item(ui: &mut Ui, t: &Tokens, icon: Icon, label: &str, on: bool) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 38.0), Sense::click());
    crate::widgets::named(ui, &resp, tl!(label), true, Some(on));
    if on || resp.hovered() {
        ui.painter().rect_filled(r, 0.0, if on { t.accent_soft } else { t.hover_row });
    }
    if on {
        ui.painter().rect_filled(egui::Rect::from_min_size(r.min, vec2(3.0, r.height())), 0.0, t.accent);
    }
    let ir = egui::Rect::from_center_size(r.left_center() + vec2(26.0, 0.0), vec2(18.0, 18.0));
    let enabled = ui.is_enabled();
    icons::paint(
        ui.painter(),
        ir,
        icon,
        if on {
            t.accent
        } else if enabled {
            t.dim
        } else {
            t.faint
        },
    );
    ui.painter().text(
        r.left_center() + vec2(46.0, 0.0),
        egui::Align2::LEFT_CENTER,
        tl!(label),
        if on { theme::bold(14.0) } else { theme::regular(14.0) },
        if enabled { t.text } else { t.faint },
    );
    resp
}

fn heading(ui: &mut Ui, t: &Tokens, text: &str) {
    ui.label(RichText::new(tl!(text)).font(theme::bold(22.0)).color(t.text));
    ui.add_space(14.0);
}

/// A card on a File page: its title and a line about it, then its rows.
fn card(ui: &mut Ui, t: &Tokens, title: &str, about: Option<&str>, body: impl FnOnce(&mut Ui)) {
    egui::Frame::new()
        .fill(t.surface)
        .stroke(egui::Stroke::new(1.0, t.border))
        .corner_radius(10)
        .inner_margin(Margin { left: 20, right: 20, top: 16, bottom: 10 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title).font(theme::bold(15.0)).color(t.text));
            if let Some(about) = about {
                ui.add_space(2.0);
                ui.label(RichText::new(about).color(t.dim).size(12.5));
            }
            ui.add_space(6.0);
            body(ui);
        });
    ui.add_space(14.0);
}

/// One row of a card: its name (and a line under it) at the start, its controls against the
/// end, centred on each other; a thin line above every row but the first. The text takes what
/// the controls leave; when that's too little (a narrow window, wide controls), the controls go
/// under the text instead, so the two never overlap.
fn setting_row(ui: &mut Ui, t: &Tokens, first: bool, label: RichText, about: Option<RichText>, control: impl FnOnce(&mut Ui)) {
    let _ = setting_row_at(ui, t, first, label, about, control);
}

/// [`setting_row`], saying where the text and the controls went.
fn setting_row_at(
    ui: &mut Ui,
    t: &Tokens,
    first: bool,
    label: RichText,
    about: Option<RichText>,
    control: impl FnOnce(&mut Ui),
) -> (egui::Rect, egui::Rect) {
    if !first {
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
        ui.painter().rect_filled(r, 0.0, t.border.gamma_multiply(0.6));
    }
    ui.add_space(7.0);
    // How wide the controls and how tall the text were last time (they're laid out after the
    // decision to stack, which they decide).
    let id = ui.scope_id().with(("setting-row", label.text().to_string()));
    let (controls_w, text_h): (f32, f32) = ui.data(|d| d.get_temp(id)).unwrap_or((300.0, 0.0));
    let full = ui.available_width();
    let gap = 24.0;
    let text = |ui: &mut Ui, label: RichText, about: Option<RichText>| {
        ui.spacing_mut().item_spacing.y = 2.0;
        match about {
            Some(about) => {
                ui.add(egui::Label::new(label).wrap());
                ui.add(egui::Label::new(about).wrap());
            }
            // One line, cut short before the controls (all of it on hover).
            None => {
                ui.add(egui::Label::new(label).truncate());
            }
        }
    };
    if full - controls_w - gap < SETTING_TEXT_MIN {
        let left = ui.scope_builder(egui::UiBuilder::new().layout(crate::bidi::column()), |ui| text(ui, label, about)).response.rect;
        ui.add_space(8.0);
        // Against the end, in the order they have beside the text.
        let row = ui.scope_builder(egui::UiBuilder::new().layout(crate::bidi::row_from_end(egui::Align::Min).with_main_wrap(true)), control);
        let w = row.response.rect.width();
        remember(ui, id, (controls_w, text_h), (if w + gap + SETTING_TEXT_MIN < full { w } else { controls_w.max(w) }, left.height()));
        ui.add_space(7.0);
        (left, row.response.rect)
    } else {
        let height = text_h.max(if about.is_some() { 38.0 } else { 28.0 });
        let top = ui.cursor().min;
        let rect = egui::Rect::from_min_size(top, vec2(full, height));
        let rtl = crate::i18n::current().rtl();
        let mut right = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(crate::bidi::row_from_end(egui::Align::Center)));
        control(&mut right);
        let controls = right.min_rect();
        let w = controls.width();
        let room = (full - w - gap).max(0.0);
        let mut text_rect = if rtl { rect.with_min_x(rect.max.x - room) } else { rect.with_max_x(rect.min.x + room) };
        // Centred on the controls (a short text), or the controls on it (a long one).
        if text_h > 0.0 {
            text_rect.min.y += ((height - text_h) / 2.0).max(0.0);
        }
        let mut left = ui.new_child(egui::UiBuilder::new().max_rect(text_rect).layout(crate::bidi::column()));
        text(&mut left, label, about);
        let h = left.min_rect().height();
        let text = left.min_rect();
        ui.advance_cursor_after_rect(rect.union(text).union(controls));
        remember(ui, id, (controls_w, text_h), (w, h));
        ui.add_space(7.0);
        (text, controls)
    }
}

/// Narrowest the text of a setting row gets beside its controls; any narrower and they go under it.
const SETTING_TEXT_MIN: f32 = 240.0;

/// Keep a setting row's measures for the next frame, and draw again when they changed (the row
/// was laid out by the old ones).
fn remember(ui: &Ui, id: egui::Id, old: (f32, f32), new: (f32, f32)) {
    if (old.0 - new.0).abs() > 0.5 || (old.1 - new.1).abs() > 0.5 {
        ui.data_mut(|d| d.insert_temp(id, new));
        ui.ctx().request_repaint();
    }
}

/// A row's name and the line under it, in the page's colours.
fn name(t: &Tokens, s: &str) -> RichText {
    RichText::new(s).color(t.text)
}

fn note(t: &Tokens, s: &str) -> RichText {
    RichText::new(s).color(t.faint).size(12.0)
}

fn info(s: &Session, t: &Tokens, has_files: bool, ui: &mut Ui, b: &mut Backstage, acts: &mut Vec<Action>) {
    heading(ui, t, "Account information");
    ui.set_max_width(ui.available_width().min(760.0));
    let Some(a) = b.account.as_deref().and_then(|id| s.account(id)).or_else(|| s.accounts.first()) else {
        ui.label(RichText::new(tl!("Add an email account")).color(t.dim));
        if ui.button(tl!("Add account…")).clicked() {
            acts.push(Action::Dialog(Dialog::add_account()));
        }
        return;
    };
    let id = a.config.id.clone();
    // The account picker.
    // In a narrow window the list is as wide as the page and the button goes under it (the two
    // in a row pushed the page, and every card on it, past the window's edge).
    ui.horizontal_wrapped(|ui| {
        let current = format!("{}   ·   {}", a.config.email, tl!(kind(a)));
        let width = (ui.available_width() - 2.0 * ui.spacing().button_padding.x).min(420.0);
        egui::ComboBox::from_id_salt("backstage-account").width(width).selected_text(current).truncate().show_ui(ui, |ui| {
            for x in &s.accounts {
                if ui.selectable_label(x.config.id == id, format!("{}   ·   {}", x.config.email, tl!(kind(x)))).clicked() {
                    b.account = Some(x.config.id.clone());
                }
            }
        });
        if widgets::pill_button(ui, Icon::Plus, "Add account…", t).clicked() {
            acts.push(Action::Dialog(Dialog::add_account()));
        }
    });
    ui.add_space(16.0);
    if !a.is_local() && b.asked.insert(id.clone()) {
        acts.push(Action::Run("account.vacation", json!({"account": id})));
    }

    // Account settings: where it connects, and every change to it, each with its own button.
    let servers = if a.is_data_file() {
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
    };
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
            let others = &a.config.identities;
            let about = if others.is_empty() {
                tl!("Addresses of this same mailbox that you also send from.").to_string()
            } else {
                others.iter().map(|i| i.email.as_str()).collect::<Vec<_>>().join(", ")
            };
            setting_row(ui, t, false, name(t, tl!("Other addresses")), Some(note(t, &about)), |ui| {
                // A JMAP server keeps them: asked for, they're added.
                if a.config.jmap.is_some() && ui.button(tl!("Find on the server")).clicked() {
                    acts.push(Action::Run("account.findAddresses", json!({"account": id})));
                }
                if ui.button(tl!("Add…")).clicked() {
                    acts.push(Action::Dialog(Dialog::Address(crate::dialogs::AddressForm::new(&id))));
                }
                if !others.is_empty() {
                    ui.menu_button(tl!("Edit"), |ui| {
                        for i in others {
                            if ui.button(i.email.as_str()).clicked() {
                                acts.push(Action::Dialog(Dialog::Address(crate::dialogs::AddressForm::of(&id, i))));
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
        if a.config.ews.is_some() {
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
        // Calendars and contacts: where they were found, or the addresses given by hand (the
        // desktop app's: the browser doesn't sync calendars).
        if !cfg!(target_arch = "wasm32") && a.config.has_server() && a.config.graph.is_none() && a.config.ews.is_none() {
            let pim = s.pim.get(&id);
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
    // S/MIME and OpenPGP, for accounts that send mail.
    if !a.is_data_file() {
        security::security(s, t, ui, a, acts);
        pgp::pgp(s, t, ui, a, acts);
        security::defaults(s, t, ui, a, acts);
    }

    // Automatic replies: on or off, and a button to change them where the server has them.
    let (color, status) = match &a.vacation {
        _ if a.is_data_file() => (t.faint, tl!("Not available: a data file has no server.").to_string()),
        _ if a.config.is_calendar_only() => (t.faint, tl!("Not available: this account has no mail server.").to_string()),
        _ if a.is_local() => (t.faint, tl!("Not available: the sample mailbox has no server.").to_string()),
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
                    acts.push(Action::Dialog(Dialog::vacation(&id, &current, exchange)));
                }
                if matches!(a.vacation, VacationState::Loading) {
                    ui.spinner();
                }
            });
        },
    );

    data_file::mailbox(s, t, has_files, ui, a, b, acts);

    // Rules.
    let n = s.rules.len();
    card(ui, t, tl!("Rules"), Some(tl!("Sort new mail as it arrives: move it, flag it, categorize it.")), |ui| {
        let count = if n == 0 { tl!("No rules yet.").to_string() } else { tf!("Rules: {n}", n = n) };
        setting_row(ui, t, true, name(t, &count), None, |ui| {
            if ui.button(tl!("Manage rules")).clicked() {
                acts.push(Action::Dialog(Dialog::Rules));
            }
        });
    });
}

/// Open & Export: Outlook data files, and the rules as a file.
fn open(s: &Session, t: &Tokens, ui: &mut Ui, b: &mut Backstage, acts: &mut Vec<Action>) {
    heading(ui, t, "Open & Export");
    ui.set_max_width(ui.available_width().min(760.0));
    backup::backup(s, t, ui, acts);
    // Not in the browser: it reads no file by its path, and keeps no data files to open or write.
    if !s.jmap_only {
        card(
            ui,
            t,
            tl!("Open a data file"),
            Some(tl!(
                "Bring in an Outlook data file (.pst) or a MailCraft one (.mailcraft). It opens as a mailbox of its own, with its folders, kept on this computer."
            )),
            |ui| {
                let files: Vec<&str> = s.accounts.iter().filter(|a| a.is_data_file()).map(|a| a.config.email.as_str()).collect();
                let open_now = (!files.is_empty()).then(|| tf!("Open now: {files}", files = files.join(", ")));
                let label = open_now.as_deref().map_or_else(|| note(t, tl!("No data file is open.")), |s| note(t, s));
                setting_row(ui, t, true, label, None, |ui| {
                    if ui.button(tl!("Open…")).clicked() {
                        acts.push(Action::OpenDataFile);
                    }
                });
            },
        );
        card(
            ui,
            t,
            tl!("Open a mail file"),
            Some(tl!(
                "See a saved message (.eml or Outlook's .msg) in a window of its own, without adding it to a folder. Its pictures from the web are loaded only if you ask, and nothing in it runs."
            )),
            |ui| {
                setting_row(ui, t, true, note(t, tl!("Nothing is kept: closing it leaves no trace in your mail.")), None, |ui| {
                    if ui.button(tl!("Open…")).clicked() {
                        acts.push(Action::OpenMailFile);
                    }
                });
            },
        );
        // The account to export: picked in the list (one line, however many accounts there are),
        // else the one being looked at.
        let chosen = b
            .export
            .as_deref()
            .and_then(|id| s.account(id))
            .or_else(|| s.current_account())
            .or_else(|| s.accounts.first())
            .map(|a| (a.config.id.clone(), format!("{}   ·   {}", a.config.email, tl!(kind(a)))));
        card(
            ui,
            t,
            tl!("Export to Outlook data file"),
            Some(tl!("Write an account's mail, with its folders, to an Outlook data file (.pst) that Outlook and other mail apps can open.")),
            |ui| {
                let Some((id, label)) = &chosen else { return };
                setting_row(
                    ui,
                    t,
                    true,
                    name(t, tl!("Account")),
                    Some(note(t, tl!("Its contacts, calendar, notes and tasks go inside it too."))),
                    |ui| {
                        if ui.button(tl!("Export…")).clicked() {
                            acts.push(Action::ExportDataFile(id.clone()));
                        }
                        egui::ComboBox::from_id_salt("export-account").width(300.0).selected_text(label).show_ui(ui, |ui| {
                            for a in &s.accounts {
                                if ui.selectable_label(&a.config.id == id, format!("{}   ·   {}", a.config.email, tl!(kind(a)))).clicked() {
                                    b.export = Some(a.config.id.clone());
                                }
                            }
                        });
                    },
                );
            },
        );
    }
    // Rules to a file and back, as Outlook's Import and Export Rules.
    card(ui, t, tl!("Rules"), None, |ui| {
        setting_row(
            ui,
            t,
            true,
            name(t, tl!("Export rules")),
            Some(note(t, tl!("Save your rules in a file, to keep them or to use them on another computer."))),
            |ui| {
                if ui.add_enabled(!s.rules.is_empty(), egui::Button::new(tl!("Export…"))).clicked() {
                    acts.push(Action::ExportRules);
                }
            },
        );
        setting_row(
            ui,
            t,
            false,
            name(t, tl!("Import rules")),
            Some(note(t, tl!("Add the rules from a file saved by MailCraft. A rule for an account this computer doesn't have comes in turned off."))),
            |ui| {
                if ui.button(tl!("Import…")).clicked() {
                    acts.push(Action::ImportRules);
                }
            },
        );
    });
}

/// Check for updates, and what it found: a newer release to download (its installer for this
/// system, else its page) and what's new in it. Nothing installs by itself.
fn updates(s: &Session, t: &Tokens, ui: &mut Ui, acts: &mut Vec<Action>) {
    use mailcraft_engine::UpdateCheck;
    if s.jmap_only {
        return;
    }
    ui.horizontal_wrapped(|ui| {
        let checking = s.update == UpdateCheck::Checking;
        if ui.add_enabled(!checking, egui::Button::new(tl!("Check for updates"))).clicked() {
            acts.push(Action::Run("app.checkUpdates", json!({})));
        }
        match &s.update {
            UpdateCheck::NotAsked => {}
            UpdateCheck::Checking => {
                ui.spinner();
                ui.label(RichText::new(tl!("Checking…")).color(t.dim));
            }
            UpdateCheck::UpToDate(_) => {
                ui.label(RichText::new(tl!("You have the newest version.")).color(t.dim));
            }
            UpdateCheck::Available(r) => {
                let name = r.tag.trim_start_matches('v').to_string();
                ui.label(RichText::new(tf!("MailCraft {version} is available.", version = name)).color(t.text).strong());
                let target = if r.download.is_empty() { r.page.clone() } else { r.download.clone() };
                if widgets::accent_button(ui, Icon::Sync, "Download", t, true).on_hover_text(&target).clicked() {
                    acts.push(Action::OpenUrl(target));
                }
                if ui.link(tl!("What's new")).on_hover_text(&r.page).clicked() {
                    acts.push(Action::OpenUrl(r.page.clone()));
                }
            }
            UpdateCheck::Failed(e) => {
                ui.label(RichText::new(tf!("Couldn't check for updates: {error}", error = e)).color(t.danger));
            }
        }
    });
}

fn about(s: &Session, t: &Tokens, update: &UpdateState, ui: &mut Ui, acts: &mut Vec<Action>) {
    heading(ui, t, "About MailCraft");
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(56.0, 56.0), Sense::hover());
        ui.painter().rect_filled(r, 12.0, t.accent);
        icons::paint(ui.painter(), icons::centered(r, 34.0), Icon::Unread, egui::Color32::WHITE);
        ui.add_space(10.0);
        ui.vertical(|ui| {
            ui.label(RichText::new(tf!("MailCraft {version}", version = env!("CARGO_PKG_VERSION"))).font(theme::bold(18.0)).color(t.text));
            ui.label(RichText::new(tl!("Free and open source · MIT OR Apache-2.0")).color(t.dim));
        });
    });
    ui.add_space(10.0);
    updates(s, t, ui, acts);
    // A test build that updates itself: its next version on the way, or downloaded.
    match update {
        UpdateState::Ready { version, .. } => {
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(tf!("MailCraft {version} is downloaded. It installs when you close MailCraft.", version = version)).color(t.text),
                );
                if widgets::accent_button(ui, Icon::Sync, "Restart to update", t, true).clicked() {
                    acts.push(Action::RestartToUpdate);
                }
            });
        }
        UpdateState::Downloading { version, done, total } => {
            ui.add_space(6.0);
            let percent = if *total > 0 { (done.saturating_mul(100) / total).min(100) } else { 0 };
            ui.label(RichText::new(tf!("Downloading MailCraft {version}… {percent}%", version = version, percent = percent)).color(t.dim));
        }
        UpdateState::None => {}
    }
    ui.add_space(14.0);
    for line in [
        "Local-first: mail is cached on this computer for fast offline search. Passwords stay in the system keychain.",
        "MailCraft talks only to your own mail servers, over encrypted connections. Pictures from the web load only when you ask.",
        "Built in the open from the email standards: no code, icons or assets from other mail apps.",
    ] {
        ui.label(RichText::new(tl!(line)).color(t.dim));
        ui.add_space(4.0);
    }
    ui.add_space(8.0);
    if ui.link(tl!("Source code and issues")).clicked() {
        acts.push(Action::OpenUrl("https://github.com/ZIDAN44/mailcraft".into()));
    }
    // Fonts built in from craft-fonts travel with their licences.
    if !crate::craft_fonts::CRAFT_FONTS.is_empty() {
        ui.add_space(12.0);
        ui.label(RichText::new(tl!("Fonts")).font(theme::bold(14.0)).color(t.text));
        for f in crate::craft_fonts::CRAFT_FONTS {
            egui::CollapsingHeader::new(RichText::new(format!("{} {} · {}", f.family, f.style, f.licence)).color(t.dim))
                .id_salt(("craft-font", f.family, f.style))
                .show(ui, |ui| {
                    egui::ScrollArea::vertical().max_height(220.0).id_salt(("craft-font-licence", f.family, f.style)).show(ui, |ui| {
                        ui.label(RichText::new(f.licence_text).small().color(t.dim));
                    });
                });
        }
    }
}

/// An address as people read it: `%40` and the like written as what they stand for (a server
/// writes `zoe%40posteo.de`); one that isn't valid UTF-8 once decoded stays as it is.
fn readable_url(url: &str) -> String {
    let b = url.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| (c as char).to_digit(16);
        match (b.get(i), b.get(i + 1).copied().and_then(hex), b.get(i + 2).copied().and_then(hex)) {
            (Some(b'%'), Some(h), Some(l)) => {
                out.push(u8::try_from(h * 16 + l).unwrap_or(b'?'));
                i += 3;
            }
            (Some(c), _, _) => {
                out.push(*c);
                i += 1;
            }
            (None, _, _) => break,
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| url.to_string())
}

#[cfg(test)]
#[test]
fn addresses_read_as_written() {
    assert_eq!(readable_url("https://p.example:8443/zoe%40posteo.example/"), "https://p.example:8443/zoe@posteo.example/");
    assert_eq!(readable_url("https://p.example/caf%C3%A9/"), "https://p.example/café/");
    assert_eq!(readable_url("https://p.example/%FF%"), "https://p.example/%FF%", "not UTF-8 once decoded");
    assert_eq!(readable_url("%4"), "%4");
}
