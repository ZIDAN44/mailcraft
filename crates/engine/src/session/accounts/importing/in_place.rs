//! The session: a POP account set up with an existing MailCraft data file (`.mailcraft`) keeps
//! its mail in that file where it is, as Outlook uses an existing `.pst`: nothing is copied or
//! imported, and the account connects at once, its new mail going into the file. A file that
//! can't be used where it is (open in another program, not closed properly, read-only, another
//! MailCraft data folder's account) is copied in with the other data files, as before, and the
//! user is told why.

use std::path::Path;

use super::*;

/// Whether `path` names MailCraft's own data file.
pub(super) fn is_own(path: &str) -> bool {
    path.to_ascii_lowercase().ends_with(".mailcraft")
}

impl Session {
    /// `account`'s data file is MailCraft's own at `path`, where it is: its folders, mail,
    /// calendar and contacts are the account's now. Why not, in plain words, when it can't be
    /// (nothing of the account changed then).
    pub(super) fn use_in_place(&mut self, account: &str, path: &str) -> Result<serde_json::Value, String> {
        let Some(store) = self.store.clone() else { return Err("MailCraft keeps no data files here".into()) };
        let file = store.use_in_place(account, Path::new(path))?;
        let folders = store.take_on_in_place(account).and_then(|_| store.load_folders(account));
        let mut folders = match folders {
            Ok(f) => f,
            Err(e) => {
                // Let go of where it is (it stays there), and the account starts a file of its own.
                store.let_go_in_place(account);
                return Err(format!("it couldn't be opened ({e})"));
            }
        };
        mailcraft_model::sort_folders(&mut folders);
        let paths: Vec<String> = folders.iter().map(|f| f.path.clone()).collect();
        if let Some(a) = self.account_mut(account) {
            a.folders = folders;
        }
        self.load_imported_pim(account);
        // What the window read of the account before the file was its (nothing) is read again,
        // and its Inbox opens, as at an import's end: found in the app, the Inbox counted the
        // file's mail and the list showed none.
        for folder in &paths {
            self.folder_changed(account, folder);
        }
        if let Some(inbox) = self.account(account).and_then(|a| a.folder(FolderRole::Inbox)).map(|f| f.path.clone()) {
            self.open_view(View::Folder { account: account.to_string(), folder: inbox });
        }
        Ok(serde_json::json!({ "account": account, "importing": false, "dataFile": file.display().to_string() }))
    }
}
