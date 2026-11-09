//! The session: messages being written, drafts and sending.

use mailcraft_model::lookalike;

use super::*;

mod drafts;
mod keeping;
mod quote;
mod reactions;
mod reopen;
mod sending;
mod signatures;
pub use drafts::Drafted;
pub(crate) use drafts::DraftedMemo;
pub use quote::Quote;
pub use reactions::Reactions;
pub use sending::SendBlock;
pub use signatures::signature_parts;

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

/// A message being written.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Compose {
    pub id: u64,
    pub account: String,
    /// The signature in the text as it was put there (`-- ` and the signature): replaced when
    /// another goes in, while it's still there unchanged.
    #[serde(default)]
    pub signature: String,
    /// That signature's name, its HTML code (sent in its place while it's there as it was) and
    /// the tag of its pictures' `cid:`s.
    #[serde(default)]
    pub signature_name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub signature_code: String,
    #[serde(default)]
    pub signature_tag: String,
    pub to: String,
    pub cc: String,
    pub bcc: String,
    pub subject: String,
    pub body: String,
    pub attachments: Vec<OutAttachment>,
    pub in_reply_to: String,
    pub references: Vec<String>,
    pub mode: Option<ReplyMode>,
    /// The message replied to or forwarded (it gets `\Answered` / `$Forwarded` once sent).
    pub source: Option<MessageKey>,
    pub sending: bool,
    pub error: Option<String>,
    /// The copy saved in Drafts, replaced on the next save.
    pub draft: Option<MessageKey>,
    /// The Message-ID it was last saved with: how the copy a server filed is found, to be
    /// replaced on the next save (the server gives it its place).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub draft_id: String,
    /// The Bcc row is shown.
    pub show_cc: bool,
    /// Send as plain text only (lists written out, the rest of the formatting dropped).
    #[serde(default)]
    pub plain: bool,
    /// The pictures placed in the text (each where a [`mime::rich::PICTURE`] stands).
    #[serde(default)]
    pub inline: Vec<mime::InlinePicture>,
    /// How the text looks: a style for each character and paragraph.
    #[serde(default)]
    pub styles: mime::rich::Styles,
    /// Written in a window of its own rather than the reading pane.
    #[serde(default)]
    pub window: bool,
    /// High (1) or low (-1) importance.
    #[serde(default)]
    pub importance: i8,
    /// Ask the recipients for a read receipt.
    #[serde(default)]
    pub read_receipt: bool,
    /// Where replies go, when not to the sender (Outlook's "Have replies sent to").
    #[serde(default)]
    pub reply_to: String,
    /// Hold it in the Outbox until then (Unix seconds): Outlook's Delay Delivery.
    #[serde(default)]
    pub send_after: Option<i64>,
    /// The Replies to row is shown.
    #[serde(default)]
    pub show_reply_to: bool,
    /// Signed, and encrypted to its recipients, as it's sent: with S/MIME or OpenPGP (`scheme`;
    /// `None`: whichever every recipient can receive), encryption following the recipients while
    /// `encrypt_auto` (the account's option, until the user chooses).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub sign: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub encrypt: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheme: Option<Scheme>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub encrypt_auto: bool,
    /// The original of a reply or forward written in HTML, kept as it came ([`Quote`]): after the
    /// text, outside it, until it's made text to edit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote: Option<Arc<Quote>>,
    /// The draft last sent to a server to save, while it may still be on the way: its source,
    /// folder and the copy it replaces. Kept as it was if MailCraft closes meanwhile, so the two
    /// are the same draft (one Message-ID) and not kept twice (#53).
    #[serde(skip)]
    pub on_its_way: Option<(Vec<u8>, String, Option<MessageKey>)>,
    /// How many saves of it are on the way to a server: `on_its_way` is the last, and stays until
    /// each is said done ([`Session::draft_saved`]).
    #[serde(skip)]
    pub saves: u32,
    /// A reply, forward or draft opened again as MailCraft made it ([`Compose::mark_made`]):
    /// unchanged since, nothing was written in it ([`Session::written`]), so closing it asks
    /// nothing and keeps no draft, as Outlook's.
    #[serde(skip)]
    pub made: Option<(u64, Option<Arc<Quote>>)>,
    /// The message as it was saved to a server, that save still on its way: what it was made as
    /// once the server has it ([`Session::draft_saved`]).
    #[serde(skip)]
    pub saved_as: Option<(u64, Option<Arc<Quote>>)>,
}

impl Compose {
    /// What the message is now, as far as writing in it goes: everything the user can change in
    /// it (who it's from and to, its subject, text and how it looks, its pictures and files byte
    /// for byte, its options), the original under it compared apart, by the copy kept. Its rows
    /// shown and where it's written aren't the message. Encryption that follows the recipients
    /// on its own counts only once chosen.
    fn fingerprint(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (&self.account, &self.to, &self.cc, &self.bcc, &self.reply_to, &self.subject, &self.body).hash(&mut h);
        (&self.signature, &self.signature_name, &self.signature_code).hash(&mut h);
        (self.importance, self.read_receipt, self.plain, self.send_after, self.sign).hash(&mut h);
        (!self.encrypt_auto).then_some(self.encrypt).hash(&mut h);
        // As the editor keeps them each frame (fitted to the text), and the scheme, by their form.
        let mut styles = self.styles.clone();
        styles.fit(&self.body);
        serde_json::to_vec(&(&styles, &self.scheme)).unwrap_or_default().hash(&mut h);
        for p in &self.inline {
            (&p.cid, &p.name, &p.mime, &p.data).hash(&mut h);
        }
        for a in &self.attachments {
            (&a.name, &a.mime, &a.data, &a.content_id).hash(&mut h);
        }
        h.finish()
    }

    /// Note the message as it was made: a reply or forward as MailCraft wrote it, a draft as it
    /// was opened or last saved ([`Compose::made`]).
    pub(crate) fn mark_made(&mut self) {
        self.made = Some(self.as_now());
    }

    /// What [`Compose::made`] notes of it.
    fn as_now(&self) -> (u64, Option<Arc<Quote>>) {
        (self.fingerprint(), self.quote.clone())
    }

    /// Whether nothing was changed since [`Compose::mark_made`].
    pub(crate) fn unchanged(&self) -> bool {
        let Some((at, quote)) = &self.made else { return false };
        let same_quote = match (quote, &self.quote) {
            (Some(was), Some(now)) => Arc::ptr_eq(was, now),
            (was, now) => was.is_none() && now.is_none(),
        };
        same_quote && *at == self.fingerprint()
    }

    /// The text, HTML and pictures to send: formatted when the text uses formatting or has
    /// pictures (and the message isn't set to plain text), else plain text alone; the original
    /// of a reply kept as it came after it ([`Quote`]).
    pub fn bodies(&self) -> (String, Option<String>, Vec<mime::InlinePicture>) {
        self.with_quote(self.own_bodies())
    }

    /// [`Compose::bodies`] of the text alone.
    fn own_bodies(&self) -> (String, Option<String>, Vec<mime::InlinePicture>) {
        let mut styles = self.styles.clone();
        styles.fit(&self.body);
        let used = mime::rich::pictures(&self.body, &styles);
        let pictures: Vec<mime::InlinePicture> = self.inline.iter().filter(|p| used.contains(&p.cid)).cloned().collect();
        let text = mime::rich::to_text(&self.body, &styles);
        if self.plain || (styles.is_plain() && pictures.is_empty()) {
            (text, None, Vec::new())
        } else {
            (text, Some(mime::rich::to_html(&self.body, &styles)), pictures)
        }
    }

    /// Set the text (and forget its formatting).
    pub fn set_text(&mut self, text: String) {
        self.styles = mime::rich::Styles::plain(&text);
        self.body = text;
    }

    /// [`Compose::bodies`], with the signature's `code` in the HTML where the signature
    /// stands (issue #11: a business card in columns, which the text can't hold), as long as it
    /// stands there as it was put in; the text keeps the signature's text. Its pictures go with
    /// it, under this message's own `cid:`s.
    pub fn bodies_with(&self, code: &str) -> (String, Option<String>, Vec<mime::InlinePicture>) {
        let (text, html, pictures) = self.own_bodies();
        let at = self.signature_at().filter(|_| !self.plain && !code.trim().is_empty());
        let Some(b) = at else { return self.with_quote((text, html, pictures)) };
        let mut styles = self.styles.clone();
        styles.fit(&self.body);
        let start = self.body.get(..b).unwrap_or("").chars().count();
        let end = start + self.signature.chars().count();
        // `-- ` stays text, as above every signature; the code is its next line.
        let (before, before_styles) = mime::rich::slice(&self.body, &styles, 0..start + "-- ".len());
        let (after, after_styles) = mime::rich::slice(&self.body, &styles, end..self.body.chars().count());
        let tag = Some(self.signature_tag.clone()).filter(|t| !t.is_empty()).unwrap_or_else(|| format!("{:012x}", self.id));
        let (code_html, code_pictures) = mime::signature_code_parts(code, &tag);
        let mut out = mime::rich::to_html(&before, &before_styles);
        // Marked, so a saved draft opened again has the code back as it was.
        out.push_str(&format!("<div class=\"{}\">{code_html}</div>", mime::SIGNATURE_CLASS));
        if !after.trim().is_empty() {
            out.push_str(&mime::rich::to_html(&after, &after_styles));
        }
        let mut used = mime::rich::pictures(&before, &before_styles);
        used.extend(mime::rich::pictures(&after, &after_styles));
        let mut pictures: Vec<mime::InlinePicture> = pictures.into_iter().filter(|p| used.contains(&p.cid)).collect();
        for p in code_pictures {
            if !pictures.iter().any(|q| q.cid == p.cid) {
                pictures.push(p);
            }
        }
        self.with_quote((text, Some(out), pictures))
    }
}

impl Session {
    pub(super) fn me(&self, account: &str) -> Vec<String> {
        let mut v: Vec<String> = self.accounts.iter().map(|a| a.config.email.clone()).collect();
        if let Some(a) = self.account(account) {
            v.push(a.config.username.clone());
        }
        v
    }

    /// A new message from a `mailto:` link (MailCraft as the system's mail app): its addresses,
    /// subject and first words, above the signature. Nothing is attached and nothing is sent.
    pub fn mailto_compose(&mut self, m: &mailcraft_model::mailto::Mailto) -> CmdResult {
        let c = self.new_compose(None)?;
        let account = c.account.clone();
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
        let sig = self.default_signature(&account, false);
        self.style_signature(sig.as_ref());
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

    /// Message `id`, the one on screen or one set aside: news of a send may come for either.
    pub(super) fn compose_by_id(&self, id: u64) -> Option<Compose> {
        self.all_composes().find(|c| c.id == id).cloned()
    }

    pub fn new_compose(&mut self, account: Option<&str>) -> Result<&mut Compose, CmdError> {
        let account = match account {
            Some(a) => self.account(a).map(|a| a.config.id.clone()),
            None => self.current_account().map(|a| self.sending_account(a.config.id.clone())),
        }
        .ok_or_else(|| CmdError::Disabled("add an account first".into()))?;
        let sig = self.default_signature(&account, false);
        let id = self.fresh_id();
        let window = self.prefs.new_mail_window;
        self.open_compose(Compose { id, account, window, ..Default::default() });
        self.put_signature(sig.as_ref(), None)?;
        self.compose.as_mut().ok_or_else(|| CmdError::Failed("compose".into()))
    }

    pub fn reply_compose(&mut self, mode: ReplyMode) -> CmdResult {
        let r = self.reading.clone().ok_or_else(|| CmdError::Disabled("open a message first".into()))?;
        let encrypted = r.security.as_ref().is_some_and(|s| matches!(s.lock, Some(Lock::Open { .. }))) && self.can_protect(&r.key.account);
        // Answered the way it came, when the account can.
        let came_as = r.security.as_ref().map(|s| if s.pgp { Scheme::Pgp } else { Scheme::Smime });
        let scheme = came_as.filter(|s| {
            encrypted && if *s == Scheme::Pgp { self.pgp_key(&r.key.account).is_some() } else { self.identity(&r.key.account).is_some() }
        });
        // The reply's own copy (the quote's pictures are put in it), the one shown left as it is.
        let mut parsed = Arc::unwrap_or_clone(r.parsed.ok_or_else(|| CmdError::Disabled("the message is still downloading".into()))?);
        // Forwarded, its files go with it: not before they've come.
        if r.text_first && mode == ReplyMode::Forward {
            return Err(CmdError::Failed(super::STILL_COMING.into()));
        }
        // Answered from an account that can send (not the data file it's in).
        let account = self.sending_account(r.key.account.clone());
        let signature = self.default_signature(&account, true);
        let block = Self::signature_text(signature.as_ref());
        let sig = block.strip_prefix("-- \n").unwrap_or_default();
        let raw = self.readable_raw(&r.key);
        let original = parsed.clone();
        // The pictures in the quoted message go with it (they showed as "[picture]", issue #10).
        let (inline, quoted, given) = raw.as_deref().map(|raw| mime::quoted_pictures_given(&mut parsed, raw)).unwrap_or_default();
        // Written in HTML, the original goes as it came, its columns and all (the editor's text
        // has no tables), after who sent it to whom and when, which stay the text's.
        let html = raw.as_deref().filter(|_| parsed.had_html).and_then(mime::html_body).unwrap_or_default();
        let quote = Quote::of(r.key.clone(), original, &parsed, &given, &html).map(Arc::new);
        let (d, styles) = if quote.is_some() {
            mime::reply_head_doc(&parsed, mode, &self.me(&account), &self.clock, sig)
        } else {
            mime::reply_doc(&parsed, mode, &self.me(&account), &self.clock, sig)
        };
        let id = self.fresh_id();
        let mut attachments = Vec::new();
        if mode == ReplyMode::Forward
            && let Some(raw) = &raw
        {
            // Pictures the quote shows go in it; any other, inline or not, as a file (a picture
            // sent inline beside plain text was lost).
            for a in parsed.attachments.iter().filter(|a| !quoted.contains(&a.index)) {
                if let Some(out) = mime::attachment_out(raw, a.index) {
                    attachments.push(OutAttachment { name: a.name.clone(), ..out });
                }
            }
        }
        self.open_compose(Compose {
            id,
            account,
            signature: block.clone(),
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
            scheme,
            window: self.prefs.reply_window,
            quote,
            ..Default::default()
        });
        self.style_signature(signature.as_ref());
        self.made_now(id);
        Ok(serde_json::json!({ "compose": id }))
    }

    /// Note message `id` as it was just made ([`Compose::mark_made`]).
    pub(crate) fn made_now(&mut self, id: u64) {
        if let Some(c) = self.compose.iter_mut().chain(self.composes.iter_mut()).find(|c| c.id == id) {
            c.mark_made();
        }
    }

    /// Forward the open message as an attachment, as Outlook's More › Forward as Attachment: the
    /// original goes along whole (an `.eml`), the new message holds only the signature.
    pub fn forward_as_attachment(&mut self) -> CmdResult {
        let r = self.reading.clone().ok_or_else(|| CmdError::Disabled("open a message first".into()))?;
        let raw =
            self.store.as_ref().and_then(|s| s.read_body(&r.key)).ok_or_else(|| CmdError::Disabled("the message is still downloading".into()))?;
        let out = self.reply_compose(ReplyMode::Forward)?;
        // The signature of the account it goes from (not the data file it's in).
        let from = self.compose.as_ref().map_or(r.key.account.clone(), |c| c.account.clone());
        let sig = self.default_signature(&from, true);
        let subject = r.parsed.as_ref().map(|p| p.subject.trim().to_string()).unwrap_or_default();
        let name = format!("{}.eml", if subject.is_empty() { "message" } else { subject.as_str() });
        if let Some(c) = self.compose.as_mut() {
            c.set_text(String::new());
            c.signature = String::new();
            c.inline.clear();
            c.quote = None;
            c.attachments = vec![OutAttachment { name, mime: "message/rfc822".into(), data: raw, ..Default::default() }];
        }
        self.put_signature(sig.as_ref(), None)?;
        if let Some(id) = out.get("compose").and_then(|v| v.as_u64()) {
            self.made_now(id);
        }
        Ok(out)
    }

    pub(super) fn compose_draft(&self, c: &Compose) -> Result<mime::Draft, CmdError> {
        let parse = |field: &str, s: &str| addr::parse_list(s).map_err(|bad| CmdError::Params(format!("{field}: not an email address: {bad}")));
        let (body, html, inline) = c.bodies_with(&c.signature_code);
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
            keep_bcc: false,
            source: String::new(),
        })
    }

    pub(super) fn compose_bytes(&mut self, c: &Compose, d: &mime::Draft) -> Result<(Vec<u8>, String), CmdError> {
        let (raw, email, _) = self.compose_bytes_with_id(c, d)?;
        Ok((raw, email))
    }

    /// [`Session::compose_bytes`], with the Message-ID it was given.
    fn compose_bytes_with_id(&mut self, c: &Compose, d: &mime::Draft) -> Result<(Vec<u8>, String, String), CmdError> {
        let email = self.account(&c.account).map(|a| a.config.email.clone()).unwrap_or_default();
        let mid = mime::new_message_id(&email, self.clock.now, self.random());
        self.compose_bytes_as(c, d, mid)
    }

    /// [`Session::compose_bytes_with_id`], written with Message-ID `mid`.
    fn compose_bytes_as(&mut self, c: &Compose, d: &mime::Draft, mid: String) -> Result<(Vec<u8>, String, String), CmdError> {
        let acc = self.account(&c.account).ok_or_else(|| CmdError::Failed("the sending account is gone".into()))?;
        let from = mailcraft_model::EmailAddress::new(acc.config.name.clone(), acc.config.email.clone());
        let email = acc.config.email.clone();
        let raw = mime::build(d, &from, self.clock.now, &mid).map_err(CmdError::Failed)?;
        Ok((raw, email, mid))
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
        self.can_send(&c.account)?;
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
        // The account's OpenPGP key goes along, for answers to be encrypted (Autocrypt).
        let raw = self.with_autocrypt(&c.account, raw);
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
    /// later, filed in Sent in the sample mailbox, or handed to the network layer. From an
    /// account that can't send ([`SendBlock`]), refused: nothing goes, nothing says it went.
    pub(super) fn deliver(&mut self, c: &Compose, raw: Vec<u8>, from: String, recipients: Vec<String>) -> CmdResult {
        self.can_send(&c.account)?;
        let local = self.account(&c.account).is_some_and(AccountState::is_local);
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
            // Not kept in Sent: the message stays open, its draft too.
            if let Some(folder) = &sent_folder {
                self.file_locally(&c.account, folder, &raw, true)?;
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
        if let Some(draft) = self.saved_copy(c) {
            self.apply_op(&draft.account.clone(), Op::Expunge { keys: vec![draft] });
        }
        if self.compose.as_ref().is_some_and(|x| x.id == c.id) {
            self.compose = None;
        }
        self.composes.retain(|x| x.id != c.id);
        self.notice("Message sent", false);
    }
}
