//! An account's worker: the look for new mail between full syncs. The Inbox, and on an Exchange
//! account every folder the server says changed (issue #145): mail an Inbox rule files into a
//! folder of its own came only with the next full sync, a quarter of an hour on. And the mail
//! looked at between the steps of an Exchange account's calendars and contacts, which took
//! minutes and held new mail up, Send / Receive with it.

use super::*;

impl Worker {
    /// Look for new mail: the Inbox, then (Exchange) the folders the server says changed.
    pub(crate) fn look_for_mail(&mut self) {
        let mut folders: Vec<String> = self.inbox().into_iter().collect();
        for f in self.watched_changes() {
            if !folders.contains(&f) {
                folders.push(f);
            }
        }
        for f in folders {
            if self.closed || self.conn.is_none() {
                break;
            }
            self.sync_one(&f);
        }
        self.looked = Some(Instant::now());
    }

    /// How long a subscription lasts between looks: three looks' time, half an hour at least.
    fn watch_minutes(&self) -> u32 {
        let every = self.check_every().unwrap_or_else(|| poll_every(&self.acc.id)).as_secs() / 60;
        u32::try_from(every.saturating_mul(3)).unwrap_or(u32::MAX).clamp(30, 1440)
    }

    /// Exchange: a subscription to the mailbox's folders, made when there's none (it says what
    /// changes from then on). One refused isn't asked for again until the next full sync's time.
    pub(crate) fn watch_from_now(&mut self) {
        if self.ews_watch.is_some() || self.watch_refused.is_some_and(|at| at.elapsed() < self.full_sync_every()) {
            return;
        }
        let minutes = self.watch_minutes();
        let Some(Conn::Ews(c)) = &self.conn else { return };
        let ids: Vec<String> = self.jstate.mailboxes.values().cloned().collect();
        match c.watch(minutes, &ids) {
            Ok(w) => {
                self.ews_watch = Some(w);
                self.watch_refused = None;
            }
            Err(e) => {
                log::info!("{}: the server won't say which folders changed ({e}): the Inbox is looked at alone", self.acc.id);
                self.watch_refused = Some(Instant::now());
            }
        }
    }

    /// Exchange: the folders (their paths) the subscription says changed since it was last asked,
    /// those that can be read. A subscription gone (lapsed while the computer slept, the server
    /// restarted) is made again: it says what changes from then on.
    fn watched_changes(&mut self) -> Vec<String> {
        if !matches!(self.conn, Some(Conn::Ews(_))) {
            return Vec::new();
        }
        if self.ews_watch.is_none() {
            self.watch_from_now();
            return Vec::new();
        }
        let changed = match (&self.conn, self.ews_watch.as_mut()) {
            (Some(Conn::Ews(c)), Some(w)) => c.changed(w),
            _ => return Vec::new(),
        };
        let ids = match changed {
            Ok(ids) => ids,
            Err(e) => {
                log::info!("{}: the folders' subscription: {e}; a new one is made", self.acc.id);
                self.ews_watch = None;
                self.watch_from_now();
                return Vec::new();
            }
        };
        let mut paths: Vec<String> = Vec::new();
        for id in ids {
            let Some(path) = self.jstate.mailboxes.iter().find(|(_, v)| **v == id).map(|(p, _)| p.clone()) else { continue };
            if self.folders.iter().any(|f| f.path == path && f.selectable) && !paths.contains(&path) {
                paths.push(path);
            }
        }
        if !paths.is_empty() {
            log::info!("{}: {} folders changed on the server", self.acc.id, paths.len());
        }
        paths
    }

    /// Between two steps of an Exchange account's calendars and contacts: what's opened or
    /// changed here served, a Send / Receive asked meanwhile done for the mail (the calendars go
    /// on after), and the look for new mail when it's due.
    pub(crate) fn between_pim_steps(&mut self, rx: Option<&Receiver<Job>>) {
        if let Some(rx) = rx {
            self.serve_between_folders(rx);
            let asked = self.queue.iter().any(|j| matches!(j, Job::SyncAll { .. }));
            if asked {
                let (send_receive, rest): (VecDeque<Job>, VecDeque<Job>) =
                    std::mem::take(&mut self.queue).into_iter().partition(|j| matches!(j, Job::SyncAll { .. }));
                self.queue = rest;
                for j in send_receive {
                    self.handle(j);
                }
                if self.conn.is_some() && !self.closed {
                    self.sync_all(rx);
                }
                return;
            }
        }
        let every = self.check_every().unwrap_or_else(|| poll_every(&self.acc.id));
        if self.conn.is_some() && self.looked.is_none_or(|t| t.elapsed() >= every) {
            self.look_for_mail();
        }
    }
}
