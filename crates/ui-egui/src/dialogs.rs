//! Modal dialogs: add an account, sign in again, move to a folder, confirmations, settings.

use egui::{Margin, Rect, RichText, Sense, Ui, vec2};
use mailcraft_engine::Compose;
use mailcraft_engine::Session;
use mailcraft_engine::model::organize::{Condition, Rule, RuleAction};
use mailcraft_engine::model::{AccountConfig, Folder, FolderRole, Security};
use serde_json::json;

use crate::theme::{self, Tokens};
use crate::{Action, Icon, widgets};

mod accounts;
mod closing;
mod folders;
mod mailbox;
mod organize;
mod other;
pub(crate) mod pim_dialogs;
mod writing;

pub use accounts::*;
pub use mailbox::*;
pub use organize::*;
pub use writing::*;

#[derive(Clone, Debug)]
pub enum Dialog {
    AddAccount(Box<AccountForm>),
    /// Outlook's Select Names: the address book, to add people to To, Cc and Bcc.
    SelectNames(Box<NamesForm>),
    /// Closing a message window with something written in it.
    SaveChanges,
    Password {
        account: String,
        email: String,
        password: String,
    },
    ConfirmDelete {
        count: usize,
    },
    /// Closing MailCraft with messages being written (how many): save them as drafts?
    SaveOnClose(usize),
    /// Closing would lose what online-only accounts hold in memory only (issue #4).
    LostOnClose {
        unsent: usize,
        changes: usize,
    },
    /// Closing with an update about to be installed, or still downloading: what becomes of it.
    UpdateOnClose(crate::UpdateState),
    /// Confirmation before `contact.delete`, which can't be undone.
    ConfirmDeleteContact {
        uid: String,
        name: String,
    },
    MoveTo {
        filter: String,
    },
    EmptySubject,
    /// What `compose.send` held the message for (a lookalike recipient, many people seeing each
    /// other's addresses).
    SendCheck(Vec<mailcraft_engine::SendCheck>),
    RemoveAccount {
        account: String,
        email: String,
    },
    /// Microsoft signed the account in but won't open its mailbox over IMAP: why, and the way out.
    MailboxClosed {
        account: String,
        email: String,
        error: String,
    },
    Signature(Box<SignatureForm>),
    /// An account's name, user name and servers.
    EditAccount(Box<EditForm>),
    /// An account's automatic replies.
    Vacation(Box<VacationForm>),
    /// Empty Deleted Items or Junk for good (`for_good`), or Delete All in another folder:
    /// everything to Deleted Items.
    ConfirmEmpty {
        account: String,
        folder: String,
        name: String,
        for_good: bool,
    },
    /// Open someone else's calendar by their address (Outlook's Open Calendar).
    OpenSharedCalendar {
        email: String,
        error: Option<String>,
    },
    /// Open another mailbox in an Exchange account by its address (Outlook's "Open these
    /// additional mailboxes").
    AddSharedMailbox {
        account: String,
        address: String,
        error: Option<String>,
    },
    /// The password of a certificate file being imported (S/MIME): from `path`, or `data` as the
    /// browser's picker gives it; neither, the one the browser keeps, unlocked.
    Certificate {
        account: String,
        path: String,
        data: Option<Vec<u8>>,
        password: String,
    },
    /// A certificate on a smart card (its PKCS #11 module and PIN), or (`system`) in the system's
    /// own store.
    SmartCard {
        account: String,
        module: String,
        pin: String,
        system: bool,
    },
    /// Word count of the message being written (or its selection: the `bool`).
    WordCount(crate::rich_edit::Counts, bool),
    /// A folder's Properties: where it is, how many messages, how much is on this computer.
    FolderProperties {
        name: String,
        account: String,
        location: String,
        total: usize,
        unread: usize,
        size: u64,
    },
    Event(Box<crate::pim_dialogs::EventForm>),
    Contact(Box<crate::pim_dialogs::ContactForm>),
    /// New folder (in `parent`), or rename `rename`.
    FolderName {
        account: String,
        parent: Option<String>,
        rename: Option<String>,
        name: String,
        error: Option<String>,
    },
    MoveFolder {
        account: String,
        folder: String,
        filter: String,
        error: Option<String>,
    },
    DeleteFolder {
        account: String,
        folder: String,
        name: String,
        permanent: bool,
        error: Option<String>,
    },
    NewCategory {
        name: String,
        color: u8,
        error: Option<String>,
    },
    Rules,
    EditRule(Box<RuleForm>),
    /// A Search Folder made or changed (Outlook's New Search Folder).
    SearchFolder(Box<SearchFolderForm>),
    /// A Quick Step made or changed.
    QuickStep(Box<QuickStepForm>),
    /// A picture attachment, larger.
    PicturePreview {
        key: mailcraft_engine::model::MessageKey,
        index: usize,
        name: String,
    },
    /// An attachment shown as text (a file, or an attached email's header and text).
    Preview {
        title: String,
        header: Vec<(String, String)>,
        text: String,
        /// An email whose sender doesn't add up.
        warning: Option<mailcraft_engine::model::lookalike::SenderWarning>,
    },
}

impl Dialog {
    pub fn add_account() -> Dialog {
        Dialog::AddAccount(Box::default())
    }

    /// The address book for the message being written; a double-click adds to `field`.
    pub fn select_names(c: &mailcraft_engine::Compose, field: &'static str) -> Dialog {
        Dialog::SelectNames(Box::new(NamesForm {
            target: field,
            to: c.to.clone(),
            cc: c.cc.clone(),
            bcc: c.bcc.clone(),
            stale: true,
            ..Default::default()
        }))
    }

    /// Dialogs that belong to the message being written (shown in its window when it has one).
    pub fn for_compose(&self) -> bool {
        matches!(self, Dialog::SelectNames(_) | Dialog::SaveChanges | Dialog::EmptySubject | Dialog::SendCheck(_) | Dialog::WordCount(..))
            || matches!(self, Dialog::Signature(f) if f.insert)
    }

    /// The add-account dialog with the address filled in (servers guessed from it).
    pub fn add_account_with(email: &str, advanced: bool) -> Dialog {
        let mut f = AccountForm { email: email.trim().to_string(), advanced, ..Default::default() };
        if !f.email.is_empty() {
            f.guess();
        }
        Dialog::AddAccount(Box::new(f))
    }

    pub fn move_to() -> Dialog {
        Dialog::MoveTo { filter: String::new() }
    }

    /// A new event (at a time, or the next hour).
    pub fn new_event(s: &Session, at: Option<i64>) -> Dialog {
        Dialog::Event(Box::new(crate::pim_dialogs::EventForm::new(s, at)))
    }

    /// A meeting about a message, as Outlook's Reply with Meeting: its subject, its sender and
    /// the others it went to as guests (not the user), and its text as the description.
    pub fn meeting_from(s: &Session, p: &mailcraft_engine::mime::Parsed) -> Dialog {
        let mut f = crate::pim_dialogs::EventForm::new(s, None);
        f.summary = p.subject.clone();
        let mine = |e: &str| s.accounts.iter().any(|a| a.config.email.eq_ignore_ascii_case(e.trim()));
        let mut guests: Vec<mailcraft_engine::model::EmailAddress> = Vec::new();
        for a in std::iter::once(&p.from).chain(&p.to).chain(&p.cc) {
            if !a.email.trim().is_empty() && !mine(&a.email) && !guests.iter().any(|g| g.email.eq_ignore_ascii_case(&a.email)) {
                guests.push(a.clone());
            }
        }
        f.guests = mailcraft_engine::model::addr::format_list(&guests);
        f.description = p.text.chars().take(4000).collect();
        Dialog::Event(Box::new(f))
    }

    /// A folder's Properties, from what this computer keeps of it.
    pub fn folder_properties(s: &Session, f: &mailcraft_engine::model::Folder) -> Dialog {
        let kept = s.store.as_ref().and_then(|st| st.load_folder(&f.account, &f.path).ok()).unwrap_or_default();
        let total = if kept.messages.is_empty() { f.total as usize } else { kept.messages.len() };
        let unread = if kept.messages.is_empty() { f.unread as usize } else { kept.messages.iter().filter(|m| !m.flags.seen).count() };
        let email = s.account(&f.account).map(|a| a.config.email.clone()).unwrap_or_default();
        Dialog::FolderProperties {
            name: crate::i18n::folder(f),
            location: {
                let decoded = mailcraft_engine::model::utf7::decode(&f.path);
                match f.delimiter {
                    Some(d) => decoded.split(d).collect::<Vec<_>>().join(" › "),
                    None => decoded,
                }
            },
            account: email,
            total,
            unread,
            size: kept.messages.iter().map(|m| u64::from(m.size)).sum(),
        }
    }

    /// An event, opened at one of its occurrences.
    pub fn open_event(s: &Session, uid: &str, occurrence: i64) -> Option<Dialog> {
        crate::pim_dialogs::EventForm::open(s, uid, occurrence).map(|f| Dialog::Event(Box::new(f)))
    }

    pub fn new_contact() -> Dialog {
        Dialog::Contact(Box::default())
    }

    pub fn edit_contact(c: &mailcraft_engine::model::vcard::Card) -> Dialog {
        Dialog::Contact(Box::new(crate::pim_dialogs::ContactForm::edit(c)))
    }

    /// A new contact for someone in a message, their name and address filled in.
    pub fn contact_from(a: &mailcraft_engine::model::EmailAddress) -> Dialog {
        Dialog::Contact(Box::new(crate::pim_dialogs::ContactForm::from_address(a)))
    }

    pub fn edit_account(a: &AccountConfig) -> Dialog {
        Dialog::EditAccount(Box::new(EditForm {
            account: a.id.clone(),
            email: a.email.clone(),
            name: a.name.clone(),
            username: a.username.clone(),
            jmap: a.jmap.is_some(),
            jmap_url: a.jmap.clone().unwrap_or_default(),
            ews: a.ews.clone(),
            pop: a.pop.as_ref().map(|p| (p.host.clone(), p.port.to_string(), p.security)),
            pop_keep: PopKeep::of(a),
            imap_host: a.imap.host.clone(),
            imap_port: a.imap.port.to_string(),
            imap_security: a.imap.security,
            smtp_host: a.smtp.host.clone(),
            smtp_port: a.smtp.port.to_string(),
            smtp_security: a.smtp.security,
            smtp_sign_in: SmtpSignIn::of(a),
            online_only: a.online_only,
            error: None,
        }))
    }

    /// Automatic replies; `exchange`: the server keeps a reply for outside the organisation too.
    pub fn vacation(account: &str, v: &mailcraft_engine::model::job::Vacation, exchange: bool) -> Dialog {
        Dialog::Vacation(Box::new(VacationForm {
            account: account.to_string(),
            enabled: v.enabled,
            subject: v.subject.clone(),
            text: v.text.clone(),
            dated: v.from.is_some() || v.until.is_some(),
            from: v.from.clone().unwrap_or_default(),
            until: v.until.clone().unwrap_or_default(),
            exchange,
            split: v.external.is_some(),
            external: v.external.clone().unwrap_or_default(),
            outside: if v.outside.is_empty() { "all".into() } else { v.outside.clone() },
            error: None,
        }))
    }

    pub fn password(a: &AccountConfig) -> Dialog {
        Dialog::Password { account: a.id.clone(), email: a.email.clone(), password: String::new() }
    }

    /// Set the error shown in the add-account form.
    pub fn set_error(&mut self, e: String) {
        match self {
            Dialog::AddAccount(f) => f.error = Some(e),
            Dialog::FolderName { error, .. }
            | Dialog::MoveFolder { error, .. }
            | Dialog::DeleteFolder { error, .. }
            | Dialog::NewCategory { error, .. } => *error = Some(e),
            Dialog::EditRule(f) => f.error = Some(e),
            Dialog::SearchFolder(f) => f.error = Some(e),
            Dialog::QuickStep(f) => f.error = Some(e),
            Dialog::EditAccount(f) => f.error = Some(e),
            Dialog::Event(f) => f.error = Some(e),
            Dialog::Contact(f) => f.error = Some(e),
            Dialog::Vacation(f) => f.error = Some(e),
            Dialog::OpenSharedCalendar { error, .. } | Dialog::AddSharedMailbox { error, .. } => *error = Some(e),
            _ => {}
        }
    }

    /// A new rule for mail like this (from its sender, into Archive).
    pub fn rule_from(sender: &str, _subject: &str) -> Dialog {
        Dialog::EditRule(Box::new(RuleForm {
            index: None,
            name: tf!("Mail from {sender}", sender = sender),
            test: RuleTest::From,
            text: sender.to_string(),
            action: RuleDo::Archive,
            target: String::new(),
            account: None,
            stop: false,
            run_now: true,
            error: None,
        }))
    }

    /// The preview of an attachment, from `attachment.preview`'s answer.
    pub fn preview(v: &serde_json::Value, clock: &mailcraft_engine::model::dates::Clock) -> Dialog {
        let get = |k: &str| v.get(k).and_then(serde_json::Value::as_str).unwrap_or("").to_string();
        let mut header = Vec::new();
        if get("kind") == "message" {
            header.push((tl!("From").to_string(), get("from")));
            header.push((tl!("To").to_string(), get("to")));
            if let Some(d) = v.get("date").and_then(serde_json::Value::as_i64).filter(|d| *d > 0) {
                header.push((tl!("Date").to_string(), crate::i18n::date(&mailcraft_engine::model::dates::full_time(d, clock))));
            }
        }
        let mut text = get("text");
        if v.get("truncated").and_then(serde_json::Value::as_bool).unwrap_or(false) {
            text.push_str("\n… (the rest isn't shown: open the file to see it all)");
        }
        let title = if get("kind") == "message" && !get("subject").is_empty() { get("subject") } else { get("name") };
        let warning = v.get("warning").and_then(|w| serde_json::from_value(w.clone()).ok());
        Dialog::Preview { title, header, text, warning }
    }

    pub fn new_folder(account: &str, parent: Option<&str>) -> Dialog {
        Dialog::FolderName { account: account.into(), parent: parent.map(str::to_string), rename: None, name: String::new(), error: None }
    }

    pub fn rename_folder(f: &Folder) -> Dialog {
        Dialog::FolderName { account: f.account.clone(), parent: None, rename: Some(f.path.clone()), name: f.display_name(), error: None }
    }

    pub fn move_folder(f: &Folder) -> Dialog {
        Dialog::MoveFolder { account: f.account.clone(), folder: f.path.clone(), filter: String::new(), error: None }
    }

    /// Delete a folder: into Deleted Items, or for good when it's already there.
    pub fn delete_folder(s: &Session, f: &Folder) -> Dialog {
        let trash = s.account(&f.account).and_then(|a| a.folder(mailcraft_engine::model::FolderRole::Trash)).map(|t| t.path.clone());
        let permanent = trash.is_none_or(|t| mailcraft_engine::model::is_within(&f.path, &t, f.delimiter));
        Dialog::DeleteFolder { account: f.account.clone(), folder: f.path.clone(), name: f.display_name(), permanent, error: None }
    }
}

fn title(ui: &mut Ui, t: &Tokens, s: &str) {
    ui.label(RichText::new(tl!(s)).font(theme::bold(18.0)).color(t.text));
    ui.add_space(6.0);
}

/// A dialog's bottom row, right-aligned and one row tall: a layout given all the room left made
/// every dialog as tall as the last one shown, with its buttons floating halfway down.
fn button_row<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.allocate_ui_with_layout(vec2(ui.available_width(), 30.0), egui::Layout::right_to_left(egui::Align::Center), add).inner
}

/// A Close button on its own row; whether the dialog stays open.
fn close_row(ui: &mut Ui) -> bool {
    button_row(ui, |ui| !ui.button(tl!("Close")).clicked())
}

fn buttons(ui: &mut Ui, t: &Tokens, ok: &str, ok_enabled: bool) -> (bool, bool) {
    let mut res = (false, false);
    ui.add_space(10.0);
    button_row(ui, |ui| {
        if widgets::primary_button(ui, ok, t, ok_enabled).clicked() {
            res.0 = true;
        }
        if ui.button(tl!("Cancel")).clicked() {
            res.1 = true;
        }
    });
    res
}

/// Show the dialog; returns whether it stays open.
pub fn show(ctx: &egui::Context, d: &mut Dialog, s: &Session, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    let frame = dialog_frame(t);
    // Each kind of dialog keeps its own size: one id for all let a small one open at the size
    // of the last big one.
    // Never taller than the window (issue #1: an invitation's notes put the dialog's title and
    // buttons off a 2160-pixel screen): what doesn't fit scrolls.
    let max = (ctx.content_rect().height() - 48.0).max(200.0);
    let modal = egui::Modal::new(egui::Id::unique(("dialog", std::mem::discriminant(d)))).frame(frame).show(ctx, |ui| {
        // As tall as it needs, up to the window: only then does it scroll.
        egui::ScrollArea::vertical().id_salt("dialog-body").max_height(max).min_scrolled_height(max).show(ui, |ui| body(ui, d, s, t, acts)).inner
    });
    let keep = modal.inner;
    keep && !modal.should_close()
}

fn body(ui: &mut Ui, d: &mut Dialog, s: &Session, t: &Tokens, acts: &mut Vec<Action>) -> bool {
    // Each kind is drawn by its group; listing them all here keeps the match exhaustive.
    match d {
        Dialog::LostOnClose { .. } | Dialog::SaveOnClose(..) | Dialog::UpdateOnClose(..) => closing::draw(ui, d, s, t, acts),
        Dialog::AddAccount(..)
        | Dialog::Password { .. }
        | Dialog::RemoveAccount { .. }
        | Dialog::MailboxClosed { .. }
        | Dialog::EditAccount(..)
        | Dialog::AddSharedMailbox { .. } => accounts::draw(ui, d, s, t, acts),
        Dialog::Signature(..) | Dialog::Vacation(..) => mailbox::draw(ui, d, s, t, acts),
        Dialog::ConfirmDelete { .. }
        | Dialog::MoveTo { .. }
        | Dialog::FolderName { .. }
        | Dialog::MoveFolder { .. }
        | Dialog::DeleteFolder { .. }
        | Dialog::FolderProperties { .. }
        | Dialog::ConfirmEmpty { .. } => folders::draw(ui, d, s, t, acts),
        Dialog::NewCategory { .. } | Dialog::Rules | Dialog::EditRule(..) | Dialog::SearchFolder(..) | Dialog::QuickStep(..) => {
            organize::draw(ui, d, s, t, acts)
        }
        Dialog::SelectNames(..) | Dialog::SaveChanges | Dialog::EmptySubject | Dialog::SendCheck(_) | Dialog::WordCount(..) => {
            writing::draw(ui, d, s, t, acts)
        }
        Dialog::ConfirmDeleteContact { .. }
        | Dialog::PicturePreview { .. }
        | Dialog::Preview { .. }
        | Dialog::OpenSharedCalendar { .. }
        | Dialog::Certificate { .. }
        | Dialog::SmartCard { .. }
        | Dialog::Event(..)
        | Dialog::Contact(..) => other::draw(ui, d, s, t, acts),
    }
}

/// How every dialog looks (the main window's, and those over a message's own window).
pub(crate) fn dialog_frame(t: &Tokens) -> egui::Frame {
    egui::Frame::new().fill(t.surface).corner_radius(12).inner_margin(Margin::same(22)).stroke(egui::Stroke::new(1.0, t.border))
}
