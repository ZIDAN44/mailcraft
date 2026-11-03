//! The store: an Outlook data file opened as a mailbox (File › Open & Export) keeps MailCraft's
//! copy beside it, `archive.pst` → `archive.mailcraft` in the same folder (`archive (2).mailcraft`
//! when that name is another file's), as Outlook keeps an account's `.pst` where the user put it.
//! MailCraft never writes into the Outlook file: everything changed in MailCraft is in the copy.
//! Opened again, the same file finds its copy there (made from a file of that name, and no
//! account's now) and goes on with it. A folder that can't be written (read-only media, a
//! protected folder, a share refusing writes) has the copy go with the other data files.

use crate::*;

/// Where MailCraft's copy of an Outlook data file goes, or is already ([`Store::copy_beside`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Beside {
    /// The copy; the folder it goes in when it's [`Beside::elsewhere`].
    pub path: PathBuf,
    /// A copy made from this file earlier, there already, and what its note says of it.
    pub found: Option<Found>,
    /// Not beside the file: its folder can't be written, so it goes with the other data files
    /// (named as any other account's, as it's first written).
    pub elsewhere: bool,
}

/// A copy of a data file made earlier, as its import's note tells it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Found {
    /// How far the import that made it got: "running" (cut off), "cancelled" (stopped by the
    /// user) or "done".
    pub state: String,
    /// The file as the copy read it: its size and when it last changed (Unix seconds).
    pub size: u64,
    pub modified: i64,
    /// The file has changed since (its size or time isn't the one read).
    pub changed: bool,
    /// The account it was last, as its folder list names it ("" when it doesn't say).
    pub account: String,
    /// It notes what came in from the file ([`Store::has_seen`]): the file changed since can be
    /// read into it again.
    pub seen: bool,
}

/// A file's size and when it last changed (Unix seconds), as the file system says; `None` when
/// it can't be told (the file isn't there, or this is the browser).
pub fn stamp(path: &Path) -> Option<(u64, i64)> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let m = fs::metadata(path).ok().filter(fs::Metadata::is_file)?;
        let modified = m
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX));
        Some((m.len(), modified))
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = path;
        None
    }
}

impl Store {
    /// Where MailCraft's copy of the Outlook data file at `source` goes when it's opened as a
    /// mailbox, or is already: beside it, else with the other data files. Nothing is changed
    /// (whether the folder can be written is tried by making the copy's file and removing it at
    /// once). `None` when the cache doesn't keep accounts in data files.
    pub fn copy_beside(&self, source: &Path) -> Option<Beside> {
        self.beside(source, None, false)
    }

    /// [`Store::copy_beside`], and `account`'s data file is that copy from now on: the one found
    /// there, or one made there as the account's mail is first written. Going elsewhere, it's
    /// named as any other account's. `None` too when `account` has a data file already.
    pub fn place_beside(&self, account: &str, source: &Path) -> Option<Beside> {
        if self.data_file(account).is_some() {
            return None;
        }
        self.beside(source, Some(account), false)
    }

    /// [`Store::place_beside`] for a copy of its own: one found there is passed over (another
    /// copy of a file already open as an account).
    pub fn place_new_beside(&self, account: &str, source: &Path) -> Option<Beside> {
        if self.data_file(account).is_some() {
            return None;
        }
        self.beside(source, Some(account), true)
    }

    fn beside(&self, source: &Path, take: Option<&str>, fresh: bool) -> Option<Beside> {
        #[cfg(not(target_arch = "wasm32"))]
        if let Files::Packed(x) = &self.files.main {
            let mut beside = x.beside(source, take.map(safe_segment).as_deref(), fresh);
            if let Some(found) = beside.found.as_mut() {
                found.changed = stamp(source).is_none_or(|now| now != (found.size, found.modified));
            }
            return Some(beside);
        }
        let _ = (source, take, fresh);
        None
    }

    /// A copy taken on as `account`'s (from an account it was before): its folders and their
    /// messages name the account they belong to, this one now. Returns how many messages it has.
    pub fn claim_folders(&self, account: &str) -> Result<usize> {
        let mut folders = self.load_folders(account)?;
        if folders.iter().any(|f| f.account != account) {
            for f in &mut folders {
                f.account = account.to_string();
            }
            self.save_folders(account, &folders)?;
        }
        let mut messages = 0;
        for f in folders {
            let mut st = self.load_folder(account, &f.path)?;
            messages += st.messages.len();
            if st.messages.iter().any(|m| m.key.account != account) {
                for m in &mut st.messages {
                    SharedSummary::make_mut(m).key.account = account.to_string();
                }
                self.save_folder(account, &f.path, &st)?;
            }
        }
        Ok(messages)
    }
}
