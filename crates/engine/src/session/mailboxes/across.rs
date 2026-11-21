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
/// How long one waiting for its download is waited for, in seconds. Past that nothing's coming
/// for it: it may be moved again, and a download that comes for another reason no longer
/// moves it.
const WAIT: i64 = 30 * 60;

/// The error of mail that couldn't be filed in `folder`.
fn not_kept(folder: &str) -> impl Fn(mailcraft_store::StoreError) -> CmdError + '_ {
    move |e| CmdError::Failed(format!("The mail couldn't be kept in {}, so it stays where it was: {e}", mailcraft_model::utf7::decode(folder)))
}

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
    /// When it was asked for (Unix seconds).
    at: i64,
    /// Its Message-ID: put into an Inbox, it didn't arrive there.
    message_id: String,
}

impl Session {
    /// Whether `account`'s folders are kept on this computer: mail is filed in them at once.
    pub fn takes_mail(&self, account: &str) -> bool {
        self.account(account).is_some_and(AccountState::is_local)
    }

    /// Move (`moved`) or copy the selection into `folder` of `account`. Nothing is deleted where
    /// it was before its copy is there.
    pub fn transfer_selection(&mut self, account: &str, folder: &str, moved: bool) -> CmdResult {
        let sel = self.selected_shared();
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
        // Those waiting for a download that nothing came of are forgotten. One handed to the
        // network layer is answered for, however long its upload takes (behind a first sync
        // of the account it goes to, hours): forgotten too, it was sent again, and was there
        // twice.
        let now = self.clock.now;
        self.transfers.retain(|t| t.sent || now.saturating_sub(t.at) < WAIT);
        // Where it's already, a move changes nothing; what waits in the Outbox isn't mail yet;
        // and one already on its way there isn't sent twice.
        let sel: Vec<&Summary> = sel
            .iter()
            .map(|m| &**m)
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
        // Into a folder kept here, each is filed as its source is read (read once, written with
        // others), and the folder's list naming them saved once nothing refuses the rest.
        let store = self.store.clone();
        let (mut filing, mut filed): (Option<Filing>, Vec<MessageKey>) = (None, Vec::new());
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
                at: now,
                message_id: m.message_id.clone(),
            };
            // Exchange's text first is MailCraft's, not the message's: its source is asked for.
            match self.body_raw(&m.key).filter(|raw| !mime::is_text_first(raw)) {
                Some(raw) if here => {
                    if filing.is_none() {
                        let store = store.as_ref().ok_or_else(|| CmdError::Failed("there is nowhere to keep it".into()))?;
                        filing = Some(Filing::new(store, account, folder).map_err(not_kept(folder))?);
                    }
                    if let Some(f) = filing.as_mut() {
                        Self::file_raw(f, raw, t.flags, t.date).map_err(not_kept(folder))?;
                        filed.push(t.key);
                    }
                }
                Some(_) => ready.push(t),
                // Kept on this computer and not here: there's nowhere to ask for it.
                None if self.takes_mail(&m.key.account) => missing += 1,
                None => waiting.push(t),
            }
        }
        if self.transfers.iter().filter(|t| !t.sent).count().saturating_add(waiting.len()) > WAITING_MAX {
            return Err(CmdError::Failed(format!(
                "More than {WAITING_MAX} of these messages aren't downloaded yet, too many to download at once: pick fewer of them."
            )));
        }
        let next = if moved && here { self.successor(&self.selection.clone()) } else { None };
        let (done, sending) = (filed.len(), ready.len());
        if let Some(f) = filing {
            self.file_and_remove(account, folder, f, filed, moved)?;
        }
        self.send_copies(ready);
        for t in &waiting {
            // Asked for already (for this, or by the reading pane): it comes all the same.
            self.jobs.push(Job::FetchBody { key: t.key.clone() });
        }
        let asked = waiting.len();
        self.transfers.extend(waiting);
        let did = if moved { "moved" } else { "copied" };
        // On its way to a server: said, as it stays where it is until its copy is there
        // (issue #85: dropped on another account's folder, nothing seemed to happen).
        if sending > 0 {
            let doing = if moved { "Moving" } else { "Copying" };
            let what = if sending == 1 { "1 message".to_string() } else { format!("{sending} messages") };
            self.notice(format!("{doing} {what} to {}…", mailcraft_model::utf7::decode(folder)), false);
        }
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
        // Put into an Inbox, Junk or Sent by hand, it didn't arrive (or go) there: no rules run on
        // it (one filed it straight out again), nothing announced, as mail moved within an account.
        if self.folder(&account, &folder).is_some_and(|f| matches!(f.role, FolderRole::Inbox | FolderRole::Junk | FolderRole::Sent)) {
            for t in mail.iter().filter(|t| !t.message_id.is_empty()) {
                self.moved_in.insert((account.clone(), folder.clone(), t.message_id.clone()));
            }
        }
        self.jobs.push(Job::FileCopies { account, folder, mail: mail.iter().map(|t| (t.key.clone(), t.flags.clone())).collect(), moved });
        self.transfers.extend(mail.into_iter().map(|t| Transfer { sent: true, ..t }));
    }

    /// Keep what was filed in `filing`, in `folder` of the local `account`: the folder's list
    /// saved naming it, then, `moved`, each of `from` deleted where it was. Filing that fails
    /// deletes nothing (and takes away the sources it wrote).
    fn file_and_remove(&mut self, account: &str, folder: &str, filing: Filing, from: Vec<MessageKey>, moved: bool) -> Result<(), CmdError> {
        // A long write is kept until what was moved is deleted.
        let _long = filing.finish().map_err(not_kept(folder))?;
        self.reload_folder(account, folder);
        self.recount_local(account, Some(&[folder.to_string()]));
        if moved {
            self.remove_moved(from, folder);
        }
        self.refresh_list();
        Ok(())
    }

    /// Delete, where they were, messages whose copies are in `folder` now. Whether all of them
    /// went.
    fn remove_moved(&mut self, keys: Vec<MessageKey>, folder: &str) -> bool {
        let mut all = true;
        for (a, keys) in keys_by_account(keys) {
            // Its copy is there already: one that can't be deleted here stays, in both.
            if let Err(e) = self.apply_op_checked(&a, Op::Expunge { keys }) {
                self.notice(
                    format!("The mail was copied to {}, but couldn't be deleted where it was: {e}", mailcraft_model::utf7::decode(folder)),
                    true,
                );
                all = false;
            }
        }
        all
    }

    /// A message's source came: it goes where it was moved or copied to while it wasn't here.
    pub(in crate::session) fn transfer_arrived(&mut self, key: &MessageKey, raw: &[u8]) {
        if mime::is_text_first(raw) || !self.transfers.iter().any(|t| &t.key == key && !t.sent) {
            return;
        }
        let (now, later): (Vec<Transfer>, Vec<Transfer>) = std::mem::take(&mut self.transfers).into_iter().partition(|t| &t.key == key && !t.sent);
        self.transfers = later;
        let fresh = self.clock.now.saturating_sub(WAIT);
        for t in now.into_iter().filter(|t| t.at > fresh) {
            // Its folder gone since: it stays where it is.
            if self.folder(&t.account, &t.folder).is_none() {
                self.notice(format!("A message couldn't be {}: the folder it was to go to is gone.", if t.moved { "moved" } else { "copied" }), true);
                continue;
            }
            if !self.takes_mail(&t.account) {
                self.send_copies(vec![t]);
                continue;
            }
            let held = self.import_holds(&t.account).and_then(|()| if t.moved { self.import_holds(&t.key.account) } else { Ok(()) });
            let (account, folder, moved) = (t.account.clone(), t.folder.clone(), t.moved);
            let kept = held.and_then(|()| {
                let store = self.store.clone().ok_or_else(|| CmdError::Failed("there is nowhere to keep it".into()))?;
                let mut filing = Filing::new(&store, &account, &folder).map_err(not_kept(&folder))?;
                Self::file_raw(&mut filing, raw.to_vec(), t.flags, t.date).map_err(not_kept(&folder))?;
                self.file_and_remove(&account, &folder, filing, vec![t.key], moved)
            });
            if let Err(e) = kept {
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
        // Looked up by key (each answer was looked for in the whole list: 10,000 took 0.85 s).
        let answered: HashSet<&MessageKey> = done.iter().chain(failed.iter().map(|(k, _)| k)).collect();
        let done_count = done.len();
        self.transfers.retain(|t| !(t.sent && t.account == account && t.folder == folder && answered.contains(&t.key)));
        drop(answered);
        // Copies that couldn't be taken out where they were said so already: not "moved".
        let mut said = false;
        if moved && !done.is_empty() {
            // The message open, if it's one of them, gives way to the next as in any move.
            let next = self.successor(&done);
            let filed: HashSet<&MessageKey> = done.iter().collect();
            let open = self.selection.iter().any(|k| filed.contains(k));
            drop(filed);
            said = !self.remove_moved(done, folder);
            if open && !said {
                self.after_removal(next);
            } else {
                self.refresh_list();
            }
        }
        let arrived = done_count;
        if arrived > 0 && !said {
            let did = if moved { "moved" } else { "copied" };
            let what = if arrived == 1 { "1 message".to_string() } else { format!("{arrived} messages") };
            self.notice(format!("{what} {did} to {}", mailcraft_model::utf7::decode(folder)), false);
        }
        if let Some((_, why)) = failed.first() {
            let did = if moved { "moved" } else { "copied" };
            let n = failed.len();
            let to = mailcraft_model::utf7::decode(folder);
            self.notice(format!("{n} {} couldn't be {did} to {to}: {why}", if n == 1 { "message" } else { "messages" }), true);
        }
    }

    /// An account is gone: nothing waits to leave it, or to go to it.
    pub(crate) fn forget_transfers(&mut self, account: &str) {
        self.transfers.retain(|t| t.key.account != account && t.account != account);
    }
}
