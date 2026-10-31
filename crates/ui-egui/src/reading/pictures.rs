//! The pictures of the messages shown: attachment thumbnails, pictures inside HTML mail
//! (`cid:`), a picture at the pane's size and a PDF's pages. A message's files are read and
//! taken out once, by one parse, and kept while it's open; each picture is then decoded a step
//! at a time. Before, a message with 36 pictures was read and parsed 36 times in one frame (2.6
//! seconds), and one with more than 64 did it again every frame.
//!
//! The steps run a frame at a time here (no threads: the browser, the tests); results come
//! back as pixels, and the window makes textures of so many bytes a frame. What's kept is
//! bounded: textures by [`BUDGET`] (the least recently shown go first; those shown now never),
//! results waiting by [`DONE_MAX`], files taken out by [`SOURCES_MAX`]. Pictures are hostile
//! input: a step that panics fails the rest of its message's pictures and nothing else.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use mailcraft_engine::mime::{self, AttachmentInfo, AttachmentKind};
use mailcraft_engine::model::MessageKey;
use mailcraft_engine::store::Store;

use crate::thumbs::{self, Fit, Fonts};

/// Most bytes of picture textures kept (RGBA).
pub(crate) const BUDGET: usize = 96 << 20;
/// Most bytes of textures made in one frame (at least one picture is, whatever its size).
const UPLOAD_PER_FRAME: usize = 8 << 20;
/// Most bytes of decoded pictures waiting for the window: the work waits past this.
const DONE_MAX: usize = 32 << 20;
/// Most bytes of a message's files kept taken out (also the most one read takes out).
const SOURCES_MAX: usize = 128 << 20;
/// Largest file taken out to be shown (the largest PDF drawn).
const MAX_EACH: usize = 40 << 20;
/// Most thumbnails asked of one message; its other chips show their kind's icon (a picture
/// opened, a `cid:` picture and a PDF's pages are always asked).
const MAX_PER_MESSAGE: usize = 200;
/// Reads of a message that may leave a picture out (for room) before it's taken as not there.
const READS: u8 = 2;
/// Most pictures remembered (shown, failed or waiting).
const MAX_SLOTS: usize = 2048;
/// Narrowest a PDF page is drawn when there's no room for it wider.
const MIN_PAGE_PX: u32 = 200;

/// What a picture is wanted for, which sets its size.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Want {
    /// An attachment chip's thumbnail.
    Thumb,
    /// A picture inside an HTML message.
    Body,
    /// A picture at the reading pane's or a dialog's size.
    Big,
    /// A PDF's first page, for its chip.
    PdfFirst,
    /// A PDF's page `n` (from 0), `px` pixels wide.
    PdfPage { n: usize, px: u32 },
}

impl Want {
    /// The same picture smaller, when there's no room for this one.
    fn smaller(self) -> Option<Want> {
        match self {
            Want::Big => Some(Want::Body),
            Want::Body => Some(Want::Thumb),
            Want::PdfPage { n, px } if px / 2 >= MIN_PAGE_PX => Some(Want::PdfPage { n, px: px / 2 }),
            _ => None,
        }
    }

    /// Most pixels on a picture's side (a chip's thumbnail is 30 points: 160 is sharp at 4x,
    /// and a ninth of the bytes of the 480 it was).
    fn max_side(self) -> u32 {
        match self {
            Want::Thumb => 160,
            Want::Body => 1200,
            Want::Big => 2048,
            Want::PdfFirst => 260,
            Want::PdfPage { px, .. } => px,
        }
    }

    /// Shown big: decoded before the thumbnails waiting.
    fn first(self) -> bool {
        matches!(self, Want::Big | Want::PdfPage { .. })
    }

    /// A chip's picture: one of so many a message has (the others were opened by the user).
    fn chip(self) -> bool {
        matches!(self, Want::Thumb | Want::PdfFirst)
    }
}

/// A picture as far as it can be had now.
pub(crate) enum Got {
    Ready(egui::TextureHandle),
    /// Being read or decoded: ask again next frame.
    Pending,
    /// Not a picture that can be shown.
    Failed,
}

impl Got {
    pub(crate) fn ready(self) -> Option<egui::TextureHandle> {
        match self {
            Got::Ready(t) => Some(t),
            Got::Pending | Got::Failed => None,
        }
    }
}

type PicId = (MessageKey, usize, Want);

enum State {
    Ready(egui::TextureHandle),
    Pending,
    Failed,
    /// Decoded, but no room among the textures shown: a smaller one is shown meanwhile.
    Over,
}

struct Slot {
    state: State,
    /// Its texture's bytes (RGBA).
    bytes: usize,
    /// The frame it was last asked for in.
    used: u64,
}

/// A decoded picture (or none), on its way to the window.
struct Done {
    key: MessageKey,
    index: usize,
    want: Want,
    image: Option<(egui::ColorImage, egui::TextureOptions)>,
    /// A PDF's page count.
    pages: Option<usize>,
}

impl Done {
    fn bytes(&self) -> usize {
        self.image.as_ref().map_or(0, |(i, _)| i.size[0].saturating_mul(i.size[1]).saturating_mul(4))
    }
}

/// Decoded pictures waiting for the window, and their bytes.
#[derive(Default)]
struct Out {
    queue: VecDeque<Done>,
    bytes: usize,
}

/// Files taken out of their messages, by (message, attachment index).
#[derive(Default)]
struct Sources {
    map: HashMap<(MessageKey, usize), Arc<Vec<u8>>>,
    bytes: usize,
}

impl Sources {
    fn remove_if(&mut self, mut drop: impl FnMut(&MessageKey) -> bool) {
        let bytes = &mut self.bytes;
        self.map.retain(|(k, _), d| {
            let keep = !drop(k);
            if !keep {
                *bytes = bytes.saturating_sub(d.len());
            }
            keep
        });
    }
}

/// How much work was done (what the tests count).
#[derive(Default)]
struct Stats {
    /// Messages read and parsed for their files.
    extractions: AtomicUsize,
    /// Pictures and PDF pages decoded.
    decodes: AtomicUsize,
}

/// What the window and the work share.
#[derive(Default)]
struct Shared {
    out: Mutex<Out>,
    sources: Mutex<Sources>,
    stats: Stats,
}

/// A lock that outlives a panic elsewhere (what it guards is whole between steps).
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// One message's pictures to decode, a step at a time.
struct Batch {
    key: MessageKey,
    store: Store,
    queue: Mutex<VecDeque<(usize, Want)>>,
    /// Taken out with the first ones asked: the message's other pictures and PDFs, so a
    /// picture opened big or a PDF's pages later need no other read.
    also: Vec<usize>,
    /// Reads that looked for an index and didn't take it out: past [`READS`] (or at once when
    /// the message hasn't it) it isn't looked for again.
    tried: Mutex<HashMap<usize, u8>>,
    /// Most bytes of files kept taken out ([`SOURCES_MAX`]; less in a test).
    sources_max: usize,
    cancelled: AtomicBool,
    /// A step panicked: the rest fail.
    broken: AtomicBool,
    /// The graphics card's largest texture side.
    limit: u32,
    fonts: Fonts,
    /// A test's picture that panics as it's decoded.
    #[cfg(test)]
    panic_at: Option<usize>,
}

impl Batch {
    /// One step: take the files out, or decode one picture. False when there's nothing left.
    fn step(&self, shared: &Shared) -> bool {
        if self.cancelled.load(Ordering::Relaxed) {
            return false;
        }
        {
            // The window hasn't taken what's waiting: wait (one waits, whatever its size).
            let out = lock(&shared.out);
            if out.bytes >= DONE_MAX && !out.queue.is_empty() {
                return true;
            }
        }
        let Some((index, want)) = lock(&self.queue).pop_front() else { return false };
        let done = if self.broken.load(Ordering::Relaxed) {
            Some(self.failed(index, want))
        } else {
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.work(shared, index, want))) {
                Ok(done) => done,
                Err(_) => {
                    log::warn!("a picture couldn't be decoded (the decoder failed); the message's others aren't tried");
                    self.broken.store(true, Ordering::Relaxed);
                    Some(self.failed(index, want))
                }
            }
        };
        if let Some(done) = done {
            let mut out = lock(&shared.out);
            out.bytes = out.bytes.saturating_add(done.bytes());
            out.queue.push_back(done);
        }
        true
    }

    fn failed(&self, index: usize, want: Want) -> Done {
        Done { key: self.key.clone(), index, want, image: None, pages: None }
    }

    /// Decode one picture; `None` when its file had to be taken out first (it goes back to
    /// the front of the queue).
    fn work(&self, shared: &Shared, index: usize, want: Want) -> Option<Done> {
        #[cfg(test)]
        if self.panic_at == Some(index) {
            std::panic::resume_unwind(Box::new("a picture that breaks the decoder"));
        }
        let source = lock(&shared.sources).map.get(&(self.key.clone(), index)).cloned();
        let Some(data) = source else {
            if lock(&self.tried).get(&index).is_some_and(|n| *n >= READS) {
                return Some(self.failed(index, want));
            }
            self.take_out(shared, index);
            lock(&self.queue).push_front((index, want));
            return None;
        };
        shared.stats.decodes.fetch_add(1, Ordering::Relaxed);
        let fonts = || self.fonts.clone();
        let (image, pages) = match want {
            Want::PdfFirst => match thumbs::pdf_page(&data, 0, Fit::Longer(want.max_side()), self.limit, fonts()) {
                Some((img, n)) => (Some((img, egui::TextureOptions::LINEAR)), Some(n)),
                None => (None, None),
            },
            Want::PdfPage { n, px } => match thumbs::pdf_page(&data, n, Fit::Width(px), self.limit, fonts()) {
                Some((img, count)) => (Some((img, egui::TextureOptions::LINEAR)), Some(count)),
                None => (None, None),
            },
            Want::Thumb | Want::Body | Want::Big => (super::decode_pixels(&data, want.max_side(), self.limit), None),
        };
        Some(Done { key: self.key.clone(), index, want, image, pages })
    }

    /// Read the message and take out, in one parse, `index`, the others waiting and the
    /// message's other pictures and PDFs (those not taken out already, or given up on). One
    /// left out for room is looked for again by a later read, [`READS`] times in all.
    fn take_out(&self, shared: &Shared, index: usize) {
        let mut wanted = vec![index];
        {
            // `index` counts a read; those taken along with it don't (but aren't looked for
            // again once given up on).
            let mut tried = lock(&self.tried);
            let n = tried.entry(index).or_insert(0);
            *n = n.saturating_add(1);
        }
        wanted.extend(lock(&self.queue).iter().map(|(i, _)| *i));
        wanted.extend(self.also.iter().copied());
        {
            let sources = lock(&shared.sources);
            let tried = lock(&self.tried);
            let mut seen = HashSet::new();
            wanted.retain(|i| {
                seen.insert(*i) && !sources.map.contains_key(&(self.key.clone(), *i)) && (*i == index || tried.get(i).is_none_or(|n| *n < READS))
            });
        }
        let Some(raw) = self.store.read_body(&self.key) else { return };
        shared.stats.extractions.fetch_add(1, Ordering::Relaxed);
        let parts = mime::attachments_data(&raw, &wanted, MAX_EACH, self.sources_max);
        drop(raw);
        let needed: usize = parts.iter().map(|(_, d)| d.len()).fold(0, usize::saturating_add);
        {
            // Nothing was left out for room (any file that fits would have): those not taken out
            // aren't in the message, or are too big, and aren't looked for again.
            let mut tried = lock(&self.tried);
            let cut = needed.saturating_add(MAX_EACH) > self.sources_max;
            for i in &wanted {
                if parts.iter().any(|(j, _)| j == i) {
                    tried.remove(i);
                } else if !cut {
                    tried.insert(*i, READS);
                }
            }
        }
        let mut sources = lock(&shared.sources);
        // Another message's files make room first, then this one's not wanted now (those can be
        // read again; the ones asked can't wait on them).
        if sources.bytes.saturating_add(needed) > self.sources_max {
            sources.remove_if(|k| *k != self.key);
        }
        if sources.bytes.saturating_add(needed) > self.sources_max {
            sources.remove_if(|_| true);
        }
        for (i, data) in parts {
            if sources.bytes.saturating_add(data.len()) > self.sources_max {
                break;
            }
            sources.bytes = sources.bytes.saturating_add(data.len());
            sources.map.insert((self.key.clone(), i), Arc::new(data));
        }
    }
}

/// The pictures shown, kept as textures, and the work making them.
pub(crate) struct Pictures {
    slots: HashMap<PicId, Slot>,
    /// Frames ended so far: what `Slot::used` counts in.
    frame: u64,
    /// Bytes of the textures kept, and of those asked for this frame.
    bytes: usize,
    frame_bytes: usize,
    budget: usize,
    sources_max: usize,
    /// Asked for this frame, not given to a batch yet.
    wanted: Vec<PicId>,
    batches: Vec<Arc<Batch>>,
    shared: Arc<Shared>,
    /// The messages whose pictures were asked for this frame.
    touched: HashSet<MessageKey>,
    /// A PDF's page count, by (message, attachment).
    pages: HashMap<(MessageKey, usize), usize>,
    #[cfg(test)]
    pub(crate) panic_at: Option<usize>,
}

impl Default for Pictures {
    fn default() -> Pictures {
        Pictures {
            slots: HashMap::new(),
            frame: 0,
            bytes: 0,
            frame_bytes: 0,
            budget: BUDGET,
            sources_max: SOURCES_MAX,
            wanted: Vec::new(),
            batches: Vec::new(),
            shared: Arc::default(),
            touched: HashSet::new(),
            pages: HashMap::new(),
            #[cfg(test)]
            panic_at: None,
        }
    }
}

impl Pictures {
    /// Attachment `index` of a message, for `want`; the caller draws the kind's icon (or a
    /// spinner) until it's ready. Asked again each frame it's shown.
    pub(crate) fn get(&mut self, key: &MessageKey, index: usize, want: Want) -> Got {
        if !self.touched.contains(key) {
            self.touched.insert(key.clone());
        }
        let id = (key.clone(), index, want);
        if let Some(slot) = self.slots.get_mut(&id) {
            if slot.used != self.frame {
                slot.used = self.frame;
                self.frame_bytes = self.frame_bytes.saturating_add(slot.bytes);
            }
            return match &slot.state {
                State::Ready(t) => Got::Ready(t.clone()),
                State::Pending => Got::Pending,
                State::Failed => Got::Failed,
                State::Over => self.smaller(key, index, want),
            };
        }
        // The textures shown this frame fill the budget: a smaller one, or none.
        if self.frame_bytes >= self.budget {
            return self.smaller(key, index, want);
        }
        if want.chip() && self.slots.keys().filter(|(k, _, w)| k == key && w.chip()).count() >= MAX_PER_MESSAGE {
            return Got::Failed;
        }
        if let Want::PdfPage { n, px } = want {
            self.drop_other_widths(key, index, n, px);
        }
        self.slots.insert(id.clone(), Slot { state: State::Pending, bytes: 0, used: self.frame });
        self.wanted.push(id);
        Got::Pending
    }

    /// The same picture smaller, when there's no room for this one. With none smaller, a chip
    /// shows its kind's icon; a picture opened waits (it's asked again next frame, when there
    /// may be room) rather than read as one that can't be shown.
    fn smaller(&mut self, key: &MessageKey, index: usize, want: Want) -> Got {
        let got = match want.smaller() {
            Some(w) => self.get(key, index, w),
            None => Got::Failed,
        };
        match got {
            Got::Failed if !want.chip() => Got::Pending,
            got => got,
        }
    }

    /// A PDF's page asked at a new width: the page at other widths, not shown this frame, is
    /// let go and not decoded, so a window being resized doesn't keep (or make) a page for
    /// each width it goes through.
    fn drop_other_widths(&mut self, key: &MessageKey, index: usize, n: usize, px: u32) {
        let frame = self.frame;
        let mut gone: Vec<Want> = Vec::new();
        let mut freed = 0usize;
        self.slots.retain(|(k, i, w), s| {
            let other = matches!(w, Want::PdfPage { n: m, px: q } if *m == n && *q != px);
            if other && s.used < frame && k == key && *i == index {
                gone.push(*w);
                freed = freed.saturating_add(s.bytes);
                return false;
            }
            true
        });
        self.bytes = self.bytes.saturating_sub(freed);
        if !gone.is_empty()
            && let Some(b) = self.batches.iter().find(|b| b.key == *key)
        {
            lock(&b.queue).retain(|(i, w)| !(*i == index && gone.contains(w)));
        }
    }

    /// Frames ended so far (what's shown least recently goes first elsewhere too).
    pub(crate) fn frame(&self) -> u64 {
        self.frame
    }

    /// How many pages a PDF attachment has, once one was drawn.
    pub(crate) fn pages(&self, key: &MessageKey, index: usize) -> Option<usize> {
        self.pages.get(&(key.clone(), index)).copied()
    }

    /// Once a frame, after drawing: start the pictures asked for, take one step of the work,
    /// make textures of what's decoded and let go of what isn't shown. `open` is the message
    /// in the reading pane and its attachments: its work goes on while it stays open (whether
    /// or not its pictures were drawn this frame); another message's stops when it isn't shown.
    pub(crate) fn end_frame(&mut self, ctx: &egui::Context, store: Option<&Store>, open: Option<(&MessageKey, &[AttachmentInfo])>, fonts: &Fonts) {
        let open_key = open.map(|(k, _)| k);
        let shown = |k: &MessageKey, touched: &HashSet<MessageKey>| open_key == Some(k) || touched.contains(k);
        // Work for a message no longer shown stops, and its pictures waiting (or failed) are
        // forgotten: opened again, it tries them again.
        let mut stopped: Vec<MessageKey> = Vec::new();
        self.batches.retain(|b| {
            let keep = shown(&b.key, &self.touched) && !b.cancelled.load(Ordering::Relaxed);
            if !keep {
                b.cancelled.store(true, Ordering::Relaxed);
                stopped.push(b.key.clone());
            }
            keep
        });
        if !stopped.is_empty() {
            self.slots.retain(|(k, _, _), s| !(matches!(s.state, State::Pending | State::Failed) && stopped.contains(k)));
        }
        lock(&self.shared.sources).remove_if(|k| !shown(k, &self.touched));
        if self.pages.len() > MAX_SLOTS {
            self.pages.retain(|(k, _), _| shown(k, &self.touched));
        }
        self.start(ctx, store, open, fonts);
        // One step of the oldest message's work.
        if let Some(b) = self.batches.first().cloned()
            && !b.step(&self.shared)
        {
            self.batches.retain(|x| !Arc::ptr_eq(x, &b));
        }
        self.upload(ctx);
        // Pictures too big for the room left are tried again when next asked.
        let frame = self.frame;
        self.slots.retain(|_, s| !(matches!(s.state, State::Over) && s.used < frame));
        self.evict_to(self.budget);
        if !self.batches.is_empty() || !lock(&self.shared.out).queue.is_empty() {
            ctx.request_repaint();
        }
        self.frame = self.frame.wrapping_add(1);
        self.frame_bytes = 0;
        self.touched.clear();
    }

    /// The pictures asked for this frame go to their message's batch (a new one for a message
    /// with none running).
    fn start(&mut self, ctx: &egui::Context, store: Option<&Store>, open: Option<(&MessageKey, &[AttachmentInfo])>, fonts: &Fonts) {
        if self.wanted.is_empty() {
            return;
        }
        let limit = ctx.input(|i| i.max_texture_side).try_into().unwrap_or(u32::MAX);
        for (key, index, want) in std::mem::take(&mut self.wanted) {
            if let Some(b) = self.batches.iter().find(|b| b.key == key) {
                let mut q = lock(&b.queue);
                if want.first() {
                    q.push_front((index, want))
                } else {
                    q.push_back((index, want))
                }
                continue;
            }
            let Some(store) = store else {
                if let Some(s) = self.slots.get_mut(&(key, index, want)) {
                    s.state = State::Failed;
                }
                continue;
            };
            let also = match open {
                Some((k, files)) if *k == key => files
                    .iter()
                    .filter(|a| matches!(a.kind(), AttachmentKind::Image | AttachmentKind::Pdf))
                    .map(|a| a.index)
                    .take(MAX_PER_MESSAGE)
                    .collect(),
                _ => Vec::new(),
            };
            self.batches.push(Arc::new(Batch {
                key,
                store: store.clone(),
                queue: Mutex::new(VecDeque::from([(index, want)])),
                also,
                tried: Mutex::new(HashMap::new()),
                sources_max: self.sources_max,
                cancelled: AtomicBool::new(false),
                broken: AtomicBool::new(false),
                limit,
                fonts: fonts.clone(),
                #[cfg(test)]
                panic_at: self.panic_at,
            }));
        }
    }

    /// Textures of the pictures decoded: so many bytes a frame, at least one.
    fn upload(&mut self, ctx: &egui::Context) {
        let mut made = 0usize;
        let mut count = 0usize;
        loop {
            let done = {
                let mut out = lock(&self.shared.out);
                let Some(bytes) = out.queue.front().map(Done::bytes) else { break };
                if count > 0 && made.saturating_add(bytes) > UPLOAD_PER_FRAME {
                    break;
                }
                out.bytes = out.bytes.saturating_sub(bytes);
                made = made.saturating_add(bytes);
                count += 1;
                match out.queue.pop_front() {
                    Some(d) => d,
                    None => break,
                }
            };
            self.land(ctx, done);
        }
    }

    fn land(&mut self, ctx: &egui::Context, done: Done) {
        let bytes = done.bytes();
        if let Some(n) = done.pages {
            self.pages.insert((done.key.clone(), done.index), n);
        }
        let name = format!("picture-{}-{}-{:?}", done.key.to_id(), done.index, done.want);
        let id = (done.key, done.index, done.want);
        // Forgotten meanwhile (its message closed): nothing to do.
        if !self.slots.get(&id).is_some_and(|s| matches!(s.state, State::Pending)) {
            return;
        }
        let state = match done.image {
            None => State::Failed,
            Some((image, options)) => {
                // Room is made among the textures not shown this frame.
                self.evict_to(self.budget.saturating_sub(bytes));
                if bytes > self.budget || self.bytes.saturating_add(bytes) > self.budget {
                    State::Over
                } else {
                    self.bytes = self.bytes.saturating_add(bytes);
                    State::Ready(ctx.load_texture(name, image, options))
                }
            }
        };
        if let Some(slot) = self.slots.get_mut(&id) {
            if matches!(state, State::Ready(_)) {
                slot.bytes = bytes;
            }
            slot.state = state;
        }
    }

    /// Let go of the textures least recently shown (never one shown this frame) until at most
    /// `target` bytes are kept, and of the oldest pictures remembered past [`MAX_SLOTS`].
    fn evict_to(&mut self, target: usize) {
        let frame = self.frame;
        while self.bytes > target {
            let oldest = self
                .slots
                .iter()
                .filter(|(_, s)| matches!(s.state, State::Ready(_)) && s.used < frame)
                .min_by_key(|(_, s)| s.used)
                .map(|(id, _)| id.clone());
            let Some(id) = oldest else { break };
            if let Some(s) = self.slots.remove(&id) {
                self.bytes = self.bytes.saturating_sub(s.bytes);
            }
        }
        while self.slots.len() > MAX_SLOTS {
            let oldest = self.slots.iter().filter(|(_, s)| s.used < frame).min_by_key(|(_, s)| s.used).map(|(id, _)| id.clone());
            let Some(id) = oldest else { break };
            if let Some(s) = self.slots.remove(&id) {
                self.bytes = self.bytes.saturating_sub(s.bytes);
            }
        }
    }
}

#[cfg(test)]
impl Pictures {
    /// Messages read for their files, and pictures decoded, so far.
    pub(crate) fn counts(&self) -> (usize, usize) {
        (self.shared.stats.extractions.load(Ordering::Relaxed), self.shared.stats.decodes.load(Ordering::Relaxed))
    }

    /// Bytes of the textures kept.
    pub(crate) fn bytes(&self) -> usize {
        self.bytes
    }

    pub(crate) fn set_budget(&mut self, budget: usize) {
        self.budget = budget;
    }

    /// For the batches started from now on.
    pub(crate) fn set_sources_max(&mut self, max: usize) {
        self.sources_max = max;
    }

    /// Pictures remembered of a message, and those of them ready and shown last frame.
    pub(crate) fn of(&self, key: &MessageKey) -> (usize, usize) {
        let last = self.frame.wrapping_sub(1);
        let mine = || self.slots.iter().filter(|((k, _, _), _)| k == key);
        (mine().count(), mine().filter(|(_, s)| matches!(s.state, State::Ready(_)) && s.used == last).count())
    }

    /// The pictures ready, and the work running.
    pub(crate) fn ready(&self) -> usize {
        self.slots.values().filter(|s| matches!(s.state, State::Ready(_))).count()
    }

    pub(crate) fn working(&self) -> usize {
        self.batches.len()
    }
}
