//! Saving a backup of what the browser keeps, and bringing one back (issue #83): what the
//! window does for the File page's two buttons and their dialogs.

use std::sync::Arc;

use crate::*;

impl MailApp {
    /// Make the backup and hand it over as a download.
    pub(crate) fn save_backup(&mut self, password: &str, mail: bool) {
        let state = self.saved_state();
        let random = self.services.random_bytes();
        match self.session.make_backup(Some(password).filter(|p| !p.is_empty()), mail, state, random) {
            Ok((name, file)) => {
                self.services.download(&name, &file);
                self.ui.dialog = None;
                self.session.notice(tf!("Backup saved as {file}", file = name), false);
            }
            Err(e) => self.backup_error(e.to_string()),
        }
    }

    /// Ask for a backup file to bring in.
    pub(crate) fn pick_backup(&mut self) {
        if !self.services.pick_later("backup") {
            self.session.notice(tl!("A backup is brought in where MailCraft runs in the browser."), true);
        }
    }

    /// A file was picked to be restored from: asked about, when it's a backup.
    pub(crate) fn backup_picked(&mut self, name: String, data: Vec<u8>) {
        if mailcraft_engine::backup::locked(&data).is_none() {
            self.session.notice(tf!("{file} isn't a MailCraft backup.", file = name), true);
            return;
        }
        self.ui.dialog = Some(Dialog::Restore(crate::dialogs::RestoreForm { name, data: Arc::new(data), ..Default::default() }));
    }

    /// Read the backup and put it in the place of what's kept: the page then starts again
    /// from it.
    pub(crate) fn restore_backup(&mut self, data: &[u8], password: &str) {
        let read = mailcraft_engine::backup::read(data, Some(password).filter(|p| !p.is_empty()));
        match read.map_err(|e| e.to_string()).and_then(|b| self.services.restore(b.files, b.state)) {
            Ok(()) => self.ui.dialog = None,
            Err(e) => self.backup_error(e),
        }
    }

    fn backup_error(&mut self, e: String) {
        // (A sentence, as the dialog's other lines are: the engine's begins small.)
        let mut letters = e.chars();
        let e = letters.next().map(|first| first.to_uppercase().chain(letters).collect::<String>()).unwrap_or_default();
        match self.ui.dialog.as_mut() {
            Some(Dialog::Backup(f)) => f.error = Some(e),
            Some(Dialog::Restore(f)) => f.error = Some(e),
            _ => self.session.notice(e, true),
        }
    }
}
