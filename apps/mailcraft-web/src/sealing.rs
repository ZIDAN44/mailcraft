//! The turns of sealing an account's remembered S/MIME passwords (`backend/smime.rs`, gap 7):
//! sealing waits on the browser, so seals can land out of order, or after keeping was turned off.
//! Only the latest seal is kept, and none once keeping is off.
#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

/// One account's sealing.
#[derive(Default)]
pub struct Sealing {
    /// Bumped by each seal begun and by keeping turned off: a seal that sees it changed is dropped.
    turn: u64,
    /// Keeping as last asked on this page; `None`: as the browser's storage says.
    wanted: Option<bool>,
}

impl Sealing {
    /// Whether the passwords are kept sealed: as last asked on this page, else as `kept` says
    /// (what's in the browser's storage, which a seal under way hasn't reached yet).
    pub fn keeping(&self, kept: impl FnOnce() -> bool) -> bool {
        self.wanted.unwrap_or_else(kept)
    }

    /// A seal begins (`start`: it turns keeping on); its turn, for [`Sealing::current`].
    pub fn begin(&mut self, start: bool) -> u64 {
        if start {
            self.wanted = Some(true);
        }
        self.turn = self.turn.wrapping_add(1);
        self.turn
    }

    /// Keeping is turned off: seals under way are dropped.
    pub fn stop(&mut self) {
        self.wanted = Some(false);
        self.turn = self.turn.wrapping_add(1);
    }

    /// Whether the seal begun at `turn` is still the one to keep: none began after it, and keeping
    /// wasn't turned off since.
    pub fn current(&self, turn: u64) -> bool {
        self.turn == turn && self.wanted != Some(false)
    }
}

/// The passwords opened from what was kept, added to those noted meanwhile (typed while they were
/// being opened); `true` when some noted meanwhile weren't kept, so they're to be sealed again.
pub fn merge(noted: &mut Vec<String>, kept: Vec<String>) -> bool {
    let more = noted.iter().any(|p| !kept.contains(p));
    for p in kept {
        if !noted.contains(&p) {
            noted.push(p);
        }
    }
    more
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_older_seal_landing_last_is_dropped() {
        let mut s = Sealing::default();
        let first = s.begin(true);
        let second = s.begin(false);
        assert!(s.current(second));
        assert!(!s.current(first), "the older list would overwrite the newer one");
    }

    #[test]
    fn a_seal_landing_after_keeping_stopped_is_dropped() {
        let mut s = Sealing::default();
        let turn = s.begin(true);
        s.stop();
        assert!(!s.current(turn), "keeping would be turned back on");
        assert!(!s.keeping(|| true), "the storage not cleared yet still reads as off");
        // Turned on again: that seal is kept.
        let again = s.begin(true);
        assert!(s.current(again));
    }

    #[test]
    fn keeping_turned_on_holds_before_its_seal_lands() {
        let mut s = Sealing::default();
        assert!(!s.keeping(|| false));
        assert!(s.keeping(|| true), "nothing asked on this page: as kept");
        let first = s.begin(true);
        // A password typed while the first seal is under way is sealed too, and that seal wins.
        assert!(s.keeping(|| false));
        let second = s.begin(false);
        assert!(!s.current(first) && s.current(second));
    }

    #[test]
    fn restored_passwords_join_those_typed_meanwhile() {
        let mut noted = vec!["typed".to_string()];
        assert!(!merge(&mut noted, vec!["kept".into(), "typed".into(), "other".into()]), "every one typed is kept");
        assert_eq!(noted, ["typed", "kept", "other"]);
        let mut noted = vec!["typed".to_string()];
        assert!(merge(&mut noted, vec!["kept".into()]), "the one typed isn't in what's kept");
        assert_eq!(noted, ["typed", "kept"]);
        let mut noted = Vec::new();
        assert!(!merge(&mut noted, vec!["kept".into()]));
        assert_eq!(noted, ["kept"]);
    }
}
