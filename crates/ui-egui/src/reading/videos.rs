//! The work behind a video attachment's thumbnail, off the window's thread: its file written
//! out for the platform's decoder, then the frame the platform made decoded. Both were done in
//! the frame that first showed the message: on a real file, reading a message to write its
//! 1.7 MB video held that frame for 12 ms.
//!
//! A chip asks each frame it's drawn; what it asked for and no longer asks (its message left)
//! is let go of, and its work, when it hasn't run yet, does nothing. Where there are no threads
//! (the tests) the work is done in the window as it's handed over.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use mailcraft_engine::mime;
use mailcraft_engine::model::MessageKey;
use mailcraft_engine::store::Store;

use crate::services::{Lane, Services, Work};

/// A video attachment: its message and index.
type Id = (MessageKey, usize);

/// A frame decoded, and how to draw it.
type Pixels = (egui::ColorImage, egui::TextureOptions);

/// Most pixels on a side of a video's frame, decoded.
const FRAME_SIDE: u32 = 480;

/// What is made for a video.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Stage {
    /// Its file, written out.
    File,
    /// The platform's frame, decoded.
    Frame,
}

/// Work asked for in a frame, not handed to a lane yet.
enum Job {
    File { store: Store, name: String },
    Frame { png: Vec<u8>, limit: u32 },
}

/// What the work made.
enum Made {
    File(Option<PathBuf>),
    Frame(Option<Pixels>),
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The video thumbnails' work, asked for by the chips and handed to the pictures lane.
#[derive(Default)]
pub(crate) struct Videos {
    wanted: Vec<((Id, Stage), Job)>,
    /// Asked for and not taken yet, and whether that work is still wanted.
    asked: HashMap<(Id, Stage), Arc<AtomicBool>>,
    /// Asked for (or taken) in this frame.
    touched: HashSet<(Id, Stage)>,
    done: Arc<Mutex<HashMap<(Id, Stage), Made>>>,
}

impl Videos {
    /// Attachment `index` of the message `key`, written out to a file for the platform's
    /// decoder: `None` while it's being written (asked for now, the first time), `Some(None)`
    /// when it can't be.
    pub(crate) fn file(&mut self, key: &MessageKey, index: usize, name: &str, store: Option<&Store>) -> Option<Option<PathBuf>> {
        let id = ((key.clone(), index), Stage::File);
        if let Some(made) = self.take(&id) {
            return Some(match made {
                Made::File(path) => path,
                Made::Frame(_) => None,
            });
        }
        let Some(store) = store else { return Some(None) };
        self.ask(id, || Job::File { store: store.clone(), name: name.to_string() });
        None
    }

    /// The frame the platform made of the video (`png`, as it encoded it) as pixels: `None`
    /// while it's being decoded, `Some(None)` when it can't be.
    pub(crate) fn frame(&mut self, key: &MessageKey, index: usize, png: &[u8], limit: u32) -> Option<Option<Pixels>> {
        let id = ((key.clone(), index), Stage::Frame);
        if let Some(made) = self.take(&id) {
            return Some(match made {
                Made::Frame(pixels) => pixels,
                Made::File(_) => None,
            });
        }
        self.ask(id, || Job::Frame { png: png.to_vec(), limit });
        None
    }

    fn take(&mut self, id: &(Id, Stage)) -> Option<Made> {
        let made = lock(&self.done).remove(id)?;
        self.asked.remove(id);
        Some(made)
    }

    fn ask(&mut self, id: (Id, Stage), job: impl FnOnce() -> Job) {
        self.touched.insert(id.clone());
        if !self.asked.contains_key(&id) {
            self.asked.insert(id.clone(), Arc::new(AtomicBool::new(false)));
            self.wanted.push((id, job()));
        }
    }

    /// Once a frame, after drawing: the work asked for goes to the pictures lane (or is done
    /// here when there's none), and what was asked for before but not this frame is let go of.
    pub(crate) fn end_frame(&mut self, ctx: &egui::Context, services: &mut dyn Services) {
        let touched = std::mem::take(&mut self.touched);
        self.asked.retain(|id, cancelled| {
            let keep = touched.contains(id);
            if !keep {
                cancelled.store(true, Ordering::Relaxed);
            }
            keep
        });
        lock(&self.done).retain(|id, _| self.asked.contains_key(id));
        for (id, job) in std::mem::take(&mut self.wanted) {
            let Some(cancelled) = self.asked.get(&id).cloned() else { continue };
            let (done, ctx) = (self.done.clone(), ctx.clone());
            let run = {
                let cancelled = cancelled.clone();
                Box::new(move || {
                    if cancelled.load(Ordering::Relaxed) {
                        return;
                    }
                    let stage = id.1;
                    // A decoder that panics leaves the chip with its kind's icon, and nothing else.
                    let made = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| make(&id.0, job))).unwrap_or_else(|_| {
                        log::warn!("a video's thumbnail couldn't be made (the decoder failed)");
                        match stage {
                            Stage::File => Made::File(None),
                            Stage::Frame => Made::Frame(None),
                        }
                    });
                    if !cancelled.load(Ordering::Relaxed) {
                        lock(&done).insert(id, made);
                    }
                    ctx.request_repaint();
                })
            };
            if let Err(work) = services.background(Lane::Pictures, Work { run, cancelled, first: false }) {
                (work.run)();
            }
        }
    }
}

/// The work itself: hostile input, behind the caps of what it calls (the decoder's limits, a
/// file taken out of its message at most as big as the cache holds).
fn make((key, index): &Id, job: Job) -> Made {
    match job {
        Job::File { store, name } => Made::File((|| {
            let raw = store.read_body(key)?;
            let data = mime::attachment_data(&raw, *index)?;
            let dir = mailcraft_engine::store::scratch_dir("thumbs")?;
            let path = dir.join(format!("{:016x}-{}", mailcraft_engine::store::fnv64(&key.to_id()), mime::sanitize_filename(&name)));
            std::fs::write(&path, &data).ok()?;
            Some(path)
        })()),
        Job::Frame { png, limit } => Made::Frame(super::decode_pixels(&png, FRAME_SIDE, limit)),
    }
}
