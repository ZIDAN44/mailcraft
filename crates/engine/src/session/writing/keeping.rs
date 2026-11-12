//! The session: messages being written kept as drafts, in Drafts and on this computer, and
//! those held while a server's save is on its way (#53).

use super::*;

/// Messages held at most while their drafts are on the way to the server ([`Session::saving`]).
const MAX_SAVING: usize = 64;

impl Session {
    /// Another message is shown in the reading pane: the message being written there (inline)
    /// makes way for it, as Outlook's does. Kept in Drafts when something was written in it,
    /// gone when nothing was. One being sent or in a window of its own stays; one that can't be
    /// kept in Drafts is set aside in a window instead, never lost.
    pub(crate) fn make_way_for_reading(&mut self) {
        let Some(written) = self.compose.as_ref().filter(|c| !c.window && !c.sending).map(|c| self.written(c)) else { return };
        if !written {
            self.compose = None;
            return;
        }
        if let Err(e) = self.save_draft() {
            if let Some(mut c) = self.compose.take() {
                c.window = true;
                self.composes.push(c);
            }
            self.notice(e.to_string(), true);
            return;
        }
        self.let_go_saved();
    }

    /// The message being written goes, its draft just saved. A server account's save has only
    /// begun: the message is held, out of sight, until the server or this computer keeps the
    /// draft, so it can come back if neither can (#53).
    pub(crate) fn let_go_saved(&mut self) {
        let Some(c) = self.compose.take() else { return };
        if self.account(&c.account).is_some_and(|a| !a.is_local()) {
            if self.saving.len() >= MAX_SAVING {
                // A front end that never answers: the oldest is kept on this computer instead.
                let gone = self.saving.remove(0);
                log::warn!("{}: a draft's save went unanswered; it's kept on this computer", gone.account);
                self.keep_held_one(&gone);
            }
            self.saving.push(c);
        }
    }

    /// A draft's save is done ([`Event::DraftSaved`]): the message held for it goes, or, when
    /// neither the server nor this computer could keep the draft, comes back in a window of its
    /// own to go on with, saying why (#53). With another save of it still on the way, it waits
    /// for that one's end: the message is still that draft (its Message-ID), and the last save
    /// says how it went (found in review: the first said done, the next save, or the draft kept
    /// on close, was made a new draft, and Drafts had two).
    pub(crate) fn draft_saved(&mut self, account: &str, message_id: &str, error: Option<String>) {
        let id = mailcraft_model::thread::norm_id(message_id);
        let same = |c: &Compose| c.account == account && mailcraft_model::thread::norm_id(&c.draft_id) == id;
        // No Message-ID read from it: the account's oldest, as its drafts are saved in turn.
        let at = if id.is_empty() { self.saving.iter().position(|c| c.account == account) } else { self.saving.iter().position(same) };
        // Held until its last save on the way is said done.
        let held_last = at.and_then(|i| self.saving.get_mut(i)).is_some_and(|c| {
            c.saves = c.saves.saturating_sub(1);
            c.saves == 0
        });
        let held = at.filter(|_| held_last).map(|i| self.saving.remove(i));
        // The message still open whose save this was: on its way no more, once it was the last.
        let mut was_on_its_way = false;
        for c in self.compose.iter_mut().chain(self.composes.iter_mut()).filter(|c| same(c)) {
            was_on_its_way |= c.saves > 0 || c.on_its_way.is_some();
            c.saves = c.saves.saturating_sub(1);
            let last = c.saves == 0;
            if last {
                c.on_its_way = None;
            }
            match &error {
                // Saved as it was then: edited back to that, it's what Drafts has (asked nothing).
                None if last => {
                    if let Some(saved) = c.saved_as.take() {
                        c.made = Some(saved);
                    }
                }
                // An earlier version saved: Drafts has that one until the last gets there.
                None => {}
                // Not saved after all: the message open is something written again, asked about
                // (as the last save would have it, if that one gets there).
                Some(_) if last => (c.made, c.saved_as) = (None, None),
                Some(_) => c.made = None,
            }
        }
        if at.is_some() && held.is_none() {
            return;
        }
        let Some(e) = error else {
            // Said once it's there, not before (#53): closed with its changes kept, or saved with
            // Save and still open.
            if held.is_some() {
                self.notice("Saved in Drafts", false);
            } else if was_on_its_way {
                self.notice("Draft saved", false);
            }
            return;
        };
        match held {
            Some(mut c) => {
                c.made = None;
                c.window = true;
                self.composes.push(c);
                self.notice(format!("The draft couldn't be kept: {e}. The message is open again, so nothing is lost."), true);
            }
            None => self.notice(format!("The draft couldn't be kept: {e}"), true),
        }
    }

    /// Save the message being written to Drafts.
    pub fn save_draft(&mut self) -> CmdResult {
        let c = self.compose.clone().ok_or_else(|| CmdError::Disabled("nothing to save".into()))?;
        let (raw, folder, mid) = self.draft_bytes(&c)?;
        if self.account(&c.account).is_none_or(AccountState::is_local) {
            // The new copy first: the earlier one goes only once this one is kept.
            let key = self.file_locally(&c.account, &folder, &raw, false)?;
            if let Some(old) = self.saved_copy(&c) {
                self.apply_op(&c.account.clone(), Op::Expunge { keys: vec![old] });
            }
            if let Some(cm) = self.compose.as_mut() {
                cm.draft = Some(key);
                cm.draft_id = mid;
            }
        } else {
            let replaces = self.saved_copy(&c);
            self.jobs.push(self.draft_job(&c, raw.clone(), folder.clone(), replaces.clone(), false));
            if let Some(cm) = self.compose.as_mut() {
                // The server gives the new copy its place; the old one is replaced by the
                // worker, and the new one found by its Message-ID when it's saved again.
                cm.draft = None;
                cm.draft_id = mid;
                cm.on_its_way = Some((raw, folder, replaces));
                cm.saves = cm.saves.saturating_add(1);
                // As saved, once the server has it: edited back to that, it's what Drafts has.
                cm.saved_as = Some(cm.as_now());
            }
            // Said saved once the server has it ([`Session::draft_saved`]).
            return Ok(serde_json::json!({ "saved": c.id }));
        }
        // As saved: edited back to that, it's what Drafts has (asked nothing).
        if let Some(cm) = self.compose.as_mut() {
            cm.mark_made();
        }
        self.notice("Draft saved", false);
        Ok(serde_json::json!({ "saved": c.id }))
    }

    /// Every message being written with something in it, kept as a draft as MailCraft closes:
    /// on this computer at once, and in Drafts on the server at the next connection. How many.
    /// Those whose drafts are still on the way to the server too ([`Session::keep_held`]).
    pub fn keep_drafts(&mut self) -> usize {
        let written: Vec<Compose> = self.all_composes().filter(|c| self.written(c) && !c.sending).cloned().collect();
        let kept = written.iter().filter(|c| self.keep_draft(c)).count();
        kept + self.keep_held()
    }

    /// The messages held while their drafts are on the way to the server ([`Session::saving`]),
    /// kept as drafts as MailCraft closes, however it closes (they were chosen to be kept): the
    /// save may never finish. How many.
    pub fn keep_held(&mut self) -> usize {
        let held = std::mem::take(&mut self.saving);
        held.iter().filter(|c| self.keep_held_one(c)).count()
    }

    /// Keep a message held for its save as the very draft that went (its source, folder and the
    /// copy it replaces), so it and the save on the way are one draft, not two (#53).
    fn keep_held_one(&mut self, c: &Compose) -> bool {
        match c.on_its_way.clone() {
            Some((raw, folder, replaces)) => {
                let job = self.draft_job(c, raw, folder, replaces, true);
                self.jobs.push(job);
                true
            }
            None => self.keep_draft(c),
        }
    }

    /// Keep `c` as a draft: on this computer at once, and in Drafts on the server at the next
    /// connection. Whether it was.
    fn keep_draft(&mut self, c: &Compose) -> bool {
        let Ok((raw, folder, _)) = self.draft_bytes(c) else { return false };
        if self.account(&c.account).is_none_or(AccountState::is_local) {
            if let Err(e) = self.file_locally(&c.account, &folder, &raw, false) {
                self.notice(e.to_string(), true);
                return false;
            }
            if let Some(old) = self.saved_copy(c) {
                self.apply_op(&c.account.clone(), Op::Expunge { keys: vec![old] });
            }
        } else {
            // Its save on its way replaces a copy: so does this, kept in its place.
            let replaces = c.on_its_way.as_ref().and_then(|(_, _, r)| r.clone()).or_else(|| self.saved_copy(c));
            let job = self.draft_job(c, raw, folder, replaces, true);
            self.jobs.push(job);
        }
        true
    }

    /// The job filing `c`'s draft `raw` in `folder` on the server (`keep`: kept on this computer
    /// first, as MailCraft closes): encrypted to the account's own certificate first when the
    /// message is to be encrypted.
    fn draft_job(&self, c: &Compose, raw: Vec<u8>, folder: String, replaces: Option<MessageKey>, keep: bool) -> Job {
        let config = self.account(&c.account).map(|a| a.config.clone());
        match config {
            Some(account) if self.seals_with_pgp(c) => {
                Job::Pgp { account, work: mailcraft_model::PgpWork::SealDraft { raw: mailcraft_model::Private(raw), folder, replaces, keep } }
            }
            Some(account) if c.encrypt && self.identity(&c.account).is_some() => {
                Job::SealDraft { account, raw: mailcraft_model::Private(raw), folder, replaces, keep }
            }
            _ if keep => Job::KeepDraft { account: c.account.clone(), raw, folder, replaces },
            _ => Job::SaveDraft { account: c.account.clone(), raw, folder, replaces },
        }
    }

    /// A message as a draft's source, the Drafts folder it goes in, and its Message-ID.
    pub(crate) fn draft_bytes(&mut self, c: &Compose) -> Result<(Vec<u8>, String, String), CmdError> {
        // Drafts may hold unfinished addresses: keep only the valid ones.
        let lenient = addr::parse_valid;
        let (body, html, inline) = c.bodies_with(&c.signature_code);
        let d = mime::Draft {
            reply_to: lenient(&c.reply_to),
            to: lenient(&c.to),
            cc: lenient(&c.cc),
            bcc: lenient(&c.bcc),
            subject: c.subject.clone(),
            body,
            in_reply_to: c.in_reply_to.clone(),
            references: c.references.clone(),
            attachments: c.attachments.clone(),
            html,
            inline,
            importance: c.importance,
            read_receipt: c.read_receipt,
            seven_bit: false,
            keep_bcc: true,
            source: match (&c.source, c.mode) {
                (Some(k), Some(ReplyMode::Forward)) => format!("forward {}", k.to_id()),
                (Some(k), Some(_)) => format!("reply {}", k.to_id()),
                _ => String::new(),
            },
        };
        // A server account without one gets Drafts made where it's filed, as Outlook makes it.
        let account = self.account(&c.account);
        let folder = match account.and_then(|a| a.folder(FolderRole::Drafts)) {
            Some(f) => f.path.clone(),
            None if account.is_some_and(|a| !a.is_local()) => "Drafts".to_string(),
            None => return Err(CmdError::Failed("this account has no Drafts folder".into())),
        };
        // While its save is on its way, kept as the same draft (its Message-ID), so the server ends
        // up with one copy of it, the newer (#53).
        let (raw, _, mid) = match c.on_its_way.as_ref().map(|_| c.draft_id.clone()).filter(|id| !id.is_empty()) {
            Some(id) => self.compose_bytes_as(c, &d, id)?,
            None => self.compose_bytes_with_id(c, &d)?,
        };
        Ok((raw, folder, mid))
    }

    /// File a message in a local account's folder; returns its key. A folder that can't be read,
    /// or a source that can't be kept, changes nothing: the folder was saved as if empty (its
    /// mail gone), or listed a message that had no source.
    pub(crate) fn file_locally(&mut self, account: &str, folder: &str, raw: &[u8], seen: bool) -> Result<MessageKey, CmdError> {
        let store = self.store.clone().ok_or_else(|| CmdError::Failed("there is nowhere to keep it".into()))?;
        let failed = |e: mailcraft_store::StoreError| CmdError::Failed(format!("The message couldn't be kept in {folder}: {e}"));
        let mut st = store.load_folder(account, folder).map_err(failed)?;
        if st.validity == 0 {
            st.validity = 1;
        }
        let mut next = st.free_uid(account, folder).map_err(failed)?;
        let uid = mailcraft_store::FolderState::take_uid(&mut next, account, folder).map_err(failed)?;
        st.uid_next = next;
        let key = MessageKey { account: account.to_string(), folder: folder.to_string(), validity: st.validity, uid };
        let flags = mailcraft_model::Flags { seen, draft: !seen, ..Default::default() };
        let size = u32::try_from(raw.len()).unwrap_or(u32::MAX);
        let s = mime::summary_from_full(raw, key.clone(), flags, size, self.clock.now);
        store.write_body(&key, raw).map_err(failed)?;
        st.messages.push(s.into());
        sort_newest_first(&mut st.messages);
        store.save_folder(account, folder, &st).map_err(failed)?;
        self.reload_folder(account, folder);
        self.recount_local(account, Some(&[folder.to_string()]));
        self.refresh_list();
        Ok(key)
    }
}
