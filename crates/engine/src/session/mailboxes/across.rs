//! Mail moved or copied into a folder of another account, or copied within one (issue #82): a
//! copy of the message's source is filed there with its flags and categories; moved, it's then
//! deleted where it was, and never before its copy is there. A message not downloaded yet is
//! asked for, and goes over when it's here.
//!
//! Into a folder kept on this computer (an Outlook data file opened here) it's filed at once.
//! Into a folder on a server the network layer uploads it ([`Job::FileCopies`]), and says which
//! went ([`Event::CopiesFiled`]).

use super::*;
use mailcraft_model::Flags;

/// The most messages waiting to be downloaded before they're moved or copied.
const WAITING_MAX: usize = 500;

/// A message on its way to where it was moved or copied to.
#[derive(Clone, Debug)]
pub(crate) struct Transfer {
    key: MessageKey,
    account: String,
    folder: String,
    /// Moved: deleted where it was once its copy is there. Else copied.
    moved: bool,
    flags: Flags,
    date: i64,
    /// Handed to the network layer, which hasn't answered yet; else waiting for its source.
    sent: bool,
}

impl Session {
    /// Whether `account`'s folders are kept on this computer: mail is filed in them at once.
    pub fn takes_mail(&self, account: &str) -> bool {
        self.account(account).is_some_and(AccountState::is_local)
    }

    /// Move (`moved`) or copy the selection into `folder` of `account`. Nothing is deleted where
    /// it was before its copy is there.
    pub fn transfer_selection(&mut self, account: &str, folder: &str, moved: bool) -> CmdResult {
        let sel = self.selected();
        if sel.is_empty() {
            return Err(CmdError::Disabled("no message selected".into()));
        }
        if self.account(account).is_none() {
            return Err(CmdError::Params(format!("no account `{account}`")));
        }
        if self.folder(account, folder).is_none() {
            return Err(CmdError::Params(format!("no folder `{folder}`")));
        }
        let here = self.takes_mail(account);
        // Where it's already, a move changes nothing; what waits in the Outbox isn't mail yet;
        // and one already on its way there isn't sent twice.
        let sel: Vec<&Summary> = sel
            .iter()
            .filter(|m| m.key.folder != OUTBOX && !(moved && m.key.account == account && m.key.folder == folder))
            .filter(|m| !self.transfers.iter().any(|t| t.key == m.key && t.account == account && t.folder == folder))
            .collect();
        // A data file's import writes its account's lists (and goes on writing them).
        self.import_holds(account)?;
        if moved {
            let mut from: Vec<&str> = sel.iter().map(|m| m.key.account.as_str()).collect();
            from.sort_unstable();
            from.dedup();
            for a in from {
                self.import_holds(a)?;
            }
        }
        let (mut ready, mut waiting): (Vec<Transfer>, Vec<Transfer>) = (Vec::new(), Vec::new());
        let mut missing = 0usize;
        for m in &sel {
            let t = Transfer {
                key: m.key.clone(),
                account: account.to_string(),
                folder: folder.to_string(),
                moved,
                // As it was marked: read, flagged, answered, its categories. Not deleted.
                flags: Flags { deleted: false, ..m.flags.clone() },
                date: m.date,
                sent: false,
            };
            // Exchange's text first is MailCraft's, not the message's: its source is asked for.
            match self.body_raw(&m.key).is_some_and(|raw| !mime::is_text_first(&raw)) {
                true => ready.push(t),
                // Kept on this computer and not here: there's nowhere to ask for it.
                false if self.takes_mail(&m.key.account) => missing += 1,
                false => waiting.push(t),
            }
        }
        if self.transfers.iter().filter(|t| !t.sent).count().saturating_add(waiting.len()) > WAITING_MAX {
            return Err(CmdError::Failed(format!(
                "More than {WAITING_MAX} of these messages aren't downloaded yet, too many to download at once: pick fewer of them."
            )));
        }
        let next = if moved && here { self.successor(&self.selection.clone()) } else { None };
        let (mut done, mut sending) = (0, 0);
        if here {
            done = ready.len();
            self.file_and_remove(account, folder, ready, moved, None)?;
        } else {
            sending = ready.len();
            self.send_copies(ready);
        }
        for t in &waiting {
            // Asked for already (for this, or by the reading pane): it comes all the same.
            self.jobs.push(Job::FetchBody { key: t.key.clone() });
        }
        let asked = waiting.len();
        self.transfers.extend(waiting);
        let did = if moved { "moved" } else { "copied" };
        if asked > 0 {
            self.notice(format!("{asked} not downloaded yet: {did} as soon as {} here.", if asked == 1 { "it's" } else { "they're" }), false);
        }
        if missing > 0 {
            self.notice(format!("{missing} couldn't be {did}: {} never downloaded.", if missing == 1 { "it was" } else { "they were" }), true);
        }
        if moved && done > 0 {
            self.after_removal(next);
        }
        Ok(serde_json::json!({ did: done, "sending": sending, "waiting": asked, "failed": missing }))
    }

    /// Hand `mail`, all for one folder on a server, to the network layer to be uploaded there.
    fn send_copies(&mut self, mail: Vec<Transfer>) {
        let Some(first) = mail.first() else { return };
        let (account, folder, moved) = (first.account.clone(), first.folder.clone(), first.moved);
        self.jobs.push(Job::FileCopies { account, folder, mail: mail.iter().map(|t| (t.key.clone(), t.flags.clone())).collect(), moved });
        self.transfers.extend(mail.into_iter().map(|t| Transfer { sent: true, ..t }));
    }

    /// File `mail`, whose sources are here (or `given`, one's that just came), in `folder` of the
    /// local `account`, then, `moved`, delete each where it was. Filing that fails deletes
    /// nothing.
    fn file_and_remove(&mut self, account: &str, folder: &str, mail: Vec<Transfer>, moved: bool, given: Option<&[u8]>) -> Result<(), CmdError> {
        if mail.is_empty() {
            return Ok(());
        }
        let store = self.store.clone().ok_or_else(|| CmdError::Failed("there is nowhere to keep it".into()))?;
        let failed =
            |e: mailcraft_store::StoreError| CmdError::Failed(format!("The mail couldn't be kept in {folder}, so it stays where it was: {e}"));
        let mut st = store.load_folder(account, folder).map_err(failed)?;
        if st.validity == 0 {
            st.validity = 1;
        }
        let mut next = st.free_uid(account, folder).map_err(failed)?;
        let mut from: Vec<MessageKey> = Vec::new();
        for t in mail {
            let Some(raw) = given.map(<[u8]>::to_vec).or_else(|| self.body_raw(&t.key)) else { continue };
            let uid = mailcraft_store::FolderState::take_uid(&mut next, account, folder).map_err(failed)?;
            let key = MessageKey { account: account.to_string(), folder: folder.to_string(), validity: st.validity, uid };
            let size = u32::try_from(raw.len()).unwrap_or(u32::MAX);
            let s = mime::summary_from_full(&raw, key.clone(), t.flags, size, t.date);
            store.write_body(&key, &raw).map_err(failed)?;
            st.messages.push(s.into());
            from.push(t.key);
        }
        st.uid_next = next;
        sort_newest_first(&mut st.messages);
        store.save_folder(account, folder, &st).map_err(failed)?;
        self.reload_folder(account, folder);
        self.recount_local(account, Some(&[folder.to_string()]));
        if moved {
            self.remove_moved(from, folder);
        }
        self.refresh_list();
        Ok(())
    }

    /// Delete, where they were, messages whose copies are in `folder` now.
    fn remove_moved(&mut self, keys: Vec<MessageKey>, folder: &str) {
        let mut from: HashMap<String, Vec<MessageKey>> = HashMap::new();
        for k in keys {
            from.entry(k.account.clone()).or_default().push(k);
        }
        let mut accounts: Vec<String> = from.keys().cloned().collect();
        accounts.sort();
        for a in accounts {
            let keys = from.remove(&a).unwrap_or_default();
            // Its copy is there already: one that can't be deleted here stays, in both.
            if let Err(e) = self.apply_op_checked(&a, Op::Expunge { keys }) {
                self.notice(format!("The mail was copied to {folder}, but couldn't be deleted where it was: {e}"), true);
            }
        }
    }

    /// A message's source came: it goes where it was moved or copied to while it wasn't here.
    pub(in crate::session) fn transfer_arrived(&mut self, key: &MessageKey, raw: &[u8]) {
        if mime::is_text_first(raw) || !self.transfers.iter().any(|t| &t.key == key && !t.sent) {
            return;
        }
        let (now, later): (Vec<Transfer>, Vec<Transfer>) = std::mem::take(&mut self.transfers).into_iter().partition(|t| &t.key == key && !t.sent);
        self.transfers = later;
        for t in now {
            if !self.takes_mail(&t.account) {
                self.send_copies(vec![t]);
                continue;
            }
            let held = self.import_holds(&t.account).and_then(|()| if t.moved { self.import_holds(&t.key.account) } else { Ok(()) });
            let (account, folder, moved) = (t.account.clone(), t.folder.clone(), t.moved);
            if let Err(e) = held.and_then(|()| self.file_and_remove(&account, &folder, vec![t], moved, Some(raw))) {
                self.notice(e.to_string(), true);
            }
        }
    }

    /// It couldn't be downloaded: it stays where it is, and that's said.
    pub(in crate::session) fn transfer_failed(&mut self, key: &MessageKey, error: &str) {
        let before = self.transfers.len();
        let moved = self.transfers.iter().any(|t| &t.key == key && !t.sent && t.moved);
        self.transfers.retain(|t| &t.key != key || t.sent);
        if self.transfers.len() < before {
            let did = if moved { "moved" } else { "copied" };
            self.notice(format!("A message couldn't be {did}, as it couldn't be downloaded: {error}"), true);
        }
    }

    /// The network layer's answer ([`Event::CopiesFiled`]): those whose copies are in the folder
    /// are deleted where they were, when they were moved; the others stay, and why is said.
    pub(in crate::session) fn copies_filed(
        &mut self,
        account: &str,
        folder: &str,
        done: Vec<MessageKey>,
        failed: Vec<(MessageKey, String)>,
        moved: bool,
    ) {
        let answered = |k: &MessageKey| done.contains(k) || failed.iter().any(|(f, _)| f == k);
        self.transfers.retain(|t| !(t.sent && t.account == account && t.folder == folder && answered(&t.key)));
        if moved && !done.is_empty() {
            // The message open, if it's one of them, gives way to the next as in any move.
            let next = self.successor(&done);
            let open = self.selection.iter().any(|k| done.contains(k));
            self.remove_moved(done, folder);
            if open {
                self.after_removal(next);
            } else {
                self.refresh_list();
            }
        }
        if let Some((_, why)) = failed.first() {
            let did = if moved { "moved" } else { "copied" };
            let n = failed.len();
            self.notice(format!("{n} {} couldn't be {did} to {folder}: {why}", if n == 1 { "message" } else { "messages" }), true);
        }
    }

    /// An account is gone: nothing waits to leave it, or to go to it.
    pub(crate) fn forget_transfers(&mut self, account: &str) {
        self.transfers.retain(|t| t.key.account != account && t.account != account);
    }
}
