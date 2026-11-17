//! Data files: a MailCraft data file the user chose for a new account (a POP account set up with
//! an existing `.mailcraft`, as Outlook takes an existing `.pst` where it is) is that account's
//! where it is: named in `datafiles.json` by its whole path (by its name when it's in the data
//! files' folder, so it moves with them), never copied. MailCraft never removes it: the account
//! removed lets go of it as of any other, and what would remove an account's data file (an import
//! thrown away, an account started again) lets go of this one and leaves it there. The file says
//! so itself ([`crate::IN_PLACE`] among its files, so compacting keeps it).

use std::fs::OpenOptions;

use super::beside::same_key;
use super::owner::{owner_of, still_has_own};
use super::*;
use crate::IN_PLACE;

impl Packs {
    /// `account`'s data file is the MailCraft data file at `source`, where it is: checked, and
    /// named as the account's. Nothing of it is written yet. Refused, with why in plain words, for
    /// a file that isn't one, can't be written, is open (here as an account, in another program,
    /// or not closed properly: opening it here would check it on this thread, for minutes), or is
    /// an account in another MailCraft data folder that still has it.
    pub(crate) fn take_in_place(&self, account: &str, source: &Path) -> std::result::Result<PathBuf, String> {
        // A path named from where MailCraft runs (the CLI's) is in that folder.
        let source = std::path::absolute(source).unwrap_or_else(|_| source.to_path_buf());
        if !source.extension().is_some_and(|x| x.eq_ignore_ascii_case(EXTENSION)) {
            return Err("it isn't a MailCraft data file".into());
        }
        match fs::symlink_metadata(&source) {
            Ok(m) if m.is_file() => {}
            Ok(_) => return Err("it isn't a file".into()),
            Err(e) => return Err(format!("it can't be read ({e})")),
        }
        let Some(whole) = source.to_str().map(str::to_string) else { return Err("its path has characters MailCraft can't keep".into()) };
        let here = |names: &BTreeMap<String, String>| names.values().any(|n| same_key(&self.dir.join(n)) == same_key(&source));
        if here(&lock(&self.names)) {
            return Err("it's open as an account here already".into());
        }
        // Opened to read only, and closed again before it's named.
        {
            let db = redb::ReadOnlyDatabase::open(&source).map_err(|_| "it's open in another program, or wasn't closed properly".to_string())?;
            let tx = db.begin_read().map_err(|e| e.to_string())?;
            if tx.open_table(FILES).is_err() {
                return Err("it isn't a MailCraft data file".into());
            }
            let (owner, owner_path) = owner_of(&tx);
            let ours = self.folder_id().is_some_and(|id| id == owner);
            if !owner.is_empty() && !ours && still_has_own(&owner_path, &owner, &source) {
                return Err(format!("it's an account in another MailCraft data folder, {owner_path}"));
            }
        }
        if OpenOptions::new().write(true).open(&source).is_err() {
            return Err("it can't be written (it's read-only, or on a drive that refuses writes)".into());
        }
        let name = match (source.parent(), source.file_name().and_then(|n| n.to_str())) {
            (Some(dir), Some(file)) if same_key(dir) == same_key(&self.dir) => file.to_string(),
            _ => whole,
        };
        let mut names = lock(&self.names);
        if names.contains_key(account) {
            return Err("the account has a data file already".into());
        }
        // Taken by another account since it was looked at.
        if here(&names) {
            return Err("it's open as an account here already".into());
        }
        names.insert(account.to_string(), name);
        super::names::save(&self.root, &names);
        Ok(source)
    }

    /// Whether `account`'s data file is the user's own, taken on where it is, and so never
    /// removed. One that can't be read to tell counts as one when it's named by its whole path
    /// (outside the data files' folder: never a file MailCraft only made for itself there).
    pub(super) fn kept_in_place(&self, account: &str) -> bool {
        let whole = lock(&self.names).get(account).is_some_and(|n| Path::new(n).is_absolute());
        self.has(account, IN_PLACE).unwrap_or(whole)
    }
}
