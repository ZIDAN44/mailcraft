//! The tests' folders in the temporary folder (the crate's own and those in `tests/`): each
//! removed when its test's thread ends, whether the test passed or failed.

use std::cell::RefCell;
use std::path::PathBuf;

/// The folders a test made ([`scratch`]), removed when its thread ends (after the test's own
/// values, open files among them, have gone).
#[derive(Default)]
struct Scratch(Vec<PathBuf>);

impl Drop for Scratch {
    fn drop(&mut self) {
        for folder in &self.0 {
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
    SCRATCH.with(|s| s.borrow_mut().0.push(folder.clone()));
    folder
}
