//! The session: messages being written, drafts and sending.

use mailcraft_model::lookalike;

use super::*;

/// From this many people (counted as [`SendCheck::Exposed`] says), sending to all of them in To
/// and Cc asks first.
const EXPOSED_FROM: usize = 8;

/// Something to look at before a message goes (`compose.send` holds it and says what).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SendCheck {
    /// A recipient's domain looks like `like`, one the user deals with, but isn't it: a typo,
    /// or a reply going to someone passing for a colleague.
    Lookalike { address: String, like: String },
    /// Everyone in To and Cc sees everyone else's address: `count` people who may not know each
    /// other (each organisation outside the sender's own counted once, each address at a big mail
    /// service on its own).
    Exposed { count: usize },
}

impl Session {
    pub(super) fn me(&self, account: &str) -> Vec<String> {
        let mut v: Vec<String> = self.accounts.iter().map(|a| a.config.email.clone()).collect();
        if let Some(a) = self.account(account) {
            v.push(a.config.username.clone());
        }
        v
    }

    pub(super) fn signature(&self, account: &str) -> String {
        self.signature_rich(account).0
    }

    /// An account's signature with its look: its text and styles (plain when it has none).
    pub fn signature_rich(&self, account: &str) -> (String, mime::rich::Styles) {
        let Some(a) = self.account(account) else { return (String::new(), mime::rich::Styles::default()) };
        if !a.config.signature_html.trim().is_empty() {
            let (text, styles) = mime::pasted_html(&a.config.signature_html);
            if !text.trim().is_empty() {
                return (text, styles);
            }
        }
        (a.config.signature.clone(), mime::rich::Styles::plain(&a.config.signature))
    }

    /// The signature in the message being made, given its look (the text went in plain).
    pub(super) fn style_signature(&mut self) {
        let Some(account) = self.compose.as_ref().map(|c| c.account.clone()) else { return };
        let sig = self.signature_rich(&account);
        if let Some(c) = self.compose.as_mut() {
            style_signature(c, &sig);
        }
    }

    /// A new message from a `mailto:` link (MailCraft as the system's mail app): its addresses,
    /// subject and first words, above the signature. Nothing is attached and nothing is sent.
    pub fn mailto_compose(&mut self, m: &mailcraft_model::mailto::Mailto) -> CmdResult {
        let c = self.new_compose(None)?;
        let cap = |x: &str| x.chars().take(1 << 20).collect::<String>();
        c.to = cap(&m.to);
        c.cc = cap(&m.cc);
        c.bcc = cap(&m.bcc);
        c.show_cc |= !c.bcc.is_empty();
        c.subject = cap(&m.subject);
        if !m.body.is_empty() {
            let text = format!("{}{}", cap(&m.body), c.body);
            c.set_text(text);
        }
        let id = c.id;
        self.style_signature();
        Ok(serde_json::json!({"compose": id}))
    }

    /// Start writing `c`: the message being written, if any, is set aside in a window of its
    /// own ([`Session::composes`]), never lost; one that's empty (nothing typed) goes.
    pub fn open_compose(&mut self, c: Compose) {
        if let Some(mut old) = self.compose.take()
            && (self.written(&old) || old.sending)
        {
            old.window = true;
            self.composes.push(old);
        }
        self.compose = Some(c);
    }

    /// Act on message `id` (one set aside comes back; the current one is set aside in turn).
    /// False when there's no such message.
    pub fn activate_compose(&mut self, id: u64) -> bool {
        if self.compose.as_ref().is_some_and(|c| c.id == id) {
            return true;
        }
        let Some(at) = self.composes.iter().position(|c| c.id == id) else { return false };
        let c = self.composes.remove(at);
        if let Some(old) = self.compose.replace(c) {
            self.composes.insert(at, old);
        }
        true
    }

    /// Every message being written: the current one, then those set aside.
    pub fn all_composes(&self) -> impl Iterator<Item = &Compose> {
        self.compose.iter().chain(self.composes.iter())
    }

    pub fn new_compose(&mut self, account: Option<&str>) -> Result<&mut Compose, CmdError> {
        let account = match account {
            Some(a) => self.account(a).map(|a| a.config.id.clone()),
            None => self.current_account().map(|a| a.config.id.clone()),
        }
        .ok_or_else(|| CmdError::Disabled("add an account first".into()))?;
        let sig = self.signature(&account);
        let id = self.fresh_id();
        let body = if sig.trim().is_empty() { String::new() } else { format!("\n\n{}", signature_block(&sig)) };
        let window = self.prefs.new_mail_window;
        let signature = if sig.trim().is_empty() { String::new() } else { signature_block(&sig) };
        let styles = mime::rich::Styles::plain(&body);
        self.open_compose(Compose { id, account, body, styles, window, signature, ..Default::default() });
        self.style_signature();
        self.compose.as_mut().ok_or_else(|| CmdError::Failed("compose".into()))
    }

    pub fn reply_compose(&mut self, mode: ReplyMode) -> CmdResult {
        let r = self.reading.clone().ok_or_else(|| CmdError::Disabled("open a message first".into()))?;
        let parsed = r.parsed.ok_or_else(|| CmdError::Disabled("the message is still downloading".into()))?;
        let account = r.key.account.clone();
        let sig = self.signature(&account);
        let (d, styles) = mime::reply_doc(&parsed, mode, &self.me(&account), &self.clock, &sig);
        let id = self.fresh_id();
        let mut attachments = Vec::new();
        if mode == ReplyMode::Forward
            && let Some(raw) = self.store.as_ref().and_then(|s| s.read_body(&r.key))
        {
            for a in parsed.attachments.iter().filter(|a| !a.inline) {
                if let Some(data) = mime::attachment_data(&raw, a.index) {
                    attachments.push(OutAttachment { name: a.name.clone(), mime: a.mime.clone(), data });
                }
            }
        }
        self.open_compose(Compose {
            id,
            account,
            signature: if sig.trim().is_empty() { String::new() } else { signature_block(&sig) },
            to: addr::format_field(&d.to),
            cc: addr::format_field(&d.cc),
            subject: d.subject,
            body: d.body,
            styles,
            in_reply_to: d.in_reply_to,
            references: d.references,
            attachments,
            mode: Some(mode),
            source: Some(r.key),
            ..Default::default()
        });
        self.style_signature();
        Ok(serde_json::json!({ "compose": id }))
    }

    /// Forward the open message as an attachment, as Outlook's More › Forward as Attachment: the
    /// original goes along whole (an `.eml`), the new message holds only the signature.
    pub fn forward_as_attachment(&mut self) -> CmdResult {
        let r = self.reading.clone().ok_or_else(|| CmdError::Disabled("open a message first".into()))?;
        let raw =
            self.store.as_ref().and_then(|s| s.read_body(&r.key)).ok_or_else(|| CmdError::Disabled("the message is still downloading".into()))?;
        let out = self.reply_compose(ReplyMode::Forward)?;
        let sig = self.signature(&r.key.account);
        let subject = r.parsed.as_ref().map(|p| p.subject.trim().to_string()).unwrap_or_default();
        let name = format!("{}.eml", if subject.is_empty() { "message" } else { subject.as_str() });
        if let Some(c) = self.compose.as_mut() {
            c.set_text(if sig.trim().is_empty() { String::new() } else { format!("\n\n{}", signature_block(&sig)) });
            c.signature = if sig.trim().is_empty() { String::new() } else { signature_block(&sig) };
            c.attachments = vec![OutAttachment { name, mime: "message/rfc822".into(), data: raw }];
        }
        self.style_signature();
        Ok(out)
    }

    pub(super) fn compose_draft(&self, c: &Compose) -> Result<mime::Draft, CmdError> {
        let parse = |field: &str, s: &str| addr::parse_list(s).map_err(|bad| CmdError::Params(format!("{field}: not an email address: {bad}")));
        let (body, html, inline) = c.bodies();
        Ok(mime::Draft {
            reply_to: parse("Replies to", &c.reply_to)?,
            to: parse("To", &c.to)?,
            cc: parse("Cc", &c.cc)?,
            bcc: parse("Bcc", &c.bcc)?,
            subject: c.subject.trim().to_string(),
            body,
            in_reply_to: c.in_reply_to.clone(),
            references: c.references.clone(),
            attachments: c.attachments.clone(),
            html,
            inline,
            importance: c.importance,
            read_receipt: c.read_receipt,
        })
    }

    pub(super) fn compose_bytes(&mut self, c: &Compose, d: &mime::Draft) -> Result<(Vec<u8>, String), CmdError> {
        let acc = self.account(&c.account).ok_or_else(|| CmdError::Failed("the sending account is gone".into()))?;
        let from = mailcraft_model::EmailAddress::new(acc.config.name.clone(), acc.config.email.clone());
        let email = acc.config.email.clone();
        let now = self.clock.now;
        let mid = mime::new_message_id(&email, now, self.random());
        let raw = mime::build(d, &from, now, &mid).map_err(CmdError::Failed)?;
        Ok((raw, email))
    }

    /// What to look at before `c` goes ([`SendCheck`]).
    pub fn send_checks(&mut self, c: &Compose, d: &mime::Draft) -> Vec<SendCheck> {
        let mut found = Vec::new();
        let known = self.known_domains();
        for a in d.to.iter().chain(&d.cc).chain(&d.bcc) {
            if let Some(like) = lookalike::imitates(&lookalike::domain_of(&a.email), &known)
                && found.len() < 20
            {
                found.push(SendCheck::Lookalike { address: a.email.trim().to_string(), like: like.to_string() });
            }
        }
        let mine = self.account(&c.account).map(|a| lookalike::domain_of(&a.config.email)).unwrap_or_default();
        let mut people = BTreeSet::new();
        for a in d.to.iter().chain(&d.cc) {
            let domain = lookalike::domain_of(&a.email);
            if domain.is_empty() || domain == mine {
                continue;
            }
            people.insert(if lookalike::is_mail_service(&domain) { a.email.trim().to_lowercase() } else { domain });
        }
        if people.len() >= EXPOSED_FROM {
            found.push(SendCheck::Exposed { count: people.len() });
        }
        found
    }

    /// Everyone in To and Cc moved to Bcc, with the sender in To: each gets the message without
    /// seeing who else did.
    pub fn hide_recipients(&mut self) -> CmdResult {
        let c = self.compose.as_ref().ok_or_else(|| CmdError::Disabled("no message is being written".into()))?;
        let me = self.account(&c.account).map(|a| EmailAddress::new(a.config.name.clone(), a.config.email.clone()).to_string()).unwrap_or_default();
        let c = self.compose.as_mut().ok_or_else(|| CmdError::Disabled("no message is being written".into()))?;
        let moved: Vec<&str> = [c.to.as_str(), c.cc.as_str(), c.bcc.as_str()]
            .into_iter()
            .map(|f| f.trim().trim_end_matches(';').trim())
            .filter(|f| !f.is_empty())
            .collect();
        c.bcc = moved.join("; ");
        c.to = if me.is_empty() { String::new() } else { format!("{me}; ") };
        c.cc.clear();
        c.show_cc = true;
        Ok(serde_json::json!({ "bcc": c.bcc }))
    }

    /// Send the message being written. Unless `checked`, it's held when [`Session::send_checks`]
    /// finds anything, and the answer says what (`{"check": [...]}`).
    pub fn send_compose(&mut self, allow_empty_subject: bool, checked: bool) -> CmdResult {
        let c = self.compose.clone().ok_or_else(|| CmdError::Disabled("nothing to send".into()))?;
        if c.sending {
            return Err(CmdError::Disabled("already sending".into()));
        }
        let mut d = self.compose_draft(&c)?;
        if d.recipients().is_empty() {
            return Err(CmdError::Params("add at least one recipient".into()));
        }
        if d.subject.is_empty() && !allow_empty_subject {
            return Err(CmdError::Params("the subject is empty (send with {\"allowEmptySubject\": true} to send anyway)".into()));
        }
        // Where the photos were taken stays with the sender (the draft keeps them as attached).
        if !self.prefs.keep_photo_location {
            for a in &mut d.attachments {
                if let Some(clean) = mime::without_location(&a.data) {
                    a.data = clean;
                }
            }
        }
        if !checked {
            let found = self.send_checks(&c, &d);
            if !found.is_empty() {
                return Ok(serde_json::json!({ "check": found }));
            }
        }
        let (raw, from) = self.compose_bytes(&c, &d)?;
        let local = self.account(&c.account).is_none_or(AccountState::is_local);
        let sent_folder = self.account(&c.account).and_then(|a| a.folder(FolderRole::Sent)).map(|f| f.path.clone());
        // Delay Delivery: into the Outbox, sent once it's time (the app has to be running then).
        if let Some(at) = c.send_after.filter(|at| *at > self.clock.now && !local) {
            let item = OutboxItem {
                id: c.id,
                from,
                recipients: d.recipients(),
                sent_folder,
                queued: self.clock.now,
                attempts: 0,
                error: None,
                not_before: at,
            };
            let store = self.store.as_ref().ok_or_else(|| CmdError::Failed("there is nowhere to keep it".into()))?;
            store.outbox_add(&c.account, &item, &raw).map_err(|e| CmdError::Failed(format!("the Outbox couldn't keep it: {e}")))?;
            self.after_outbox_change();
            self.finish_sent(&c, false);
            // finish_sent says "sent"; it isn't yet.
            self.notices.pop();
            self.notice(format!("It will be sent {}", mailcraft_model::dates::full_time(at, &self.clock)), false);
            return Ok(serde_json::json!({ "queued": c.id, "sendAfter": at }));
        }
        if local {
            if let Some(folder) = &sent_folder {
                self.file_locally(&c.account, folder, &raw, true);
            }
            self.finish_sent(&c, true);
            return Ok(serde_json::json!({ "sent": c.id, "local": true }));
        }
        self.jobs.push(Job::Send { account: c.account.clone(), id: c.id, raw, from, recipients: d.recipients(), sent_folder });
        if let Some(cm) = self.compose.as_mut() {
            cm.sending = true;
            cm.error = None;
        }
        Ok(serde_json::json!({ "sending": c.id }))
    }

    pub(super) fn save_pending_marks(&self) {
        let list: Vec<(u64, &MessageKey, &Flag)> = self.pending_marks.iter().map(|(id, (k, f))| (*id, k, f)).collect();
        if let Some(store) = &self.store
            && let Err(e) = store.save_doc(PENDING_MARKS, &list)
        {
            log::warn!("{PENDING_MARKS}: {e}");
        }
    }

    /// The compose is finished. `delivered`: the server accepted it, so the original gets its
    /// Replied or Forwarded flag now; otherwise it sits in the Outbox and the flag is owed until
    /// delivery.
    pub(super) fn finish_sent(&mut self, c: &Compose, delivered: bool) {
        if let (Some(src), Some(mode)) = (&c.source, c.mode) {
            let flag = if mode == ReplyMode::Forward { Flag::Forwarded } else { Flag::Answered };
            if delivered {
                self.apply_op(&src.account.clone(), Op::SetFlag { keys: vec![src.clone()], flag, on: true });
            } else if self.pending_marks.len() < 1000 {
                self.pending_marks.insert(c.id, (src.clone(), flag));
                self.save_pending_marks();
            }
        }
        if let Some(draft) = &c.draft {
            self.apply_op(&draft.account.clone(), Op::Expunge { keys: vec![draft.clone()] });
        }
        if self.compose.as_ref().is_some_and(|x| x.id == c.id) {
            self.compose = None;
        }
        self.composes.retain(|x| x.id != c.id);
        self.notice("Message sent", false);
    }

    /// Save the message being written to Drafts.
    pub fn save_draft(&mut self) -> CmdResult {
        let c = self.compose.clone().ok_or_else(|| CmdError::Disabled("nothing to save".into()))?;
        let (raw, folder) = self.draft_bytes(&c)?;
        if self.account(&c.account).is_none_or(AccountState::is_local) {
            if let Some(old) = &c.draft {
                self.apply_op(&c.account.clone(), Op::Expunge { keys: vec![old.clone()] });
            }
            let key = self.file_locally(&c.account, &folder, &raw, false);
            if let Some(cm) = self.compose.as_mut() {
                cm.draft = key;
            }
        } else {
            self.jobs.push(Job::SaveDraft { account: c.account.clone(), raw, folder, replaces: c.draft.clone() });
            if let Some(cm) = self.compose.as_mut() {
                // The server assigns the new copy's UID; the old one is replaced by the worker.
                cm.draft = None;
            }
        }
        self.notice("Draft saved", false);
        Ok(serde_json::json!({ "saved": c.id }))
    }

    /// Every message being written with something in it, kept as a draft as MailCraft closes:
    /// on this computer at once, and in Drafts on the server at the next connection. How many.
    pub fn keep_drafts(&mut self) -> usize {
        let written: Vec<Compose> = self.all_composes().filter(|c| self.written(c) && !c.sending).cloned().collect();
        let mut kept = 0;
        for c in written {
            let Ok((raw, folder)) = self.draft_bytes(&c) else { continue };
            if self.account(&c.account).is_none_or(AccountState::is_local) {
                if let Some(old) = &c.draft {
                    self.apply_op(&c.account.clone(), Op::Expunge { keys: vec![old.clone()] });
                }
                self.file_locally(&c.account, &folder, &raw, false);
            } else {
                self.jobs.push(Job::KeepDraft { account: c.account.clone(), raw, folder, replaces: c.draft.clone() });
            }
            kept += 1;
        }
        kept
    }

    /// A message as a draft's source, and the Drafts folder it goes in.
    pub(super) fn draft_bytes(&mut self, c: &Compose) -> Result<(Vec<u8>, String), CmdError> {
        // Drafts may hold unfinished addresses: keep only the valid ones.
        let lenient = |s: &str| s.split([',', ';']).filter_map(|p| addr::parse_list(p).ok()).flatten().collect::<Vec<_>>();
        let (body, html, inline) = c.bodies();
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
        };
        let folder = self
            .account(&c.account)
            .and_then(|a| a.folder(FolderRole::Drafts))
            .map(|f| f.path.clone())
            .ok_or_else(|| CmdError::Failed("this account has no Drafts folder".into()))?;
        let (raw, _) = self.compose_bytes(c, &d)?;
        Ok((raw, folder))
    }

    /// File a message in a local account's folder; returns its key.
    pub(super) fn file_locally(&mut self, account: &str, folder: &str, raw: &[u8], seen: bool) -> Option<MessageKey> {
        let store = self.store.clone()?;
        let mut st = store.load_folder(account, folder).unwrap_or_default();
        if st.validity == 0 {
            st.validity = 1;
        }
        let uid = st.uid_next.max(st.messages.iter().map(|m| m.key.uid).max().unwrap_or(0) + 1);
        st.uid_next = uid + 1;
        let key = MessageKey { account: account.to_string(), folder: folder.to_string(), validity: st.validity, uid };
        let flags = mailcraft_model::Flags { seen, draft: !seen, ..Default::default() };
        let size = u32::try_from(raw.len()).unwrap_or(u32::MAX);
        let s = mime::summary_from_full(raw, key.clone(), flags, size, self.clock.now);
        let _ = store.write_body(&key, raw);
        st.messages.push(s);
        sort_newest_first(&mut st.messages);
        if let Err(e) = store.save_folder(account, folder, &st) {
            self.notice(e.to_string(), true);
            return None;
        }
        self.reload_folder(account, folder);
        self.recount_local(account, Some(&[folder.to_string()]));
        self.refresh_list();
        Some(key)
    }
}
