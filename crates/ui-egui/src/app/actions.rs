//! What the window was asked to do (`Action`), done; new mail announced.

mod files;

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
            let target = news.first().map_or(NotificationTarget::Inbox, |message| NotificationTarget::Message { key: message.key.to_id() });
            self.services.notify_for(&title, &body, target);
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
                // A data file being imported, then what becomes of an update, are still said
                // before MailCraft goes; the answer closes (and Cancel there leaves the next
                // close asking everything again).
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
            // Committed only once nothing stops the close (`frame.rs`): a message being written can
            // still be kept, and Cancel there leaves MailCraft as it was.
            Action::RestartToUpdate => {
                if self.ui.update.ready().is_some() {
                    self.ui.restart_wanted = true;
                    self.ui.update_answered = true;
                    self.ui.close_now = true;
                }
            }
            Action::CompactOnClose(account) => self.compact_on_close(account),
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
            // A message's source in its window, brought to the front (asked for again while open).
            Action::Run("message.source", p) => match self.session.run("message.source", &p) {
                Ok(v) => self.ui.raise_source = v.get("key").and_then(Value::as_str).and_then(MessageKey::from_id),
                Err(e) => self.session.notice(e.to_string(), true),
            },
            Action::Run(id, p) => {
                // Module navigation can happen inside the mobile folder drawer. Close it as
                // soon as the destination changes so the new module is immediately usable.
                if id == "view.module" {
                    self.ui.mobile_folder_open = false;
                }
                if self.ui.phone_mode {
                    if id == "list.open" {
                        self.ui.phone_reader_open = true;
                    } else if id == "view.module" {
                        self.ui.phone_reader_open = false;
                    }
                }
                if id == "mail.delete" && p.get("permanent").and_then(Value::as_bool) != Some(true) && self.needs_delete_confirm() {
                    self.ui.dialog = Some(Dialog::ConfirmDelete { count: self.session.selection.len() });
                    return;
                }
                let staged_attachment =
                    if id == "compose.attach" { p.get("path").and_then(Value::as_str).map(std::path::PathBuf::from) } else { None };
                // A Search Folder isn't a search typed in: the box is left empty.
                if matches!(id, "search.clear" | "searchFolder.open") {
                    self.ui.search_text.clear();
                }
                self.run(id, p);
                if let Some(path) = staged_attachment {
                    self.services.discard_picked_file(&path);
                }
                if super::keys::moves_focus(id) {
                    self.ui.scroll_to_focus = true;
                }
            }
            Action::Open(k) => {
                if self.ui.phone_mode {
                    self.ui.phone_reader_open = true;
                }
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
            Action::PickAccountDataFile if self.services.pick_later("account-data-file") => {}
            Action::PickAccountDataFile => {
                if let Some(path) = self.services.pick_data_file()
                    && let Some(dialogs::Dialog::AddAccount(f)) = self.ui.dialog.as_mut()
                {
                    f.pst = path.to_string_lossy().into_owned();
                }
            }
            // What the file holds, and where its copy goes, are shown before it's imported. Looked
            // at afresh each time it's picked: the room freed, or the file changed, since.
            Action::OpenDataFile if self.services.pick_later("data-file") => {}
            Action::OpenDataFile => {
                if let Some(path) = self.services.pick_data_file() {
                    if crate::is_message_or_pim_file(&path) {
                        self.open_handed(path.to_string_lossy().trim());
                        return;
                    }
                    let path = path.to_string_lossy().trim().to_string();
                    match self.session.run("account.inspectDataFile", &json!({"path": path, "again": true})) {
                        Ok(_) => {
                            self.ui.backstage = None;
                            self.ui.dialog = Some(Dialog::OpenDataFile { path });
                        }
                        Err(e) => self.session.notice(e.to_string(), true),
                    }
                }
            }
            // The status bar and the folder pane show how it goes from here.
            Action::ImportDataFile { path, again, changes, take_over } => {
                match self.session.run("account.importPst", &json!({"path": path, "again": again, "changes": changes, "takeOver": take_over})) {
                    Ok(_) => {
                        self.services.retain_picked_file(std::path::Path::new(&path));
                        self.ui.backstage = None;
                    }
                    // Its copy found held by another data folder since the file was looked at:
                    // the dialog again, saying so and offering to take it over.
                    Err(_) if !take_over && !again && self.session.data_file_facts(&path).is_some_and(|f| !f.copy_held_by.is_empty()) => {
                        self.ui.dialog = Some(Dialog::OpenDataFile { path });
                    }
                    Err(e) => self.session.notice(e.to_string(), true),
                }
            }
            Action::ImportDetails(account) => self.ui.import_details = Some(account),
            Action::ExportDataFile(account) if self.services.downloads_files() => {
                let account_name = self
                    .session
                    .account(&account)
                    .map(|a| if a.config.email.trim().is_empty() { a.config.name.clone() } else { a.config.email.clone() })
                    .unwrap_or_default();
                let name = mailcraft_engine::mime::sanitize_filename(if account_name.trim().is_empty() { "Mail" } else { account_name.trim() });
                let Some(base) = mailcraft_engine::store::scratch_dir("pst-exports") else {
                    self.session.notice(tf!("Couldn't open {file}: {error}", file = "PST export", error = "temporary storage is unavailable"), true);
                    return;
                };
                static NEXT_EXPORT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
                let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |duration| duration.as_nanos());
                let export_dir = base.join(format!("{now}-{}", NEXT_EXPORT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
                if let Err(error) = std::fs::create_dir(&export_dir) {
                    self.session.notice(error.to_string(), true);
                    return;
                }
                let path = export_dir.join("export.pst");
                match self.session.run("account.exportPst", &json!({"account": account, "path": path.to_string_lossy()})) {
                    Ok(_) => {
                        self.ui.backstage = None;
                        if let Err(error) = self.services.export_file(&account, &format!("{name}.pst"), path) {
                            let _ = std::fs::remove_file(export_dir.join("export.pst"));
                            let _ = std::fs::remove_dir(&export_dir);
                            self.session.notice(error, true);
                        } else {
                            self.session.notice(tf!("Exporting to {file}…", file = format!("{name}.pst")), false);
                        }
                    }
                    Err(error) => {
                        let _ = std::fs::remove_file(export_dir.join("export.pst"));
                        let _ = std::fs::remove_dir(&export_dir);
                        self.session.notice(error.to_string(), true);
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
            Action::ExportRules if self.services.downloads() => match self.session.rules_file() {
                Ok(bytes) => self.services.save_as_owned("MailCraft rules.json", "application/json", bytes),
                Err(error) => self.session.notice(error.to_string(), true),
            },
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
            Action::ImportPgpKey(account) if self.services.pick_later(&format!("pgp-secret:{account}")) => {}
            Action::ImportPgpKey(account) => {
                if let Some(path) = self.services.pick_pgp_key() {
                    self.ui.dialog =
                        Some(Dialog::PgpKey { account, path: path.to_string_lossy().into_owned(), data: None, passphrase: String::new() });
                }
            }
            Action::ImportPublicKey(account) if self.services.pick_later(&format!("pgp-public:{account}")) => {}
            Action::ImportPublicKey(account) => {
                if let Some(path) = self.services.pick_pgp_key() {
                    // The session says what it kept.
                    if let Err(e) = self.session.run("pgp.importPublic", &json!({"account": account, "path": path.to_string_lossy()})) {
                        self.session.notice(e.to_string(), true);
                    }
                }
            }
            Action::UseSmartCard(account) => {
                let module = self.session.card_search().modules.first().cloned().unwrap_or_default();
                self.ui.dialog = Some(Dialog::SmartCard { account, module, pin: String::new(), system: false });
            }
            Action::UseSystemCertificate(account) => {
                self.run("security.findKeys", json!({"keychain": true}));
                self.ui.dialog = Some(Dialog::SmartCard { account, module: String::new(), pin: String::new(), system: true });
            }
            Action::ImportRules if self.services.pick_later("rules") => {}
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
            Action::ImportCategories if self.services.pick_later("categories") => {}
            Action::ExportCategories if self.services.downloads() => match self.session.categories_file() {
                Ok(bytes) => self.services.save_as_owned("MailCraft categories.json", "application/json", bytes),
                Err(error) => self.session.notice(error.to_string(), true),
            },
            Action::ExportCategories => {
                if let Some(path) = self.services.save_path("MailCraft categories.json") {
                    match self.session.run("category.export", &json!({"path": path.to_string_lossy()})) {
                        Ok(v) => {
                            let n = v.get("categories").and_then(Value::as_u64).unwrap_or(0);
                            let file = path.file_name().unwrap_or_default().to_string_lossy().to_string();
                            self.session.notice(tf!("Categories saved in {file}: {n}", n = n, file = file), false);
                        }
                        Err(e) => self.session.notice(e.to_string(), true),
                    }
                }
            }
            Action::ImportCategories => {
                if let Some(path) = self.services.pick_categories_file() {
                    match self.session.run("category.import", &json!({"path": path.to_string_lossy()})) {
                        Ok(v) => {
                            let n = v.get("added").and_then(Value::as_u64).unwrap_or(0);
                            let changed = v.get("changed").and_then(Value::as_u64).unwrap_or(0);
                            let skipped = v.get("skipped").and_then(Value::as_array).map_or(0, Vec::len);
                            let text = if skipped > 0 {
                                tf!(
                                    "Categories imported: {n} new, {changed} changed, {skipped} left out (their keyword is another category's here)",
                                    n = n,
                                    changed = changed,
                                    skipped = skipped
                                )
                            } else {
                                tf!("Categories imported: {n} new, {changed} changed", n = n, changed = changed)
                            };
                            self.session.notice(text, skipped > 0);
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
                    if id == "account.add"
                        && let Some(path) = p.get("pst").and_then(Value::as_str)
                    {
                        self.services.retain_picked_file(std::path::Path::new(path));
                    }
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
                }
                Err(e) => {
                    if let Some(d) = self.ui.dialog.as_mut() {
                        d.set_error(e.to_string().trim_start_matches("bad parameters: ").to_string());
                    }
                }
            },
            Action::SetDark(d) => self.ui.dark = d,
            Action::ShowFormatted(key, on) => self.ui.shown_as.set(&key, on, self.session.prefs.show_html),
            Action::SetLanguage(l) => self.ui.language = l,
            Action::OpenUrl(url) => self.open_link(&url),
            Action::DialPhone(number) => {
                if let Err(error) = self.services.dial_phone(&number) {
                    self.session.notice(tf!("The link couldn't be opened: {error}", error = error), true);
                }
            }
            Action::OpenMap(address) => {
                if let Err(error) = self.services.open_map(&address) {
                    self.session.notice(tf!("The link couldn't be opened: {error}", error = error), true);
                }
            }
            Action::ShareContact { title, vcard } => {
                if let Err(error) = self.services.share_contact(&title, &vcard) {
                    self.session.notice(error, true);
                }
            }
            Action::ExportContact { name, vcard } => {
                let name = format!("{}.vcf", mailcraft_engine::mime::sanitize_filename(&name));
                self.services.save_as_owned(&name, "text/vcard", vcard.into_bytes());
            }
            Action::AddToSystemContacts { contact } => {
                if let Err(error) = self.services.add_to_system_contacts(&contact) {
                    self.session.notice(error, true);
                }
            }
            Action::ShareCalendarEvent { title, ical } => {
                if let Err(error) = self.services.share_calendar_event(&title, &ical) {
                    self.session.notice(error, true);
                }
            }
            Action::ExportCalendarEvent { name, ical } => {
                let name = format!("{}.ics", mailcraft_engine::mime::sanitize_filename(&name));
                self.services.save_as_owned(&name, "text/calendar", ical.into_bytes());
            }
            Action::AddToSystemCalendar { title, location, description, start, end, all_day, timezone, rrule, rdates, exdates, attendees } => {
                if let Err(error) = self.services.add_to_system_calendar(
                    &title,
                    &location,
                    &description,
                    start,
                    end,
                    all_day,
                    timezone.as_deref(),
                    rrule.as_deref(),
                    &rdates,
                    &exdates,
                    &attendees,
                ) {
                    self.session.notice(error, true);
                }
            }
            Action::SignInAgain(id) => self.sign_in_again(&id),
            Action::OpenInCraft(i, cmd) => {
                let Some(app) = self.ui.craft_apps.iter().find(|(c, _)| *c == cmd).map(|(_, p)| p.clone()) else { return };
                if let Some(path) = self.attachment_to_temp(i, true)
                    && let Err(e) = self.services.open_with(&app, &path)
                {
                    self.session.notice(tf!("{app} couldn't open it: {error}", app = app.display(), error = e), true);
                }
            }
            Action::Print if self.services.downloads() => {
                let name = format!("{}.pdf", self.message_file_name());
                match self.session.message_pdf() {
                    Ok(bytes) => self.services.open_file_owned_with_mime(&name, "application/pdf", bytes),
                    Err(error) => self.session.notice(error.to_string(), true),
                }
            }
            Action::ExportPdf if self.services.downloads() => {
                let name = format!("{}.pdf", self.message_file_name());
                match self.session.message_pdf() {
                    Ok(bytes) => self.services.save_as_owned(&name, "application/pdf", bytes),
                    Err(error) => self.session.notice(error.to_string(), true),
                }
            }
            Action::SaveMessage if self.services.downloads() => {
                let name = format!("{}.eml", self.message_file_name());
                match self.session.message_source() {
                    Ok(b) => self.services.save_as_owned(&name, "message/rfc822", b),
                    Err(e) => self.session.notice(e.to_string(), true),
                }
            }
            Action::ShareMessage if self.services.can_share_files() => {
                let name = format!("{}.eml", self.message_file_name());
                match self.session.message_source() {
                    Ok(bytes) => {
                        if let Err(error) = self.services.share_message(&name, "message/rfc822", &bytes) {
                            self.session.notice(error, true);
                        }
                    }
                    Err(error) => self.session.notice(error.to_string(), true),
                }
            }
            Action::ShareMessage => {}
            Action::NoteFile { uid, index, open } if self.services.downloads() => match self.session.note_file(&uid, index) {
                Ok((name, _bytes)) if open && self.services.opens_files_after_saving() && mailcraft_engine::mime::runs_code(&name) => self
                    .session
                    .notice(tf!("The file {name} could run a program, so it isn't opened from here; save it and check it first.", name = name), true),
                Ok((name, bytes)) if open => self.services.open_file_owned(&name, bytes),
                Ok((name, bytes)) => self.services.save_as_owned(&name, "application/octet-stream", bytes),
                Err(error) => self.session.notice(error.to_string(), true),
            },
            Action::NoteFile { uid, index, open } => self.note_file(&uid, index, open),
            Action::SaveAttachment(i) | Action::OpenAttachment(i) if self.services.downloads() => {
                let name = mailcraft_engine::mime::sanitize_filename(&self.attachment_name(i));
                let mime = self.attachment_mime(i);
                let open = matches!(&a, Action::OpenAttachment(_));
                if open && self.services.opens_files_after_saving() && mailcraft_engine::mime::runs_code(&name) {
                    self.session.notice(
                        tf!("The file {name} could run a program, so it isn't opened from here; save it and check it first.", name = name),
                        true,
                    );
                    return;
                }
                match self.session.attachment_bytes(i) {
                    Ok(bytes) if open => self.services.open_file_owned_with_mime(&name, &mime, bytes),
                    Ok(bytes) => self.services.save_as_owned(&name, &mime, bytes),
                    Err(e) => self.session.notice(e.to_string(), true),
                }
            }
            Action::ShareAttachment(i) if self.services.can_share_files() => {
                let name = mailcraft_engine::mime::sanitize_filename(&self.attachment_name(i));
                let mime = self.attachment_mime(i);
                match self.session.attachment_bytes(i) {
                    Ok(bytes) => {
                        if let Err(error) = self.services.share_file(&name, &mime, &bytes) {
                            self.session.notice(error, true);
                        }
                    }
                    Err(error) => self.session.notice(error.to_string(), true),
                }
            }
            Action::ShareAttachments(indices) if self.services.can_share_files() => {
                const MAX_FILES: usize = 50;
                const MAX_BYTES: usize = 25 << 20;
                if indices.len() > MAX_FILES {
                    self.session.notice(tf!("You can share up to {n} attachments at once", n = MAX_FILES), true);
                    return;
                }
                let mut files = Vec::with_capacity(indices.len());
                let mut total = 0usize;
                for index in indices {
                    let name = mailcraft_engine::mime::sanitize_filename(&self.attachment_name(index));
                    let mime = self.attachment_mime(index);
                    match self.session.attachment_bytes(index) {
                        Ok(bytes) => {
                            total = total.saturating_add(bytes.len());
                            if total > MAX_BYTES {
                                self.session.notice(tl!("The selected attachments are too large to share").to_string(), true);
                                return;
                            }
                            files.push((name, mime, bytes));
                        }
                        Err(error) => {
                            self.session.notice(error.to_string(), true);
                            return;
                        }
                    }
                }
                let refs: Vec<_> = files.iter().map(|(name, mime, bytes)| (name.as_str(), mime.as_str(), bytes.as_slice())).collect();
                if let Err(error) = self.services.share_files(&refs) {
                    self.session.notice(error, true);
                }
            }
            Action::ShareAttachment(_) | Action::ShareAttachments(_) => {}
            Action::SaveAttachments(indices) if self.services.downloads() => {
                if self.services.downloads_files() {
                    let files: Vec<_> = indices.into_iter().map(|index| (index, self.attachment_name(index), self.attachment_mime(index))).collect();
                    self.queue_attachment_downloads(files);
                    return;
                }
                for index in indices {
                    let name = mailcraft_engine::mime::sanitize_filename(&self.attachment_name(index));
                    let mime = self.attachment_mime(index);
                    match self.session.attachment_bytes(index) {
                        Ok(bytes) => self.services.download_owned_with_mime(&name, &mime, bytes),
                        Err(error) => self.session.notice(error.to_string(), true),
                    }
                }
            }
            Action::SaveAll if self.services.downloads() => {
                let files: Vec<(usize, String, String)> = self
                    .session
                    .reading
                    .as_ref()
                    .and_then(|r| r.parsed.as_ref())
                    .map(|p| p.attachments.iter().filter(|a| !a.inline).map(|a| (a.index, a.name.clone(), a.mime.clone())).collect())
                    .unwrap_or_default();
                if self.services.downloads_files() {
                    self.queue_attachment_downloads(files);
                    return;
                }
                for (i, name, mime) in files {
                    match self.session.attachment_bytes(i) {
                        Ok(bytes) => self.services.download_owned_with_mime(&mailcraft_engine::mime::sanitize_filename(&name), &mime, bytes),
                        Err(error) => self.session.notice(error.to_string(), true),
                    }
                }
            }
            Action::InsertPicture if self.services.pick_later("picture") => {}
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
            Action::InsertSignature(name) => {
                let mut p = json!({"at": self.picture_at()});
                if let Some(name) = name {
                    p["name"] = json!(name);
                }
                let r = self.session.run("compose.insertSignature", &p);
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
                    // Wider than a page: in at a page's width (its header read, nothing decoded).
                    let head = std::fs::File::open(&p).and_then(|f| {
                        let mut v = Vec::new();
                        std::io::Read::read_to_end(&mut std::io::Read::take(f, 1 << 20), &mut v).map(|_| v)
                    });
                    let size = head.ok().as_deref().and_then(rich_edit::own_size);
                    let (width, line) = (rich_edit::fitted_width(size), rich_edit::own_line(size));
                    let r = self
                        .session
                        .run("compose.insertPicture", &self.picture_place(json!({"path": p.to_string_lossy(), "width": width, "line": line})));
                    self.place_picture(r);
                }
            }
            Action::PastePicture => {
                // Formatted text that came with a picture (from Word or a web page) was pasted
                // as text.
                let text = clipboard_html().is_some_and(|h| !mailcraft_engine::mime::pasted_html(&h).0.trim().is_empty());
                if !text && let Some(compose_id) = self.session.compose.as_ref().map(|compose| compose.id) {
                    let target = crate::PendingClipboardPicture::Compose { compose_id, params: self.picture_place(json!({"name": "pasted.png"})) };
                    if self.clipboard_picture_later(target.clone()) {
                        return;
                    }
                    if let Some(png) = self.services.clipboard_picture() {
                        self.pending_clipboard_picture = Some(target);
                        self.finish_clipboard_picture(png);
                    }
                }
            }
            Action::SignaturePicture { paste: false }
                if matches!(&self.ui.dialog, Some(Dialog::Signature(_))) && self.services.pick_later("signature-picture") => {}
            Action::SignaturePicture { paste } => self.signature_picture(paste),
            Action::SignatureFile if matches!(&self.ui.dialog, Some(Dialog::Signature(_))) && self.services.pick_later("signature") => {}
            Action::SignatureFile => self.signature_file(),
            Action::PasteFiles => self.paste_files(),
            Action::AttachClipboardFiles(files) => self.attach_clipboard_files(files),
            Action::AttachFiles if self.services.pick_later("attach") => {}
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
}
