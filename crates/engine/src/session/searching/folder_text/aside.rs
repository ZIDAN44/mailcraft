//! Search: summaries' texts made aside, on threads of their own, as soon as a folder's list is in
//! memory (native only: the browser has no threads, and makes them a slice a tick).
//!
//! A search typed in the ticks after the window opened made the texts it needed as it went, on
//! the window's thread, a few units a frame: on a real file's 34,691 summaries (117 ms of making
//! them) a word took 126–228 ms the first time and a phrase 190–259 ms. Made aside, a batch of a
//! folder's summaries on each of a few threads, they're made in a fraction of that, and the
//! window only takes them in, a batch at a time. What a thread made is taken only while each of
//! its summaries is still the one at its place in the folder's list (the same summary, not only
//! the same uid: one changed in place since is another), so a list that changed meanwhile has
//! nothing stale taken into it. What's handed out at a time is held to the room texts are kept
//! in ([`Aside::hand_out`]): handed out all at once, a big mailbox's waited to be taken in at
//! well past it.

use std::sync::{Arc, mpsc};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::{Mutex, PoisonError};

use super::*;

/// Summaries a thread makes the texts of at a time: about 2 ms of a real file's.
pub(crate) const BATCH: usize = 512;
/// Most threads making texts: on a real file's 34,691, two made them as fast as the search used
/// them, and eight held the window's first frame to 22.6–27.6 ms (19.1–21.0 ms with four,
/// 16.5–21.3 ms in the build before them).
#[cfg(not(target_arch = "wasm32"))]
const THREADS: usize = 4;
/// Batches a tick takes in at most: about 0.7 ms (all 50,000 of the made-up mailbox's at once
/// held a search's step for 5.4 ms).
pub(crate) const TICK_TAKES: usize = 16;
/// Batches a unit of a search takes in at most, its own folder's first.
pub(crate) const UNIT_TAKES: usize = 2;

/// A folder's summaries to make the texts of, from `start` in its list (none is sent in the
/// browser).
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
struct Batch {
    key: (String, String),
    start: usize,
    summaries: Vec<SharedSummary>,
    /// About the bytes of their texts ([`about`]).
    bytes: usize,
}

/// Texts made aside: the summaries they were made of, from `start` in their folder's list.
pub(crate) struct Made {
    pub(crate) key: (String, String),
    pub(crate) start: usize,
    pub(crate) summaries: Vec<SharedSummary>,
    /// One a summary; fewer when making them failed unexpectedly.
    pub(crate) texts: Vec<SummaryText>,
    /// About their bytes, as handed out ([`about`]).
    pub(crate) bytes: usize,
}

/// The threads making texts, and what's handed to them.
pub(crate) struct Aside {
    to: mpsc::Sender<Batch>,
    from: mpsc::Receiver<Made>,
    /// Batches handed out and not taken back yet, by folder.
    out: HashMap<(String, String), usize>,
    /// Batches handed out so far.
    pub(crate) handed: u64,
    /// Made, not taken in yet: waiting for the batches before them in their folder, or for
    /// the next tick's or unit's turn.
    held: Vec<Made>,
    /// About the bytes of the texts handed out and not taken in (or let go of) yet.
    pending: usize,
    /// Folders a batch of which failed to be made (unexpectedly): not handed out again, their
    /// texts made by the searches that need them.
    gave_up: HashSet<(String, String)>,
}

impl std::fmt::Debug for Aside {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Aside").field("out", &self.out).field("held", &self.held.len()).field("pending", &self.pending).finish()
    }
}

impl Aside {
    /// The threads started (`None`: none could be, or there are none here).
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn start() -> Option<Aside> {
        let (to, batches) = mpsc::channel::<Batch>();
        let (made, from) = mpsc::channel::<Made>();
        let batches = Arc::new(Mutex::new(batches));
        let threads = std::thread::available_parallelism().map_or(2, |n| n.get()).saturating_sub(2).clamp(1, THREADS);
        let mut started = 0;
        for i in 0..threads {
            let (batches, made) = (batches.clone(), made.clone());
            let spawned = std::thread::Builder::new().name(format!("search-texts-{i}")).spawn(move || {
                loop {
                    // Held only while one is taken, not while it's made.
                    let next = batches.lock().unwrap_or_else(PoisonError::into_inner).recv();
                    let Ok(b) = next else { break };
                    let texts = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        #[cfg(test)]
                        fault(&b.summaries);
                        b.summaries.iter().map(|m| SummaryText::of(m)).collect::<Vec<_>>()
                    }));
                    // (One that failed unexpectedly comes back with none: its folder isn't handed
                    // out again, and the searches make them.)
                    let texts = texts.unwrap_or_default();
                    if made.send(Made { key: b.key, start: b.start, summaries: b.summaries, texts, bytes: b.bytes }).is_err() {
                        break;
                    }
                }
            });
            match spawned {
                Ok(_) => started += 1,
                Err(e) => log::warn!("search: a thread to make summaries' texts on didn't start: {e}"),
            }
        }
        (started > 0).then(|| Aside { to, from, out: HashMap::new(), handed: 0, held: Vec::new(), pending: 0, gave_up: HashSet::new() })
    }

    /// No threads in the browser: the ticks make the texts.
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn start() -> Option<Aside> {
        None
    }

    /// Hand out the texts of `messages[from..]` (the folder `key`'s list), a batch a thread,
    /// while those handed out and not taken in yet come to at most `room` bytes, about (one batch
    /// at least, when none is: the rest is handed out as they're taken in). `false`: the threads
    /// are gone.
    pub(crate) fn hand_out(&mut self, key: &(String, String), messages: &[SharedSummary], from: usize, room: usize) -> bool {
        let mut start = from;
        while let Some(chunk) = messages.get(start..).filter(|c| !c.is_empty()) {
            let chunk = chunk.get(..BATCH.min(chunk.len())).unwrap_or_default();
            let bytes = chunk.iter().map(|m| about(m)).fold(0usize, usize::saturating_add);
            if self.pending > 0 && self.pending.saturating_add(bytes) > room {
                break;
            }
            if self.to.send(Batch { key: key.clone(), start, summaries: chunk.to_vec(), bytes }).is_err() {
                return false;
            }
            *self.out.entry(key.clone()).or_default() += 1;
            self.handed = self.handed.wrapping_add(1);
            self.pending = self.pending.saturating_add(bytes);
            start = start.saturating_add(chunk.len());
        }
        true
    }

    /// Whether `key`'s texts may be handed out now: none being made or waiting, and none of its
    /// batches failed.
    pub(crate) fn may_hand_out(&self, key: &(String, String)) -> bool {
        !self.busy_with(key) && !self.gave_up.contains(key)
    }

    /// Whether more may be handed out within `room` bytes, about.
    pub(crate) fn has_room(&self, room: usize) -> bool {
        self.pending < room
    }

    /// Whether a batch of `key`'s failed to be made: the searches make its texts.
    pub(crate) fn gave_up_on(&self, key: &(String, String)) -> bool {
        self.gave_up.contains(key)
    }

    /// A batch of `key`'s failed to be made: it isn't handed out again (it failed again and
    /// again, a thread kept at it and the window asking for frames for good).
    pub(crate) fn give_up(&mut self, key: &(String, String)) {
        if self.gave_up.insert(key.clone()) {
            log::warn!("search: making a folder's summaries' texts aside failed; its searches make them");
        }
    }

    /// A test's: about the bytes of the texts handed out and not taken in (or let go of) yet.
    #[cfg(test)]
    pub(crate) fn pending(&self) -> usize {
        self.pending
    }

    /// Whether `key`'s texts are being made, or wait to be taken in (handed out again meanwhile,
    /// a folder whose batches were back but not taken in yet was made again and again: 559
    /// batches for a real file's 68).
    pub(crate) fn busy_with(&self, key: &(String, String)) -> bool {
        self.out.get(key).is_some_and(|n| *n > 0) || self.held.iter().any(|m| m.key == *key)
    }

    /// Whether any are being made, or waiting to be taken in.
    pub(crate) fn busy(&self) -> bool {
        !self.out.is_empty() || !self.held.is_empty()
    }

    /// What the threads made since, with what was held: the caller takes in what it can and
    /// gives back what waits for batches before it ([`Aside::hold`]).
    pub(crate) fn take(&mut self) -> Vec<Made> {
        let mut all = std::mem::take(&mut self.held);
        let mut gone = all.iter().map(|m| m.bytes).fold(0usize, usize::saturating_add);
        while let Ok(m) = self.from.try_recv() {
            gone = gone.saturating_add(m.bytes);
            if let Some(n) = self.out.get_mut(&m.key) {
                *n = n.saturating_sub(1);
                if *n == 0 {
                    self.out.remove(&m.key);
                }
            }
            all.push(m);
        }
        // (Counted again for those given back.)
        self.pending = self.pending.saturating_sub(gone);
        all
    }

    /// `m` waits for the batches before it, while any of its folder's are still out (once none
    /// is, those before it were let go of, and it goes too).
    pub(crate) fn hold(&mut self, m: Made) {
        if self.out.get(&m.key).is_some_and(|n| *n > 0) {
            self.defer(m);
        }
    }

    /// `m` waits for the next turn to take texts in.
    pub(crate) fn defer(&mut self, m: Made) {
        self.pending = self.pending.saturating_add(m.bytes);
        self.held.push(m);
    }
}

/// About the bytes of `m`'s text ([`SummaryText::bytes`]), before it's made: its subject and
/// addresses twice (folded and as written), its preview once.
fn about(m: &Summary) -> usize {
    let addresses = std::iter::once(&m.from).chain(&m.to).chain(&m.cc);
    let raw = addresses.map(|a| a.name.len().saturating_add(a.email.len()).saturating_add(2)).fold(m.subject.len(), usize::saturating_add);
    raw.saturating_mul(2).saturating_add(m.preview.len()).saturating_add(std::mem::size_of::<SummaryText>())
}

/// A test's: making the texts of a batch with a summary of this subject fails, as a fault would.
#[cfg(test)]
pub(crate) const FAULT: &str = "\u{1}a fault made for a test";

#[cfg(test)]
#[allow(clippy::panic)]
fn fault(summaries: &[SharedSummary]) {
    if summaries.iter().any(|m| m.subject == FAULT) {
        panic!("a fault made for a test");
    }
}

impl FolderTexts {
    /// Take in texts made aside (of the folder `k`, its list `messages` as it is now): those past
    /// what's kept are added while each is still the summary at its place, within the room texts
    /// made ahead leave. Given back when they start past what's kept, to wait.
    pub(crate) fn take_in(&mut self, k: &(String, String), messages: &[SharedSummary], made: Made) -> Option<Made> {
        self.still_kept(k, messages);
        let have = self.made_of(k);
        if made.start > have {
            return Some(made);
        }
        let room = TEXT_BYTES.saturating_sub(AHEAD_ROOM);
        let mut added = 0usize;
        let ft = self.folders.entry(k.clone()).or_default();
        if ft.texts.is_empty() {
            // Room for the folder's, grown once (not a copy of all those taken in each time).
            ft.texts.reserve(messages.len());
            ft.uids.reserve(messages.len());
        }
        let skip = have.saturating_sub(made.start);
        for (at, (m, st)) in made.summaries.iter().zip(made.texts).enumerate().skip(skip) {
            let place = made.start.saturating_add(at);
            if !messages.get(place).is_some_and(|now| Arc::ptr_eq(now, m)) || self.bytes.saturating_add(added) >= room {
                break;
            }
            added = added.saturating_add(st.bytes());
            ft.uids.push(m.key.uid);
            ft.texts.push(st);
            self.aside = self.aside.wrapping_add(1);
        }
        ft.bytes = ft.bytes.saturating_add(added);
        self.bytes = self.bytes.saturating_add(added);
        None
    }
}
