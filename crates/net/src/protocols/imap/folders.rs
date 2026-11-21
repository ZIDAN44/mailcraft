//! Folders: listed with their roles, selected (with QRESYNC's changes since a known state),
//! counted, made, renamed, deleted and emptied.

use super::*;

/// What `SELECT` reports.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SelectInfo {
    pub exists: u32,
    pub validity: u32,
    pub uid_next: u32,
    pub highest_modseq: u64,
    pub read_only: bool,
    /// The folder can't be given new keywords: its `PERMANENTFLAGS` has no `\*` (RFC 3501
    /// §7.1), as on a shared folder without the right to write flags. Only the keywords in
    /// [`permanent_keywords`](Self::permanent_keywords) last.
    pub no_new_keywords: bool,
    /// The keywords `PERMANENTFLAGS` named.
    pub permanent_keywords: Vec<String>,
}

impl SelectInfo {
    /// Whether the server keeps `keyword` on this folder's mail (no `PERMANENTFLAGS` at all:
    /// everything may be changed for good).
    pub fn keeps_keyword(&self, keyword: &str) -> bool {
        !self.no_new_keywords || self.permanent_keywords.iter().any(|k| k.eq_ignore_ascii_case(keyword))
    }
}

/// What a QRESYNC `SELECT` reports: the usual data, the UID ranges expunged since the cached
/// state, and the messages whose flags changed.
pub type Resync = (SelectInfo, Vec<std::ops::RangeInclusive<u32>>, Vec<Fetched>);

impl<S: Read + Write> Client<S> {
    /// All folders, with special-use roles.
    pub fn list(&mut self, account: &str) -> Result<Vec<Folder>> {
        let cmd = if self.has("SPECIAL-USE") && self.has("LIST-EXTENDED") { "LIST \"\" \"*\" RETURN (SPECIAL-USE)" } else { "LIST \"\" \"*\"" };
        let rs = self.cmd(&[Part::Text(cmd)])?;
        let mut out: Vec<Folder> = Vec::new();
        // Which roles the server gave (RFC 6154), not guessed from a name.
        let mut flagged = HashSet::new();
        for r in rs {
            let Response::MailboxData(MailboxDatum::List(l)) = r else { continue };
            let delimiter = l.delimiter.as_deref().and_then(|d| d.chars().next());
            let path = l.name.to_string();
            let mut role = None;
            let mut selectable = true;
            for a in &l.name_attributes {
                match a {
                    NameAttribute::NoSelect => selectable = false,
                    NameAttribute::Extension(e) if e.eq_ignore_ascii_case("\\NonExistent") => selectable = false,
                    NameAttribute::Sent => role = Some(FolderRole::Sent),
                    NameAttribute::Drafts => role = Some(FolderRole::Drafts),
                    NameAttribute::Trash => role = Some(FolderRole::Trash),
                    NameAttribute::Junk => role = Some(FolderRole::Junk),
                    NameAttribute::Archive => role = Some(FolderRole::Archive),
                    NameAttribute::All => role = role.or(Some(FolderRole::Archive)),
                    NameAttribute::Extension(e) => role = role.or(FolderRole::from_attribute(e)),
                    _ => {}
                }
            }
            if out.iter().any(|f| f.path == path) {
                continue;
            }
            if role.is_some() && !path.eq_ignore_ascii_case("INBOX") {
                flagged.insert(path.clone());
            }
            let role =
                if path.eq_ignore_ascii_case("INBOX") { FolderRole::Inbox } else { role.unwrap_or_else(|| FolderRole::from_name(&path, delimiter)) };
            out.push(Folder { account: account.to_string(), path, delimiter, role, selectable, ..Default::default() });
        }
        one_per_role(&mut out, &flagged);
        Ok(out)
    }

    /// `SELECT` (or `EXAMINE` when `read_only`), asking for CONDSTORE data when supported.
    pub fn select(&mut self, folder: &str, read_only: bool) -> Result<SelectInfo> {
        let verb = if read_only { "EXAMINE " } else { "SELECT " };
        let arg = mailbox_arg(folder);
        let condstore = if self.has("CONDSTORE") || self.qresync { " (CONDSTORE)" } else { "" };
        let cmd = format!("{verb}{arg}{condstore}");
        // Refused, the server has no folder selected (RFC 3501 §6.3.1): nor has this.
        self.selected = None;
        let rs = self.cmd(&[Part::Text(&cmd)])?;
        let (info, _, _) = self.selected_from(folder, rs);
        Ok(info)
    }

    /// `SELECT … (QRESYNC (validity modseq known-uids))` (RFC 7162 §3.2.5): besides the usual
    /// data, the server reports what was expunged (`VANISHED (EARLIER)`, as UID ranges) and the
    /// flags that changed since `modseq`, in one round trip. Only for a cache whose UIDVALIDITY
    /// is `validity`: when the server's differs it sends neither, and the caller starts over.
    pub fn select_qresync(&mut self, folder: &str, validity: u32, modseq: u64, known: &str) -> Result<Resync> {
        let known = if known.is_empty() { String::new() } else { format!(" {known}") };
        let cmd = format!("SELECT {} (QRESYNC ({validity} {modseq}{known}))", mailbox_arg(folder));
        self.selected = None;
        let rs = self.cmd(&[Part::Text(&cmd)])?;
        Ok(self.selected_from(folder, rs))
    }

    pub(super) fn selected_from(&mut self, folder: &str, rs: Vec<Response<'static>>) -> Resync {
        let mut info = SelectInfo::default();
        let mut vanished = Vec::new();
        let mut changed = Vec::new();
        for r in rs {
            match r {
                Response::Vanished { uids, .. } => vanished.extend(uids),
                Response::Fetch(_, attrs) => {
                    let f = fetched(attrs);
                    if f.uid != 0 {
                        changed.push(f);
                    }
                }
                Response::MailboxData(MailboxDatum::Exists(n)) => info.exists = n,
                Response::Data { outcome, .. } | Response::Done { outcome, .. } => match outcome.code {
                    Some(ResponseCode::UidValidity(v)) => info.validity = v,
                    Some(ResponseCode::UidNext(v)) => info.uid_next = v,
                    Some(ResponseCode::HighestModSeq(v)) => info.highest_modseq = v,
                    Some(ResponseCode::ReadOnly) => info.read_only = true,
                    Some(ResponseCode::PermanentFlags(flags)) => {
                        info.no_new_keywords = !flags.iter().any(|f| f == r"\*");
                        info.permanent_keywords = flags.iter().filter(|f| !f.starts_with('\\')).take(1000).map(|f| f.to_string()).collect();
                    }
                    _ => {}
                },
                _ => {}
            }
        }
        self.selected = Some(folder.to_string());
        self.exists = info.exists;
        self.news = false;
        self.seen.insert(folder.to_string(), info.clone());
        (info, vanished, changed)
    }

    /// What the last `SELECT` of `folder` on this connection reported (a sync's, say).
    pub fn seen(&self, folder: &str) -> Option<&SelectInfo> {
        self.seen.get(folder)
    }

    /// Whether the selected folder grew during a command since it was selected (and since the
    /// last time this was asked).
    pub fn take_news(&mut self) -> bool {
        std::mem::take(&mut self.news)
    }

    /// `STATUS` counts for a folder that isn't selected: (messages, unseen).
    pub fn status(&mut self, folder: &str) -> Result<(u32, u32)> {
        let cmd = format!("STATUS {} (MESSAGES UNSEEN)", mailbox_arg(folder));
        let rs = self.cmd(&[Part::Text(&cmd)])?;
        // Asked for: not a change the server told of (NOTIFY sends others' unasked).
        self.notified.retain(|f| f != folder);
        let (mut total, mut unseen) = (0, 0);
        for r in rs {
            if let Response::MailboxData(MailboxDatum::Status { mailbox, status }) = r
                && mailbox.as_ref() == folder
            {
                for s in status {
                    match s {
                        StatusAttribute::Messages(n) => total = n,
                        StatusAttribute::Unseen(n) => unseen = n,
                        _ => {}
                    }
                }
            }
        }
        Ok((total, unseen))
    }

    /// Delete every message in a folder for good.
    pub fn empty(&mut self, folder: &str) -> Result<()> {
        let info = self.select(folder, false)?;
        if info.exists == 0 {
            return Ok(());
        }
        self.uid_store("1:*", true, "\\Deleted")?;
        self.cmd(&[Part::Text("EXPUNGE")]).map(|_| ())
    }

    /// Mark every message in a folder read, those not downloaded too (issue #197).
    pub fn mark_read(&mut self, folder: &str) -> Result<()> {
        let info = self.select(folder, false)?;
        if info.exists == 0 {
            return Ok(());
        }
        self.uid_store("1:*", true, "\\Seen")
    }

    /// Create a folder and subscribe to it (some clients only show subscribed folders). A folder
    /// that already exists counts as created.
    pub fn create(&mut self, folder: &str) -> Result<()> {
        let cmd = format!("CREATE {}", mailbox_arg(folder));
        match self.cmd(&[Part::Text(&cmd)]) {
            Ok(_) => {}
            Err(NetError::No(m)) if m.to_ascii_uppercase().contains("ALREADYEXISTS") || m.to_ascii_lowercase().contains("exists") => {}
            Err(e) => return Err(e),
        }
        let _ = self.cmd(&[Part::Text(&format!("SUBSCRIBE {}", mailbox_arg(folder)))]);
        Ok(())
    }

    /// Rename a folder (folders inside it move with it, RFC 3501 §6.3.5).
    pub fn rename(&mut self, from: &str, to: &str) -> Result<()> {
        if self.selected.as_deref().is_some_and(|s| s.starts_with(from)) {
            // Renaming the selected folder is allowed, but leaves the session in an odd state on
            // some servers: deselect first.
            self.unselect();
        }
        let cmd = format!("RENAME {} {}", mailbox_arg(from), mailbox_arg(to));
        self.cmd(&[Part::Text(&cmd)])?;
        let _ = self.cmd(&[Part::Text(&format!("UNSUBSCRIBE {}", mailbox_arg(from)))]);
        let _ = self.cmd(&[Part::Text(&format!("SUBSCRIBE {}", mailbox_arg(to)))]);
        Ok(())
    }

    /// Delete a folder. One that is already gone counts as deleted.
    pub fn delete(&mut self, folder: &str) -> Result<()> {
        if self.selected.as_deref().is_some_and(|s| s.starts_with(folder)) {
            self.unselect();
        }
        let _ = self.cmd(&[Part::Text(&format!("UNSUBSCRIBE {}", mailbox_arg(folder)))]);
        match self.cmd(&[Part::Text(&format!("DELETE {}", mailbox_arg(folder)))]) {
            Ok(_) => Ok(()),
            Err(NetError::No(m)) if m.to_ascii_uppercase().contains("NONEXISTENT") => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Leave the selected folder (`UNSELECT` when offered; `CLOSE` would expunge).
    pub(super) fn unselect(&mut self) {
        if self.has("UNSELECT") {
            let _ = self.cmd(&[Part::Text("UNSELECT")]);
        } else if self.selected.is_some() {
            // A failed EXAMINE leaves the selected state without expunging (RFC 3501 §6.3.1).
            let _ = self.cmd(&[Part::Text("EXAMINE \"MailCraft no such folder\"")]);
        }
        self.selected = None;
    }
}

/// One folder per special role, the rest ordinary folders: the one the server flags for it, else
/// one that can be opened, else the first listed. Servers don't list it first (Dovecot lists cPanel's
/// flagged `INBOX.Sent` after `INBOX.Sent Items` made by another program: issue #19, where sent
/// mail's copies went in the one nobody looks at).
pub(super) fn one_per_role(folders: &mut [Folder], flagged: &HashSet<String>) {
    let mut order: Vec<usize> = (0..folders.len()).collect();
    order.sort_by_key(|&i| folders.get(i).map(|f| (!flagged.contains(&f.path), !f.selectable)));
    let mut seen = HashSet::new();
    for i in order {
        if let Some(f) = folders.get_mut(i)
            && f.role != FolderRole::Other
            && !seen.insert(f.role)
        {
            f.role = FolderRole::Other;
        }
    }
}
