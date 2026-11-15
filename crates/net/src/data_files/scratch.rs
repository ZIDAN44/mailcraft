//! Net's tests' folders in the temporary folder: each removed when its test's thread ends,
//! whether the test passed or failed, once the threads of the test's pools have ended too.

use std::cell::RefCell;
use std::path::PathBuf;
use std::time::Instant;

/// The folders a test made ([`scratch`]), removed when its thread ends (after the test's own
/// values, stores and files among them, have gone); and its pools' threads ([`wait_for`]),
/// waited for first, so none writes in a folder after it went.
#[derive(Default)]
struct Scratch {
    folders: Vec<PathBuf>,
    running: Vec<Box<dyn Fn() -> bool>>,
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if self.folders.is_empty() {
            return;
        }
        let end = Instant::now() + crate::waiting::DEADLINE;
        for running in &self.running {
            while running() && Instant::now() < end {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
        for folder in &self.folders {
            // Leftovers in the temporary folder are harmless; a failed clean-up isn't a failed test.
            let _ = std::fs::remove_dir_all(folder);
        }
    }
}

thread_local!(static SCRATCH: RefCell<Scratch> = RefCell::default());

/// A new, empty folder for the test, `mailcraft-<name>-<pid>` in the temporary folder: what a
/// run before left there is removed now, and the folder with all in it once the test is over.
pub(crate) fn scratch(name: &str) -> PathBuf {
    let folder = std::env::temp_dir().join(format!("mailcraft-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    SCRATCH.with(|s| s.borrow_mut().folders.push(folder.clone()));
    folder
}

/// Threads the test started (a pool's: [`crate::worker::Pool::new`]) that may write in its
/// folders: its folders go once `running` says they've all ended (or the tests' deadline passed).
pub(crate) fn wait_for(running: impl Fn() -> bool + 'static) {
    SCRATCH.with(|s| s.borrow_mut().running.push(Box::new(running)));
}
