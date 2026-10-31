//! The session: moving through the list and selecting a range of it, from the keyboard (Up,
//! Down, Home, End, Page Up, Page Down, with Shift to select as they go) and with Shift+click
//! (issue #26).

use super::*;

/// Where the focus goes in the list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Toward {
    /// So many rows down (up, when less than 0).
    By(i64),
    First,
    Last,
}

impl Session {
    /// The key of each message row, in the order the list shows them (a conversation's: its
    /// newest message's).
    pub fn row_keys(&self) -> Vec<MessageKey> {
        self.rows()
            .iter()
            .filter_map(|r| match r {
                Row::Message { index, .. } => self.list.get(*index).map(|m| m.key.clone()),
                _ => None,
            })
            .collect()
    }

    /// The row a message is on: its own, or its conversation's (one read from the conversation
    /// shown above it in the reading pane has no row of its own).
    fn row_of(&self, rows: &[MessageKey], key: &MessageKey) -> Option<usize> {
        if let Some(i) = rows.iter().position(|k| k == key) {
            return Some(i);
        }
        let at = self.list.iter().position(|m| &m.key == key)?;
        let t = self.threads.iter().find(|t| t.messages.contains(&at))?;
        let members: HashSet<&MessageKey> = t.messages.iter().filter_map(|&i| self.list.get(i)).map(|m| &m.key).collect();
        rows.iter().position(|k| members.contains(k))
    }

    /// The row the focus goes to from where it is (no focus: from the top, the first step down
    /// landing on the first row, as n single steps do).
    fn row_toward(&self, rows: &[MessageKey], toward: Toward) -> Option<usize> {
        let last = rows.len().checked_sub(1)?;
        let cur = self.focus.as_ref().and_then(|k| self.row_of(rows, k));
        Some(match (toward, cur) {
            (Toward::First, _) => 0,
            (Toward::Last, _) => last,
            (Toward::By(n), None) => usize::try_from(n.saturating_sub(1).max(0)).unwrap_or(usize::MAX).min(last),
            (Toward::By(n), Some(c)) => {
                let c = i64::try_from(c).unwrap_or(i64::MAX);
                usize::try_from(c.saturating_add(n).max(0)).unwrap_or(0).min(last)
            }
        })
    }

    /// Down, Up, Home, End, Page Down, Page Up: the message there is selected alone and read.
    pub fn move_focus(&mut self, toward: Toward) -> CmdResult {
        let rows = self.row_keys();
        let Some(key) = self.row_toward(&rows, toward).and_then(|i| rows.get(i)).cloned() else {
            return Ok(serde_json::json!({"key": null}));
        };
        self.open_message(&key);
        Ok(serde_json::json!({"key": key.to_id()}))
    }

    /// The same with Shift: the focus moves and everything from the anchor to it is selected
    /// (going back toward the anchor selects less), as in Outlook's list.
    pub fn extend_selection(&mut self, toward: Toward) -> CmdResult {
        let rows = self.row_keys();
        // Nothing chosen yet: a step of n starts where the first of n single steps lands, the top.
        let placed = |k: &Option<MessageKey>| k.as_ref().and_then(|k| self.row_of(&rows, k)).is_some();
        if matches!(toward, Toward::By(_)) && !placed(&self.anchor) && !placed(&self.focus) {
            self.anchor = rows.first().cloned();
        }
        match self.row_toward(&rows, toward) {
            Some(to) => self.select_to(&rows, to, false),
            None => Ok(serde_json::json!({"selected": self.selection.len(), "key": null})),
        }
    }

    /// Shift+click: from the anchor to `key`, in the order the list shows them (pinned mail
    /// first, a conversation as its one row); `add` (Ctrl+Shift+click) keeps what else was
    /// selected.
    pub fn select_range(&mut self, key: &MessageKey, add: bool) -> CmdResult {
        let rows = self.row_keys();
        let to = self.row_of(&rows, key).ok_or_else(|| CmdError::Params(format!("`{}` is not in the list", key.to_id())))?;
        self.select_to(&rows, to, add)
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

    fn select_to(&mut self, rows: &[MessageKey], to: usize, add: bool) -> CmdResult {
        // No anchor (nothing was chosen yet, or it left the list): where the focus is, else here.
        let from =
            self.anchor.as_ref().and_then(|k| self.row_of(rows, k)).or_else(|| self.focus.as_ref().and_then(|k| self.row_of(rows, k))).unwrap_or(to);
        let range = rows.get(from.min(to)..=from.max(to)).unwrap_or_default();
        if add {
            let have: HashSet<MessageKey> = self.selection.iter().cloned().collect();
            self.selection.extend(range.iter().filter(|k| !have.contains(*k)).cloned());
        } else {
            self.selection = range.to_vec();
        }
        self.anchor = rows.get(from).cloned();
        self.focus = rows.get(to).cloned();
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
                if self.reading.is_some() {
                    self.leave_reading(None);
                    self.reading = None;
                }
            }
        }
    }
}
