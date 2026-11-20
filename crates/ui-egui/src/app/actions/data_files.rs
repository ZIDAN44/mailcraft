//! Data files and the data folder: picked, looked at, imported, exported, moved.

use crate::*;

impl MailApp {
    pub(super) fn pick_account_data_file(&mut self) {
        if let Some(path) = self.services.pick_data_file()
            && let Some(dialogs::Dialog::AddAccount(f)) = self.ui.dialog.as_mut()
        {
            f.pst = path.to_string_lossy().into_owned();
        }
    }

    /// What the file holds, and where its copy goes, are shown before it's imported. Looked at
    /// afresh each time it's picked: the room freed, or the file changed, since.
    pub(super) fn open_mail_file(&mut self) {
        if let Some(path) = self.services.pick_mail_file() {
            self.open_handed(&path.to_string_lossy());
        }
    }

    pub(super) fn open_data_file(&mut self) {
        if let Some(path) = self.services.pick_data_file() {
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

    /// The status bar and the folder pane show how it goes from here.
    pub(super) fn import_data_file(&mut self, path: String, again: bool, changes: Option<mailcraft_engine::Changes>, take_over: bool) {
        match self.session.run(
            "account.importPst",
            &json!({"path": path, "again": again, "changes": changes.map(mailcraft_engine::Changes::code), "takeOver": take_over}),
        ) {
            Ok(_) => self.ui.backstage = None,
            // Its copy found held by another data folder since the file was looked at:
            // the dialog again, saying so and offering to take it over.
            Err(_) if !take_over && !again && self.session.data_file_facts(&path).is_some_and(|f| !f.copy_held_by.is_empty()) => {
                self.ui.dialog = Some(Dialog::OpenDataFile { path });
            }
            Err(e) => self.session.notice(e.to_string(), true),
        }
    }

    /// The file old mail is archived to, picked and put in the dialog asking for it: Clean up
    /// old items, AutoArchive's settings, a folder's own.
    pub(super) fn pick_archive_file(&mut self) {
        let shown = match &self.ui.dialog {
            Some(Dialog::Archive(f)) => f.path.clone(),
            Some(Dialog::AutoArchive(f)) => f.rule.path.clone(),
            Some(Dialog::FolderProperties { archive: Some(a), .. }) => a.rule.path.clone(),
            _ => return,
        };
        let suggested =
            if shown.trim().is_empty() { "archive.pst".to_string() } else { mailcraft_engine::model::file_name(shown.trim()).to_string() };
        let Some(path) = self.services.save_path(&suggested) else { return };
        let path = path.to_string_lossy().into_owned();
        match self.ui.dialog.as_mut() {
            Some(Dialog::Archive(f)) => f.path = path,
            Some(Dialog::AutoArchive(f)) => f.rule.path = path,
            Some(Dialog::FolderProperties { archive: Some(a), .. }) => a.rule.path = path,
            _ => {}
        }
    }

    pub(super) fn export_data_file(&mut self, account: &str) {
        // Named after the account's address, as Outlook names a data file.
        let name = self
            .session
            .account(account)
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

    pub(super) fn open_folder(&mut self, path: &std::path::Path) {
        if let Err(e) = self.services.open_path(path) {
            self.session.notice(e, true);
        }
    }

    pub(super) fn move_data_folder(&mut self) {
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

    pub(super) fn cancel_data_move(&mut self) {
        if let Err(e) = self.services.move_data_folder(None) {
            self.session.notice(e, true);
        }
        self.ui.pending_move = None;
    }
}
