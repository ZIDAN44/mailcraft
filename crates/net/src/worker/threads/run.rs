use super::*;

impl Worker {
    pub(in crate::worker) fn run(mut self, rx: Receiver<Job>) {
        self.status(SyncStatus::Connecting);
        while !self.closed {
            self.working(true);
            self.drain(&rx);
            if self.closed {
                break;
            }
            self.fresh_token();
            // Connected by the last job itself: the drafts kept here go first.
            if self.file_next_kept() {
                continue;
            }
            if let Some(job) = self.queue.pop_front() {
                self.handle(job);
                continue;
            }
            if let Some(key) = self.rest.pop_front() {
                self.fetch_whole(key);
                continue;
            }
            if self.conn.is_none() {
                if self.auth_failed {
                    self.working(false);
                    match rx.recv() {
                        Ok(j) => push_job(&mut self.queue, j),
                        Err(_) => break,
                    }
                    continue;
                }
                let wait = self.next_attempt.saturating_duration_since(Instant::now());
                if !wait.is_zero() {
                    #[cfg(test)]
                    connect::waits::began(&self.store, &self.acc.id);
                    self.working(false);
                    match rx.recv_timeout(wait) {
                        Ok(j) => push_job(&mut self.queue, j),
                        Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                    continue;
                }
                self.connect_and_sync();
                continue;
            }
            if self.last_full.is_none_or(|t| t.elapsed() >= FULL_SYNC_EVERY) {
                let _ = self.sync_all(&rx);
                continue;
            }
            self.wait_for_mail(&rx);
        }
        self.keep_unsaved_drafts(&rx);
        if let Some(Conn::Imap(c)) = self.conn.as_mut() {
            c.logout();
        }
    }
}
