//! The session: the days an import's flagged messages are due, and its completed ones were done,
//! which the network side leaves in the account's [`FOLLOW_UP_DOC`] and [`FOLLOW_UP_DONE_DOC`]
//! as it reads them from the data file, taken into the preferences' own (`Prefs::follow_up`,
//! `Prefs::follow_up_done`) as the import, or a read of its file again, ends. A day set here
//! stays, and one cleared here isn't brought back.
//!
//! The preferences reach the disk only when the app next saves them, so the notes stay until
//! they have: taking them notes their fingerprint in the preferences (`Prefs::follow_up_taken`),
//! and at the next start notes whose fingerprint the saved preferences hold are emptied, while
//! ones they don't (MailCraft ended before they were saved) are taken again.

use mailcraft_model::organize::{FOLLOW_UP_DOC, FOLLOW_UP_DONE_DOC, FollowUpDays, merge_follow_up};

use super::*;

/// The notes' fingerprint (FNV-1a over their JSON, the same from one build to the next): the due
/// note's, and then the done note's when it has any (so a due note alone keeps the fingerprint
/// it had before there were done notes).
fn fingerprint(days: &FollowUpDays) -> u64 {
    let mut bytes = serde_json::to_vec(&days.due).unwrap_or_default();
    if !days.done.is_empty() {
        bytes.extend(serde_json::to_vec(&days.done).unwrap_or_default());
    }
    bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0000_0100_0000_01b3))
}

/// The days of `days` not in `taken` with the same day.
fn not_taken(days: &BTreeMap<String, i64>, taken: Option<&BTreeMap<String, i64>>) -> BTreeMap<String, i64> {
    days.iter().filter(|(id, day)| taken.and_then(|t| t.get(*id)) != Some(*day)).map(|(id, day)| (id.clone(), *day)).collect()
}

impl Session {
    /// `account`'s notes of imported due and done days, when they have any.
    fn imported_follow_up(&self, account: &str) -> Option<FollowUpDays> {
        let store = self.store.as_ref()?;
        let read = |doc: &str| match store.load_account_doc::<BTreeMap<String, i64>>(account, doc) {
            Ok(days) => days,
            Err(e) => {
                log::warn!("{account}: the days of its messages' follow-ups ({doc}) couldn't be read: {e}");
                BTreeMap::new()
            }
        };
        let days = FollowUpDays { due: read(FOLLOW_UP_DOC), done: read(FOLLOW_UP_DONE_DOC) };
        (!days.due.is_empty() || !days.done.is_empty()).then_some(days)
    }

    /// `account`'s imported due and done days, taken into the preferences: those not taken
    /// already since MailCraft started.
    pub(super) fn take_imported_follow_up(&mut self, account: &str) {
        let Some(days) = self.imported_follow_up(account) else { return };
        self.take_follow_up_days(account, days);
    }

    fn take_follow_up_days(&mut self, account: &str, days: FollowUpDays) {
        let taken = self.importing.follow_up_taken.get(account);
        let due = not_taken(&days.due, taken.map(|t| &t.due));
        let done = not_taken(&days.done, taken.map(|t| &t.done));
        merge_follow_up(&mut self.prefs.follow_up, due);
        merge_follow_up(&mut self.prefs.follow_up_done, done);
        self.prefs.follow_up_taken.insert(account.to_string(), fingerprint(&days));
        self.importing.follow_up_taken.insert(account.to_string(), days);
    }

    /// Once, at the first tick (the saved preferences restored by then): each account's notes of
    /// imported due and done days left from before. Notes the saved preferences say they hold
    /// are emptied; ones they don't are taken again (a day set or kept here stays).
    pub(crate) fn imported_follow_up_at_start(&mut self) {
        if self.importing.follow_up_looked {
            return;
        }
        self.importing.follow_up_looked = true;
        let accounts: Vec<String> = self.accounts.iter().map(|a| a.config.id.clone()).collect();
        self.prefs.follow_up_taken.retain(|a, _| accounts.contains(a));
        let Some(store) = self.store.clone() else { return };
        for account in accounts {
            let Some(days) = self.imported_follow_up(&account) else {
                self.prefs.follow_up_taken.remove(&account);
                continue;
            };
            if self.prefs.follow_up_taken.get(&account) != Some(&fingerprint(&days)) {
                self.take_follow_up_days(&account, days);
            } else if self.importing.running.contains_key(&account) {
                // Its import may still write the notes: they stay, as taken.
                self.importing.follow_up_taken.insert(account, days);
            } else {
                let emptied = [(FOLLOW_UP_DOC, &days.due), (FOLLOW_UP_DONE_DOC, &days.done)]
                    .into_iter()
                    .filter(|(_, d)| !d.is_empty())
                    .map(|(doc, _)| store.save_account_doc(&account, doc, &BTreeMap::<String, i64>::new()))
                    .collect::<Result<Vec<()>, _>>();
                match emptied {
                    Ok(_) => {
                        self.prefs.follow_up_taken.remove(&account);
                    }
                    Err(e) => log::warn!("{account}: {e}"),
                }
            }
        }
    }
}
