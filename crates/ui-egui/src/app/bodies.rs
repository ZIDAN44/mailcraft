//! The open message's body got ready on the reading lane: read from the cache and parsed there
//! (the biggest message took 0.1 s, a frame the window couldn't draw), shown when it's back.

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use mailcraft_engine::{BodyWork, PreparedBody};

use crate::*;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What the window and the reading lane share.
#[derive(Default)]
pub(crate) struct Bodies {
    /// The latest work for the lane: work handed over before it ran finds the newest here (or
    /// nothing), so a body the pane moved on from is never read.
    next: Arc<Mutex<Option<BodyWork>>>,
    /// What the lane made of it, for the window to show (the latest).
    done: Arc<Mutex<Option<PreparedBody>>>,
}

impl MailApp {
    /// From now on (after the commands asked for at start and the files handed over, which want
    /// their answers at once), the open message's body is got ready on the reading lane, where
    /// there is one.
    pub fn defer_bodies(&mut self) {
        let on = self.services.has_background();
        self.session.defer_bodies(on);
    }

    /// A body the lane got ready is shown (its source given to the pictures, which needn't read
    /// it again).
    pub(crate) fn take_body(&mut self) {
        let done = lock(&self.ui.bodies.done).take();
        if let Some(p) = done {
            if let Some(raw) = p.raw().filter(|_| self.session.body_waiting(p.key())) {
                self.ui.pictures.give_source(p.key(), raw.clone());
            }
            self.session.finish_body(p);
        }
    }

    /// The body the open message waits for goes to the reading lane (once a frame, the latest
    /// only), or is got ready here when no lane takes it.
    pub(crate) fn send_body_work(&mut self, ctx: &egui::Context) {
        let Some(work) = self.session.take_body_work() else { return };
        // Work handed over before that hasn't run yet takes this one instead.
        if lock(&self.ui.bodies.next).replace(work).is_some() {
            return;
        }
        let (next, done, ctx) = (self.ui.bodies.next.clone(), self.ui.bodies.done.clone(), ctx.clone());
        let run = Box::new(move || {
            let work = lock(&next).take();
            if let Some(work) = work {
                let prepared = work.run();
                *lock(&done) = Some(prepared);
                ctx.request_repaint();
            }
        });
        if let Err(work) = self.services.background(Lane::Reading, Work { run, cancelled: Arc::new(AtomicBool::new(false)), first: false }) {
            (work.run)();
            self.take_body();
        }
    }
}
