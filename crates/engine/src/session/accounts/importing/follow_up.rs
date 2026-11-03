//! The session: the days an import's flagged messages are due, which the network side leaves in
//! the account's [`FOLLOW_UP_DOC`] as it reads them from the data file, taken into the
//! preferences' own (`Prefs::follow_up`) as the import, or a read of its file again, ends. A day
//! set here stays; the note is emptied once taken, so a day cleared here isn't brought back.

use mailcraft_model::organize::{FOLLOW_UP_DOC, merge_follow_up};

use super::*;

impl Session {
    /// `account`'s imported due days, taken into the preferences.
    pub(super) fn take_imported_follow_up(&mut self, account: &str) {
        let Some(store) = self.store.clone() else { return };
        let days = match store.load_account_doc::<BTreeMap<String, i64>>(account, FOLLOW_UP_DOC) {
            Ok(days) if !days.is_empty() => days,
            Ok(_) => return,
            Err(e) => {
                log::warn!("{account}: the days its flagged messages are due couldn't be read: {e}");
                return;
            }
        };
        merge_follow_up(&mut self.prefs.follow_up, days);
        if let Err(e) = store.save_account_doc(account, FOLLOW_UP_DOC, &BTreeMap::<String, i64>::new()) {
            log::warn!("{account}: {e}");
        }
    }
}
