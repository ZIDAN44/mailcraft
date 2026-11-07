//! An account's worker: drafts saved to the server, and kept on this computer when they can't be
//! (#53). The kept drafts' file, the order drafts were handed out in, the drafts the pool keeps
//! as it stops a worker, and MailCraft's own copies of a draft on an IMAP server. The kept drafts
//! filed at each connection: `drafts/kept.rs`.

use super::*;

use mailcraft_model::MessageKey;
use mailcraft_model::thread::written_message_id;

mod kept;

pub(crate) use kept::KeptDrafts;

/// Drafts kept on this computer until the account connects again: those that couldn't be saved
/// to the server (offline, or it failed), and those the pool or a worker kept as the worker
/// stopped before saving them.
pub(crate) const PENDING_DRAFTS: &str = "drafts-pending.json";

#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct PendingDraft {
    pub(crate) folder: String,
    #[serde(default)]
    pub(crate) replaces: Option<MessageKey>,
    pub(crate) raw: Vec<u8>,
}

/// The order, in this run, in which drafts were handed out to be saved and kept as the newest
/// version of their draft (`Job::KeepDraft`, as MailCraft closes): a save handed out before its
/// draft was kept so, kept as it fails or its worker stops, never replaces it (found in review:
/// saved, written on and closed, and what was written after the save was lost). Kept in memory
/// only: a save made in a later run, or later in this one, is newer, and replaces it (found in
/// review: a mark kept in the file stopped the next run's saves of the draft, and lost them).
#[derive(Default)]
pub(crate) struct DraftOrder {
    next: u64,
    /// Each draft handed out to be saved (its source's hash), and when.
    handed: VecDeque<(u64, u64)>,
    /// Each draft kept as the newest version of itself, and when.
    newest: VecDeque<Newest>,
}

struct Newest {
    account: String,
    message_id: String,
    replaces: Option<MessageKey>,
    at: u64,
}

/// Drafts handed out and kept as the newest remembered, at most (the oldest forgotten: a save
/// not remembered is kept as it always was).
const MAX_ORDER: usize = 1024;

pub(crate) type Order = Arc<std::sync::Mutex<DraftOrder>>;

fn hash_of(raw: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    raw.hash(&mut h);
    h.finish()
}

impl DraftOrder {
    fn tick(&mut self) -> u64 {
        self.next = self.next.saturating_add(1);
        self.next
    }

    /// A draft handed out to be saved.
    pub(crate) fn handed(order: &Order, raw: &[u8]) {
        let Ok(mut o) = order.lock() else { return };
        let at = o.tick();
        if o.handed.len() >= MAX_ORDER {
            o.handed.pop_front();
        }
        o.handed.push_back((hash_of(raw), at));
    }

    /// When `raw` was last handed out to be saved, if this run remembers it.
    fn handed_at(&self, raw: &[u8]) -> Option<u64> {
        let hash = hash_of(raw);
        self.handed.iter().rev().find(|(h, _)| *h == hash).map(|(_, at)| *at)
    }

    /// Whether a version of a draft (`same`) was kept as the newest after `at`.
    fn kept_newest_after(&self, account: &str, at: u64, same: impl Fn(&str, Option<&MessageKey>) -> bool) -> bool {
        self.newest.iter().any(|n| n.account == account && n.at > at && same(&n.message_id, n.replaces.as_ref()))
    }

    /// Whether `raw`, a version of a draft (`same`), was handed out to be saved before a version
    /// of it was kept as the newest.
    fn older(&self, account: &str, raw: &[u8], same: impl Fn(&str, Option<&MessageKey>) -> bool) -> bool {
        self.handed_at(raw).is_some_and(|handed| self.kept_newest_after(account, handed, same))
    }

    /// Whether `kept`, another version of the draft `filed` (`same`) kept on this computer, is
    /// older than `filed`, filed just now: handed out to be saved before it (to a worker, or kept
    /// at once with the account offline), or not in this run (kept in an earlier one), and with
    /// no version of the draft kept as the newest since `filed` was handed out. Never when this
    /// run doesn't remember `filed` handed out (a draft read from the kept drafts' file itself).
    fn superseded(&self, account: &str, filed: &[u8], kept: &[u8], same: impl Fn(&str, Option<&MessageKey>) -> bool) -> bool {
        let Some(at) = self.handed_at(filed) else { return false };
        !self.handed_at(kept).is_some_and(|k| k > at) && !self.kept_newest_after(account, at, same)
    }

    fn kept_newest(&mut self, account: &str, message_id: String, replaces: Option<MessageKey>) {
        let at = self.tick();
        if self.newest.len() >= MAX_ORDER {
            self.newest.pop_front();
        }
        self.newest.push_back(Newest { account: account.to_string(), message_id, replaces, at });
    }
}

/// The kept drafts' file is read and written by more than one thread (an account's old worker
/// keeping what it had still to save while its new one takes them to the server): one at a time.
static PENDING_FILE: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn pending_file() -> std::sync::MutexGuard<'static, ()> {
    PENDING_FILE.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Keep a draft on this computer for the next connection (`newest`: as MailCraft closes, the
/// draft as it was written last). A list that can't be read stops it: saved as just this one, the
/// drafts waiting in it would be lost.
pub(crate) fn keep_draft(
    store: &Store,
    account: &str,
    folder: &str,
    replaces: Option<MessageKey>,
    raw: Vec<u8>,
    newest: bool,
    order: &Order,
) -> Result<()> {
    let _one_at_a_time = pending_file();
    // Another version of the same draft (its Message-ID, or the copy it replaces) is replaced
    // by this one, as is the same one kept twice (MailCraft closing while its save was on the
    // way: kept by both).
    let id = written_message_id(&raw);
    let same =
        |message_id: &str, copy: Option<&MessageKey>| (replaces.is_some() && copy == replaces.as_ref()) || (!id.is_empty() && message_id == id);
    // Unless this is a save handed out before the draft was kept as the newest version of itself:
    // older, and that one stays.
    if !newest && order.lock().is_ok_and(|o| o.older(account, &raw, same)) {
        log::info!("{account}: a save older than the draft kept as MailCraft closed is left");
        return Ok(());
    }
    let mut list: Vec<PendingDraft> = store.load_account_doc(account, PENDING_DRAFTS)?;
    list.retain(|d| !same(&written_message_id(&d.raw), d.replaces.as_ref()));
    list.push(PendingDraft { folder: folder.to_string(), replaces: replaces.clone(), raw });
    store.save_account_doc(account, PENDING_DRAFTS, &list).map_err(NetError::from)?;
    if newest && let Ok(mut o) = order.lock() {
        o.kept_newest(account, id, replaces);
    }
    Ok(())
}

/// The drafts MailCraft filed on an account's IMAP server, by Message-ID and size: a copy there
/// found by its Message-ID is MailCraft's own, as it filed it, only if it's one of these
/// ([`Worker::copies_of`]).
const FILED_DRAFTS: &str = "drafts-filed.json";
/// Drafts filed remembered, at most (the oldest forgotten).
const MAX_FILED: usize = 256;

#[derive(serde::Serialize, serde::Deserialize, PartialEq)]
struct FiledDraft {
    message_id: String,
    size: u32,
}

/// What's said of drafts kept on this computer as an account's connection stopped: where they are.
const KEPT_HERE: &str = "the draft is kept on this computer; it goes to Drafts when the account connects";
/// What's said of a draft kept on this computer as the account's connection before this one is
/// still saving a version of it.
const KEPT_MEANWHILE: &str = "the draft is kept on this computer while the account's previous connection finishes saving it; it goes to Drafts then";

/// A draft handed to an account's worker to save, until its save is said done: by the worker, or
/// by the pool, which keeps it on this computer if it stops the worker first (#53).
pub(crate) struct SaveOnItsWay {
    message_id: String,
    /// Its source's hash, which the worker knows it by.
    hash: u64,
    raw: Vec<u8>,
    folder: String,
    replaces: Option<MessageKey>,
}

/// The pool's and a worker's: what the pool did as it stopped the worker, and the saves handed to
/// the worker, each said done once, by whichever of the two takes it first.
#[derive(Default)]
pub(crate) struct Stopped {
    /// That it did: the account's status, and its folders read again, are its next worker's to
    /// say (or Work Offline's), never this one's (found in review: a save filed after Work Offline
    /// was turned on read Drafts again, and the status bar said the account was online).
    pub(crate) done: AtomicBool,
    saves: std::sync::Mutex<Saves>,
}

/// The saves handed to a worker ([`Stopped`]).
#[derive(Default)]
struct Saves {
    /// Not said done yet, at most [`MAX_ON_ITS_WAY`] (one forgotten past it is the worker's alone
    /// to say).
    on_their_way: VecDeque<SaveOnItsWay>,
    /// Those the pool said done as it stopped the worker, and kept on this computer (or said why
    /// not), by their sources' hashes: the worker leaves them.
    said_by_pool: std::collections::HashSet<u64>,
}

/// Saves on their way to a worker, at most (the oldest forgotten past it).
const MAX_ON_ITS_WAY: usize = 64;

impl Stopped {
    fn saves(&self) -> std::sync::MutexGuard<'_, Saves> {
        self.saves.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl Pool {
    /// A draft handed to a worker to save.
    pub(crate) fn save_on_its_way(&self, job: &Job) {
        let Job::SaveDraft { account, raw, folder, replaces } = job else { return };
        let Some(h) = self.workers.get(account) else { return };
        let message_id = written_message_id(raw);
        DraftOrder::handed(&self.order, raw);
        let mut saves = h.stop.saves();
        if saves.on_their_way.len() >= MAX_ON_ITS_WAY {
            saves.on_their_way.pop_front();
        }
        let hash = hash_of(raw);
        saves.on_their_way.push_back(SaveOnItsWay { message_id, hash, raw: raw.clone(), folder: folder.clone(), replaces: replaces.clone() });
    }

    /// An account's worker stopping (working offline, its settings or password changed,
    /// MailCraft closing): the drafts it was handed and hasn't said done with are kept on this
    /// computer now, before a new worker reads what's kept, and said done; the old worker, which
    /// may only notice it's stopped after a whole sync, leaves them be, and says nothing more of
    /// the account ([`Stopped`]). Taken, kept and noted as the pool's while the worker waits to say
    /// any done itself; one the worker has said done is the pool's no more (found in review: taken
    /// off the list only as the window heard it said done, a save filed and said done was kept
    /// again, the worker stopped before the window heard, said done twice, and filed again at the
    /// next connection).
    pub(crate) fn keep_saves_on_their_way(&self, account: &str, stopped: &Stopped) {
        stopped.done.store(true, Ordering::Relaxed);
        let mut said = Vec::new();
        {
            let mut saves = stopped.saves();
            for s in std::mem::take(&mut saves.on_their_way) {
                saves.said_by_pool.insert(s.hash);
                let error = keep_draft(&self.store, account, &s.folder, s.replaces, s.raw, false, &self.order).err().map(|e| e.to_string());
                said.push((s.message_id, error));
            }
        }
        let kept_here = said.iter().any(|(_, error)| error.is_none());
        for (message_id, error) in said {
            self.sink.emit(Event::DraftSaved { account: account.to_string(), message_id, error });
        }
        // Said where it went, after the saves' ends ("Saved in Drafts" alone would be wrong).
        if kept_here {
            self.sink.emit(Event::Notice { account: account.to_string(), message: KEPT_HERE.into() });
        }
    }

    /// An account removed: what was on its way goes with it.
    pub(crate) fn forget_saves(&self, account: &str) {
        if let Some(h) = self.workers.get(account) {
            h.stop.saves().on_their_way.clear();
        }
    }
}

/// How a save handed to a worker went, short of failing.
enum Saving {
    /// Filed: where, and whether that folder was made for it; being filed until it's kept on
    /// this computer no more.
    Filed((String, bool), Filed),
    /// Left to the pool, which kept it as it stopped this worker (found once connected: the
    /// connecting can take long).
    LeftToPool,
    /// Another worker is filing a version of it: the account's connection before this one, or
    /// after it (which read it as kept).
    Elsewhere,
}

impl Worker {
    /// Save a draft handed to this worker on the server ([`Job::SaveDraft`]), and say how that
    /// went (#53): saved; or kept on this computer, to be saved when the account connects; or,
    /// kept nowhere, why, for the session to give the message back.
    pub(super) fn save_draft(&mut self, raw: Vec<u8>, folder: String, replaces: Option<MessageKey>) {
        let (message_id, hash) = (written_message_id(&raw), hash_of(&raw));
        // Not connected, and not to connect again yet (a connection failed a moment ago, or the
        // sign-in was refused): kept on this computer for the next connection, as offline. Its
        // own connection would ignore the wait (found in review: with a kept draft that dropped
        // each connection, every save connected again at once, 605 sign-ins in 6 s).
        if self.conn.is_none() && (self.auth_failed || Instant::now() < self.next_attempt) {
            if !self.says_done(hash) {
                log::info!("{}: a draft the pool kept as it stopped this connection is left to it", self.acc.id);
                return;
            }
            let Some(kept) = self.keep_here(&folder, replaces, raw) else {
                log::info!("{}: a draft of an account removed isn't kept", self.acc.id);
                return;
            };
            let error = kept.err().map(|k| format!("the account isn't connected, and this computer couldn't keep it: {k}"));
            let kept = error.is_none();
            self.sink.emit(Event::DraftSaved { account: self.id(), message_id, error });
            if kept {
                self.kept.waiting.insert(hash);
                self.notice(KEPT_HERE);
            }
            return;
        }
        let connected = self.connect();
        // Connected for this save: the drafts kept on this computer, older, go first. Once one
        // has failed in this run (it may drop each connection), only the versions of this draft do.
        if connected.is_ok() {
            self.queue_kept_drafts();
            if !self.kept.goes_ahead(&message_id, replaces.as_ref()) {
                self.queue.push_front(Job::SaveDraft { account: self.id(), raw, folder, replaces });
                return;
            }
        }
        // Whether a failure is the filing's own (not the connection's).
        let filing = connected.is_ok();
        // Filed once no other worker is filing a version of it, the pool's keeping looked at
        // after that.
        let r = connected.and_then(|()| {
            let Some(being_filed) = Filing::begin(&self.filing, &self.acc.id, hash, &message_id) else { return Ok(Saving::Elsewhere) };
            if self.pool_said(hash) {
                return Ok(Saving::LeftToPool);
            }
            self.file_draft(&folder, replaces.clone(), &raw, &message_id).map(|filed| Saving::Filed(filed, being_filed))
        });
        // This worker's to say done from here, unless the pool said it first, as it stopped this
        // worker; taken before the draft is kept, or kept no more, so the pool never keeps it after.
        let ours = self.says_done(hash);
        match r {
            Ok(Saving::LeftToPool) => log::info!("{}: a draft the pool kept as it stopped this connection is left to it", self.acc.id),
            Ok(Saving::Elsewhere) if !ours => {
                log::info!("{}: a draft the pool kept as it stopped this connection is left to it", self.acc.id);
            }
            Ok(Saving::Elsewhere) => self.keep_meanwhile(&folder, replaces, raw, message_id, hash),
            // Failed once the pool had kept it: kept already, and said so.
            Err(e) if !ours => log::info!("{}: a draft the pool kept wasn't saved here: {e}", self.acc.id),
            // Its account removed meanwhile: nothing of it written again, nor said.
            Ok(Saving::Filed(..)) if self.removed() => log::info!("{}: a draft of an account removed was saved", self.acc.id),
            // Filed: said done, unless the pool kept it and said so as it stopped this worker; kept
            // on this computer no more either way (the next worker set it aside, and leaves it).
            Ok(Saving::Filed(filed, being_filed)) => {
                if ours {
                    self.sink.emit(Event::DraftSaved { account: self.id(), message_id: message_id.clone(), error: None });
                }
                self.draft_filed(&raw, replaces.as_ref(), &message_id, filed, being_filed);
            }
            // Its account removed meanwhile: nothing to keep it for.
            Err(e) if self.removed() => log::info!("{}: a draft of an account removed isn't kept: {e}", self.acc.id),
            Err(e) => {
                // Not lost: kept on this computer and saved when the account connects.
                // Neither kept: the session gives the message back to be written, saying
                // both reasons (#53). One notice either way: the connection's own trouble
                // still goes to its status.
                let error = self
                    .keep_here(&folder, replaces, raw)
                    .and_then(|r| r.err())
                    .map(|k| format!("{e}; and this computer couldn't keep it either: {k}"));
                let kept = error.is_none();
                self.sink.emit(Event::DraftSaved { account: self.id(), message_id, error });
                // Said after the save's end, so it's the last word on it.
                if kept {
                    self.notice(format!(
                        "the draft couldn't be saved to the server ({e}); it's kept on this computer and saved there when the account connects"
                    ));
                }
                // Its own filing failed (it may be what drops the connection): kept now, it's
                // tried again after a wait of its own, as a kept draft that failed is, and said
                // no more; a connection it dropped is made again at once.
                if filing && kept {
                    self.kept.note_failed(hash);
                    self.filing_failed(e);
                    return;
                }
                if kept {
                    self.kept.waiting.insert(hash);
                }
                if e.is_fatal() || matches!(e, NetError::Auth(_)) {
                    self.failed(e);
                } else {
                    log::warn!("{}: {e}", self.acc.id);
                }
            }
        }
    }

    /// A save of a draft another worker is filing a version of (the account's connection started
    /// again as it did): kept on this computer and set aside, filed once that filing ends, and
    /// said kept, so the message held for it goes; or, kept nowhere, why.
    fn keep_meanwhile(&mut self, folder: &str, replaces: Option<MessageKey>, raw: Vec<u8>, message_id: String, hash: u64) {
        let Some(kept) = self.keep_here(folder, replaces, raw) else {
            log::info!("{}: a draft of an account removed isn't kept", self.acc.id);
            return;
        };
        let error = kept.err().map(|k| format!("the account's previous connection is saving this draft, and this computer couldn't keep it: {k}"));
        let kept = error.is_none();
        self.sink.emit(Event::DraftSaved { account: self.id(), message_id: message_id.clone(), error });
        if kept {
            self.kept.aside.push(&self.acc.id, hash, &message_id);
            self.notice(KEPT_MEANWHILE);
        }
    }

    /// File a draft in `folder` on the server, replacing the copy it replaces, and MailCraft's own
    /// copies of it there already: the folder it went in, and whether that was made for it.
    fn file_draft(&mut self, folder: &str, replaces: Option<MessageKey>, raw: &[u8], message_id: &str) -> Result<(String, bool)> {
        if let Some(Conn::Pop) = self.conn.as_ref() {
            let flags = mailcraft_model::Flags { draft: true, seen: true, ..Default::default() };
            let (store, id) = (&self.store, &self.acc.id);
            self.store
                .while_kept(id, self.removals, || -> Result<()> {
                    crate::pop::file_locally(store, id, folder, raw, flags, now())?;
                    if let Some(old) = replaces {
                        store.apply_op(id, &Op::Expunge { keys: vec![old] })?;
                    }
                    Ok(())
                })
                .ok_or_else(|| NetError::Closed("the account was removed".into()))??;
            return Ok((folder.to_string(), false));
        }
        if let Some(Conn::Jmap(c)) = self.conn.as_mut() {
            jmap::append(c, &self.jstate, folder, raw, &["$draft", "$seen"])?;
            if let Some(old) = replaces {
                jmap::apply_remote(c, &self.jstate, &Op::Expunge { keys: vec![old] })?;
            }
            return Ok((folder.to_string(), false));
        }
        if let Some(Conn::Graph(c)) = self.conn.as_mut() {
            crate::graph::append(c, &self.jstate, folder, raw)?;
            if let Some(old) = replaces {
                crate::graph::apply_remote(c, &self.jstate, &Op::Expunge { keys: vec![old] })?;
            }
            return Ok((folder.to_string(), false));
        }
        if let Some(Conn::Ews(c)) = self.conn.as_mut() {
            crate::ews::append(c, &self.jstate, folder, raw)?;
            if let Some(old) = replaces {
                crate::ews::apply_remote(c, &self.jstate, &Op::Expunge { keys: vec![old] })?;
            }
            return Ok((folder.to_string(), false));
        }
        // A server without Drafts gets one, as it gets Sent Items: once its folders are known
        // (those listed here before, when this connection hasn't listed them).
        if self.folders.is_empty()
            && let Ok(listed) = self.store.load_folders(&self.acc.id)
        {
            self.folders = listed;
        }
        let mut filed = folder.to_string();
        let missing = !self.folders.is_empty() && !self.folders.iter().any(|f| f.path == folder);
        if missing && FolderRole::from_name(folder, None) == FolderRole::Drafts {
            filed = self.role_folder(FolderRole::Drafts, "Drafts")?.unwrap_or(filed);
        }
        let trash = self.folders.iter().find(|f| f.role == FolderRole::Trash).map(|f| f.path.clone());
        // MailCraft's own copies of the same draft there already (its save got there before
        // MailCraft closed, and it was kept too): replaced by this one, as the copy it replaces.
        let mut old: Vec<MessageKey> = self.copies_of(&filed, message_id);
        // Remembered as MailCraft's own before it goes: the server may keep it without its
        // answer getting here (the connection lost, MailCraft closed meanwhile), and the draft,
        // kept here, is filed again (found in review: twice in Drafts). Remembered for a filing
        // that failed, it matches no copy there.
        self.filed_draft(message_id, raw.len());
        let (filed, made) = self.file_in(&filed, FolderRole::Drafts, "\\Draft \\Seen", raw)?;
        old.extend(replaces);
        let c = self.client()?;
        if !old.is_empty() {
            expunge_keys(c, &old, trash.as_deref())?;
        }
        Ok((filed, made))
    }

    /// A draft filed on the server (`filed`: where, and whether that folder was made for it): kept
    /// on this computer no more, nor an older version of it, and then filed (`being_filed` ended,
    /// for another worker that set it aside); the folder read again.
    fn draft_filed(&mut self, raw: &[u8], replaces: Option<&MessageKey>, message_id: &str, (folder, made): (String, bool), being_filed: Filed) {
        self.kept_no_more(raw, replaces, message_id);
        drop(being_filed);
        // Stopped meanwhile: the folders are the next connection's to read.
        if self.stopped() {
            return;
        }
        if made {
            self.relist();
        }
        self.sync_one(&folder);
    }

    /// A draft filed: kept on this computer no more, nor another version of it kept there that's
    /// older ([`DraftOrder::superseded`]), which would replace it on the server at the next
    /// connection. A newer one stays, to replace it.
    fn kept_no_more(&mut self, raw: &[u8], replaces: Option<&MessageKey>, message_id: &str) {
        let account = self.id();
        let same = |id: &str, copy: Option<&MessageKey>| (replaces.is_some() && copy == replaces) || (!message_id.is_empty() && id == message_id);
        let (store, order, queued) = (&self.store, &self.order, &mut self.kept.queued);
        // The account, then the file, then the order, as a draft is kept ([`Self::keep_here`]).
        let written = store.while_kept(&account, self.removals, || -> Result<()> {
            let _one_at_a_time = pending_file();
            let order = order.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let gone = |d: &PendingDraft| {
                d.raw == raw || (same(&written_message_id(&d.raw), d.replaces.as_ref()) && order.superseded(&account, raw, &d.raw, same))
            };
            queued.retain(|d| !gone(d));
            let mut list: Vec<PendingDraft> = store.load_account_doc(&account, PENDING_DRAFTS)?;
            let before = list.len();
            list.retain(|d| !gone(d));
            if list.len() != before {
                store.save_account_doc(&account, PENDING_DRAFTS, &list)?;
            }
            Ok(())
        });
        if let Some(Err(e)) = written {
            log::warn!("{account}: a draft filed is still kept on this computer: {e}");
        }
    }

    /// Stopped with drafts still to be saved: each is kept on this computer, saved to the server
    /// when the account connects, and said done, so the message held for it goes; one that can't
    /// be kept is said so, and the message comes back (#53). Those the pool kept as it stopped
    /// this worker are left to it, and those read from the kept drafts' file stay there.
    pub(crate) fn keep_unsaved_drafts(&mut self, rx: &Receiver<Job>) {
        self.drain(rx);
        let mut kept_here = false;
        for job in std::mem::take(&mut self.queue) {
            let Job::SaveDraft { raw, folder, replaces, .. } = job else { continue };
            if !self.says_done(hash_of(&raw)) {
                continue;
            }
            let message_id = written_message_id(&raw);
            let Some(kept) = self.keep_here(&folder, replaces, raw) else {
                log::info!("{}: a draft of an account removed isn't kept", self.acc.id);
                continue;
            };
            let error = kept.err().map(|e| e.to_string());
            kept_here |= error.is_none();
            self.sink.emit(Event::DraftSaved { account: self.id(), message_id, error });
        }
        if kept_here {
            self.notice(KEPT_HERE);
        }
    }

    /// Keep a draft this worker couldn't save on this computer, unless its account was removed
    /// since the worker started (`None`): an account added again later by the same name would
    /// have it (found in review).
    fn keep_here(&self, folder: &str, replaces: Option<MessageKey>, raw: Vec<u8>) -> Option<Result<()>> {
        self.store.while_kept(&self.acc.id, self.removals, || keep_draft(&self.store, &self.acc.id, folder, replaces, raw, false, &self.order))
    }

    /// Whether this worker's account was removed since it started.
    pub(crate) fn removed(&self) -> bool {
        self.store.while_kept(&self.acc.id, self.removals, || ()).is_none()
    }

    /// Whether the pool said a save handed to this worker (its source's `hash`) done as it stopped
    /// the worker, and kept it on this computer (or said why not).
    fn pool_said(&self, hash: u64) -> bool {
        self.stop.saves().said_by_pool.contains(&hash)
    }

    /// Whether this worker says a save handed to it (its source's `hash`) done: taken off the
    /// saves on their way to it, so the pool never keeps it again as it stops the worker, nor says
    /// it done too. Not when the pool did so first.
    fn says_done(&self, hash: u64) -> bool {
        let mut saves = self.stop.saves();
        if let Some(i) = saves.on_their_way.iter().position(|s| s.hash == hash) {
            saves.on_their_way.remove(i);
            return true;
        }
        !saves.said_by_pool.contains(&hash)
    }

    /// Whether the pool has stopped this worker ([`Stopped`]).
    pub(crate) fn stopped(&self) -> bool {
        self.stop.done.load(Ordering::Relaxed)
    }

    /// A draft filed on the server, remembered as MailCraft's own ([`Self::copies_of`]); not for
    /// an account removed since the worker started.
    fn filed_draft(&self, message_id: &str, size: usize) {
        let Ok(size) = u32::try_from(size) else { return };
        if message_id.is_empty() {
            return;
        }
        let kept = self.store.while_kept(&self.acc.id, self.removals, || {
            let _one_at_a_time = pending_file();
            let mut list: Vec<FiledDraft> = self.store.load_account_doc(&self.acc.id, FILED_DRAFTS).unwrap_or_default();
            let filed = FiledDraft { message_id: message_id.to_string(), size };
            if list.contains(&filed) {
                return Ok(());
            }
            if list.len() >= MAX_FILED {
                list.remove(0);
            }
            list.push(filed);
            self.store.save_account_doc(&self.acc.id, FILED_DRAFTS, &list)
        });
        if let Some(Err(e)) = kept {
            log::info!("{}: a draft filed wasn't remembered: {e}", self.acc.id);
        }
    }

    /// MailCraft's own copies of a draft (its Message-ID) in `folder` on an IMAP server, as it
    /// filed them: found by the header, then each one's Message-ID and size read to be sure (a
    /// server's search may match the words of it, and another program may have saved the draft
    /// again with the same Message-ID, which stays). None when the server can't remove exactly
    /// those (without UIDPLUS a plain `EXPUNGE` removes whatever else is marked deleted there),
    /// or can't be asked (the folder gone, another kind of server).
    fn copies_of(&mut self, folder: &str, message_id: &str) -> Vec<MessageKey> {
        if message_id.is_empty() || !matches!(self.conn, Some(Conn::Imap(_))) {
            return Vec::new();
        }
        let sizes: Vec<u32> = self
            .store
            .load_account_doc::<Vec<FiledDraft>>(&self.acc.id, FILED_DRAFTS)
            .unwrap_or_default()
            .into_iter()
            .filter(|d| d.message_id == message_id)
            .map(|d| d.size)
            .collect();
        if sizes.is_empty() {
            return Vec::new();
        }
        let account = self.id();
        let found = (|| -> Result<Vec<MessageKey>> {
            let c = self.client()?;
            if !c.has("UIDPLUS") {
                return Ok(Vec::new());
            }
            let info = c.select(folder, false)?;
            let id = format!("<{message_id}>");
            let args =
                [crate::sync::SearchArg::Atom("HEADER".into()), crate::sync::SearchArg::Atom("Message-ID".into()), crate::sync::SearchArg::Str(id)];
            let uids: Vec<u32> = c.uid_search_args(&args)?.into_iter().take(16).collect();
            let fetched = c.uid_fetch(&crate::imap::uid_set(&uids), "RFC822.SIZE BODY.PEEK[HEADER]", "")?;
            Ok(fetched
                .into_iter()
                .filter(|f| {
                    uids.contains(&f.uid) && sizes.contains(&f.size) && f.header.as_deref().is_some_and(|h| written_message_id(h) == message_id)
                })
                .map(|f| MessageKey { account: account.clone(), folder: folder.to_string(), validity: info.validity, uid: f.uid })
                .collect())
        })();
        found.unwrap_or_else(|e| {
            log::info!("{account}: copies of a draft in {folder} weren't looked for: {e}");
            Vec::new()
        })
    }
}
