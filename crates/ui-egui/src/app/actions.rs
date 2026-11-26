//! What the window was asked to do (`Action`), done; new mail announced. Each family's work
//! is in a piece of its own: data files, saving and attaching files, what's inserted into a
//! message, an account's keys.

mod data_files;
mod inserting;
mod names;
mod saving;
mod security;

use crate::*;

/// Something a pane asks for; run after drawing so panes only borrow what they read.
pub enum Action {
    Run(&'static str, Value),
    /// Close MailCraft though online-only accounts hold unsent mail or changes (issue #4).
    CloseAnyway,
    /// Close, install the downloaded version, and open it.
    RestartToUpdate,
    /// Closing, with the update asked about: installed as MailCraft closes (true), or kept for
    /// later (false).
    UpdateOnClose(bool),
    /// Closing, with a data file to compact: that account's compacted first, MailCraft closing
    /// once it's done (`Some`), or closed now (`None`). The account rides in the action: the
    /// dialog that asked is gone by the time it runs.
    CompactOnClose(Option<String>),
    Open(MessageKey),
    Dialog(Dialog),
    /// An action from one message's window: done to that message (several can be written at
    /// once, each in a window of its own).
    ForCompose(u64, Box<Action>),
    /// Check Names again, a directory it waited on having answered (issue #211).
    RecheckNames,
    /// Check the names typed in the meeting form's guests (issue #253); `true`: again, a
    /// directory it waited on having answered.
    CheckGuestNames(bool),
    /// The address book for the meeting form's guests (issue #253): the form comes back as it
    /// closes.
    GuestsFromBook,
    OpenAttachment(usize),
    SaveAttachment(usize),
    /// Something to tell the user, as the app's notices do (already in their language).
    Notice(String),
    /// A picture in an open message (`src` as it names it) saved as a file (issue #110).
    SavePicture {
        key: MessageKey,
        src: String,
    },
    AttachFiles,
    FocusSearch,
    /// Make a note and write on it.
    NewNote,
    /// A note's file (by its place among the note's files): opened in another app, or saved
    /// where asked.
    NoteFile {
        uid: String,
        index: usize,
        open: bool,
    },
    /// Everything with the person at this address (issue #178).
    Person(String),
    /// Someone else looked for from that dialog, by name or address: who it finds.
    PersonLookup(String),
    /// Search every folder for this (shown in the box, with the Search tab).
    Search(String),
    SetDark(Option<bool>),
    /// MailCraft made to start as the user signs in, or not (issue #87).
    StartAtSignIn(bool),
    /// Print the open message (as a PDF opened in the system's viewer).
    Print,
    /// Save the open message as a PDF, or as its `.eml` source.
    ExportPdf,
    SaveMessage,
    /// Open an attachment in a Craft app (its command name).
    OpenInCraft(usize, &'static str),
    /// Save every attachment of the open message into a folder.
    SaveAll,
    /// Save some attachments of the open message (by index) into a folder.
    SaveAttachments(Vec<usize>),
    /// The rules to a file, or rules from one (Outlook's Export and Import Rules).
    ExportRules,
    ImportRules,
    /// The category list to a file, or categories from one.
    ExportCategories,
    ImportCategories,
    /// Save a backup of what the browser keeps (issue #83), locked with the password when
    /// there's one, with the downloaded mail when asked; pick one to bring back; bring it back.
    Backup {
        password: String,
        mail: bool,
    },
    PickBackup,
    /// The archive file for Clean up old items, picked (issue #179).
    PickArchiveFile,
    Restore {
        data: std::sync::Arc<Vec<u8>>,
        password: String,
    },
    /// Pick a `.p12` / `.pfx` file for an account's certificate, then ask its password.
    ImportCertificate(String),
    /// Pick a certificate file (`.pem`, `.crt`, `.cer`) for an account's mail servers to trust:
    /// an authority the organisation runs, or a server's own (issue #105).
    TrustCertificateFile(String),
    /// The browser: open the account's certificate it keeps, with its password, for this page.
    UnlockCertificate(String),
    /// Pick an OpenPGP secret key file for an account, then ask its passphrase.
    ImportPgpKey(String),
    /// Pick a file of correspondents' OpenPGP keys for an account, and keep them.
    ImportPublicKey(String),
    /// Choose a certificate on a smart card (or in the system's keychain) for an account.
    UseSmartCard(String),
    /// Choose one of the system's own certificates (Windows' store, the macOS Keychain).
    UseSystemCertificate(String),
    /// Sign an account in again: the browser for OAuth accounts, else the password dialog.
    SignInAgain(String),
    /// Insert a picture at the cursor of the message being written (from a file).
    InsertPicture,
    /// A signature of the account (by name; `None`: the one in it, else its own for this kind
    /// of message) into the message being written: in place of the one there, else at the cursor.
    InsertSignature(Option<String>),
    /// Paste a picture from the clipboard, if there is one.
    PastePicture,
    /// A picture into the signature being edited: from a file, or (`paste`) the clipboard's.
    SignaturePicture {
        paste: bool,
    },
    /// A signature saved as a web page (an Outlook `.htm` signature) as the one being edited's
    /// HTML code, with the pictures beside it.
    SignatureFile,
    /// The text's right-click Paste: files on the clipboard are attached.
    PasteFiles,
    /// Choose the interface language (`auto` or a code).
    SetLanguage(String),
    /// Open the signature editor for an account (`insert`: from a message, putting it in).
    EditSignature {
        account: String,
        insert: bool,
    },
    /// A link in a message was clicked (web pages open in the browser; `mailto:` starts a message).
    OpenUrl(String),
    /// Run a command for the open dialog: it closes on success and shows the error otherwise.
    Submit(&'static str, Value),
    /// Bring the main window to the front (after acting from a message window).
    FocusMain,
    /// Open the File page at a page.
    Backstage(backstage::Page),
    /// Attachments are being dragged out of the window: hand them to the system as files.
    DragOut(Vec<usize>),
    /// Choose an Outlook data file and look at it before it's imported.
    OpenDataFile,
    /// A saved message to look at (`.eml`, `.msg`), picked here as one handed over by the system.
    OpenMailFile,
    /// Import the data file at `path` (`again`: though it's open as another account already;
    /// `changes`: what becomes of a file changed since MailCraft's copy of it was read;
    /// `take_over`: its copy is an account in another MailCraft data folder, and becomes this
    /// one's).
    ImportDataFile {
        path: String,
        again: bool,
        changes: Option<mailcraft_engine::Changes>,
        take_over: bool,
    },
    /// Show how an account's import is going, beside the status bar.
    ImportDetails(String),
    /// Choose an existing data file for the account being added (its mail comes in first).
    PickAccountDataFile,
    /// An action from a message's own window, on that message (its attachments, Save As, Print).
    InWindow(MessageKey, Box<Action>),
    /// Choose where to write an account's mail as an Outlook data file.
    ExportDataFile(String),
    /// Show a folder in the system's file manager.
    OpenFolder(PathBuf),
    /// Choose a folder to move MailCraft's data into (at the next start).
    MoveDataFolder,
    /// Show one message formatted (true) or as plain text, whatever the setting for all mail
    /// says (that stays as it is).
    ShowFormatted(MessageKey, bool),
    /// Forget a move asked for.
    CancelDataMove,
}

impl MailApp {
    /// New mail: a desktop notification and the unread count in the window title.
    pub(crate) fn announce(&mut self, ctx: &egui::Context) {
        // An account with no password kept (a portable copy's, at each start) asks for it, one
        // at a time, when nothing else is open (issue #4).
        if self.ui.dialog.is_none() && self.ui.backstage.is_none() && !self.session.ask_password.is_empty() {
            let id = self.session.ask_password.remove(0);
            if let Some(a) = self.session.account(&id).filter(|a| matches!(a.status, mailcraft_engine::model::SyncStatus::NeedsPassword { .. })) {
                self.ui.dialog = Some(Dialog::password(&a.config));
            }
        }
        // A mail server with a certificate no authority vouches for asks to be trusted, the same
        // way (issues #77 and #105).
        if self.ui.dialog.is_none() && self.ui.backstage.is_none() && !self.session.ask_certificate.is_empty() {
            let (account, shown) = self.session.ask_certificate.remove(0);
            self.ui.dialog = Some(Dialog::TrustCertificate { account, shown: Box::new(shown) });
        }
        // AutoArchive is due and asks first (issue #180), when nothing else is open.
        if self.ui.dialog.is_none() && self.ui.backstage.is_none() && self.session.take_auto_archive_ask() {
            self.ui.dialog = Some(Dialog::AutoArchiveDue);
        }
        let news = self.session.take_new_mail();
        if !news.is_empty() {
            // Issue #4: the system keeps its notifications (Windows, in its history): one for an
            // account kept online only names neither sender nor subject.
            let private = news.iter().any(|m| self.session.account(&m.key.account).is_some_and(|a| a.config.keeps_online()));
            let (title, body) = match news.as_slice() {
                _ if private => (
                    if news.len() == 1 { tl!("New message").to_string() } else { tf!("{n} new messages", n = news.len()) },
                    tl!("Open MailCraft to read it.").to_string(),
                ),
                [m] => (m.from.display().to_string(), if m.subject.is_empty() { tl!("(no subject)").to_string() } else { m.subject.clone() }),
                many => {
                    let mut who: Vec<&str> = Vec::new();
                    for m in many.iter().rev() {
                        let n = m.from.display();
                        if !who.contains(&n) {
                            who.push(n);
                        }
                    }
                    let names = match who.as_slice() {
                        [a] => (*a).to_string(),
                        [a, b] => tf!("{a} and {b}", a = a, b = b),
                        [a, b, ..] => tf!("{a}, {b} and others", a = a, b = b),
                        [] => String::new(),
                    };
                    (tf!("{n} new messages", n = many.len()), tf!("From {names}", names = names))
                }
            };
            // Only the system notification and the title's unread count announce mail. No
            // RequestUserAttention: Windows keeps that highlight on the taskbar until focus returns.
            // Clicked, it shows the newest of them (issue #273).
            let newest = news.iter().max_by_key(|m| m.date).map(|m| m.key.clone());
            self.services.notify_about(&title, &body, newest);
        }
        let unread = self.session.inbox_unread();
        let view = match self.session.module {
            Module::Mail => view_title(&self.session),
            Module::Calendar => tl!("Calendar").to_string(),
            Module::People => tl!("People").to_string(),
            Module::Tasks => tl!("Tasks").to_string(),
            Module::Notes => tl!("Notes").to_string(),
        };
        let base = tl!(&self.ui.title_base).to_string();
        let title = if self.session.view == mailcraft_engine::View::Empty {
            base.to_string()
        } else if unread > 0 {
            format!("({unread}) {view} – {base}")
        } else {
            format!("{view} – {base}")
        };
        if title != self.ui.title_shown {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.ui.title_shown = title;
        }
    }

    /// Do what was asked: each family's work is in a piece of its own (`actions/`), the match
    /// only dispatches. The browser's ways (a picker that answers later, a download for a file
    /// saved) are decided in each piece, not by the order of this match's arms.
    pub(crate) fn act(&mut self, a: Action) {
        match a {
            // Done to the message whose window asked; a dialog it opens shows there.
            Action::ForCompose(id, inner) => {
                if self.use_compose(id) {
                    if matches!(*inner, Action::Dialog(ref d) if d.for_compose()) {
                        self.ui.dialog_for = Some(id);
                    }
                    self.act(*inner);
                }
            }
            Action::InWindow(key, inner) => {
                // The window's message stands in as the open one while the action runs. The
                // main window's copy only once it's read: the window's message is opened there as
                // its action begins, and read on another thread, and an attachment dragged out
                // of the window was named "attachment", not found in it yet (issue #120).
                if self.session.reading.as_ref().is_some_and(|r| r.key == key && r.parsed.is_some()) {
                    self.act(*inner);
                } else if let Some(w) = self.session.windows.iter().find(|w| w.key == key).cloned() {
                    let main = self.session.reading.replace(w);
                    self.act(*inner);
                    self.session.reading = main;
                }
            }
            // Closing.
            Action::CloseAnyway => self.close_anyway(),
            Action::RestartToUpdate => self.restart_to_update(),
            Action::CompactOnClose(account) => self.compact_on_close(account),
            Action::UpdateOnClose(install) => self.update_on_close(install),
            // Commands, and what the window does itself.
            Action::Run(id, p) => self.act_run(id, p),
            Action::RecheckNames => self.check_names(true),
            Action::CheckGuestNames(again) => self.check_guest_names(again),
            Action::GuestsFromBook => self.guests_from_book(),
            Action::Submit(id, p) => self.submit(id, p),
            Action::Open(k) => self.run("list.open", json!({"key": k.to_id()})),
            Action::Dialog(d) => self.ui.dialog = Some(d),
            Action::FocusSearch => self.ui.focus_search = true,
            Action::FocusMain => self.ui.focus_main = true,
            Action::Backstage(p) => self.ui.backstage = Some(backstage::Backstage::new(p)),
            Action::NewNote => self.new_note(),
            Action::Search(q) => self.search_everywhere(q),
            // In the main window, brought forward: asked for from a message's own window too.
            Action::Person(email) => {
                self.ui.dialog = Some(Dialog::person(&mut self.session, &email));
                self.ui.focus_main = true;
            }
            Action::PersonLookup(text) => {
                if let Some(Dialog::Person(f)) = &mut self.ui.dialog {
                    f.people_found =
                        if text.trim().is_empty() { Vec::new() } else { self.session.suggest_contacts(&text, crate::dialogs::PEOPLE_OFFERED) };
                }
            }
            // Not while the organisation has the theme locked (issue #245).
            Action::SetDark(d) => {
                if !self.session.org.theme.as_ref().is_some_and(|g| g.locked) {
                    self.ui.dark = d;
                }
            }
            // The switch then says what the system has, read back.
            Action::StartAtSignIn(on) => match self.services.set_start_at_sign_in(on) {
                Ok(()) => self.ui.at_sign_in = self.services.starts_at_sign_in(),
                Err(e) => self.session.notice(tf!("Starting MailCraft at sign-in couldn't be changed: {error}", error = e), true),
            },
            Action::ShowFormatted(key, on) => self.ui.shown_as.set(&key, on, self.session.prefs.show_html),
            Action::SetLanguage(l) => {
                if !self.session.org.language.as_ref().is_some_and(|g| g.locked) {
                    self.ui.language = l;
                }
            }
            Action::OpenUrl(url) => self.open_link(&url),
            Action::SignInAgain(id) => self.sign_in_again(&id),
            // Data files and the data folder (`actions/data_files.rs`).
            Action::PickAccountDataFile => self.pick_account_data_file(),
            Action::OpenMailFile => self.open_mail_file(),
            Action::OpenDataFile => self.open_data_file(),
            Action::ImportDataFile { path, again, changes, take_over } => self.import_data_file(path, again, changes, take_over),
            Action::ImportDetails(account) => self.ui.import_details = Some(account),
            Action::ExportDataFile(account) => self.export_data_file(&account),
            Action::OpenFolder(path) => self.open_folder(&path),
            Action::MoveDataFolder => self.move_data_folder(),
            Action::CancelDataMove => self.cancel_data_move(),
            // Rules, categories and the browser's backup, to files and from them.
            Action::ExportRules => self.export_rules(),
            Action::ImportRules => self.import_rules(),
            Action::ExportCategories => self.export_categories(),
            Action::ImportCategories => self.import_categories(),
            // The browser's picker answers later (with the file's bytes).
            Action::Backup { password, mail } => self.save_backup(&password, mail),
            Action::PickBackup => self.pick_backup(),
            Action::PickArchiveFile => self.pick_archive_file(),
            Action::Restore { data, password } => self.restore_backup(&data, &password),
            // An account's keys (`actions/security.rs`).
            Action::ImportCertificate(account) => self.import_certificate(account),
            Action::TrustCertificateFile(account) => self.trust_certificate_file(account),
            Action::UnlockCertificate(account) => {
                self.ui.dialog = Some(Dialog::Certificate { account, path: String::new(), data: None, password: String::new() });
            }
            Action::ImportPgpKey(account) => self.import_pgp_key(account),
            Action::ImportPublicKey(account) => self.import_public_key(&account),
            Action::UseSmartCard(account) => self.use_smart_card(account),
            Action::UseSystemCertificate(account) => self.use_system_certificate(account),
            // The message and its attachments to files (`actions/saving.rs`).
            Action::Print => self.print(),
            Action::ExportPdf => self.save_message(true),
            Action::SaveMessage => self.save_message(false),
            Action::SaveAttachment(i) => self.save_attachment(i),
            Action::Notice(text) => self.session.notice(text, true),
            Action::SavePicture { key, src } => self.save_picture(&key, &src),
            Action::OpenAttachment(i) => self.open_attachment(i),
            Action::SaveAll => self.save_attachments(None),
            Action::SaveAttachments(v) => self.save_attachments(Some(v)),
            Action::OpenInCraft(i, cmd) => self.open_in_craft(i, cmd),
            Action::DragOut(v) => self.drag_out(v),
            Action::NoteFile { uid, index, open } => self.note_file(&uid, index, open),
            // Into the message being written or the signature being edited (`actions/inserting.rs`).
            Action::EditSignature { account, insert } => {
                self.ui.dialog = Some(Dialog::Signature(Box::new(dialogs::SignatureForm::new(&self.session, &account, insert))));
            }
            Action::InsertSignature(name) => self.insert_signature(name),
            Action::InsertPicture => self.insert_picture(),
            Action::PastePicture => self.paste_picture(),
            Action::SignaturePicture { paste } => self.signature_picture(paste),
            Action::SignatureFile => self.signature_file(),
            Action::PasteFiles => self.paste_files(),
            Action::AttachFiles => self.attach_files(),
        }
    }

    /// A command asked for, with what the window does about some first (or instead).
    fn act_run(&mut self, id: &'static str, p: Value) {
        // Deleting from Deleted Items is for good: asked first (the answer deletes, `permanent`).
        if id == "mail.delete" && p.get("permanent").is_none() && self.ask_before_deleting() {
            return;
        }
        if id == "list.rowAction" && p.get("permanent").is_none() && self.ask_before_row_delete(&p) {
            return;
        }
        match id {
            "attachment.preview" => match self.session.run(id, &p) {
                Ok(v) => self.ui.dialog = Some(Dialog::preview(&v, &self.session.clock)),
                Err(e) => self.session.notice(e.to_string(), true),
            },
            // Check Names: who's several people asked about, who's no one said (issue #211).
            "compose.checkNames" => self.check_names(false),
            // A message's source in its window, brought to the front (asked for again while open).
            "message.source" => match self.session.run(id, &p) {
                Ok(v) => self.ui.raise_source = v.get("key").and_then(Value::as_str).and_then(MessageKey::from_id),
                Err(e) => self.session.notice(e.to_string(), true),
            },
            _ => {
                // A Search Folder isn't a search typed in: the box is left empty.
                if matches!(id, "search.clear" | "searchFolder.open") {
                    self.ui.search_text.clear();
                }
                self.run(id, p);
                if super::keys::moves_focus(id) {
                    self.ui.scroll_to_focus = true;
                }
            }
        }
    }

    /// A command for the open dialog: it closes when it's done, else says why.
    fn submit(&mut self, id: &'static str, p: Value) {
        match self.session.run(id, &p) {
            Ok(v) => {
                if id == "contact.create" {
                    self.ui.person = v.get("uid").and_then(Value::as_str).map(str::to_string);
                }
                // A task made is the one picked in the list.
                if id == "task.create" {
                    self.ui.task = v.get("uid").and_then(Value::as_str).map(str::to_string);
                }
                // A Search Folder made or changed opens: not a search typed in.
                if self.session.open_search_folder_name().is_some() {
                    self.ui.search_text.clear();
                }
                self.ui.dialog = None;
                // A rule saved to be run now: run, now that it is.
                if id == "rules.set"
                    && let Some(at) = p.get("runNow").and_then(Value::as_u64)
                {
                    self.run("rules.run", json!({"index": at, "inbox": true}));
                }
            }
            // Said on the dialog's own error line, or as a notice when it has none (or
            // none is open): never dropped.
            Err(e) => {
                let said = e.to_string().trim_start_matches("bad parameters: ").to_string();
                let shown = match self.ui.dialog.as_mut() {
                    Some(d) => d.set_error(said.clone()),
                    None => false,
                };
                if !shown {
                    self.session.notice(said, true);
                }
            }
        }
    }

    /// A note made, to write on.
    fn new_note(&mut self) {
        match self.session.run("note.create", &json!({})) {
            Ok(v) => {
                if let Some(uid) = v.get("uid").and_then(Value::as_str) {
                    self.ui.notes_query.clear();
                    self.ui.note_edit = Some((uid.to_string(), String::new()));
                    self.ui.note_focus = true;
                }
            }
            Err(e) => self.session.notice(e.to_string(), true),
        }
    }

    /// Every folder searched for `q`, shown in the box with the Search tab.
    fn search_everywhere(&mut self, q: String) {
        self.ui.search_text = q.clone();
        self.ui.tab = RibbonTab::Search;
        self.run("search.run", json!({"query": q, "scope": "all"}));
    }

    /// Close though there's something to say first: a data file being imported, then what
    /// becomes of an update, are still said before MailCraft goes; the answer closes (and
    /// Cancel there leaves the next close asking everything again).
    fn close_anyway(&mut self) {
        if super::importing::ask_on_close(&self.session) {
            self.ask_import_on_close();
        } else if self.ask_compact_on_close() {
            // Data files compacted first, when chosen; closing goes on once they're done.
        } else if self.ask_about_update() {
            self.ask_update_on_close();
        } else {
            self.ui.closing_anyway = true;
            self.ui.close_now = true;
        }
    }

    /// Committed only once nothing stops the close (`frame.rs`): a message being written can
    /// still be kept, and Cancel there leaves MailCraft as it was.
    fn restart_to_update(&mut self) {
        if self.ui.update.ready().is_some() {
            self.ui.restart_wanted = true;
            self.ui.update_answered = true;
            self.ui.close_now = true;
        }
    }

    fn update_on_close(&mut self, install: bool) {
        if !install {
            self.services.keep_update_for_later();
        }
        self.ui.update_answered = true;
        self.ui.closing_anyway = true;
        self.ui.close_now = true;
    }

    pub(crate) fn sign_in_again(&mut self, id: &str) {
        use mailcraft_engine::model::FailureKind;
        let Some((config, status)) = self.session.account(id).map(|a| (a.config.clone(), a.status.clone())) else { return };
        match status {
            // A server with a certificate no authority vouches for: asked about again.
            SyncStatus::Error { kind: FailureKind::UntrustedCertificate(shown), .. } => {
                self.ui.dialog = Some(Dialog::TrustCertificate { account: id.to_string(), shown });
                return;
            }
            // Microsoft took the sign-in but won't open the mailbox: say why, and what helps.
            SyncStatus::Error { error, kind: FailureKind::MicrosoftImapClosed } => {
                self.ui.dialog = Some(Dialog::MailboxClosed { account: id.to_string(), email: config.email.clone(), error });
                return;
            }
            _ => {}
        }
        // A Microsoft account with a password, which Microsoft refuses: Sign in with Microsoft.
        if config.oauth.is_some()
            || mailcraft_engine::model::microsoft_password_account(&config) && self.session.oauth_available.iter().any(|p| p == "microsoft")
        {
            self.run("account.signIn", json!({"account": id}));
        } else {
            self.ui.dialog = Some(Dialog::password(&config));
        }
    }

    /// Follow a link from a message: only web pages and mail addresses, never local files or
    /// other schemes. A `mailto:` link is read as one the system hands MailCraft is (RFC 6068:
    /// its addresses decoded, its subject, copies and first words taken, nothing attached).
    pub(crate) fn open_link(&mut self, url: &str) {
        // Where it really goes, without what says who clicked (File › Options › Safety).
        let cleaned = if self.session.prefs.clean_links { mailcraft_engine::mime::clean_link(url) } else { url.to_string() };
        let u = cleaned.trim();
        let lower = u.to_ascii_lowercase();
        if lower.starts_with("mailto:") {
            self.run("mail.mailto", json!({"url": u}));
        } else if lower.starts_with("https://") || lower.starts_with("http://") {
            if let Err(e) = self.services.open_url(u) {
                self.session.notice(tf!("The link couldn't be opened: {error}", error = e), true);
            }
        } else {
            self.session.notice(tf!("Links like {link} aren't opened", link = widgets::shorten(u, 40)), true);
        }
    }
}
