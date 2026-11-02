//! The window's settings written on a thread of their own ([`App::keep_state`](crate::App)).
//!
//! Each write is flushed to the disk before it replaces the file (`write_atomic`): on the
//! window's thread that took 5–15 ms of a frame every few seconds, and 197 ms while the disk was
//! busy flushing a 38 GB data file. The window hands over the settings as they are now; the
//! thread writes the latest of them when they differ from what it wrote last (older ones handed
//! over meanwhile are skipped), in order. Closing waits for the last one.

use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex, PoisonError};

use serde_json::Value;

/// What the window hands over, and what the thread has done with it.
#[derive(Default)]
struct Next {
    /// The settings to write next (`None`: nothing new).
    state: Option<Value>,
    closing: bool,
    /// Handed over, and of them looked at by the thread (written, or the same as before).
    handed: u64,
    done: u64,
}

#[derive(Default)]
struct Shared {
    next: Mutex<Next>,
    /// Something was handed over, or done.
    changed: Condvar,
    /// Held by a test to keep the thread from writing.
    #[cfg(test)]
    pause: Mutex<()>,
}

impl Shared {
    fn next(&self) -> std::sync::MutexGuard<'_, Next> {
        self.next.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Writes the settings to `path` on its thread (or, when there's none, as they're handed over).
pub(crate) struct StateWriter {
    path: PathBuf,
    shared: Arc<Shared>,
    thread: Option<std::thread::JoinHandle<()>>,
    /// What was written last, when it's written here (no thread).
    written: Vec<u8>,
}

impl StateWriter {
    pub(crate) fn new(path: PathBuf) -> StateWriter {
        let shared = Arc::new(Shared::default());
        let (to, theirs) = (path.clone(), shared.clone());
        let thread = std::thread::Builder::new()
            .name("settings".into())
            .spawn(move || write_on(&to, &theirs))
            .inspect_err(|e| log::warn!("the settings will be saved on the window's thread: {e}"))
            .ok();
        StateWriter { path, shared, thread, written: Vec::new() }
    }

    /// Hand over the settings as they are now: written soon, if they changed.
    pub(crate) fn keep(&mut self, state: Value) {
        if self.thread.is_none() {
            write_changed(&self.path, &state, &mut self.written);
            return;
        }
        let mut next = self.shared.next();
        next.state = Some(state);
        next.handed = next.handed.saturating_add(1);
        self.shared.changed.notify_all();
    }

    /// Closing: the settings handed over last are written before this returns.
    pub(crate) fn close(&mut self) {
        let Some(thread) = self.thread.take() else { return };
        self.shared.next().closing = true;
        self.shared.changed.notify_all();
        if thread.join().is_err() {
            log::warn!("the settings weren't saved on closing");
        }
    }

    /// Wait until what was handed over has been looked at (tests).
    #[cfg(test)]
    fn settle(&self) {
        let mut next = self.shared.next();
        while next.done < next.handed {
            next = self.shared.changed.wait(next).unwrap_or_else(PoisonError::into_inner);
        }
    }
}

impl Drop for StateWriter {
    fn drop(&mut self) {
        self.close();
    }
}

/// The thread: the latest settings handed over written, until closing (the last ones first).
fn write_on(path: &std::path::Path, shared: &Shared) {
    let mut written = Vec::new();
    loop {
        let (state, handed, closing) = {
            let mut next = shared.next();
            while next.state.is_none() && !next.closing {
                next = shared.changed.wait(next).unwrap_or_else(PoisonError::into_inner);
            }
            (next.state.take(), next.handed, next.closing)
        };
        #[cfg(test)]
        drop(shared.pause.lock().unwrap_or_else(PoisonError::into_inner));
        if let Some(state) = state {
            write_changed(path, &state, &mut written);
        }
        let mut next = shared.next();
        next.done = next.done.max(handed);
        shared.changed.notify_all();
        if closing && next.state.is_none() {
            return;
        }
    }
}

/// Write `state` to `path` when it isn't what was `written` last.
fn write_changed(path: &std::path::Path, state: &Value, written: &mut Vec<u8>) {
    let Ok(bytes) = serde_json::to_vec_pretty(state) else { return };
    if bytes == *written {
        return;
    }
    match mailcraft_engine::store::write_atomic(path, &bytes) {
        Ok(()) => *written = bytes,
        Err(e) => log::warn!("the settings weren't saved: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Handing the settings over never waits for the disk (the thread held, as a busy disk would):
    /// the latest of them is written once it's free, and closing writes the last ones handed over.
    #[test]
    fn settings_are_written_off_the_window() {
        let dir = std::env::temp_dir().join(format!("mailcraft-settings-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("ui.json");
        let read = || std::fs::read(&path).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok());
        let mut w = StateWriter::new(path.clone());
        // The thread kept from writing.
        let shared = w.shared.clone();
        let held = shared.pause.lock().unwrap_or_else(PoisonError::into_inner);
        let began = std::time::Instant::now();
        for n in 0..100 {
            w.keep(json!({ "n": n }));
        }
        assert!(began.elapsed() < std::time::Duration::from_secs(1), "{:?}", began.elapsed());
        drop(held);
        w.settle();
        assert_eq!(read(), Some(json!({ "n": 99 })), "the latest");
        w.keep(json!({ "n": 100 }));
        w.close();
        assert_eq!(read(), Some(json!({ "n": 100 })), "the last, written on closing");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
