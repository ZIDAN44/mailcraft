//! The session: the days an import's flagged messages are due, which the network side leaves in
//! the account's [`FOLLOW_UP_DOC`] as it reads them from the data file, taken into the
//! preferences' own (`Prefs::follow_up`) as the import, or a read of its file again, ends. A day
//! set here stays, and one cleared here isn't brought back.
//!
//! The preferences reach the disk only when the app next saves them, so the note stays until
//! they have: taking it notes its fingerprint in the preferences (`Prefs::follow_up_taken`), and
//! at the next start a note whose fingerprint the saved preferences hold is emptied, while one
//! they don't (MailCraft ended before they were saved) is taken again.

use mailcraft_model::organize::{FOLLOW_UP_DOC, merge_follow_up};

use super::*;

/// A note's fingerprint (FNV-1a over its JSON, the same from one build to the next).
fn fingerprint(days: &BTreeMap<String, i64>) -> u64 {
    let bytes = serde_json::to_vec(days).unwrap_or_default();
    bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0000_0100_0000_01b3))
}

impl Session {
    /// `account`'s note of imported due days, when it has any.
    fn imported_follow_up(&self, account: &str) -> Option<BTreeMap<String, i64>> {
        let store = self.store.as_ref()?;
        match store.load_account_doc::<BTreeMap<String, i64>>(account, FOLLOW_UP_DOC) {
            Ok(days) if !days.is_empty() => Some(days),
            Ok(_) => None,
            Err(e) => {
                log::warn!("{account}: the days its flagged messages are due couldn't be read: {e}");
                None
            }
        }
    }

    /// `account`'s imported due days, taken into the preferences: those not taken already since
    /// MailCraft started.
    pub(super) fn take_imported_follow_up(&mut self, account: &str) {
        let Some(days) = self.imported_follow_up(account) else { return };
        self.take_follow_up_days(account, days);
    }

    fn take_follow_up_days(&mut self, account: &str, days: BTreeMap<String, i64>) {
        let taken = self.importing.follow_up_taken.get(account);
        let new: BTreeMap<String, i64> =
            days.iter().filter(|(id, day)| taken.and_then(|t| t.get(*id)) != Some(*day)).map(|(id, day)| (id.clone(), *day)).collect();
        merge_follow_up(&mut self.prefs.follow_up, new);
        self.prefs.follow_up_taken.insert(account.to_string(), fingerprint(&days));
        self.importing.follow_up_taken.insert(account.to_string(), days);
    }

    /// Once, at the first tick (the saved preferences restored by then): each account's note of
    /// imported due days left from before. One the saved preferences say they hold is emptied;
    /// one they don't is taken again (a day set or kept here stays).
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
                // Its import may still write the note: it stays, as taken.
                self.importing.follow_up_taken.insert(account, days);
            } else {
                match store.save_account_doc(&account, FOLLOW_UP_DOC, &BTreeMap::<String, i64>::new()) {
                    Ok(()) => {
                        self.prefs.follow_up_taken.remove(&account);
                    }
                    Err(e) => log::warn!("{account}: {e}"),
                }
            }
        }
    }
}
