//! What the window was asked to do (`Action`), done; new mail announced.

use crate::*;

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
            self.services.notify(&title, &body);
        }
        let unread = self.session.inbox_unread();
        let view = match self.session.module {
            Module::Mail => view_title(&self.session),
            Module::Calendar => tl!("Calendar").to_string(),
            Module::People => tl!("People").to_string(),
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
            Action::CloseAnyway => {
                // What becomes of an update is still said before MailCraft goes; its answer closes
                // (and Cancel there leaves the next close asking everything again).
                if self.ask_about_update() {
                    self.ask_update_on_close();
                } else {
                    self.ui.closing_anyway = true;
                    self.ui.close_now = true;
                }
            }
            // Committed only once nothing stops the close (`frame.rs`): a message being written can
            // still be kept, and Cancel there leaves MailCraft as it was.
            Action::RestartToUpdate => {
                if self.ui.update.ready().is_some() {
                    self.ui.restart_wanted = true;
                    self.ui.update_answered = true;
                    self.ui.close_now = true;
                }
            }
            Action::UpdateOnClose(install) => {
                if !install {
                    self.services.keep_update_for_later();
                }
                self.ui.update_answered = true;
                self.ui.closing_anyway = true;
                self.ui.close_now = true;
            }
            Action::Run("attachment.preview", p) => match self.session.run("attachment.preview", &p) {
                Ok(v) => self.ui.dialog = Some(Dialog::preview(&v, &self.session.clock)),
                Err(e) => self.session.notice(e.to_string(), true),
            },
            Action::Run(id, p) => {
                // A Search Folder isn't a search typed in: the box is left empty.
                if matches!(id, "search.clear" | "searchFolder.open") {
                    self.ui.search_text.clear();
                }
                self.run(id, p);
                if matches!(id, "list.next" | "list.prev") {
                    self.ui.scroll_to_focus = true;
                }
            }
            Action::Open(k) => {
                self.run("list.open", json!({"key": k.to_id()}));
            }
            Action::Dialog(d) => self.ui.dialog = Some(d),
            Action::FocusSearch => self.ui.focus_search = true,
            Action::NewNote => match self.session.run("note.create", &json!({})) {
                Ok(v) => {
                    if let Some(uid) = v.get("uid").and_then(Value::as_str) {
                        self.ui.notes_query.clear();
                        self.ui.note_edit = Some((uid.to_string(), String::new()));
                        self.ui.note_focus = true;
                    }
                }
                Err(e) => self.session.notice(e.to_string(), true),
            },
            Action::Search(q) => {
                self.ui.search_text = q.clone();
                self.ui.tab = RibbonTab::Search;
                self.run("search.run", json!({"query": q, "scope": "all"}));
            }
            Action::FocusMain => self.ui.focus_main = true,
            Action::Backstage(p) => self.ui.backstage = Some(backstage::Backstage::new(p)),
            Action::InWindow(key, inner) => {
                // The window's message stands in as the open one while the action runs.
                if self.session.reading.as_ref().is_some_and(|r| r.key == key) {
                    self.act(*inner);
                } else if let Some(w) = self.session.windows.iter().find(|w| w.key == key).cloned() {
                    let main = self.session.reading.replace(w);
                    self.act(*inner);
                    self.session.reading = main;
                }
            }
            Action::PickAccountDataFile => {
                if let Some(path) = self.services.pick_data_file()
                    && let Some(dialogs::Dialog::AddAccount(f)) = self.ui.dialog.as_mut()
                {
                    f.pst = path.to_string_lossy().into_owned();
                }
            }
            Action::OpenDataFile => {
                if let Some(path) = self.services.pick_data_file() {
                    match self.session.run("account.importPst", &json!({"path": path.to_string_lossy()})) {
                        Ok(_) => {
                            self.ui.backstage = None;
                            self.session.notice(tf!("Opening {file}…", file = path.file_name().unwrap_or_default().to_string_lossy()), false);
                        }
                        Err(e) => self.session.notice(e.to_string(), true),
                    }
                }
            }
            Action::ExportDataFile(account) => {
                // Named after the account's address, as Outlook names a data file.
                let name = self
                    .session
                    .account(&account)
                    .map(|a| if a.config.email.trim().is_empty() { a.config.name.clone() } else { a.config.email.clone() })
                    .unwrap_or_default();
                let name = mailcraft_engine::mime::sanitize_filename(if name.trim().is_empty() { "Mail" } else { name.trim() });
                if let Some(path) = self.services.save_path(&format!("{name}.pst")) {
                    match self.session.run("account.exportPst", &json!({"account": account, "path": path.to_string_lossy()})) {
                        Ok(_) => {
                            self.ui.backstage = None;
                            self.session.notice(tf!("Exporting to {file}…", file = path.file_name().unwrap_or_default().to_string_lossy()), false);
                        }
                        Err(e) => self.session.notice(e.to_string(), true),
                    }
                }
            }
            Action::ExportRules => {
                if let Some(path) = self.services.save_path("MailCraft rules.json") {
                    match self.session.run("rules.export", &json!({"path": path.to_string_lossy()})) {
                        Ok(v) => {
                            let n = v.get("rules").and_then(Value::as_u64).unwrap_or(0);
                            let file = path.file_name().unwrap_or_default().to_string_lossy().to_string();
                            self.session.notice(tf!("Rules saved in {file}: {n}", n = n, file = file), false);
                        }
                        Err(e) => self.session.notice(e.to_string(), true),
                    }
                }
            }
            // The browser's picker answers later (with the file's bytes).
            Action::ImportCertificate(account) if self.services.pick_later(&format!("certificate:{account}")) => {}
            Action::ImportCertificate(account) => {
                if let Some(path) = self.services.pick_certificate() {
                    self.ui.dialog =
                        Some(Dialog::Certificate { account, path: path.to_string_lossy().into_owned(), data: None, password: String::new() });
                }
            }
            Action::UnlockCertificate(account) => {
                self.ui.dialog = Some(Dialog::Certificate { account, path: String::new(), data: None, password: String::new() });
            }
            Action::UseSmartCard(account) => {
                let module = self.session.card_search().modules.first().cloned().unwrap_or_default();
                self.ui.dialog = Some(Dialog::SmartCard { account, module, pin: String::new(), system: false });
            }
            Action::UseSystemCertificate(account) => {
                self.run("security.findKeys", json!({"keychain": true}));
                self.ui.dialog = Some(Dialog::SmartCard { account, module: String::new(), pin: String::new(), system: true });
            }
            Action::ImportRules => {
                if let Some(path) = self.services.pick_rules_file() {
                    match self.session.run("rules.import", &json!({"path": path.to_string_lossy()})) {
                        Ok(v) => {
                            let n = v.get("added").and_then(Value::as_u64).unwrap_or(0);
                            let off = v.get("turnedOff").and_then(Value::as_u64).unwrap_or(0);
                            let text = if off > 0 {
                                tf!("Rules imported: {n} ({off} turned off, as their account isn't on this computer)", n = n, off = off)
                            } else {
                                tf!("Rules imported: {n}", n = n)
                            };
                            self.session.notice(text, false);
                        }
                        Err(e) => self.session.notice(e.to_string(), true),
                    }
                }
            }
            Action::OpenFolder(path) => {
                if let Err(e) = self.services.open_path(&path) {
                    self.session.notice(e, true);
                }
            }
            Action::MoveDataFolder => {
                if let Some(to) = self.services.pick_folder() {
                    match self.services.move_data_folder(Some(&to)) {
                        Ok(Some(target)) if self.ui.data_folder.as_ref().is_some_and(|(d, _)| *d == target) => {
                            self.ui.pending_move = None;
                        }
                        Ok(target) => {
                            self.ui.pending_move = target;
                            self.session.notice(tl!("Your mail moves there the next time MailCraft starts.").to_string(), false);
                        }
                        Err(e) => self.session.notice(e, true),
                    }
                }
            }
            Action::CancelDataMove => {
                if let Err(e) = self.services.move_data_folder(None) {
                    self.session.notice(e, true);
                }
                self.ui.pending_move = None;
            }
            Action::DragOut(i) => {
                if self.services.can_drag_out() {
                    self.ui.drag_out = self.attachment_to_temp(i, false);
                }
            }
            Action::Submit(id, p) => match self.session.run(id, &p) {
                Ok(v) => {
                    if id == "contact.create" {
                        self.ui.person = v.get("uid").and_then(Value::as_str).map(str::to_string);
                    }
                    // A Search Folder made or changed opens: not a search typed in.
                    if self.session.open_search_folder_name().is_some() {
                        self.ui.search_text.clear();
                    }
                    self.ui.dialog = None;
                }
                Err(e) => {
                    if let Some(d) = self.ui.dialog.as_mut() {
                        d.set_error(e.to_string().trim_start_matches("bad parameters: ").to_string());
                    }
                }
            },
            Action::SetDark(d) => self.ui.dark = d,
            Action::SetLanguage(l) => self.ui.language = l,
            Action::OpenUrl(url) => self.open_link(&url),
            Action::SignInAgain(id) => self.sign_in_again(&id),
            Action::OpenInCraft(i, cmd) => {
                let Some(app) = self.ui.craft_apps.iter().find(|(c, _)| *c == cmd).map(|(_, p)| p.clone()) else { return };
                if let Some(path) = self.attachment_to_temp(i, true)
                    && let Err(e) = self.services.open_with(&app, &path)
                {
                    self.session.notice(tf!("{app} couldn't open it: {error}", app = app.display(), error = e), true);
                }
            }
            Action::Print | Action::ExportPdf if self.services.downloads() => {
                let name = format!("{}.pdf", self.message_file_name());
                match self.session.message_pdf() {
                    Ok(b) => self.services.download(&name, &b),
                    Err(e) => self.session.notice(e.to_string(), true),
                }
            }
            Action::SaveMessage if self.services.downloads() => {
                let name = format!("{}.eml", self.message_file_name());
                match self.session.message_source() {
                    Ok(b) => self.services.download(&name, &b),
                    Err(e) => self.session.notice(e.to_string(), true),
                }
            }
            Action::SaveAttachment(i) | Action::OpenAttachment(i) if self.services.downloads() => {
                let name = mailcraft_engine::mime::sanitize_filename(&self.attachment_name(i));
                match self.session.attachment_bytes(i) {
                    Ok(b) => self.services.download(&name, &b),
                    Err(e) => self.session.notice(e.to_string(), true),
                }
            }
            Action::SaveAll if self.services.downloads() => {
                let files: Vec<(usize, String)> = self
                    .session
                    .reading
                    .as_ref()
                    .and_then(|r| r.parsed.as_ref())
                    .map(|p| p.attachments.iter().filter(|a| !a.inline).map(|a| (a.index, a.name.clone())).collect())
                    .unwrap_or_default();
                for (i, name) in files {
                    if let Ok(b) = self.session.attachment_bytes(i) {
                        self.services.download(&mailcraft_engine::mime::sanitize_filename(&name), &b);
                    }
                }
            }
            Action::InsertPicture if self.services.pick_later("picture") => {}
            Action::AttachFiles if self.services.pick_later("attach") => {}
            Action::Print => {
                // The message as a PDF, saved where asked, then opened to print from the viewer.
                let Some(path) = self.services.save_path(&format!("{}.pdf", self.message_file_name())) else { return };
                match self.session.run("message.exportPdf", &json!({"path": path.to_string_lossy()})) {
                    Ok(_) => match self.services.open_path(&path) {
                        Ok(()) => self.session.notice(tl!("Opened as a PDF: print it from the viewer"), false),
                        Err(e) => self.session.notice(tf!("The PDF couldn't be opened: {error}", error = e), true),
                    },
                    Err(e) => self.session.notice(e.to_string(), true),
                }
            }
            Action::ExportPdf | Action::SaveMessage => {
                let (ext, id) = if matches!(a, Action::ExportPdf) { ("pdf", "message.exportPdf") } else { ("eml", "message.saveAs") };
                if let Some(path) = self.services.save_path(&format!("{}.{ext}", self.message_file_name())) {
                    match self.session.run(id, &json!({"path": path.to_string_lossy()})) {
                        Ok(_) => self.session.notice(tf!("Saved {path}", path = path.display()), false),
                        Err(e) => self.session.notice(e.to_string(), true),
                    }
                }
            }
            Action::SaveAll | Action::SaveAttachments(_) => {
                let indices = if let Action::SaveAttachments(v) = &a { json!(v) } else { Value::Null };
                if let Some(dir) = self.services.pick_folder() {
                    match self.session.run("attachment.saveAll", &json!({"dir": dir.to_string_lossy(), "indices": indices})) {
                        Ok(v) => {
                            let n = v.get("saved").and_then(Value::as_array).map_or(0, Vec::len);
                            self.session.notice(tf!("Saved {n} attachments in {folder}", n = n, folder = dir.display()), false);
                        }
                        Err(e) => self.session.notice(e.to_string(), true),
                    }
                }
            }
            Action::EditSignature { account, insert } => {
                self.ui.dialog = Some(Dialog::Signature(Box::new(dialogs::SignatureForm::new(&self.session, &account, insert))));
            }
            Action::InsertSignature => {
                let r = self.session.run("compose.insertSignature", &json!({"at": self.picture_at()}));
                match r {
                    Ok(v) => {
                        // The cursor after it.
                        let end = v.get("end").and_then(serde_json::Value::as_u64).and_then(|e| usize::try_from(e).ok()).unwrap_or(0);
                        if let Some(ed) = self.ui.editor.as_mut() {
                            ed.refocus = Some(end..end);
                        }
                    }
                    Err(e) => self.session.notice(e.to_string(), true),
                }
            }
            Action::InsertPicture => {
                for p in self.services.pick_pictures() {
                    let r = self.session.run("compose.insertPicture", &json!({"path": p.to_string_lossy(), "at": self.picture_at()}));
                    self.place_picture(r);
                }
            }
            Action::PastePicture => {
                // Formatted text that came with a picture (from Word or a web page) was pasted
                // as text.
                let text = clipboard_html().is_some_and(|h| !mailcraft_engine::mime::pasted_html(&h).0.trim().is_empty());
                if !text && let Some(png) = self.services.clipboard_picture() {
                    let r = self.session.run("compose.insertPicture", &json!({"name": "pasted.png", "data": png, "at": self.picture_at()}));
                    self.place_picture(r);
                }
            }
            Action::AttachFiles => {
                for p in self.services.pick_files() {
                    self.run("compose.attach", json!({"path": p.to_string_lossy()}));
                }
            }
            Action::SaveAttachment(i) => {
                let name = self.attachment_name(i);
                if let Some(path) = self.services.save_path(&name) {
                    self.run("attachment.save", json!({"index": i, "path": path.to_string_lossy()}));
                }
            }
            Action::OpenAttachment(i) => {
                let name = self.attachment_name(i);
                if let Some(path) = self.attachment_to_temp(i, true)
                    && let Err(e) = self.services.open_path(&path)
                {
                    self.session.notice(tf!("{name} could not be opened: {error}", name = name, error = e), true);
                }
            }
        }
    }

    pub(crate) fn sign_in_again(&mut self, id: &str) {
        let Some((config, status)) = self.session.account(id).map(|a| (a.config.clone(), a.status.clone())) else { return };
        // Microsoft took the sign-in but won't open the mailbox: say why, and what helps.
        if let SyncStatus::Error { error } = &status
            && error.starts_with(mailcraft_engine::model::MICROSOFT_IMAP_CLOSED)
        {
            self.ui.dialog = Some(Dialog::MailboxClosed { account: id.to_string(), email: config.email.clone(), error: error.clone() });
            return;
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

    /// Put a picture's mark at the body's cursor.
    /// Where a picture goes: at the cursor in the message.
    pub(crate) fn picture_at(&self) -> usize {
        let n = self.session.compose.as_ref().map_or(0, |c| c.body.chars().count());
        self.ui.editor.as_ref().filter(|e| self.session.compose.as_ref().is_some_and(|c| c.id == e.id)).map_or(n, |e| e.sel.start.min(n))
    }

    pub(crate) fn place_picture(&mut self, r: mailcraft_engine::CmdResult) {
        match r {
            // In place: the cursor goes after it.
            Ok(_) => {
                if let Some(e) = self.ui.editor.as_mut() {
                    e.sel = e.sel.start + 1..e.sel.start + 1;
                }
            }
            Err(e) => self.session.notice(e.to_string(), true),
        }
    }

    /// Follow a link from a message: only web pages and mail addresses, never local files or
    /// other schemes.
    pub(crate) fn open_link(&mut self, url: &str) {
        // Where it really goes, without what says who clicked (File › Options › Safety).
        let cleaned = if self.session.prefs.clean_links { mailcraft_engine::mime::clean_link(url) } else { url.to_string() };
        let u = cleaned.trim();
        let lower = u.to_ascii_lowercase();
        if let Some(rest) = u.get(7..).filter(|_| lower.starts_with("mailto:")) {
            let to = rest.split('?').next().unwrap_or("").replace("%40", "@");
            self.run("mail.new", json!({}));
            self.run("compose.update", json!({"to": to}));
        } else if lower.starts_with("https://") || lower.starts_with("http://") {
            if let Err(e) = self.services.open_url(u) {
                self.session.notice(tf!("The link couldn't be opened: {error}", error = e), true);
            }
        } else {
            self.session.notice(tf!("Links like {link} aren't opened", link = widgets::shorten(u, 40)), true);
        }
    }

    /// A file name for the open message (its subject, made safe).
    pub(crate) fn message_file_name(&self) -> String {
        let subject = self.session.reading.as_ref().and_then(|r| r.parsed.as_ref()).map(|p| p.subject.clone()).unwrap_or_default();
        let name = mailcraft_engine::mime::sanitize_filename(if subject.trim().is_empty() { "message" } else { &subject });
        name.chars().take(80).collect()
    }

    /// Write an attachment to a temporary file to hand to another app: dragged out (a copy, as
    /// saving), or to be opened (never a program, script or shortcut: the engine refuses).
    pub(crate) fn attachment_to_temp(&mut self, i: usize, open: bool) -> Option<PathBuf> {
        if !self.ui.has_files {
            return None;
        }
        let name = self.attachment_name(i);
        let Some(dir) = mailcraft_engine::store::scratch_dir("open") else {
            self.session.notice(tl!("There's no folder to put the attachment in for another app"), true);
            return None;
        };
        let path = dir.join(mailcraft_engine::mime::sanitize_filename(&name));
        match self.session.run("attachment.save", &json!({"index": i, "path": path.to_string_lossy(), "open": open})) {
            Ok(_) => Some(path),
            Err(e) => {
                self.session.notice(e.to_string(), true);
                None
            }
        }
    }

    pub(crate) fn attachment_name(&self, i: usize) -> String {
        self.session
            .reading
            .as_ref()
            .and_then(|r| r.parsed.as_ref())
            .and_then(|p| p.attachments.iter().find(|a| a.index == i))
            .map(|a| a.name.clone())
            .unwrap_or_else(|| "attachment".into())
    }

    /// Every attachment of the message being read, selected (Ctrl+A in its attachments).
    pub(crate) fn select_all_attachments(&mut self) {
        let Some(r) = self.session.reading.as_ref() else { return };
        let all: Vec<usize> = r.parsed.as_ref().map(|p| p.attachments.iter().filter(|a| !a.inline).map(|a| a.index).collect()).unwrap_or_default();
        self.ui.attachment_selected = Some((r.key.clone(), all));
    }
}
