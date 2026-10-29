//! An account's worker: sending (on a second connection when it's free), the copy in Sent Items, and the Outbox.

use super::*;

/// Said when a sent message's copy has nowhere to go.
const NO_SENT: &str = "sent, but no copy was saved: the server has no Sent Items folder, and one couldn't be made";

/// Whether a folder's name says Sent Items.
fn sent_name(path: &str, delimiter: Option<char>) -> bool {
    FolderRole::from_name(path, delimiter) == FolderRole::Sent
}

impl Worker {
    /// The account's second connection: sends mail and reads or changes the automatic replies,
    /// whatever the main connection is doing. It connects when there's something to do, and lets
    /// go of an idle connection after a while.
    pub(super) fn run_quick(mut self, rx: Receiver<Job>) {
        loop {
            let job = match rx.recv_timeout(QUICK_IDLE) {
                Ok(j) => j,
                Err(RecvTimeoutError::Timeout) => {
                    self.let_go();
                    continue;
                }
                Err(RecvTimeoutError::Disconnected) => break,
            };
            self.fresh_token();
            match job {
                Job::Send { account, id, raw, from, recipients, sent_folder } => {
                    self.send_quick(Job::Send { account, id, raw, from, recipients, sent_folder });
                }
                j @ (Job::GetVacation { .. } | Job::SetVacation { .. }) => self.handle(j),
                // Anything else belongs to the main connection.
                other => {
                    if let Some(m) = &self.main {
                        let _ = m.send(other);
                    }
                }
            }
        }
        self.let_go();
    }

    /// Close an idle connection (servers drop them anyway).
    pub(super) fn let_go(&mut self) {
        if let Some(Conn::Imap(c)) = self.conn.as_mut() {
            c.logout();
        }
        self.conn = None;
    }

    /// Send on the second connection. Sent: a copy filed in Sent Items (IMAP), and the main
    /// connection told to look there. Not sent for now (offline, asked to wait): handed to the
    /// main connection, which keeps it in the Outbox; refused: said.
    pub(super) fn send_quick(&mut self, job: Job) {
        let Job::Send { id, raw, from, recipients, sent_folder, .. } = &job else { return };
        match self.deliver(raw, from, recipients, sent_folder.as_deref()) {
            Ok(()) => {
                log::info!("{}: message {id} sent to {} recipient(s)", self.acc.id, recipients.len());
                self.sink.emit(Event::Sent { account: self.id(), id: *id });
                self.file_sent_quick(raw, sent_folder.clone());
            }
            Err(e) if e.is_retryable() => {
                log::info!("{}: sending on the second connection: {e}; left to the main one", self.acc.id);
                self.conn = None;
                if let Some(m) = &self.main
                    && m.send(job.clone()).is_err()
                {
                    self.sink.emit(Event::SendFailed { account: self.id(), id: *id, error: e.to_string() });
                }
            }
            Err(e) => {
                if matches!(e, NetError::Closed(_) | NetError::Io(_)) {
                    self.conn = None;
                }
                self.sink.emit(Event::SendFailed { account: self.id(), id: *id, error: e.to_string() });
            }
        }
    }

    /// Gmail files sent mail itself.
    fn files_itself(&self) -> bool {
        self.acc.smtp.host.ends_with("gmail.com") || self.acc.smtp.host.ends_with("googlemail.com")
    }

    /// The folder a sent message's copy goes in: the one it was sent with, else the account's
    /// Sent Items. An IMAP account with none gets one, as Thunderbird, Evolution and Outlook make
    /// one (issue #19: sent mail was kept nowhere, and nothing said so): `Sent`, or `INBOX.Sent`
    /// on a server that keeps every folder in the Inbox. Connects to make it.
    pub(super) fn sent_folder(&mut self, given: Option<String>) -> Result<Option<String>> {
        if let Some(f) = given.or_else(|| self.folders.iter().find(|f| f.role == FolderRole::Sent).map(|f| f.path.clone())) {
            return Ok(Some(f));
        }
        if self.acc.is_api() || self.files_itself() {
            return Ok(None);
        }
        if self.conn.is_none() {
            self.connect()?;
        }
        if !matches!(self.conn, Some(Conn::Imap(_))) {
            return Ok(None);
        }
        // A folder there by the name, without the role (another client's, unflagged): that one.
        if let Some(f) = self.folders.iter_mut().find(|f| f.role == FolderRole::Other && sent_name(&f.path, f.delimiter)) {
            f.role = FolderRole::Sent;
            let path = f.path.clone();
            self.publish_folders();
            return Ok(Some(path));
        }
        let delimiter = self
            .folders
            .iter()
            .find(|f| f.role == FolderRole::Inbox)
            .and_then(|f| f.delimiter)
            .or_else(|| self.folders.iter().find_map(|f| f.delimiter));
        let others: Vec<&Folder> = self.folders.iter().filter(|f| !f.path.eq_ignore_ascii_case("INBOX")).collect();
        let in_inbox = delimiter
            .map(|d| format!("INBOX{d}"))
            .filter(|p| !others.is_empty() && others.iter().all(|f| f.path.to_ascii_uppercase().starts_with(p.as_str())));
        let mut names = Vec::new();
        if let Some(p) = &in_inbox {
            names.push(format!("{p}Sent"));
        }
        names.push("Sent".to_string());
        if let (None, Some(d)) = (&in_inbox, delimiter) {
            names.push(format!("INBOX{d}Sent"));
        }
        for name in names {
            match self.client()?.create(&name) {
                Ok(()) => {
                    log::info!("{}: no Sent folder on the server; made {name}", self.acc.id);
                    self.folders.retain(|f| f.path != name);
                    self.folders.push(Folder {
                        account: self.id(),
                        path: name.clone(),
                        delimiter,
                        role: FolderRole::Sent,
                        selectable: true,
                        ..Default::default()
                    });
                    sort_folders(&mut self.folders);
                    self.publish_folders();
                    return Ok(Some(name));
                }
                // Not allowed there (a server that keeps folders in the Inbox): the next name.
                Err(NetError::No(m)) => log::info!("{}: making {name}: {m}", self.acc.id),
                Err(e) => return Err(e),
            }
        }
        Ok(None)
    }

    /// A sent message's copy: Exchange, Graph and JMAP file it themselves; IMAP servers get one
    /// (not Gmail, which files its own). Then the main connection reads Sent Items, now and, where
    /// the server files it a moment later, again then.
    pub(super) fn file_sent_quick(&mut self, raw: &[u8], sent_folder: Option<String>) {
        // What the main connection has listed since this one started.
        if let Ok(listed) = self.store.load_folders(&self.acc.id)
            && !listed.is_empty()
        {
            self.folders = listed;
        }
        let folder = if self.acc.is_api() || self.files_itself() {
            sent_folder.or_else(|| self.folders.iter().find(|f| f.role == FolderRole::Sent).map(|f| f.path.clone()))
        } else {
            let r = (|| -> Result<Option<String>> {
                let Some(folder) = self.sent_folder(sent_folder)? else { return Ok(None) };
                if self.conn.is_none() {
                    self.connect()?;
                }
                self.client()?.append(&folder, "\\Seen", raw)?;
                Ok(Some(folder))
            })();
            match r {
                Ok(Some(f)) => {
                    log::info!("{}: sent copy saved in {f}", self.acc.id);
                    Some(f)
                }
                Ok(None) => {
                    self.notice(NO_SENT.to_string());
                    None
                }
                Err(e) => {
                    self.conn = None;
                    self.notice(format!("sent, but no copy was saved in Sent Items: {e}"));
                    return;
                }
            }
        };
        let Some(folder) = folder else { return };
        let Some(main) = self.main.clone() else { return };
        let job = Job::SyncFolder { account: self.id(), folder: folder.clone() };
        let _ = main.send(job.clone());
        if self.acc.is_api() {
            // Graph and Exchange file the copy a moment after taking the message.
            let _ = std::thread::Builder::new().name("sent-later".into()).spawn(move || {
                for wait in [SENT_SOON, SENT_LATER.saturating_sub(SENT_SOON)] {
                    std::thread::sleep(wait);
                    if main.send(job.clone()).is_err() {
                        return;
                    }
                }
            });
        }
    }

    /// Hand a message to the server: SMTP, or for JMAP accounts a submission (which also files
    /// the copy in Sent).
    /// An OAuth token the server refuses (SMTP's XOAUTH2, Graph's 401) is renewed from the
    /// refresh token and the message tried once more.
    pub(super) fn deliver(&mut self, raw: &[u8], from: &str, recipients: &[String], sent_folder: Option<&str>) -> Result<()> {
        match self.deliver_once(raw, from, recipients, sent_folder) {
            Err(NetError::Auth(m)) if self.acc.oauth.is_some() => {
                log::info!("{}: sending: token refused ({m}); refreshing it", self.acc.id);
                self.renew_token()?;
                self.deliver_once(raw, from, recipients, sent_folder)
            }
            r => r,
        }
    }

    fn deliver_once(&mut self, raw: &[u8], from: &str, recipients: &[String], sent_folder: Option<&str>) -> Result<()> {
        if self.acc.graph.is_some() {
            if self.conn.is_none() {
                self.connect()?;
            }
            let Some(Conn::Graph(c)) = &self.conn else { return Err(NetError::Closed("not connected".into())) };
            return crate::graph::send(c, raw, recipients);
        }
        if self.acc.ews.is_some() {
            if self.conn.is_none() {
                self.connect()?;
            }
            let Some(Conn::Ews(c)) = &self.conn else { return Err(NetError::Closed("not connected".into())) };
            return crate::ews::send(c, raw, recipients);
        }
        if self.acc.jmap.is_some() {
            if self.conn.is_none() {
                self.connect()?;
            }
            let sent = sent_folder.map(str::to_string).or_else(|| self.folders.iter().find(|f| f.role == FolderRole::Sent).map(|f| f.path.clone()));
            let Some(Conn::Jmap(c)) = &self.conn else { return Err(NetError::Closed("not connected".into())) };
            return jmap::send(c, &self.jstate, sent.as_deref(), raw, from, recipients);
        }
        let (user, secret) = match self.acc.smtp_username.as_deref().map(str::trim).filter(|u| !u.is_empty() && self.acc.oauth.is_none()) {
            // The outgoing server's own sign-in (issue #3: its user name differs from the
            // incoming server's), with its own password.
            Some(u) => {
                let pw = self.creds.as_ref().and_then(|c| c.get_smtp(&self.acc)).ok_or_else(|| {
                    NetError::Auth(format!(
                        "the outgoing server's password for {u} isn't on this computer: enter it in the account's Server settings"
                    ))
                })?;
                (u.to_string(), pw)
            }
            None => (if self.acc.username.is_empty() { self.acc.email.clone() } else { self.acc.username.clone() }, self.secret()?),
        };
        smtp::send_with(&self.acc.smtp, &user, &secret, self.acc.oauth.is_some(), from, recipients, raw).map_err(|e| match e {
            // Say whether the sign-in covered sending: Microsoft lists what it granted.
            NetError::Auth(m) if self.acc.oauth.as_deref() == Some("microsoft") => {
                let granted = self.granted();
                let why = if granted.is_empty() {
                    String::new()
                } else if granted.to_ascii_lowercase().contains("smtp.send") {
                    " (the sign-in includes sending: Microsoft's server refused it)".to_string()
                } else {
                    format!(" (Microsoft's sign-in didn't include sending; it granted: {granted})")
                };
                NetError::Auth(format!("{m}{why}"))
            }
            e => e,
        })
    }

    pub(super) fn send(&mut self, id: u64, raw: &[u8], from: &str, recipients: &[String], sent_folder: Option<String>) {
        let sent = self.deliver(raw, from, recipients, sent_folder.as_deref());
        if let Err(e) = sent {
            if e.is_retryable() {
                // Offline or asked to wait: keep it in the Outbox and send it on reconnect.
                let item = OutboxItem {
                    id,
                    from: from.to_string(),
                    recipients: recipients.to_vec(),
                    sent_folder,
                    queued: now(),
                    attempts: 1,
                    error: Some(e.to_string()),
                    not_before: 0,
                };
                match self.store.outbox_add(&self.acc.id, &item, raw) {
                    Ok(()) => self.sink.emit(Event::Queued { account: self.id(), id, error: e.to_string() }),
                    Err(se) => {
                        self.sink.emit(Event::SendFailed { account: self.id(), id, error: format!("{e} (and the Outbox couldn't keep it: {se})") })
                    }
                }
            } else {
                self.sink.emit(Event::SendFailed { account: self.id(), id, error: e.to_string() });
            }
            return;
        }
        self.after_sent(id, raw, sent_folder);
    }

    /// Send what waits in the Outbox, oldest first. Stops at the first failure that may clear
    /// up later (still offline); a message refused for good stays, marked with the error.
    pub(super) fn flush_outbox(&mut self) {
        for mut item in self.store.outbox(&self.acc.id) {
            // Held back (Delay Delivery) until its time; then dated when it goes.
            if item.not_before > now() {
                continue;
            }
            let Some(raw) = self.store.outbox_raw(&self.acc.id, item.id) else { continue };
            let raw = if item.not_before > 0 { mailcraft_mime::with_date(&raw, now()) } else { raw };
            if item.error.as_deref().is_some_and(|e| e.starts_with("refused:")) {
                // Refused for good: waits for the user to edit or delete it.
                continue;
            }
            let sent = self.deliver(&raw, &item.from, &item.recipients, item.sent_folder.as_deref());
            match sent {
                Ok(()) => {
                    self.store.outbox_remove(&self.acc.id, item.id);
                    let folder = item.sent_folder.take();
                    self.after_sent(item.id, &raw, folder);
                }
                Err(e) => {
                    let retry = e.is_retryable();
                    item.attempts = item.attempts.saturating_add(1);
                    item.error = Some(if retry { e.to_string() } else { format!("refused: {e}") });
                    let _ = self.store.outbox_update(&self.acc.id, &item);
                    if retry {
                        self.sink.emit(Event::Queued { account: self.id(), id: item.id, error: e.to_string() });
                        return;
                    }
                    self.sink.emit(Event::SendFailed { account: self.id(), id: item.id, error: e.to_string() });
                }
            }
        }
    }

    /// A message went out: tell the engine and file a copy in Sent.
    pub(super) fn after_sent(&mut self, id: u64, raw: &[u8], sent_folder: Option<String>) {
        self.sink.emit(Event::Sent { account: self.id(), id });
        if self.acc.is_api() {
            // Filed when it was submitted.
            let sent = sent_folder.or_else(|| self.folders.iter().find(|f| f.role == FolderRole::Sent).map(|f| f.path.clone()));
            if let Some(f) = sent.filter(|_| self.conn.is_some()) {
                self.sync_one(&f);
                // Graph and Exchange file the copy a moment later: look again then.
                if matches!(self.conn, Some(Conn::Graph(_) | Conn::Ews(_))) {
                    let now = Instant::now();
                    self.soon.retain(|(_, g)| *g != f);
                    self.soon.push((now + SENT_SOON, f.clone()));
                    self.soon.push((now + SENT_LATER, f));
                }
            }
            return;
        }
        // Gmail files sent mail itself; elsewhere keep a copy in Sent.
        if self.files_itself() {
            return;
        }
        let r = (|| -> Result<Option<String>> {
            if self.conn.is_none() {
                self.connect()?;
            }
            // POP: Sent Items is on this computer.
            if let Some(Conn::Pop) = self.conn.as_ref() {
                let folder = sent_folder.clone().unwrap_or_else(|| "Sent Items".into());
                let flags = mailcraft_model::Flags { seen: true, ..Default::default() };
                return crate::pop::file_locally(&self.store, &self.acc.id, &folder, raw, flags, now()).map(|_| Some(folder));
            }
            let Some(folder) = self.sent_folder(sent_folder.clone())? else { return Ok(None) };
            self.client()?.append(&folder, "\\Seen", raw)?;
            Ok(Some(folder))
        })();
        match r {
            Ok(Some(folder)) => {
                log::info!("{}: message {id}'s sent copy saved in {folder}", self.acc.id);
                self.sync_one(&folder);
            }
            Ok(None) => self.notice(NO_SENT.to_string()),
            Err(e) => self.notice(format!("sent, but no copy was saved in Sent Items: {e}")),
        }
    }
}
