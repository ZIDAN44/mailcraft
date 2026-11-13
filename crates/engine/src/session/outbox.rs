//! The session: the Outbox.

use super::*;

impl Session {
    /// Re-read the Outbox from disk.
    pub fn refresh_outbox(&mut self) {
        let Some(store) = self.store.clone() else { return };
        let before: Vec<(String, u64)> = self.outbox.iter().map(|(a, i)| (a.clone(), i.id)).collect();
        self.outbox = self.accounts.iter().flat_map(|a| store.outbox(&a.config.id).into_iter().map(|i| (a.config.id.clone(), i))).collect();
        // A row's key is another message's now: what was decrypted under it isn't that one's
        // (it was shown as it, and opened to be changed as it). Wherever the Outbox is read.
        if !before.iter().map(|(a, i)| (a.as_str(), *i)).eq(self.outbox.iter().map(|(a, i)| (a.as_str(), i.id))) {
            self.forget_opened_in(OUTBOX);
        }
    }

    pub(super) fn after_outbox_change(&mut self) {
        let before: Vec<(String, u64)> = self.outbox.iter().map(|(a, i)| (a.clone(), i.id)).collect();
        self.refresh_outbox();
        if self.view == View::Outbox {
            self.follow_outbox_rows(&before);
            self.refresh_list();
            // The one being read is asked for again under the row it has now.
            if let Some(key) = self.reading.as_ref().map(|r| r.key.clone()).filter(|k| k.folder == OUTBOX) {
                self.show_again(&key);
            }
        }
    }

    /// An Outbox key counts rows, so a message's changes when one above it is sent or deleted.
    /// What's selected and what's open follow their messages to their new rows: left on the row
    /// number, they were another message, and Delete then took that one, never sent.
    fn follow_outbox_rows(&mut self, before: &[(String, u64)]) {
        let now = &self.outbox;
        let moved = |k: &MessageKey| -> Option<MessageKey> {
            if k.folder != OUTBOX {
                return Some(k.clone());
            }
            let (account, id) = before.get(usize::try_from(k.uid).ok()?.checked_sub(1)?)?;
            let at = now.iter().position(|(a, i)| a == account && i.id == *id)?;
            Some(MessageKey { uid: u32::try_from(at + 1).ok()?, ..k.clone() })
        };
        let selection: Vec<MessageKey> = self.selection.iter().filter_map(&moved).collect();
        let focus = self.focus.as_ref().and_then(moved);
        let anchor = self.anchor.as_ref().and_then(moved);
        let reading = self.reading.as_ref().map(|r| moved(&r.key));
        (self.selection, self.focus, self.anchor) = (selection, focus, anchor);
        match (reading, &mut self.reading) {
            (Some(Some(key)), Some(r)) => r.key = key,
            (Some(None), _) => self.reading = None,
            _ => {}
        }
    }

    /// Queued messages, as list rows. Their key is `(account, OUTBOX, 0, n)`, `n` counting from 1
    /// in Outbox order.
    pub(super) fn outbox_summaries(&self) -> Vec<Summary> {
        let Some(store) = &self.store else { return Vec::new() };
        let mut out = Vec::new();
        for (n, (account, item)) in self.outbox.iter().enumerate() {
            let key = MessageKey { account: account.clone(), folder: OUTBOX.to_string(), validity: 0, uid: u32::try_from(n + 1).unwrap_or(u32::MAX) };
            let raw = store.outbox_raw(account, item.id).unwrap_or_default();
            let size = u32::try_from(raw.len()).unwrap_or(u32::MAX);
            let flags = mailcraft_model::Flags { seen: true, ..Default::default() };
            let mut m = mime::summary_from_full(&raw, key, flags, size, item.queued);
            m.preview = match &item.error {
                Some(e) if e != "offline" => format!("Not sent yet: {e}"),
                _ if item.not_before > self.clock.now => format!("Will be sent {}", mailcraft_model::dates::full_time(item.not_before, &self.clock)),
                _ => "Waiting to be sent".to_string(),
            };
            out.push(m);
        }
        out
    }

    /// The Outbox entry behind an Outbox key.
    pub fn outbox_entry(&self, key: &MessageKey) -> Option<(String, OutboxItem)> {
        if key.folder != OUTBOX {
            return None;
        }
        let n = usize::try_from(key.uid).ok()?.checked_sub(1)?;
        self.outbox.get(n).filter(|(a, _)| *a == key.account).cloned()
    }

    /// Send one held-back Outbox message now (Delay Delivery off), as its Send now button.
    pub fn send_outbox_now(&mut self, key: &MessageKey) -> CmdResult {
        let (account, mut item) = self.outbox_entry(key).ok_or_else(|| CmdError::Params("not an Outbox message".into()))?;
        self.can_send(&account)?;
        if item.not_before != 0 {
            item.not_before = 0;
            if let Some(store) = &self.store {
                store.outbox_update(&account, &item).map_err(|e| CmdError::Failed(e.to_string()))?;
            }
            self.after_outbox_change();
        }
        self.jobs.push(Job::FlushOutbox { account });
        Ok(serde_json::json!({ "sending": item.id }))
    }

    /// Ask the accounts holding Outbox mail whose time has come to send it. Called with
    /// [`Session::tick`]; each message is asked for once.
    pub(crate) fn flush_due_outbox(&mut self) {
        let now = self.clock.now;
        let due: Vec<(String, u64)> = self
            .outbox
            .iter()
            .filter(|(a, i)| i.not_before != 0 && i.not_before <= now && !self.flushed.contains(&i.id) && self.send_block(a).is_none())
            .map(|(a, i)| (a.clone(), i.id))
            .collect();
        let mut accounts: Vec<String> = Vec::new();
        for (a, id) in due {
            self.flushed.insert(id);
            if !accounts.contains(&a) {
                accounts.push(a);
            }
        }
        for account in accounts {
            self.jobs.push(Job::FlushOutbox { account });
        }
    }

    /// Try to send everything in the Outbox now (what's held for an account that can't send
    /// stays: [`SendBlock`]).
    pub fn send_outbox(&mut self) -> CmdResult {
        let mut accounts: Vec<String> = self.outbox.iter().map(|(a, _)| a.clone()).filter(|a| self.send_block(a).is_none()).collect();
        accounts.dedup();
        if accounts.is_empty()
            && let Some(b) = self.outbox.first().and_then(|(a, _)| self.send_block(a))
        {
            return Err(CmdError::Disabled(b.reason().into()));
        }
        for a in &accounts {
            self.jobs.push(Job::FlushOutbox { account: a.clone() });
        }
        Ok(serde_json::json!({ "accounts": accounts.len(), "messages": self.outbox.len() }))
    }

    /// Delete the selected Outbox messages (they are never sent).
    pub fn delete_outbox_selection(&mut self) -> CmdResult {
        let gone: Vec<(String, OutboxItem)> = self.selection.iter().filter_map(|k| self.outbox_entry(k)).collect();
        if let Some(store) = &self.store {
            for (a, i) in &gone {
                store.outbox_remove(a, i.id);
            }
        }
        // Removed from the Outbox undelivered: whatever flags they owed are dropped.
        let before = self.pending_marks.len();
        self.pending_marks.retain(|id, _| !gone.iter().any(|(_, i)| i.id == *id));
        if self.pending_marks.len() != before {
            self.save_pending_marks();
        }
        self.reading = None;
        self.selection.clear();
        self.focus = None;
        self.after_outbox_change();
        Ok(serde_json::json!({ "deleted": gone.len() }))
    }

    /// Take a message back out of the Outbox to change it: it opens for editing.
    pub fn edit_outbox(&mut self, key: &MessageKey) -> CmdResult {
        let (account, item) = self.outbox_entry(key).ok_or_else(|| CmdError::Params("not an Outbox message".into()))?;
        let store = self.store.clone().ok_or_else(|| CmdError::Failed("no mail cache".into()))?;
        let stored = store.outbox_raw(&account, item.id).ok_or_else(|| CmdError::Failed("the message is gone".into()))?;
        // What waits there is what goes: signed or encrypted already. It's changed as it was
        // written, so its signature comes off; one encrypted opens once it's been decrypted
        // (it's asked for as the message is shown). Opened as it stood, the text was the
        // encryption's file, and the message that waited was gone.
        let (raw, signed, encrypted) = self.as_written(key, stored);
        if crate::session::security::is_sealed(&raw) {
            return Err(CmdError::Disabled("the message is encrypted: it opens to be changed once it has been decrypted".into()));
        }
        let p = mime::parse(&raw).ok_or_else(|| CmdError::Failed("the message can't be read".into()))?;
        // As it went in: formatted, its pictures in place, a reply's original as it was.
        let mut c = self.compose_from_saved(&account, key, &raw)?;
        // Bcc isn't written into a message: it's the envelope recipients not in To or Cc.
        let shown: Vec<String> = p.to.iter().chain(&p.cc).map(|a| a.email.to_ascii_lowercase()).collect();
        let bcc: Vec<String> = item.recipients.iter().filter(|r| !shown.contains(&r.to_ascii_lowercase())).cloned().collect();
        c.bcc = if bcc.is_empty() { String::new() } else { format!("{}; ", bcc.join("; ")) };
        c.show_cc = !bcc.is_empty();
        // What it answers comes with what the Outbox owed it, below.
        (c.source, c.mode) = (None, None);
        // Not a copy in Drafts to replace.
        c.draft_id.clear();
        // Signed and encrypted again as it goes; held back until the time it was to go.
        c.sign = signed;
        c.encrypt = encrypted;
        c.send_after = Some(item.not_before).filter(|at| *at > self.clock.now);
        let id = c.id;
        self.open_compose(c);
        // Pulled back for editing: the owed flag moves onto the compose, for its delivery.
        if let Some((src, flag)) = self.pending_marks.remove(&item.id) {
            self.save_pending_marks();
            if let Some(c) = self.compose.as_mut() {
                c.source = Some(src);
                c.mode = Some(if flag == Flag::Forwarded { ReplyMode::Forward } else { ReplyMode::Reply });
            }
        }
        store.outbox_remove(&account, item.id);
        self.reading = None;
        self.selection.clear();
        self.after_outbox_change();
        Ok(serde_json::json!({ "compose": id }))
    }
}
