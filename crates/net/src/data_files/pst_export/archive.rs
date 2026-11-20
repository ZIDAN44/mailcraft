//! Archiving (issue #179): an account's old mail, in a folder and the folders under it, written
//! to a data file as an export writes it, into folders of the same names (Inbox/Projects into the
//! file's Inbox/Projects). Only messages the file has once it's written are named as archived,
//! so nothing leaves the mailbox before its copy is there.

use super::*;
use mailcraft_model::Summary;

/// Which mail an archive takes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pick {
    /// These folders' paths and the folders under each; every folder when empty.
    pub folders: Vec<String>,
    /// Those folders alone, not the folders under them (AutoArchive's, each by its own settings).
    pub only: bool,
    /// Received before this (Unix seconds).
    pub before: i64,
}

impl Pick {
    /// Whether it takes the mail of `f`: one of its folders or under one. Never the Outbox,
    /// whose mail isn't sent yet (kept apart, its path no server's).
    pub(super) fn takes(&self, f: &Folder) -> bool {
        if f.path.starts_with('\0') {
            return false;
        }
        if self.only {
            return self.folders.contains(&f.path);
        }
        self.folders.is_empty()
            || self
                .folders
                .iter()
                .any(|p| f.path == *p || f.delimiter.is_some_and(|d| f.path.strip_prefix(p.as_str()).is_some_and(|rest| rest.starts_with(d))))
    }

    /// Whether `m` is old enough: received before `before` (dated before it, when its delivery
    /// time isn't known). One with no date at all is never taken as old.
    pub(super) fn old(&self, m: &Summary) -> bool {
        let when = m.when();
        when > 0 && when < self.before
    }
}

/// What an archive did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Archived {
    /// Messages written to the file.
    pub messages: usize,
    /// Old messages the file had already, in the folder they'd go to: not written again.
    pub already: usize,
    /// Old messages whose body isn't on this computer, or that couldn't be read: not archived.
    pub skipped: usize,
    /// Folders whose list couldn't be read: their mail isn't archived.
    pub folders_skipped: usize,
    /// The messages the file has now, written or there already: these may leave the mailbox.
    pub archived: Vec<MessageKey>,
}

/// Write `account`'s mail that `pick` takes to the data file at `path` (made, named `name`,
/// when it isn't there; added to when it is). Calls `progress` with the number written so far.
pub fn archive(
    store: &Store,
    account: &str,
    name: &str,
    path: &Path,
    pick: &Pick,
    follow_up: &FollowUpDays,
    progress: &mut dyn FnMut(usize),
) -> Result<Archived, String> {
    let (done, archived) = write_file(store, account, name, path, follow_up, Some(pick), progress)?;
    Ok(Archived { messages: done.messages, already: done.already, skipped: done.skipped, folders_skipped: done.folders_skipped, archived })
}
