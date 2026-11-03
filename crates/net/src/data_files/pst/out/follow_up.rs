//! The days flagged messages are due, as they come in: kept by each message's follow-up id
//! ([`Summary::follow_up_id`](mailcraft_model::Summary::follow_up_id)), at most
//! [`MAX_FOLLOW_UP`], and written to the account's [`FOLLOW_UP_DOC`] with the checkpoints (when
//! they changed) and at the end, for the engine to take into the preferences, which is where
//! MailCraft keeps them.

use mailcraft_model::organize::{FOLLOW_UP_DOC, MAX_FOLLOW_UP, MAX_FOLLOW_UP_ID, merge_follow_up};

use super::*;

/// The due days read so far, and whether any came in since they were last written.
#[derive(Default)]
pub(in crate::data_files::pst) struct Due {
    days: BTreeMap<String, i64>,
    dirty: bool,
}

impl Due {
    /// `summary`'s due day, when it's flagged and has one.
    pub(super) fn note(&mut self, summary: &mailcraft_model::Summary, day: Option<i64>) {
        let Some(day) = day.filter(|_| summary.flags.flagged) else { return };
        let id = summary.follow_up_id();
        if id.len() > MAX_FOLLOW_UP_ID || (self.days.len() >= MAX_FOLLOW_UP && !self.days.contains_key(&id)) {
            return;
        }
        self.days.insert(id, day);
        self.dirty = true;
    }
}

impl Out<'_> {
    /// The due days to write with a checkpoint, when they changed since the last one written.
    pub(in crate::data_files::pst) fn due_doc(&self) -> Option<(&'static str, Vec<u8>)> {
        if !self.due.dirty {
            return None;
        }
        serde_json::to_vec(&self.due.days).inspect_err(|e| log::warn!("import: {e}")).ok().map(|b| (FOLLOW_UP_DOC, b))
    }

    /// A checkpoint with the due days was written.
    pub(in crate::data_files::pst) fn due_written(&mut self) {
        self.due.dirty = false;
    }

    /// Going on from a checkpoint: the due days read before it (none when there's no note of
    /// them, or it can't be read; what isn't a due day, too long a key or a day out of range,
    /// is left out, and the note written clean with the next checkpoint).
    pub(in crate::data_files::pst) fn resume_due(&mut self) {
        match self.store.load_account_doc::<BTreeMap<String, i64>>(self.account, FOLLOW_UP_DOC) {
            Ok(days) => {
                let read = days.len();
                let mut kept = BTreeMap::new();
                merge_follow_up(&mut kept, days);
                self.due.dirty = kept.len() != read;
                self.due.days = kept;
            }
            Err(e) => log::warn!("import: the due days read so far couldn't be read: {e}"),
        }
    }

    /// At the end: the due days written, when any came in since the last checkpoint.
    pub(super) fn save_due(&mut self) -> Result<(), String> {
        if !self.due.dirty {
            return Ok(());
        }
        self.store.save_account_doc(self.account, FOLLOW_UP_DOC, &self.due.days).map_err(|e| e.to_string())?;
        self.due.dirty = false;
        Ok(())
    }
}
