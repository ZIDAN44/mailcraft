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
use crate::{Action, Dialog, UiState, widgets};

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
    /// Bytes cached per account, measured once per visit.
    cached: HashMap<String, u64>,
}

impl Backstage {
    pub fn new(page: Page) -> Backstage {
        Backstage { page, account: None, asked: HashSet::new(), cached: HashMap::new() }
    }
}

/// What kind of account it is, for the picker.
fn kind(a: &AccountState) -> &'static str {
    if a.is_data_file() {
        "Outlook data file"
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
    egui::Panel::left("backstage-rail").exact_size(200.0).frame(egui::Frame::new().fill(t.chrome).inner_margin(Margin::symmetric(0, 10))).show(
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
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
        },
    );
    egui::CentralPanel::default().frame(egui::Frame::new().fill(t.bg)).show(ui, |ui| {
        egui::ScrollArea::vertical().auto_shrink(false).id_salt("backstage").show(ui, |ui| {
            egui::Frame::new().inner_margin(Margin { left: 36, right: 36, top: 24, bottom: 32 }).show(ui, |ui| {
                ui.set_max_width(820.0);
                match b.page {
                    Page::Info => info(s, &t, st.has_files, ui, &mut b, acts),
                    Page::Open => open(s, &t, ui, acts),
                    Page::Options => options(s, st, ui, acts),
                    Page::About => about(&t, ui, acts),
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

/// A big square button with an icon, and its title, description and anything else beside it.
fn tile(ui: &mut Ui, t: &Tokens, icon: Icon, label: &str, title: &str, about: &str, extra: impl FnOnce(&mut Ui)) -> egui::Response {
    let mut out = None;
    ui.horizontal_top(|ui| {
        let (r, resp) = ui.allocate_exact_size(vec2(104.0, 92.0), Sense::click());
        let fill = if resp.hovered() { t.hover } else { t.surface };
        ui.painter().rect(r, 6.0, fill, egui::Stroke::new(1.0, t.border), egui::StrokeKind::Inside);
        // The icon over its label, the two centred in the tile together (a label in two lines,
        // as some languages need, moves the icon up), each line of the label centred.
        let mut job = egui::text::LayoutJob::simple(tl!(label).to_string(), theme::regular(12.5), t.text, 100.0);
        job.halign = egui::Align::Center;
        let galley = ui.painter().layout_job(job);
        let block = 28.0 + 8.0 + galley.size().y;
        let top = r.center().y - block / 2.0;
        icons::paint(ui.painter(), egui::Rect::from_center_size(egui::pos2(r.center().x, top + 14.0), vec2(28.0, 28.0)), icon, t.accent);
        ui.painter().galley(egui::pos2(r.center().x, top + 36.0), galley, t.text);
        ui.add_space(14.0);
        ui.vertical(|ui| {
            ui.label(RichText::new(tl!(title)).font(theme::bold(16.0)).color(t.text));
            ui.label(RichText::new(tl!(about)).color(t.dim));
            ui.add_space(4.0);
            extra(ui);
        });
        out = Some(resp);
    });
    ui.add_space(18.0);
    out.unwrap_or_else(|| ui.label(""))
}

fn info(s: &Session, t: &Tokens, has_files: bool, ui: &mut Ui, b: &mut Backstage, acts: &mut Vec<Action>) {
    heading(ui, t, "Account information");
    let Some(a) = b.account.as_deref().and_then(|id| s.account(id)).or_else(|| s.accounts.first()) else {
        ui.label(RichText::new(tl!("Add an email account")).color(t.dim));
        if ui.button(tl!("Add account…")).clicked() {
            acts.push(Action::Dialog(Dialog::add_account()));
        }
        return;
    };
    let id = a.config.id.clone();
    // The account picker.
    ui.horizontal(|ui| {
        let current = format!("{}   ·   {}", a.config.email, tl!(kind(a)));
        egui::ComboBox::from_id_salt("backstage-account").width(420.0).selected_text(current).show_ui(ui, |ui| {
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
    ui.add_space(18.0);
    if !a.is_local() && b.asked.insert(id.clone()) {
        acts.push(Action::Run("account.vacation", json!({"account": id})));
    }

    // Account settings.
    let servers = if a.is_data_file() {
        tl!("Opened from an Outlook data file, kept on this computer only").to_string()
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
    let r = tile(
        ui,
        t,
        Icon::Person,
        "Account settings",
        "Account settings",
        "Change this account's name, servers and sign-in, or its signature.",
        |ui| {
            ui.label(RichText::new(servers).color(t.faint).small());
        },
    );
    egui::Popup::menu(&r).show(|ui| {
        if !a.is_local() && a.config.graph.is_none() && ui.button(tl!("Server settings…")).clicked() {
            acts.push(Action::Dialog(Dialog::edit_account(&a.config)));
        }
        if !a.is_local() && ui.button(tl!(if a.config.oauth.is_some() { "Sign in again…" } else { "Password…" })).clicked() {
            acts.push(Action::SignInAgain(id.clone()));
        }
        if ui.button(tl!("Signature…")).clicked() {
            acts.push(Action::Dialog(Dialog::Signature { account: id.clone(), text: a.config.signature.clone() }));
        }
        ui.separator();
        if ui.button(tl!("Remove account…")).clicked() {
            acts.push(Action::Dialog(Dialog::RemoveAccount { account: id.clone(), email: a.config.email.clone() }));
        }
    });

    // Automatic replies.
    let status = match &a.vacation {
        _ if a.is_data_file() => (t.faint, tl!("Not available: a data file has no server.").to_string()),
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
    let r = tile(
        ui,
        t,
        Icon::Away,
        "Automatic replies",
        "Automatic replies",
        "Tell people you're away: each sender gets one reply while they're on, even when this computer is off.",
        |ui| {
            ui.horizontal(|ui| {
                if matches!(a.vacation, VacationState::Loading) {
                    ui.spinner();
                }
                ui.label(RichText::new(status.1).color(status.0));
            });
        },
    );
    if r.clicked() && !a.is_local() {
        let current = match &a.vacation {
            VacationState::Loaded(v) => v.clone(),
            _ => Default::default(),
        };
        acts.push(Action::Dialog(Dialog::vacation(&id, &current)));
    }

    // Mailbox: space used, what this computer keeps, emptying.
    let cached = *b.cached.entry(id.clone()).or_insert_with(|| s.cache_size(&id));
    let size = |n: u64| widgets::human_size(usize::try_from(n).unwrap_or(usize::MAX));
    tile(ui, t, Icon::Storage, "Mailbox", "Mailbox settings", "See how much space your mail takes, and empty Deleted Items and Junk Email.", |ui| {
        match a.quota {
            Some((used, limit)) if limit > 0 => {
                let frac = (used as f32 / limit as f32).clamp(0.0, 1.0);
                ui.add(egui::ProgressBar::new(frac).desired_width(260.0).fill(if frac > 0.9 { t.danger } else { t.accent }));
                ui.label(
                    RichText::new(tf!("{free} free of {total}", free = size(limit.saturating_sub(used)), total = size(limit))).color(t.dim).small(),
                );
            }
            Some((used, _)) => {
                ui.label(RichText::new(tf!("{used} used on the server", used = size(used))).color(t.dim).small());
            }
            None if !a.is_local() => {
                ui.label(RichText::new(tl!("The server doesn't say how much space is used.")).color(t.faint).small());
            }
            None => {}
        }
        ui.label(RichText::new(tf!("Kept on this computer: {size}", size = size(cached))).color(t.faint).small());
        // Where, as Outlook shows a data file's place; and a copy as one.
        if has_files && let Some(file) = s.store.as_ref().map(|store| store.account_path(&id)) {
            ui.label(RichText::new(file.display().to_string()).color(t.faint).small());
            ui.horizontal(|ui| {
                if ui.add(egui::Button::new(tl!("Open folder")).small()).clicked() {
                    // The folder the data file is in (or the account's own folder).
                    let dir = if file.is_dir() { file.clone() } else { file.parent().map_or_else(|| file.clone(), Path::to_path_buf) };
                    acts.push(Action::OpenFolder(dir));
                }
                if ui.add(egui::Button::new(tl!("Save as an Outlook data file…")).small()).clicked() {
                    acts.push(Action::ExportDataFile(id.clone()));
                }
            });
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            for (role, label) in [
                (mailcraft_engine::model::FolderRole::Trash, "Empty Deleted Items…"),
                (mailcraft_engine::model::FolderRole::Junk, "Empty Junk Email…"),
            ] {
                if let Some(f) = a.folder(role)
                    && ui.add_enabled(f.total > 0 || a.is_local(), egui::Button::new(tl!(label)).small()).clicked()
                {
                    acts.push(Action::Dialog(Dialog::ConfirmEmpty {
                        account: id.clone(),
                        folder: f.path.clone(),
                        name: crate::i18n::tc("folder", &f.display_name()).to_string(),
                        for_good: true,
                    }));
                }
            }
        });
    });

    // Rules.
    let n = s.rules.len();
    let r = tile(ui, t, Icon::Funnel, "Manage rules", "Rules", "Sort new mail as it arrives: move it, flag it, categorize it.", |ui| {
        ui.label(RichText::new(if n == 0 { tl!("No rules yet.").to_string() } else { tf!("Rules: {n}", n = n) }).color(t.faint).small());
    });
    if r.clicked() {
        acts.push(Action::Dialog(Dialog::Rules));
    }
}

/// Open & Export: Outlook data files.
fn open(s: &Session, t: &Tokens, ui: &mut Ui, acts: &mut Vec<Action>) {
    heading(ui, t, "Open & Export");
    let r = tile(
        ui,
        t,
        Icon::Folder,
        "Open data file",
        "Open a data file",
        "Bring in an Outlook data file (.pst) or a MailCraft one (.mailcraft). It opens as a mailbox of its own, with its folders, kept on this computer.",
        |ui| {
            let files: Vec<&str> = s.accounts.iter().filter(|a| a.is_data_file()).map(|a| a.config.email.as_str()).collect();
            if !files.is_empty() {
                ui.label(RichText::new(tf!("Open now: {files}", files = files.join(", "))).color(t.faint).small());
            }
        },
    );
    if r.clicked() {
        acts.push(Action::OpenDataFile);
    }
    let current = s.current_account().map(|a| a.config.id.clone());
    let r = tile(
        ui,
        t,
        Icon::Outbox,
        "Export",
        "Export to Outlook data file",
        "Write an account's mail, with its folders, to an Outlook data file (.pst) that Outlook and other mail apps can open. Mail that was never downloaded is left out.",
        |ui| {
            ui.label(RichText::new(tl!("Its calendar and contacts go beside it, as .ics and .vcf files.")).color(t.faint).small());
            ui.horizontal_wrapped(|ui| {
                for a in &s.accounts {
                    if ui.button(RichText::new(&a.config.email).small()).clicked() {
                        acts.push(Action::ExportDataFile(a.config.id.clone()));
                    }
                }
            });
        },
    );
    if r.clicked()
        && let Some(id) = current
    {
        acts.push(Action::ExportDataFile(id));
    }
}

fn options(s: &Session, st: &UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    let t = st.tokens;
    heading(ui, &t, "Options");
    let group = |ui: &mut Ui, title: &str, body: &mut dyn FnMut(&mut Ui)| {
        egui::Frame::new().fill(t.surface).stroke(egui::Stroke::new(1.0, t.border)).corner_radius(8).inner_margin(Margin::same(14)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(tl!(title)).font(theme::bold(15.0)).color(t.text));
            ui.add_space(6.0);
            body(ui);
        });
        ui.add_space(12.0);
    };
    let check = |ui: &mut Ui, on: bool, label: &str, id: &'static str, acts: &mut Vec<Action>| {
        let mut v = on;
        if ui.checkbox(&mut v, tl!(label)).changed() {
            acts.push(Action::Run(id, json!({"on": v})));
        }
    };
    group(ui, "General", &mut |ui| {
        ui.horizontal(|ui| {
            ui.label(tl!("Theme"));
            for (v, label) in [(None, "Same as the system"), (Some(false), "Light"), (Some(true), "Dark")] {
                if ui.radio(st.dark == v, tl!(label)).clicked() {
                    acts.push(Action::SetDark(v));
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label(tl!("Language"));
            let language = st.language.as_str();
            let current = crate::i18n::Lang::from_code(language).map_or_else(|| tl!("Same as the system").to_string(), |l| l.name().to_string());
            egui::ComboBox::from_id_salt("language").selected_text(current).show_ui(ui, |ui| {
                if ui.selectable_label(language == crate::i18n::AUTO, tl!("Same as the system")).clicked() {
                    acts.push(Action::SetLanguage(crate::i18n::AUTO.into()));
                }
                for l in crate::i18n::Lang::all() {
                    if ui.selectable_label(language == l.code(), l.name()).clicked() {
                        acts.push(Action::SetLanguage(l.code().into()));
                    }
                }
            });
        });
    });
    // Where the data files are (one per account, as Outlook's .pst files); movable as those are.
    if let Some((dir, movable)) = &st.data_folder {
        group(ui, "Data files", &mut |ui| {
            ui.label(
                RichText::new(tl!(
                    "Each account's mail, contacts and calendar are kept in a MailCraft data file (.mailcraft) in this folder, as Outlook keeps each account in a .pst in Outlook Files. An Outlook data file you open gets a .mailcraft file of its own here; Outlook's file is left as it was. To have a copy as a .pst, use Save as an Outlook data file under Info."
                ))
                .color(t.dim),
            );
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(dir.display().to_string()).color(t.text));
                if ui.small_button(tl!("Open folder")).clicked() {
                    acts.push(Action::OpenFolder(dir.clone()));
                }
                if *movable && ui.small_button(tl!("Move…")).clicked() {
                    acts.push(Action::MoveDataFolder);
                }
            });
            if let Some(to) = &st.pending_move {
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new(tf!("Moves to {folder} the next time MailCraft starts.", folder = to.display().to_string()))
                            .color(t.warn)
                            .small(),
                    );
                    if ui.small_button(tl!("Cancel")).clicked() {
                        acts.push(Action::CancelDataMove);
                    }
                });
            } else if !*movable {
                ui.label(
                    RichText::new(tl!("Chosen when MailCraft was started (--data or MAILCRAFT_DATA), so it can't be moved here."))
                        .color(t.faint)
                        .small(),
                );
            }
        });
    }
    group(ui, "Reading", &mut |ui| {
        ui.horizontal(|ui| {
            ui.label(tl!("Reading pane"));
            for (v, label, code) in
                [(ReadingPane::Right, "Right", "right"), (ReadingPane::Bottom, "Bottom", "bottom"), (ReadingPane::Off, "Off", "off")]
            {
                if ui.radio(s.prefs.reading_pane == v, tl!(label)).clicked() {
                    acts.push(Action::Run("view.readingPane", json!({"value": code})));
                }
            }
        });
        check(ui, s.prefs.conversations, "Group messages by conversation", "view.conversations", acts);
        check(ui, s.prefs.show_preview, "Show a line of each message in the list", "view.preview", acts);
        check(ui, s.prefs.compact, "Compact list", "view.compact", acts);
        check(ui, s.prefs.mark_read_on_open, "Mark messages as read when opened", "view.markReadOnOpen", acts);
        check(ui, s.prefs.new_mail_window, "Write new messages in a window of their own", "view.newMailWindow", acts);
        check(ui, s.prefs.show_html, "Show HTML mail formatted (otherwise as plain text)", "view.html", acts);
    });
    group(ui, "Inbox and notifications", &mut |ui| {
        check(ui, s.prefs.notify, "Show a desktop notification when new mail arrives", "view.notifications", acts);
        check(ui, s.prefs.focused_inbox, "Focused Inbox: put newsletters and automatic mail in Other", "view.focused", acts);
        check(ui, s.prefs.confirm_permanent_delete, "Ask before deleting for good", "view.confirmDelete", acts);
    });
    group(ui, "Privacy", &mut |ui| {
        ui.label(RichText::new(tl!("Pictures from the web load only when you ask. These senders' pictures always load:")).color(t.dim));
        if s.prefs.trusted_senders.is_empty() {
            ui.label(RichText::new(tl!("No senders yet.")).color(t.faint).small());
        }
        for who in &s.prefs.trusted_senders {
            ui.horizontal(|ui| {
                ui.label(who);
                if ui.small_button(tl!("Remove")).clicked() {
                    acts.push(Action::Run("view.trustSender", json!({"address": who, "on": false})));
                }
            });
        }
    });
}

fn about(t: &Tokens, ui: &mut Ui, acts: &mut Vec<Action>) {
    heading(ui, t, "About MailCraft");
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(56.0, 56.0), Sense::hover());
        ui.painter().rect_filled(r, 12.0, t.accent);
        icons::paint(ui.painter(), icons::centered(r, 34.0), Icon::Unread, egui::Color32::WHITE);
        ui.add_space(10.0);
        ui.vertical(|ui| {
            ui.label(RichText::new(tf!("MailCraft {version}", version = env!("CARGO_PKG_VERSION"))).font(theme::bold(18.0)).color(t.text));
            ui.label(RichText::new(tl!("An ArtCraft app · MIT OR Apache-2.0")).color(t.dim));
        });
    });
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
