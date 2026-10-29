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

/// A signature as it goes into a message: after the usual `-- ` line (RFC 3676's delimiter).
pub fn signature_block(sig: &str) -> String {
    format!(
        "-- 
{}",
        sig.trim_end()
    )
}

/// Give the signature in a message (`c.signature`, where it still stands as put there) the look
/// it has: `sig` is its text and styles. Its `pictures` go into the message too, each under a
/// `cid:` of this message's own (`tag`, from [`Session::signature_tag`]): one `cid:` in every
/// message isn't unique, as RFC 2392 asks (#16). Pictures of a signature put in before and no
/// longer in the text go.
pub fn style_signature(c: &mut Compose, sig: &(String, mime::rich::Styles), pictures: &[mime::InlinePicture], tag: &str) {
    let (text, styles) = sig;
    if c.signature.is_empty() || styles.is_plain() {
        return;
    }
    let mut block = "-- 
"
    .to_string();
    let mut block_styles = mime::rich::Styles::plain(&block);
    mime::rich::append(&mut block, &mut block_styles, text.trim_end(), styles);
    if block != c.signature {
        return;
    }
    let Some(b) = c.body.rfind(&c.signature) else { return };
    let start = c.body.get(..b).unwrap_or("").chars().count();
    let n = block.chars().count();
    for (i, ch) in block.chars().enumerate() {
        let Some(p) = block_styles.style(i).picture.and_then(|cid| pictures.iter().find(|p| p.cid == cid)).filter(|_| ch == mime::rich::PICTURE)
        else {
            continue;
        };
        let cid = format!("{}.{tag}@mailcraft", p.cid.split('@').next().unwrap_or("signature"));
        block_styles.restyle(&block, i..i + 1, |s| mime::rich::Style { picture: Some(cid.clone()), ..s.clone() });
        if !c.inline.iter().any(|q| q.cid == cid) {
            c.inline.push(mime::InlinePicture { cid, ..p.clone() });
        }
    }
    c.styles.fit(&c.body);
    mime::rich::splice(&mut c.body, &mut c.styles, start..start + n, &block, &block_styles);
    let shown = mime::rich::pictures(&c.body, &c.styles);
    c.inline.retain(|p| !p.cid.starts_with("signature") || shown.contains(&p.cid));
}

impl Session {
    /// A tag for the `cid:`s of a signature's pictures in the message being written
    /// ([`style_signature`]): the one it has already, so a signature put in again keeps them.
    pub(crate) fn signature_tag(&mut self) -> String {
        let had = self.compose.as_ref().and_then(|c| {
            c.inline.iter().find_map(|p| p.cid.strip_prefix("signature")?.split_once('.')?.1.strip_suffix("@mailcraft").map(str::to_string))
        });
        had.unwrap_or_else(|| format!("{:012x}", self.random() & 0xFFFF_FFFF_FFFF))
    }

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
        let (text, styles, _) = self.signature_full(account);
        (text, styles)
    }

    /// The pictures in an account's signature (issue #11).
    pub fn signature_pictures(&self, account: &str) -> Vec<mime::InlinePicture> {
        self.signature_full(account).2
    }

    fn signature_full(&self, account: &str) -> (String, mime::rich::Styles, Vec<mime::InlinePicture>) {
        let Some(a) = self.account(account) else { return (String::new(), mime::rich::Styles::default(), Vec::new()) };
        if !a.config.signature_html.trim().is_empty() {
            let (text, styles, pictures) = mime::signature_html(&a.config.signature_html);
            if !text.trim().is_empty() {
                return (text, styles, pictures);
            }
        }
        (a.config.signature.clone(), mime::rich::Styles::plain(&a.config.signature), Vec::new())
    }

    /// The signature in the message being made, given its look (the text went in plain), and
    /// its pictures.
    pub(super) fn style_signature(&mut self) {
        let Some(account) = self.compose.as_ref().map(|c| c.account.clone()) else { return };
        let (text, styles, pictures) = self.signature_full(&account);
        let tag = self.signature_tag();
        if let Some(c) = self.compose.as_mut() {
            style_signature(c, &(text, styles), &pictures, &tag);
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
    pub fn open_compose(&mut self, mut c: Compose) {
        self.smime_defaults(&mut c);
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
        let encrypted = r.security.as_ref().is_some_and(|s| matches!(s.lock, Some(Lock::Open { .. }))) && self.identity(&r.key.account).is_some();
        let mut parsed = r.parsed.ok_or_else(|| CmdError::Disabled("the message is still downloading".into()))?;
        let account = r.key.account.clone();
        let sig = self.signature(&account);
        let raw = self.readable_raw(&r.key);
        // The pictures in the quoted message go with it (they showed as "[picture]", issue #10).
        let (inline, quoted) = raw.as_deref().map(|raw| mime::quoted_pictures(&mut parsed, raw)).unwrap_or_default();
        let (d, styles) = mime::reply_doc(&parsed, mode, &self.me(&account), &self.clock, &sig);
        let id = self.fresh_id();
        let mut attachments = Vec::new();
        if mode == ReplyMode::Forward
            && let Some(raw) = &raw
        {
            // Pictures the quote shows go in it; any other, inline or not, as a file (a picture
            // sent inline beside plain text was lost).
            for a in parsed.attachments.iter().filter(|a| !quoted.contains(&a.index)) {
                if let Some(data) = mime::attachment_data(raw, a.index) {
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
            inline,
            mode: Some(mode),
            source: Some(r.key),
            // An answer to encrypted mail quotes it: encrypted too, and signed, when it can be.
            sign: encrypted,
            encrypt: encrypted,
            window: self.prefs.reply_window,
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
            seven_bit: false,
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
        // Signed or encrypted (S/MIME): its parts must reach the reader byte for byte.
        if c.sign || c.encrypt {
            d.seven_bit = true;
        }
        let (raw, from) = self.compose_bytes(&c, &d)?;
        let recipients = d.recipients();
        if let Some(job) = self.protect_job(&c, &raw, &recipients, &from)? {
            self.jobs.push(job);
            self.mark_sending(c.id);
            return Ok(serde_json::json!({ "protecting": c.id }));
        }
        self.deliver(&c, raw, from, recipients)
    }

    fn mark_sending(&mut self, id: u64) {
        for cm in self.compose.iter_mut().chain(self.composes.iter_mut()).filter(|x| x.id == id) {
            cm.sending = true;
            cm.error = None;
        }
    }

    /// `c` as `raw` (signed or encrypted already, if it's to be), on its way: into the Outbox for
    /// later, filed in Sent on an account with no server, or handed to the network layer.
    pub(super) fn deliver(&mut self, c: &Compose, raw: Vec<u8>, from: String, recipients: Vec<String>) -> CmdResult {
        let local = self.account(&c.account).is_none_or(AccountState::is_local);
        let sent_folder = self.account(&c.account).and_then(|a| a.folder(FolderRole::Sent)).map(|f| f.path.clone());
        // Delay Delivery: into the Outbox, sent once it's time (the app has to be running then).
        if let Some(at) = c.send_after.filter(|at| *at > self.clock.now && !local) {
            let item = OutboxItem { id: c.id, from, recipients, sent_folder, queued: self.clock.now, attempts: 0, error: None, not_before: at };
            let store = self.store.as_ref().ok_or_else(|| CmdError::Failed("there is nowhere to keep it".into()))?;
            store.outbox_add(&c.account, &item, &raw).map_err(|e| CmdError::Failed(format!("the Outbox couldn't keep it: {e}")))?;
            self.after_outbox_change();
            self.finish_sent(c, false);
            // finish_sent says "sent"; it isn't yet.
            self.notices.pop();
            self.notice(format!("It will be sent {}", mailcraft_model::dates::full_time(at, &self.clock)), false);
            return Ok(serde_json::json!({ "queued": c.id, "sendAfter": at }));
        }
        if local {
            if let Some(folder) = &sent_folder {
                self.file_locally(&c.account, folder, &raw, true);
            }
            self.finish_sent(c, true);
            return Ok(serde_json::json!({ "sent": c.id, "local": true }));
        }
        self.jobs.push(Job::Send { account: c.account.clone(), id: c.id, raw, from, recipients, sent_folder });
        self.mark_sending(c.id);
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
            self.jobs.push(self.draft_job(&c, raw, folder, false));
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
                let job = self.draft_job(&c, raw, folder, true);
                self.jobs.push(job);
            }
            kept += 1;
        }
        kept
    }

    /// The job filing `c`'s draft `raw` in `folder` on the server (`keep`: kept on this computer
    /// first, as MailCraft closes): encrypted to the account's own certificate first when the
    /// message is to be encrypted.
    fn draft_job(&self, c: &Compose, raw: Vec<u8>, folder: String, keep: bool) -> Job {
        let config = self.account(&c.account).map(|a| a.config.clone());
        match config {
            Some(account) if c.encrypt && self.identity(&c.account).is_some() => {
                Job::SealDraft { account, raw: mailcraft_model::Private(raw), folder, replaces: c.draft.clone(), keep }
            }
            _ if keep => Job::KeepDraft { account: c.account.clone(), raw, folder, replaces: c.draft.clone() },
            _ => Job::SaveDraft { account: c.account.clone(), raw, folder, replaces: c.draft.clone() },
        }
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
            seven_bit: false,
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
