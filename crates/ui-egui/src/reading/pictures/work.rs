//! The work of making a message's pictures: taking its files out and decoding them a step at
//! a time, on the pictures lane's thread or in the window, and what it shares with the window.

use super::*;

/// Decoded pictures waiting for the window, and their bytes.
#[derive(Default)]
pub(super) struct Out {
    pub(super) queue: VecDeque<Done>,
    pub(super) bytes: usize,
}

/// Files taken out of their messages, by (message, attachment index).
#[derive(Default)]
pub(super) struct Sources {
    pub(super) map: HashMap<(MessageKey, usize), Arc<Vec<u8>>>,
    pub(super) bytes: usize,
}

impl Sources {
    pub(super) fn remove_if(&mut self, mut drop: impl FnMut(&MessageKey) -> bool) {
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
pub(super) struct Stats {
    /// Messages read and parsed for their files.
    pub(super) extractions: AtomicUsize,
    /// Pictures and PDF pages decoded.
    pub(super) decodes: AtomicUsize,
    /// Steps done on the window's thread.
    pub(super) inline: AtomicUsize,
    /// Frames asked for by work on a thread, as it decoded.
    pub(super) wakes: AtomicUsize,
}

/// What the window and the work share.
#[derive(Default)]
pub(super) struct Shared {
    pub(super) out: Mutex<Out>,
    /// Told when the window takes what's waiting in `out`, or work is cancelled.
    pub(super) room: Condvar,
    pub(super) sources: Mutex<Sources>,
    pub(super) stats: Stats,
}

impl Shared {
    /// Wait (on a thread) until the window has taken what's waiting, or a while.
    fn wait_for_room(&self, cancelled: &AtomicBool) {
        let out = lock(&self.out);
        if out.bytes >= DONE_MAX && !out.queue.is_empty() && !cancelled.load(Ordering::Relaxed) {
            drop(self.room.wait_timeout(out, ROOM_WAIT).unwrap_or_else(PoisonError::into_inner));
        }
    }
}

/// What one step of a message's work came to.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Step {
    /// A picture decoded (or failed), waiting for the window.
    Made,
    /// Files taken out: the picture is decoded next.
    Read,
    /// The window hasn't taken what's waiting: nothing done.
    Full,
    /// Nothing left, or cancelled.
    Ended,
}

/// One message's pictures to decode, a step at a time.
pub(super) struct Batch {
    pub(super) key: MessageKey,
    pub(super) store: Store,
    pub(super) queue: Mutex<VecDeque<(usize, Want)>>,
    /// Taken out with the first ones asked: the message's other pictures and PDFs, so a
    /// picture opened big or a PDF's pages later need no other read.
    pub(super) also: Vec<usize>,
    /// Reads that looked for an index and didn't take it out: past [`READS`] (or at once when
    /// the message hasn't it) it isn't looked for again.
    pub(super) tried: Mutex<HashMap<usize, u8>>,
    /// Most bytes of files kept taken out ([`SOURCES_MAX`]; less in a test).
    pub(super) sources_max: usize,
    pub(super) cancelled: Arc<AtomicBool>,
    /// Its queue ran out (set under the queue's lock): what's asked later goes to a new batch.
    pub(super) ended: AtomicBool,
    /// Done a step a frame on the window's thread (no lane took it).
    pub(super) inline: AtomicBool,
    /// A step panicked: the rest fail.
    pub(super) broken: AtomicBool,
    /// The graphics card's largest texture side.
    pub(super) limit: u32,
    pub(super) fonts: Fonts,
    /// A test's picture that panics as it's decoded.
    #[cfg(test)]
    pub(super) panic_at: Option<usize>,
}

impl Batch {
    /// One step: take the files out, or decode one picture.
    pub(super) fn step(&self, shared: &Shared) -> Step {
        if self.cancelled.load(Ordering::Relaxed) {
            return Step::Ended;
        }
        {
            // The window hasn't taken what's waiting: wait (one waits, whatever its size).
            let out = lock(&shared.out);
            if out.bytes >= DONE_MAX && !out.queue.is_empty() {
                return Step::Full;
            }
        }
        let next = {
            let mut queue = lock(&self.queue);
            let next = queue.pop_front();
            if next.is_none() {
                self.ended.store(true, Ordering::Relaxed);
            }
            next
        };
        let Some((index, want)) = next else { return Step::Ended };
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
        let Some(done) = done else { return Step::Read };
        let mut out = lock(&shared.out);
        out.bytes = out.bytes.saturating_add(done.bytes());
        out.queue.push_back(done);
        Step::Made
    }

    /// The work as a lane runs it: every step, a frame asked for after each picture decoded
    /// (each comes to the window as soon as it's made), waiting while the window has enough.
    pub(super) fn run(self: Arc<Batch>, shared: Arc<Shared>, ctx: egui::Context) {
        loop {
            match self.step(&shared) {
                Step::Made => {
                    shared.stats.wakes.fetch_add(1, Ordering::Relaxed);
                    ctx.request_repaint();
                }
                Step::Read => {}
                Step::Full => shared.wait_for_room(&self.cancelled),
                Step::Ended => break,
            }
        }
        ctx.request_repaint();
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
            Want::Thumb | Want::Body | Want::Big => (crate::reading::decode_pixels(&data, want.max_side(), self.limit), None),
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
        // One read looks for so many (`index` first): the others aren't looked for, so they
        // aren't taken as not there either, and a later read looks for them.
        wanted.truncate(mime::MAX_WANTED);
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
