//! The session: Undo Send (issue #135). A message sent waits in the Outbox for the seconds
//! File › Options › Mail sets, as Outlook's delay rule and its Undo Send hold it, and Undo takes
//! it back into the form, as it was written, before it goes; then it goes as Delay Delivery's
//! do ([`Session::flush_due_outbox`]).

use super::*;

/// The most seconds a message can be held to be undone.
pub const UNDO_SEND_MAX: u32 = 60;
/// Messages held to be undone at once at most; the oldest beyond goes without its Undo.
const HOLDS_MAX: usize = 8;

/// A message waiting in the Outbox to be undone: as it was written, to open as it was.
#[derive(Clone, Debug)]
pub struct UndoHold {
    pub account: String,
    pub id: u64,
    /// When it goes.
    pub until: i64,
    pub compose: Compose,
}

impl Session {
    /// When a message sent now goes, held for Undo Send; `None` when it isn't held: Undo Send
    /// off, an account with no server (the sample's), or one held already by Delay Delivery.
    pub(super) fn undo_hold_until(&self, c: &Compose, local: bool) -> Option<i64> {
        let secs = self.prefs.undo_send_secs.min(UNDO_SEND_MAX);
        let delayed = c.send_after.is_some_and(|at| at > self.clock.now);
        (secs > 0 && !local && !delayed).then(|| self.clock.now.saturating_add(i64::from(secs)))
    }

    /// Keep `c` as it was written, to be undone until `until`.
    pub(super) fn hold_for_undo(&mut self, c: &Compose, until: i64) {
        if self.undo_holds.len() >= HOLDS_MAX {
            self.undo_holds.remove(0);
        }
        self.undo_holds.push(UndoHold { account: c.account.clone(), id: c.id, until, compose: c.clone() });
    }

    /// The messages that can still be undone, the newest last.
    pub fn undo_holds(&self) -> impl Iterator<Item = &UndoHold> {
        let now = self.clock.now;
        self.undo_holds.iter().filter(move |h| h.until > now && !self.flushed.contains(&h.id))
    }

    /// Those past their time are let go: they're on their way.
    pub(crate) fn undo_holds_due(&mut self) {
        let now = self.clock.now;
        self.undo_holds.retain(|h| h.until > now);
    }

    /// Take a message held to be undone (`id`, else the newest) out of the Outbox and open it
    /// again as it was written, before it goes; refused once it's on its way.
    pub fn undo_send(&mut self, id: Option<u64>) -> CmdResult {
        let at = match id {
            Some(id) => self.undo_holds.iter().rposition(|h| h.id == id),
            None => self.undo_holds.len().checked_sub(1),
        };
        let Some(at) = at else { return Err(CmdError::Failed("there's no message to undo".into())) };
        let gone = self.undo_holds.get(at).is_none_or(|h| h.until <= self.clock.now || self.flushed.contains(&h.id));
        if gone {
            self.undo_holds.remove(at);
            return Err(CmdError::Failed("it has already gone".into()));
        }
        let hold = self.undo_holds.remove(at);
        let store = self.store.clone().ok_or_else(|| CmdError::Failed("no mail cache".into()))?;
        store.outbox_remove(&hold.account, hold.id).map_err(|e| CmdError::Failed(format!("it couldn't be taken out of the Outbox: {e}")))?;
        // The Replied or Forwarded flag it owed its original goes back with it, for when it's sent.
        if self.pending_marks.remove(&hold.id).is_some() {
            self.save_pending_marks();
        }
        let mut c = hold.compose;
        // Its copy in Drafts went as it was sent: a new one is saved as it's written again.
        c.draft_id.clear();
        c.sending = false;
        c.error = None;
        let id = c.id;
        self.open_compose(c);
        self.after_outbox_change();
        Ok(serde_json::json!({ "compose": id }))
    }

    /// Closing: every message held to be undone goes now rather than at the next start; `true`
    /// when one was, and is asked to be sent.
    pub fn release_undo_holds(&mut self) -> bool {
        let held: Vec<u64> = self.undo_holds.drain(..).map(|h| h.id).collect();
        if held.is_empty() {
            return false;
        }
        let now = self.clock.now;
        let store = self.store.clone();
        for (account, item) in self.outbox.iter_mut().filter(|(_, i)| held.contains(&i.id) && i.not_before > now) {
            item.not_before = now;
            if let Some(store) = &store
                && let Err(e) = store.outbox_update(account, item)
            {
                log::warn!("the Outbox couldn't let a message go: {e}");
            }
        }
        self.flush_due_outbox();
        true
    }
}
