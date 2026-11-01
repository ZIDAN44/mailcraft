//! Room left on the drive a data file is written to: asked before a data file is imported (is
//! there room for MailCraft's copy?) and while it is (an import stops itself before the drive is
//! full, rather than failing every write after and counting the messages as lost).

use std::fmt::Debug;
use std::path::Path;

/// Below this much room an import stops itself, to go on once there's more: enough for the
/// batch in hand, the folder lists and the checkpoint that end it.
pub const LOW_SPACE: u64 = 1 << 30;

/// Where the room left on a drive is asked (the drive itself; a test's full one).
pub trait Space: Debug + Send + Sync {
    /// Bytes free to this user on the drive that holds `dir` (none: it can't be told).
    fn free(&self, dir: &Path) -> Option<u64>;
}

/// The computer's own drives.
#[derive(Clone, Copy, Debug, Default)]
pub struct Drives;

impl Space for Drives {
    fn free(&self, dir: &Path) -> Option<u64> {
        fs4::available_space(nearest_existing(dir)?).ok()
    }
}

/// The folder at `dir` or, when it isn't there yet (the data files' folder before the first is
/// made), the nearest folder above it that is: it's on the drive the files go to.
pub fn nearest_existing(dir: &Path) -> Option<&Path> {
    // A relative path ends in "", which is the folder MailCraft runs in.
    dir.ancestors().map(|a| if a.as_os_str().is_empty() { Path::new(".") } else { a }).find(|a| a.is_dir())
}

/// A test's drive that fills up: roomy for its first `roomy` asks, then with `left` bytes free.
#[cfg(test)]
#[derive(Debug)]
pub(crate) struct Filling {
    asks: std::sync::atomic::AtomicUsize,
    roomy: usize,
    left: u64,
}

#[cfg(test)]
impl Filling {
    pub(crate) fn new(roomy: usize, left: u64) -> Filling {
        Filling { asks: std::sync::atomic::AtomicUsize::new(0), roomy, left }
    }

    /// How many times the room was asked.
    pub(crate) fn asks(&self) -> usize {
        self.asks.load(std::sync::atomic::Ordering::Relaxed)
    }
}

#[cfg(test)]
impl Space for Filling {
    fn free(&self, _dir: &Path) -> Option<u64> {
        let n = self.asks.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Some(if n < self.roomy { 1 << 40 } else { self.left })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn room_is_asked_of_the_nearest_folder_there() {
        let dir = std::env::temp_dir().join(format!("mailcraft-space-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let deep = dir.join("not").join("made").join("yet");
        assert_eq!(nearest_existing(&deep), Some(dir.as_path()));
        assert_eq!(nearest_existing(&dir), Some(dir.as_path()));
        // A file's folder, and a path under a file (no folder there): the folder above.
        let file = dir.join("a.mailcraft");
        std::fs::write(&file, b"x").unwrap();
        assert_eq!(nearest_existing(&file.join("x")), Some(dir.as_path()));
        assert!(Drives.free(&deep).is_some_and(|n| n > 0), "the temporary folder's drive has room");
        assert_eq!(nearest_existing(Path::new("")), Some(Path::new(".")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
