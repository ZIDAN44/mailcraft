//! The days flagged messages are due, the days completed ones were done, and when flagged ones
//! remind, as they come in: kept by each message's follow-up id
//! ([`Summary::follow_up_id`](mailcraft_model::Summary::follow_up_id)), at most
//! [`MAX_FOLLOW_UP`] of each, and written to the account's [`FOLLOW_UP_DOC`],
//! [`FOLLOW_UP_DONE_DOC`] and [`FOLLOW_UP_REMINDER_DOC`] with the checkpoints (when they changed)
//! and at the end, for the engine to take into the preferences, which is where MailCraft keeps
//! them.

use mailcraft_model::organize::{
    FOLLOW_UP_DOC, FOLLOW_UP_DONE_DOC, FOLLOW_UP_REMINDER_DOC, MAX_FOLLOW_UP, MAX_FOLLOW_UP_ID, merge_follow_up, merge_reminders,
};

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

    /// The days of the note read from disk: only what are days (`merge`: a key too long, a day
    /// out of range or too many are left out, and the note marked to be written clean).
    fn resume(&mut self, read: BTreeMap<String, i64>, merge: fn(&mut BTreeMap<String, i64>, BTreeMap<String, i64>) -> usize) {
        let count = read.len();
        let mut kept = BTreeMap::new();
        merge(&mut kept, read);
        self.dirty = kept.len() != count;
        self.days = kept;
    }
}

/// The due days read so far, the days done, and the flags' reminders.
#[derive(Default)]
pub(in crate::data_files::pst) struct Due {
    due: Days,
    done: Days,
    reminder: Days,
}

impl Due {
    /// `summary`'s due day, when it's flagged (or its follow-up complete) and has one, the day
    /// it was done, when it's complete and has one, and when it reminds, when it's flagged and
    /// does.
    pub(super) fn note(&mut self, summary: &mailcraft_model::Summary, due: Option<i64>, done: Option<i64>, reminder: Option<i64>) {
        let complete = summary.flags.follow_up_complete();
        if (!summary.flags.flagged && !complete) || (due.is_none() && done.is_none() && reminder.is_none()) {
            return;
        }
        let id = summary.follow_up_id();
        if let Some(day) = due {
            self.due.note(&id, day);
        }
        if let Some(day) = done.filter(|_| complete) {
            self.done.note(&id, day);
        }
        if let Some(at) = reminder.filter(|_| summary.flags.flagged) {
            self.reminder.note(&id, at);
        }
    }

    /// Each note: its days, its document's name, and how what's read of it is checked.
    #[allow(clippy::type_complexity)]
    fn notes(&mut self) -> [(&mut Days, &'static str, fn(&mut BTreeMap<String, i64>, BTreeMap<String, i64>) -> usize); 3] {
        [
            (&mut self.due, FOLLOW_UP_DOC, merge_follow_up),
            (&mut self.done, FOLLOW_UP_DONE_DOC, merge_follow_up),
            (&mut self.reminder, FOLLOW_UP_REMINDER_DOC, merge_reminders),
        ]
    }
}

impl Out<'_> {
    /// The due and done days and reminders to write with a checkpoint, those that changed since
    /// the last one written.
    pub(in crate::data_files::pst) fn due_doc(&self) -> Vec<(&'static str, Vec<u8>)> {
        let d = &self.due;
        d.due.doc(FOLLOW_UP_DOC).into_iter().chain(d.done.doc(FOLLOW_UP_DONE_DOC)).chain(d.reminder.doc(FOLLOW_UP_REMINDER_DOC)).collect()
    }

    /// A checkpoint with the due and done days and reminders was written.
    pub(in crate::data_files::pst) fn due_written(&mut self) {
        for (days, ..) in self.due.notes() {
            days.dirty = false;
        }
    }

    /// Going on from a checkpoint: the due and done days read before it (none when there's no
    /// note of them, or it can't be read; what isn't a day, too long a key or a day out of
    /// range, is left out, and the note written clean with the next checkpoint).
    pub(in crate::data_files::pst) fn resume_due(&mut self) {
        let (store, account) = (self.store, self.account);
        for (days, name, merge) in self.due.notes() {
            match store.load_account_doc::<BTreeMap<String, i64>>(account, name) {
                Ok(read) => days.resume(read, merge),
                Err(e) => log::warn!("import: the follow-up days read so far ({name}) couldn't be read: {e}"),
            }
        }
    }

    /// At the end: the due and done days written, when any came in since the last checkpoint.
    pub(super) fn save_due(&mut self) -> Result<(), String> {
        let (store, account) = (self.store, self.account);
        for (days, name, _) in self.due.notes() {
            if days.dirty {
                store.save_account_doc(account, name, &days.days).map_err(|e| e.to_string())?;
                days.dirty = false;
            }
        }
        Ok(())
    }
}
