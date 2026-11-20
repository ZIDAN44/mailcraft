//! Archiving old mail (issue #179), as Outlook's Clean Up Old Items does: mail received before a
//! day, in a folder and the folders under it, goes into an Outlook data file on this computer, in
//! folders of the same names, and then leaves the mailbox (on Exchange for good, so the mailbox
//! shrinks). The file then opens in the folder list, or, open already, has what's new read in.
//! AutoArchive (issue #180, `auto_archive.rs`) archives the same way, folder by folder, or
//! deletes the old mail for good where a folder is set to.
//!
//! An account on a server keeps only its newest mail here, so each round first searches the
//! server for the old mail (which brings it into the folders' lists) and downloads what of it
//! isn't here; then the file is written, and only what the file has once it's written leaves
//! the mailbox. A server finds so many a folder at a time: while a folder found that many, and
//! the round archived something, another round follows.

use std::collections::{HashMap, HashSet};

use super::*;

/// The most a server's search brings of one folder at a time (`mailcraft_jmap::FOUND_MAX`, what
/// every kind of server is held to): a folder that found this many may have more.
const FOUND_PER_FOLDER: usize = 500;
/// The most rounds one archive takes: 200,000 messages of a folder.
const ROUNDS_MAX: u32 = 400;
/// How long downloads that stopped coming are waited for, in seconds; past that what came is
/// archived, and the rest stays in the mailbox.
const DOWNLOAD_WAIT: i64 = 10 * 60;

/// Whether `path` names a file from the top of a drive or folder tree (`/`, `C:\`, `C:/`, `\\server`),
/// on whichever system it's typed: not read from this one, as the engine runs in the browser too.
fn whole_path(path: &str) -> bool {
    path.starts_with('/') || path.starts_with("\\\\") || matches!(path.as_bytes(), [d, b':', b'\\' | b'/', ..] if d.is_ascii_alphabetic())
}

/// What's to be archived: an account's mail received before `before`, in `folders` (and those
/// under them, unless `only`; every folder when there are none), into the data file at `path`,
/// or (`delete`) deleted for good.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::session) struct ArchiveTask {
    pub(in crate::session) account: String,
    pub(in crate::session) folders: Vec<String>,
    pub(in crate::session) only: bool,
    pub(in crate::session) before: i64,
    pub(in crate::session) path: String,
    pub(in crate::session) delete: bool,
    /// The file shown in the folder list once it's written.
    pub(in crate::session) show: bool,
}

/// Old mail being archived. One at a time: two writing one file would leave one's mail out.
#[derive(Clone, Debug)]
pub(crate) struct Archiving {
    pub(crate) account: String,
    pub(crate) path: String,
    /// The folders asked for, as [`Job::ArchivePst`] takes them (none: every folder).
    roots: Vec<String>,
    only: bool,
    /// Those folders (and those under them, unless `only`).
    folders: Vec<String>,
    before: i64,
    delete: bool,
    show: bool,
    stage: Stage,
    round: u32,
    /// What this round's search found, by folder.
    found: HashMap<String, usize>,
    /// The server's search was stopped part-way by another search, or by the search box
    /// cleared: there may be more.
    cut: bool,
    /// Why the server couldn't search, when it couldn't.
    search_error: Option<String>,
    /// The messages archived (or deleted), over every round.
    gone: HashSet<MessageKey>,
    /// Messages written to the file, over every round.
    written: usize,
    /// When it last moved on (Unix seconds).
    moved: i64,
}

#[derive(Clone, Debug)]
enum Stage {
    /// The server is searched for the old mail (its query, as sent).
    Searching(String),
    /// The old mail not here yet is downloaded: those still to come, and how many couldn't be.
    Downloading(HashSet<MessageKey>, usize),
    /// The file is written.
    Writing,
}

/// What an archive did, as [`Event::Archived`] says it.
#[derive(Debug, Default)]
pub(in crate::session) struct ArchiveReport {
    /// Messages written to the file.
    pub(in crate::session) messages: usize,
    /// The messages the file has now: these leave the mailbox.
    pub(in crate::session) archived: Vec<MessageKey>,
    /// Old messages not on this computer, which stay.
    pub(in crate::session) skipped: usize,
    /// Folders whose list couldn't be read.
    pub(in crate::session) folders: usize,
}

impl Session {
    /// The account whose old mail is being archived, and the file it goes to (empty: deleted).
    pub fn archiving_now(&self) -> Option<(&str, &str)> {
        self.archiving.as_ref().map(|a| (a.account.as_str(), a.path.as_str()))
    }

    /// Archive `account`'s mail received before `before` (Unix seconds), in `folder` and those
    /// under it (all its folders when none), into the data file at `path`.
    pub fn archive_old(&mut self, account: &str, folder: Option<&str>, before: i64, path: &str) -> CmdResult {
        if let Some(f) = folder
            && self.account(account).is_some()
            && self.folder(account, f).is_none()
        {
            return Err(CmdError::Params(format!("no folder `{f}`")));
        }
        let task = ArchiveTask {
            account: account.to_string(),
            folders: folder.map(|f| vec![f.to_string()]).unwrap_or_default(),
            only: false,
            before,
            path: path.trim().to_string(),
            delete: false,
            show: true,
        };
        self.start_archive(task)?;
        Ok(serde_json::json!({ "account": account, "archiving": true }))
    }

    /// Start archiving `task`, when nothing is being archived and it can be.
    pub(in crate::session) fn start_archive(&mut self, task: ArchiveTask) -> Result<(), CmdError> {
        let ArchiveTask { account, folders: roots, only, before, path, delete, show } = task;
        let account = account.as_str();
        if !delete {
            if path.is_empty() {
                return Err(CmdError::Params("`path` is required".into()));
            }
            if !path.to_ascii_lowercase().ends_with(".pst") {
                return Err(CmdError::Params("the archive is an Outlook data file: its name ends in .pst".into()));
            }
            // A name alone would be written wherever MailCraft happened to start.
            if !whole_path(&path) {
                return Err(CmdError::Params("give the archive file's whole path, or pick it with Browse".into()));
            }
        }
        if before <= 0 {
            return Err(CmdError::Params("`before` is a time (Unix seconds)".into()));
        }
        let a = self.account(account).ok_or_else(|| CmdError::Params(format!("no account `{account}`")))?;
        // Its mail is only ever in memory here, where the file can't be written from.
        if a.config.keeps_online() {
            return Err(CmdError::Params("this account keeps none of its mail on this computer, so it can't be archived here".into()));
        }
        let on_server = !a.is_local() && a.config.pop.is_none();
        if self.archiving.is_some() {
            return Err(CmdError::Failed("Old mail is being archived already: wait until that's done.".into()));
        }
        // An account whose mail is being imported hasn't all of it yet, nor is it taken out from
        // under the import.
        self.import_holds(account)?;
        // The file, when it's open here: not the one this account's mail is in, nor one being
        // read in.
        if !delete && let Some(open) = self.duplicate_of(&path, "", None) {
            if open == account {
                return Err(CmdError::Params("this is the data file the mail is in: choose another file to archive it to".into()));
            }
            self.import_holds(&open)?;
        }
        let under = |p: &str, d: Option<char>| {
            roots.is_empty()
                || roots.iter().any(|f| p == f || (!only && d.is_some_and(|d| p.strip_prefix(f.as_str()).is_some_and(|r| r.starts_with(d)))))
        };
        let folders = self
            .account(account)
            .map(|a| a.folders.iter().filter(|f| f.selectable && f.path != OUTBOX && under(&f.path, f.delimiter)).map(|f| f.path.clone()).collect())
            .unwrap_or_default();
        self.archiving = Some(Archiving {
            account: account.to_string(),
            path,
            roots,
            only,
            folders,
            before,
            delete,
            show,
            stage: Stage::Writing,
            round: 1,
            found: HashMap::new(),
            cut: false,
            search_error: None,
            gone: HashSet::new(),
            written: 0,
            moved: self.clock.now,
        });
        // All its mail is here: written (or deleted) at once. On a server: looked for first.
        if on_server {
            self.archive_search();
        } else if delete {
            self.archive_delete();
        } else {
            self.archive_write();
        }
        Ok(())
    }

    /// A round's search of the server, for the folders' mail from before the day (a day more:
    /// what comes is checked here).
    fn archive_search(&mut self) {
        let Some(a) = self.archiving.as_ref() else { return };
        let day = mailcraft_model::dates::day_string(self.clock.local_day(a.before));
        let sent = self.server_query(&format!("before:{day}"));
        let (account, folders) = (a.account.clone(), a.folders.clone());
        self.searches = self.searches.wrapping_add(1).max(1);
        let search = self.searches;
        if let Some(a) = self.archiving.as_mut() {
            a.stage = Stage::Searching(sent.clone());
            a.found.clear();
            a.moved = self.clock.now;
        }
        self.jobs.push(Job::SearchServer { account, folders, query: sent, search });
    }

    /// The server's search answered ([`Event::ServerSearched`]): what it found counted, and once
    /// it's done, what of the old mail isn't here is downloaded (or the old mail deleted).
    pub(in crate::session) fn archive_searched(&mut self, account: &str, query: &str, keys: &[MessageKey], error: Option<&str>, done: bool) {
        let Some(a) = self.archiving.as_mut().filter(|a| a.account == account && matches!(&a.stage, Stage::Searching(q) if q == query)) else {
            return;
        };
        for k in keys {
            *a.found.entry(k.folder.clone()).or_default() += 1;
        }
        if let Some(e) = error {
            a.search_error = Some(e.to_string());
        }
        a.moved = self.clock.now;
        if done && a.delete {
            self.archive_delete();
        } else if done {
            self.archive_download();
        }
    }

    /// The folders' old mail as their lists here have it, but what's gone already.
    fn archive_old_here(&self) -> Vec<Summary> {
        let (Some(a), Some(store)) = (self.archiving.as_ref(), self.store.as_ref()) else { return Vec::new() };
        let mut old = Vec::new();
        for f in &a.folders {
            let Ok(st) = store.load_folder(&a.account, f) else { continue };
            old.extend(st.messages.iter().filter(|m| (1..a.before).contains(&m.when()) && !a.gone.contains(&m.key)).map(|m| Summary::clone(m)));
        }
        old
    }

    /// The old mail of the folders that isn't here, asked for; the file is written once it's
    /// all come (or at once, when it's all here).
    fn archive_download(&mut self) {
        let store = self.store.clone();
        let waiting: HashSet<MessageKey> =
            self.archive_old_here().into_iter().filter(|m| store.as_ref().is_some_and(|s| !s.has_body(&m.key))).map(|m| m.key).collect();
        if waiting.is_empty() {
            return self.archive_write();
        }
        let n = waiting.len();
        self.notice(
            if n == 1 { "Archiving: downloading 1 old message…".to_string() } else { format!("Archiving: downloading {n} old messages…") },
            false,
        );
        for key in &waiting {
            self.jobs.push(Job::FetchBody { key: key.clone() });
        }
        if let Some(a) = self.archiving.as_mut() {
            a.stage = Stage::Downloading(waiting, 0);
            a.moved = self.clock.now;
        }
    }

    /// A message's source came, or couldn't (`failed`): one the archive waits for is no longer
    /// waited for, and the file is written once none is.
    pub(in crate::session) fn archive_source(&mut self, key: &MessageKey, failed: bool) {
        let now = self.clock.now;
        let Some(a) = self.archiving.as_mut() else { return };
        let Stage::Downloading(waiting, missed) = &mut a.stage else { return };
        if !waiting.remove(key) {
            return;
        }
        if failed {
            *missed += 1;
        }
        a.moved = now;
        if waiting.is_empty() {
            self.archive_write();
        }
    }

    /// Downloads that stopped coming aren't waited for for ever: what came is archived.
    pub(in crate::session) fn archive_waits_due(&mut self) {
        let due = self
            .archiving
            .as_ref()
            .is_some_and(|a| matches!(a.stage, Stage::Downloading(..)) && self.clock.now.saturating_sub(a.moved) > DOWNLOAD_WAIT);
        if due {
            log::info!("archive: downloads stopped coming; archiving what came");
            self.archive_write();
        }
    }

    /// The file written with the old mail that's here.
    fn archive_write(&mut self) {
        let Some(a) = self.archiving.as_mut() else { return };
        a.stage = Stage::Writing;
        a.moved = self.clock.now;
        let job = Job::ArchivePst {
            account: a.account.clone(),
            path: a.path.clone(),
            folders: a.roots.clone(),
            only: a.only,
            before: a.before,
            follow_up: mailcraft_model::organize::FollowUpDays {
                due: self.prefs.follow_up.clone(),
                done: self.prefs.follow_up_done.clone(),
                reminder: self.prefs.mail_reminders.clone(),
            },
        };
        self.jobs.push(job);
    }

    /// The folders' old mail deleted for good (a folder AutoArchive deletes from), and another
    /// round, or the end.
    fn archive_delete(&mut self) {
        let old: Vec<MessageKey> = self.archive_old_here().into_iter().map(|m| m.key).collect();
        let on_server = self.archiving.as_ref().and_then(|a| self.account(&a.account)).is_some_and(|x| !x.is_local() && x.config.pop.is_none());
        let Some(a) = self.archiving.as_mut() else { return };
        a.gone.extend(old.iter().cloned());
        let more = on_server && a.found.values().any(|n| *n >= FOUND_PER_FOLDER) && !old.is_empty() && a.round < ROUNDS_MAX && !a.cut;
        let account = a.account.clone();
        if !old.is_empty()
            && let Err(e) = self.remove_archived(&account, old)
        {
            self.archiving = None;
            self.notice(format!("Old mail couldn't be deleted: {e}"), true);
            return self.next_auto_archive();
        }
        if more {
            if let Some(a) = self.archiving.as_mut() {
                a.round += 1;
            }
            return self.archive_search();
        }
        if let Some(a) = self.archiving.take() {
            self.archive_end(a, 0, 0);
        }
    }

    /// Delete what the file has (or what's deleted for good) from the mailbox, the open message
    /// giving way to the next.
    fn remove_archived(&mut self, account: &str, keys: Vec<MessageKey>) -> Result<(), String> {
        let next = self.successor(&keys);
        let open = self.selection.iter().any(|k| keys.contains(k));
        self.apply_op_checked(account, Op::Expunge { keys })?;
        if open {
            self.after_removal(next);
        }
        Ok(())
    }

    /// The server's search was stopped by another search, or by the search box cleared, while
    /// the archive's ran: what it hadn't looked through yet isn't archived this time.
    pub(in crate::session) fn archive_search_cut(&mut self) {
        if let Some(a) = self.archiving.as_mut().filter(|a| matches!(a.stage, Stage::Searching(_))) {
            a.cut = true;
        }
    }

    /// An account gone: its archive with it, and AutoArchive's for it.
    pub(in crate::session) fn forget_archiving(&mut self, account: &str) {
        self.auto_archive_forget(account);
        if self.archiving.as_ref().is_some_and(|a| a.account == account) {
            self.archiving = None;
            self.next_auto_archive();
        }
    }

    /// How archiving goes ([`Event::Archived`]). Done: what the file has now leaves the mailbox;
    /// then another round, or the file opens in the folder list.
    pub(in crate::session) fn archived(&mut self, account: &str, path: &str, report: ArchiveReport, done: bool, error: Option<String>) {
        let ArchiveReport { messages, archived, skipped, folders } = report;
        // Only an account on a server has more to search for.
        let on_server = matches!(self.account(account), Some(x) if !x.is_local() && x.config.pop.is_none());
        let Some(a) = self.archiving.as_mut().filter(|a| a.account == account && a.path == path && matches!(a.stage, Stage::Writing)) else { return };
        if !done {
            let so_far = a.written.saturating_add(messages);
            self.notice(format!("Archiving: {so_far} messages so far…"), false);
            return;
        }
        let file = mailcraft_model::file_name(path).to_string();
        if let Some(e) = error {
            self.archiving = None;
            self.notice(format!("Old mail couldn't be archived, so it stays where it was: {e}"), true);
            return self.next_auto_archive();
        }
        a.written = a.written.saturating_add(messages);
        // Only this account's, and each once (a round may name again what's still going).
        let new: Vec<MessageKey> = archived.into_iter().filter(|k| k.account == account && !a.gone.contains(k)).collect();
        a.gone.extend(new.iter().cloned());
        let more = on_server && a.found.values().any(|n| *n >= FOUND_PER_FOLDER) && !new.is_empty() && a.round < ROUNDS_MAX && !a.cut;
        if !new.is_empty()
            && let Err(e) = self.remove_archived(account, new)
        {
            self.archiving = None;
            self.notice(format!("The old mail is in {file}, but couldn't be deleted from the mailbox: {e}"), true);
            return self.next_auto_archive();
        }
        if more {
            if let Some(a) = self.archiving.as_mut() {
                a.round += 1;
                let so_far = a.gone.len();
                self.notice(format!("Archiving: {so_far} messages so far…"), false);
            }
            return self.archive_search();
        }
        if let Some(a) = self.archiving.take() {
            self.archive_end(a, skipped, folders);
        }
    }

    /// An archive done: said, the file shown in the folder list, and AutoArchive's next one.
    fn archive_end(&mut self, a: Archiving, missed: usize, folders: usize) {
        let count = a.gone.len();
        let file = mailcraft_model::file_name(&a.path).to_string();
        let said = match (a.delete, count) {
            (true, 0) => None,
            (true, 1) => Some("1 old message deleted".to_string()),
            (true, n) => Some(format!("{n} old messages deleted")),
            (false, 0) => Some(format!("No mail old enough to archive was found, so nothing went to {file}.")),
            (false, 1) => Some(format!("1 message archived to {file}")),
            (false, n) => Some(format!("{n} messages archived to {file}")),
        };
        if let Some(said) = said {
            self.notice(said, false);
        }
        if missed > 0 {
            let (n, why) = if missed == 1 {
                ("1 old message".to_string(), "it couldn't be downloaded")
            } else {
                (format!("{missed} old messages"), "they couldn't be downloaded")
            };
            self.notice(format!("{n} stayed in the mailbox, as {why}."), true);
        }
        if folders > 0 {
            let which = if folders == 1 { "1 folder".to_string() } else { format!("{folders} folders") };
            self.notice(format!("The mail of {which} couldn't be read, so it wasn't archived."), true);
        }
        if let Some(e) = &a.search_error {
            self.notice(format!("The server couldn't be searched for all the old mail ({e}): archive again to take the rest."), true);
        } else if a.cut {
            self.notice("Another search stopped the one for old mail: archive again to take the rest.".to_string(), true);
        }
        // The file in the folder list: opened, or what's new read into it when it's open already.
        if !a.delete
            && a.show
            && (a.written > 0 || count > 0)
            && let Err(e) = self.import_pst(&a.path, false, Some(Changes::Read), false)
        {
            self.notice(format!("{file} couldn't be opened: {e}"), true);
        }
        self.next_auto_archive();
    }
}
