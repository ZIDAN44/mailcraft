//! Whether the account and folder lists a page without IndexedDB keeps in local storage
//! (`web.rs`) are behind the cache: written again only when a file among them changed, and
//! until the write is known to have happened.
#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

/// The lists in local storage against the cache's.
#[derive(Default)]
pub struct Lists {
    /// A file among them changed since they were last written whole.
    behind: bool,
}

impl Lists {
    /// Whether to write them now; `changed`: a file among them was written or removed since
    /// the last ask. One that changed before and wasn't written since still counts: the cache
    /// names a change once, and a write local storage refused was never tried again.
    pub fn due(&mut self, changed: bool) -> bool {
        self.behind |= changed;
        self.behind
    }

    /// They were written; `kept`: local storage has them as written (it refuses what it has no
    /// room for without saying so).
    pub fn written(&mut self, kept: bool) {
        self.behind = !kept;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The lists are written when a file among them changed, not otherwise, and again at each
    /// save until local storage has them.
    #[test]
    fn a_write_that_didnt_happen_is_tried_again() {
        let mut lists = Lists::default();
        assert!(!lists.due(false), "nothing changed: what's there is still right");
        assert!(lists.due(true));
        lists.written(true);
        assert!(!lists.due(false), "written: not again");
        // Refused: tried at the next save, and the one after, though nothing changed since.
        assert!(lists.due(true));
        lists.written(false);
        assert!(lists.due(false));
        assert!(lists.due(false), "not written meanwhile");
        lists.written(true);
        assert!(!lists.due(false));
    }
}
