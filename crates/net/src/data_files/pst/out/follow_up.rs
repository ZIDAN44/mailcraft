//! The days flagged messages are due, and the days completed ones were done, as they come in:
//! kept by each message's follow-up id
//! ([`Summary::follow_up_id`](mailcraft_model::Summary::follow_up_id)), at most
//! [`MAX_FOLLOW_UP`] of each, and written to the account's [`FOLLOW_UP_DOC`] and
//! [`FOLLOW_UP_DONE_DOC`] with the checkpoints (when they changed) and at the end, for the engine
//! to take into the preferences, which is where MailCraft keeps them.

use mailcraft_model::organize::{FOLLOW_UP_DOC, FOLLOW_UP_DONE_DOC, MAX_FOLLOW_UP, MAX_FOLLOW_UP_ID, merge_follow_up};

use super::*;

/// The days read so far of one kind, and whether any came in since they were last written.
#[derive(Default)]
struct Days {
    days: BTreeMap<String, i64>,
    dirty: bool,
}

impl Days {
    fn note(&mut self, id: &str, day: i64) {
        if id.len() > MAX_FOLLOW_UP_ID || (self.days.len() >= MAX_FOLLOW_UP && !self.days.contains_key(id)) {
            return;
        }
        self.days.insert(id.to_string(), day);
        self.dirty = true;
    }

    /// The days as their document, when they changed since they were last written.
    fn doc(&self, name: &'static str) -> Option<(&'static str, Vec<u8>)> {
        if !self.dirty {
            return None;
        }
        serde_json::to_vec(&self.days).inspect_err(|e| log::warn!("import: {e}")).ok().map(|b| (name, b))
    }

    /// The days of the note read from disk: only what are days (a key too long, a day out of
    /// range or too many are left out, and the note marked to be written clean).
    fn resume(&mut self, read: BTreeMap<String, i64>) {
        let count = read.len();
        let mut kept = BTreeMap::new();
        merge_follow_up(&mut kept, read);
        self.dirty = kept.len() != count;
        self.days = kept;
    }
}

/// The due days read so far, and the days done.
#[derive(Default)]
pub(in crate::data_files::pst) struct Due {
    due: Days,
    done: Days,
}

impl Due {
    /// `summary`'s due day, when it's flagged (or its follow-up complete) and has one, and the
    /// day it was done, when it's complete and has one.
    pub(super) fn note(&mut self, summary: &mailcraft_model::Summary, due: Option<i64>, done: Option<i64>) {
        let complete = summary.flags.follow_up_complete();
        if (!summary.flags.flagged && !complete) || (due.is_none() && done.is_none()) {
            return;
        }
        let id = summary.follow_up_id();
        if let Some(day) = due {
            self.due.note(&id, day);
        }
        if let Some(day) = done.filter(|_| complete) {
            self.done.note(&id, day);
        }
    }
}

impl Out<'_> {
    /// The due and done days to write with a checkpoint, those that changed since the last one
    /// written.
    pub(in crate::data_files::pst) fn due_doc(&self) -> Vec<(&'static str, Vec<u8>)> {
        self.due.due.doc(FOLLOW_UP_DOC).into_iter().chain(self.due.done.doc(FOLLOW_UP_DONE_DOC)).collect()
    }

    /// A checkpoint with the due and done days was written.
    pub(in crate::data_files::pst) fn due_written(&mut self) {
        self.due.due.dirty = false;
        self.due.done.dirty = false;
    }

    /// Going on from a checkpoint: the due and done days read before it (none when there's no
    /// note of them, or it can't be read; what isn't a day, too long a key or a day out of
    /// range, is left out, and the note written clean with the next checkpoint).
    pub(in crate::data_files::pst) fn resume_due(&mut self) {
        for (days, name) in [(&mut self.due.due, FOLLOW_UP_DOC), (&mut self.due.done, FOLLOW_UP_DONE_DOC)] {
            match self.store.load_account_doc::<BTreeMap<String, i64>>(self.account, name) {
                Ok(read) => days.resume(read),
                Err(e) => log::warn!("import: the follow-up days read so far ({name}) couldn't be read: {e}"),
            }
        }
    }

    /// At the end: the due and done days written, when any came in since the last checkpoint.
    pub(super) fn save_due(&mut self) -> Result<(), String> {
        for (days, name) in [(&mut self.due.due, FOLLOW_UP_DOC), (&mut self.due.done, FOLLOW_UP_DONE_DOC)] {
            if days.dirty {
                self.store.save_account_doc(self.account, name, &days.days).map_err(|e| e.to_string())?;
                days.dirty = false;
            }
        }
        Ok(())
    }
}
