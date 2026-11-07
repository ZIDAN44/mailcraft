//! An account's worker: the drafts kept on this computer, filed at each connection ahead of
//! anything queued, and those that failed tried again after a wait of their own (#53).

use super::*;

/// What an account's worker files of the drafts kept on this computer. Each stays in the kept
/// drafts' file until it's filed, or replaced there by a newer version (found in review: taken
/// out of it as the connection was made, the drafts still to file were held in memory only, and
/// lost when MailCraft closed meanwhile).
#[derive(Default)]
pub(crate) struct KeptDrafts {
    /// Connected, with the kept drafts still to read ([`Worker::queue_kept_drafts`]).
    pub(crate) due: bool,
    /// Those read for this connection, filed ahead of anything queued.
    pub(super) queued: VecDeque<PendingDraft>,
    /// Those that failed on this run's connections (by their source's hash): each tried again
    /// once a wait of its own is over, after the others (found in review: one that dropped the
    /// connection was read again ahead of everything at each connection, and nothing else got
    /// through).
    failed: HashMap<u64, Retry>,
    /// Saves kept on this computer while the account waited to connect again (by their source's
    /// hash): filed first at the next connection, ahead of the other kept drafts (found in review:
    /// behind each kept draft that dropped the connection, a save waited one connection more).
    pub(super) waiting: std::collections::HashSet<u64>,
    /// Those set aside as another worker files a version of each: read again once one of those
    /// filings ends.
    pub(crate) aside: SetAside,
    /// When a connection a draft's own filing dropped was made again at once, in the last
    /// [`RETRY_FIRST`] ([`Worker::filing_failed`]).
    at_once: VecDeque<Instant>,
}

/// When a kept draft that failed is tried again, and the wait after that if it fails again.
struct Retry {
    at: Instant,
    wait: Duration,
}

/// The first wait before a kept draft that failed is tried again; each failure doubles it, up to
/// [`RETRY_MOST`].
const RETRY_FIRST: Duration = Duration::from_secs(30);
const RETRY_MOST: Duration = Duration::from_secs(60 * 60);

/// Connections a draft's own filing dropped made again at once, at most, in [`RETRY_FIRST`]; past
/// that, each waits as a connection that failed does.
const AT_ONCE_MOST: usize = 4;

impl KeptDrafts {
    /// Send / Receive asked for: the kept drafts are read again, and those that failed are tried
    /// again at once.
    pub(crate) fn retry_now(&mut self) {
        self.due = true;
        let now = Instant::now();
        for r in self.failed.values_mut() {
            r.at = now;
        }
    }

    /// Whether a save of a draft (`message_id`, the copy it `replaces`) goes ahead of the kept
    /// drafts read for this connection: once one has failed in this run (it may drop each
    /// connection), unless one of them is a version of the same draft, older, which goes first.
    pub(super) fn goes_ahead(&self, message_id: &str, replaces: Option<&MessageKey>) -> bool {
        let same = |d: &PendingDraft| {
            (replaces.is_some() && d.replaces.as_ref() == replaces) || (!message_id.is_empty() && written_message_id(&d.raw) == message_id)
        };
        self.queued.is_empty() || (!self.failed.is_empty() && !self.queued.iter().any(same))
    }

    /// A kept draft failed (its source's hash): tried again after its wait, doubled from the
    /// last. Whether it's the first time in this run (only that is said).
    pub(super) fn note_failed(&mut self, hash: u64) -> bool {
        let now = Instant::now();
        match self.failed.entry(hash) {
            std::collections::hash_map::Entry::Occupied(mut e) => {
                let r = e.get_mut();
                r.wait = r.wait.saturating_mul(2).min(RETRY_MOST);
                r.at = now.checked_add(r.wait).unwrap_or(now);
                false
            }
            std::collections::hash_map::Entry::Vacant(e) => {
                e.insert(Retry { at: now.checked_add(RETRY_FIRST).unwrap_or(now), wait: RETRY_FIRST });
                true
            }
        }
    }

    /// Whether a connection a draft's own filing dropped is made again at once: no more than
    /// [`AT_ONCE_MOST`] times in [`RETRY_FIRST`].
    fn again_at_once(&mut self) -> bool {
        let now = Instant::now();
        while self.at_once.front().is_some_and(|t| now.saturating_duration_since(*t) >= RETRY_FIRST) {
            self.at_once.pop_front();
        }
        if self.at_once.len() >= AT_ONCE_MOST {
            return false;
        }
        self.at_once.push_back(now);
        true
    }
}

impl Worker {
    /// The drafts kept on this computer, read once connected ([`KeptDrafts::due`]) to be filed
    /// ahead of anything queued: ahead of any save of the same draft queued or on its way, newer
    /// (found in review: a save that connected by itself went first, and the older one kept,
    /// filed at the next connection, replaced it on the server). One that failed in this run goes
    /// only once its own wait is over, after the others, read again then if the account is still
    /// connected; a newer version saved meanwhile has replaced it. The saves kept while the
    /// account waited to connect again go first. Those set aside are read again with the others
    /// once another worker's filing of one of them ends. How many.
    pub(crate) fn queue_kept_drafts(&mut self) -> usize {
        let now = Instant::now();
        let retry = self.conn.is_some() && self.kept.failed.values().any(|r| r.at <= now);
        let aside = self.conn.is_some() && self.kept.aside.ended(&self.filing);
        if !std::mem::take(&mut self.kept.due) && !retry && !aside {
            return 0;
        }
        // Each is set aside again as it comes, if still filed elsewhere.
        self.kept.aside.clear();
        let kept: Vec<PendingDraft> = {
            let _one_at_a_time = pending_file();
            self.store.load_account_doc(&self.acc.id, PENDING_DRAFTS).unwrap_or_default()
        };
        let hashes: Vec<u64> = kept.iter().map(|d| hash_of(&d.raw)).collect();
        // Those kept no more (filed, or replaced by a newer version) are forgotten.
        self.kept.failed.retain(|h, _| hashes.contains(h));
        self.kept.waiting.retain(|h| hashes.contains(h));
        let (mut queued, mut others, mut later) = (VecDeque::new(), VecDeque::new(), VecDeque::new());
        for (d, hash) in kept.into_iter().zip(hashes) {
            match self.kept.failed.get_mut(&hash) {
                None if self.kept.waiting.contains(&hash) => queued.push_back(d),
                None => others.push_back(d),
                Some(r) if r.at <= now => {
                    // Not read again for another wait, unless it fails first.
                    r.at = now.checked_add(r.wait).unwrap_or(now);
                    later.push_back(d);
                }
                Some(_) => {}
            }
        }
        queued.append(&mut others);
        queued.append(&mut later);
        self.kept.queued = queued;
        self.kept.queued.len()
    }

    /// The next kept draft read for this connection ([`Self::queue_kept_drafts`]) filed, ahead of
    /// anything queued; `false` when there's none (nor a connection: they're read again at the
    /// next one). It stays kept on this computer until it's filed. Nothing is said of it filed: it
    /// was said kept as it was kept. A failure is said the first time in this run, and it's tried
    /// again after a wait of its own. One another worker is filing a version of (the account's
    /// connection started again meanwhile) is set aside, and looked at once that ends.
    pub(crate) fn file_next_kept(&mut self) -> bool {
        self.queue_kept_drafts();
        if self.conn.is_none() {
            self.kept.queued.clear();
            return false;
        }
        // A save waiting in the queue goes ahead of the kept drafts of other drafts once one has
        // failed in this run (found in review: each that dropped the connection held it back by
        // one more connection).
        let goes_ahead =
            |j: &Job| matches!(j, Job::SaveDraft { raw, replaces, .. } if self.kept.goes_ahead(&written_message_id(raw), replaces.as_ref()));
        let ahead = if self.kept.queued.is_empty() { None } else { self.queue.iter().position(goes_ahead) };
        if let Some(Job::SaveDraft { raw, folder, replaces, .. }) = ahead.and_then(|i| self.queue.remove(i)) {
            self.save_draft(raw, folder, replaces);
            return true;
        }
        let Some(d) = self.kept.queued.pop_front() else { return false };
        let (message_id, hash) = (written_message_id(&d.raw), hash_of(&d.raw));
        // Never waited for (found in review: the new connection waited for the old one's filing
        // to end, a minute on a link that hung, and the whole account with it).
        let Some(being_filed) = Filing::begin(&self.filing, &self.acc.id, hash, &message_id) else {
            self.kept.aside.push(&self.acc.id, hash, &message_id);
            return true;
        };
        // Filed since it was read, or replaced by a newer version: not filed again.
        if !self.still_kept(&d.raw) {
            return true;
        }
        match self.file_draft(&d.folder, d.replaces.clone(), &d.raw, &message_id) {
            Ok(_) if self.removed() => log::info!("{}: a draft of an account removed was saved", self.acc.id),
            Ok(filed) => self.draft_filed(&d.raw, d.replaces.as_ref(), &message_id, filed, being_filed),
            Err(e) if self.removed() => log::info!("{}: a draft of an account removed wasn't saved: {e}", self.acc.id),
            Err(e) => {
                if self.kept.note_failed(hash) {
                    self.notice(format!(
                        "a draft kept on this computer couldn't be saved to the server ({e}); it stays kept here, and is tried again later"
                    ));
                }
                self.filing_failed(e);
            }
        }
        true
    }

    /// A draft's own filing failed (`e`), and it's kept on this computer, tried again after a wait
    /// of its own ([`KeptDrafts::note_failed`]). A connection it dropped (the draft too big for a
    /// proxy, say) is made again at once for everything else, rather than after the wait for a
    /// connection that failed, a few times in a while at most (found in review: each save that
    /// dropped it kept those after it waiting one wait more, and a save behind three got there
    /// after 19 s). One that can't be made waits as ever.
    pub(super) fn filing_failed(&mut self, e: NetError) {
        if !e.is_fatal() && !matches!(e, NetError::Auth(_)) {
            log::warn!("{}: {e}", self.acc.id);
            return;
        }
        let (dropped, backoff) = (e.is_fatal(), self.backoff);
        self.failed(e);
        if dropped && self.conn.is_none() && !self.auth_failed && self.kept.again_at_once() {
            self.next_attempt = Instant::now();
            self.backoff = backoff;
        }
    }

    /// Whether the kept drafts' file still has `raw` (not filed since, nor replaced by a newer
    /// version).
    fn still_kept(&self, raw: &[u8]) -> bool {
        let _one_at_a_time = pending_file();
        self.store.load_account_doc::<Vec<PendingDraft>>(&self.acc.id, PENDING_DRAFTS).is_ok_and(|l| l.iter().any(|d| d.raw == raw))
    }
}
