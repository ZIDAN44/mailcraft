//! The session: moving through the list and selecting a range of it, from the keyboard (Up,
//! Down, Home, End, Page Up, Page Down, with Shift to select as they go) and with Shift+click
//! (issue #26).

use super::*;

/// Where the focus goes in the list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Toward {
    /// So many rows down (up, when less than 0): a page. With no focus, the top row.
    By(i64),
    /// So many single steps down (up, when less than 0): Down or Up pressed n times. With no
    /// focus, where n single steps land, the first of them on the top row.
    Steps(i64),
    First,
    Last,
}

impl Session {
    /// The key of each message row, in the order the list shows them (a conversation's: its
    /// newest message's). A copy of every key: for tests and the command line, not each step.
    pub fn row_keys(&self) -> Vec<MessageKey> {
        (0..self.row_count()).filter_map(|p| self.row_key(p).cloned()).collect()
    }

    /// The message row the focus goes to from where it is (no focus: a page goes to the top
    /// row, n single steps from the top, the first of them landing on the first row).
    fn row_toward(&self, toward: Toward) -> Option<usize> {
        let last = self.row_count().checked_sub(1)?;
        let cur = self.focus.as_ref().and_then(|k| self.msg_row_of(k));
        Some(match (toward, cur) {
            (Toward::First, _) | (Toward::By(_), None) => 0,
            (Toward::Last, _) => last,
            (Toward::Steps(n), None) => usize::try_from(n.saturating_sub(1).max(0)).unwrap_or(usize::MAX).min(last),
            (Toward::By(n) | Toward::Steps(n), Some(c)) => {
                let c = i64::try_from(c).unwrap_or(i64::MAX);
                usize::try_from(c.saturating_add(n).max(0)).unwrap_or(0).min(last)
            }
        })
    }

    /// Down, Up, Home, End, Page Down, Page Up: the message there is selected alone and read.
    /// A lookup in the rows kept with the list: one key is copied, whatever the list's length.
    pub fn move_focus(&mut self, toward: Toward) -> CmdResult {
        let Some(key) = self.row_toward(toward).and_then(|p| self.row_key(p)).cloned() else {
            return Ok(serde_json::json!({"key": null}));
        };
        self.open_message(&key);
        Ok(serde_json::json!({"key": key.to_id()}))
    }

    /// The same with Shift: the focus moves and everything from the anchor to it is selected
    /// (going back toward the anchor selects less), as in Outlook's list.
    pub fn extend_selection(&mut self, toward: Toward) -> CmdResult {
        // Nothing chosen yet: n steps start where the first of n single steps lands, the top (a
        // page selects the top row alone, as it did before).
        let placed = |k: &Option<MessageKey>| k.as_ref().and_then(|k| self.msg_row_of(k)).is_some();
        if matches!(toward, Toward::Steps(_)) && !placed(&self.anchor) && !placed(&self.focus) {
            self.anchor = self.row_key(0).cloned();
        }
        match self.row_toward(toward) {
            Some(to) => self.select_to(to, false),
            None => Ok(serde_json::json!({"selected": self.selection.len(), "key": null})),
        }
    }

    /// Shift+click: from the anchor to `key`, in the order the list shows them (pinned mail
    /// first, a conversation as its one row); `add` (Ctrl+Shift+click) keeps what else was
    /// selected.
    pub fn select_range(&mut self, key: &MessageKey, add: bool) -> CmdResult {
        let to = self.msg_row_of(key).ok_or_else(|| CmdError::Params(format!("`{}` is not in the list", key.to_id())))?;
        self.select_to(to, add)
    }

    /// Ctrl+click: a message in or out of the selection; the focus and the anchor go to it.
    pub fn toggle_selected(&mut self, key: &MessageKey) {
        if let Some(i) = self.selection.iter().position(|x| x == key) {
            self.selection.remove(i);
        } else {
            self.selection.push(key.clone());
        }
        self.focus = Some(key.clone());
        self.anchor = Some(key.clone());
        self.after_selecting();
    }

    /// Ctrl+A: every message in the list. Several are then selected as with Shift, so the
    /// reading pane says how many and Reply waits for one (it read on, and Reply answered the
    /// message read before: found testing issue #26).
    pub fn select_every_message(&mut self) {
        self.selection = self.list.iter().map(|m| m.key.clone()).collect();
        if !self.selection.is_empty() {
            self.after_selecting();
        }
    }

    /// From the anchor to message row `to`: as many keys copied as rows in the range.
    fn select_to(&mut self, to: usize, add: bool) -> CmdResult {
        // No anchor (nothing was chosen yet, or it left the list): where the focus is, else here.
        let from =
            self.anchor.as_ref().and_then(|k| self.msg_row_of(k)).or_else(|| self.focus.as_ref().and_then(|k| self.msg_row_of(k))).unwrap_or(to);
        let range: Vec<MessageKey> = (from.min(to)..=from.max(to)).filter_map(|p| self.row_key(p).cloned()).collect();
        let (anchor, focus) = (self.row_key(from).cloned(), self.row_key(to).cloned());
        if add {
            let have: HashSet<MessageKey> = self.selection.iter().cloned().collect();
            self.selection.extend(range.into_iter().filter(|k| !have.contains(k)));
        } else {
            self.selection = range;
        }
        self.anchor = anchor;
        self.focus = focus;
        self.after_selecting();
        Ok(serde_json::json!({"selected": self.selection.len(), "key": self.focus.as_ref().map(MessageKey::to_id)}))
    }

    /// One message selected is read in the reading pane; several (or none) aren't, the pane says
    /// how many there are instead, as Outlook's does. Reply and the like wait for one.
    fn after_selecting(&mut self) {
        match self.selection.as_slice() {
            [one] => {
                if self.reading.as_ref().is_none_or(|r| &r.key != one) {
                    let (one, anchor) = (one.clone(), self.anchor.clone());
                    self.show_message(&one, true);
                    self.anchor = anchor;
                }
            }
            _ => {
                // Several selected: the pane says how many, so a message written in it goes too.
                if self.selection.len() > 1 {
                    self.make_way_for_reading();
                }
                if self.reading.is_some() {
                    self.leave_reading(None);
                    self.reading = None;
                }
            }
        }
    }
}
