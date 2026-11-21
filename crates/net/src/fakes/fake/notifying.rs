//! NOTIFY (RFC 5465), as far as a client watching for new mail needs it: `NOTIFY SET` when the
//! server says it has it (`State::caps`), then, while idling, an untagged `STATUS` for each other
//! folder that got mail since (issue #232).

use std::collections::HashMap;

use super::*;

impl Fake {
    /// `NOTIFY SET …`: on, every folder's next UID noted to tell what comes after; refused when
    /// the server isn't said to have it, or is to refuse it.
    pub(super) fn notify_set(&mut self, st: &State, tag: &str) {
        if st.refuse_notify || !st.caps.split_whitespace().any(|c| c.eq_ignore_ascii_case("NOTIFY")) {
            self.say(&format!("{tag} BAD unknown command"));
            return;
        }
        self.notifying = Some(st.boxes.iter().map(|b| (b.name.clone(), b.uid_next)).collect::<HashMap<_, _>>());
        self.say(&format!("{tag} OK notifying"));
    }

    /// While idling with NOTIFY on: a `STATUS` for each folder but the selected one that has mail
    /// since it was last told of.
    pub(super) fn tell_notified(&mut self) {
        if self.idling.is_none() {
            return;
        }
        let Some(told) = self.notifying.as_mut() else { return };
        let Ok(st) = self.st.lock() else { return };
        let mut lines = Vec::new();
        for b in st.boxes.iter().filter(|b| Some(&b.name) != self.selected.as_ref()) {
            let before = told.insert(b.name.clone(), b.uid_next);
            if before.is_some_and(|n| n < b.uid_next) {
                lines.push(format!("* STATUS \"{}\" (MESSAGES {} UIDNEXT {})", b.name, b.msgs.len(), b.uid_next));
            }
        }
        drop(st);
        for l in lines {
            self.say(&l);
        }
    }
}
