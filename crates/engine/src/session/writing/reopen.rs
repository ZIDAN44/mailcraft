//! A message saved before, opened to carry on writing it, as Outlook opens a draft from Drafts:
//! as it was saved. Its recipients, subject, importance and read receipt; its text with its
//! formatting, the line over a quote and its signature as it went in (a signature written as
//! HTML code is that code again, to be sent as written); its pictures in place and its files;
//! the original under a reply as it came, with what was changed in it; and the message it
//! answers, which gets its Replied flag once it's sent. Saving it again replaces the saved copy.

use super::signatures::{signature_block, style_signature};
use super::*;

impl Session {
    /// Carry on writing draft `key`. One being written already is brought forward instead.
    pub fn edit_draft(&mut self, key: &MessageKey) -> CmdResult {
        let open = self.all_composes().find(|c| c.draft.as_ref() == Some(key)).map(|c| c.id);
        if let Some(id) = open {
            self.activate_compose(id);
            return Ok(serde_json::json!({ "compose": id }));
        }
        let raw = self.readable_raw(key).ok_or_else(|| CmdError::Disabled("the draft is still downloading".into()))?;
        // From an account that can send (not the data file it's in, whose copy stays as it is).
        let account = self.sending_account(key.account.clone());
        let mut c = self.compose_from_saved(&account, key, &raw)?;
        if account == key.account {
            c.draft = Some(key.clone());
        }
        c.window = self.prefs.new_mail_window;
        let id = c.id;
        self.open_compose(c);
        self.made_now(id);
        Ok(serde_json::json!({ "compose": id }))
    }

    /// Message `raw`, kept at `key` (its pictures are shown from there), as one being written
    /// from `account`.
    pub(in crate::session) fn compose_from_saved(&mut self, account: &str, key: &MessageKey, raw: &[u8]) -> Result<Compose, CmdError> {
        let p = mime::parse(raw).ok_or_else(|| CmdError::Failed("the message can't be read".into()))?;
        let html = mime::html_body(raw).filter(|_| p.had_html);
        // Pictures its text shows stay in it; the rest are its files.
        let mut attachments = Vec::new();
        let mut inline = Vec::new();
        for a in &p.attachments {
            let Some(out) = mime::attachment_out(raw, a.index) else { continue };
            let shown =
                a.content_id.clone().filter(|c| a.inline && !c.is_empty() && html.as_deref().is_some_and(|h| h.contains(&format!("cid:{c}"))));
            match shown {
                Some(cid) => inline.push(mime::InlinePicture { cid, name: a.name.clone(), mime: out.mime, data: out.data }),
                None => attachments.push(OutAttachment { name: a.name.clone(), ..out }),
            }
        }
        let (importance, read_receipt, source) = mime::sending_options(raw);
        let mut c = Compose {
            id: self.fresh_id(),
            account: account.to_string(),
            to: addr::format_field(&p.to),
            cc: addr::format_field(&p.cc),
            bcc: addr::format_field(&p.bcc),
            show_cc: !p.bcc.is_empty(),
            reply_to: addr::format_field(&p.reply_to),
            show_reply_to: !p.reply_to.is_empty(),
            subject: p.subject.clone(),
            in_reply_to: p.in_reply_to.clone(),
            references: p.references.clone(),
            attachments,
            inline,
            importance,
            read_receipt,
            draft_id: p.message_id.clone(),
            ..Default::default()
        };
        match html.as_deref() {
            Some(h) => {
                // The original under a reply, as it came (with what was changed in it).
                let own = match mime::split_marked(h, mime::QUOTE_CLASS) {
                    Some((own, q, _)) => {
                        let parsed = mime::with_html(&p, &q);
                        let blocks =
                            if parsed.layout.is_empty() { vec![mime::layout::Block::Text(parsed.rich.clone())] } else { parsed.layout.clone() };
                        c.quote = Some(Arc::new(Quote { key: key.clone(), parsed, blocks, html: q, ..Default::default() }));
                        own
                    }
                    None => h.to_string(),
                };
                own_text(&mut c, &own);
            }
            None => c.set_text(p.text.clone()),
        }
        rule_over_quote(&mut c);
        // The message it answers or forwards, for its flag once it's sent: as its copy names it,
        // else (a draft saved elsewhere) the one its In-Reply-To names.
        let named = source.split_once(' ').and_then(|(mode, k)| {
            let mode = match mode {
                "forward" => ReplyMode::Forward,
                "reply" => ReplyMode::Reply,
                _ => return None,
            };
            MessageKey::from_id(k.trim()).filter(|k| self.account(&k.account).is_some()).map(|k| (k, mode))
        });
        let found = || (!p.in_reply_to.is_empty()).then(|| self.find_message(&key.account, &p.in_reply_to)).flatten().map(|k| (k, ReplyMode::Reply));
        if let Some((src, mode)) = named.or_else(found) {
            c.source = Some(src);
            c.mode = Some(mode);
        }
        Ok(c)
    }

    /// The message with Message-ID `id` in one of `account`'s folders (Drafts first), from what
    /// this computer lists of them.
    pub(crate) fn find_message(&self, account: &str, id: &str) -> Option<MessageKey> {
        let store = self.store.as_ref()?;
        let a = self.account(account)?;
        let id = id.trim().trim_matches(['<', '>']);
        if id.is_empty() {
            return None;
        }
        let drafts = a.folder(FolderRole::Drafts).map(|f| f.path.clone());
        let order = drafts.iter().cloned().chain(a.folders.iter().map(|f| f.path.clone()).filter(|p| Some(p) != drafts.as_ref()));
        for folder in order {
            let Ok(st) = store.load_folder(account, &folder) else { continue };
            if let Some(m) = st.messages.iter().find(|m| m.message_id.trim().trim_matches(['<', '>']) == id) {
                return Some(m.key.clone());
            }
        }
        None
    }

    /// The copy of `c` saved in Drafts, when there is one: the one it was opened from or last
    /// saved as, or (saved to a server, which gives the copy its place) the one with the Message-ID
    /// it was last saved with.
    pub(super) fn saved_copy(&self, c: &Compose) -> Option<MessageKey> {
        c.draft.clone().or_else(|| {
            let folder = self.account(&c.account)?.folder(FolderRole::Drafts)?.path.clone();
            let id = c.draft_id.trim().trim_matches(['<', '>']);
            if id.is_empty() {
                return None;
            }
            let st = self.store.as_ref()?.load_folder(&c.account, &folder).ok()?;
            st.messages.iter().find(|m| m.message_id.trim().trim_matches(['<', '>']) == id).map(|m| m.key.clone())
        })
    }
}

/// The text of a saved message (`own`: its HTML without the original under it) into `c`, its
/// formatting kept, and its signature as it went in when it was written as HTML code.
fn own_text(c: &mut Compose, own: &str) {
    let Some((before, code, after)) = mime::split_marked(own, mime::SIGNATURE_CLASS) else {
        let (text, styles) = mime::rich::from_rich(&mime::html_to_rich(own));
        c.body = text;
        c.styles = styles;
        return;
    };
    let (mut text, mut styles) = mime::rich::from_rich(&mime::html_to_rich(&before));
    // The `-- ` line above the code is the signature's own.
    if let Some(cut) = text.rfind('\n').filter(|i| text.get(i + 1..).is_some_and(|l| l.trim() == "--")).or_else(|| (text.trim() == "--").then_some(0))
    {
        text.truncate(cut);
        styles.fit(&text);
    }
    // The code's pictures, written back into it (as a signature's code keeps them).
    let tag = c
        .inline
        .iter()
        .filter_map(|p| p.cid.strip_prefix("signature").and_then(|r| r.split_once('.')).and_then(|(_, r)| r.strip_suffix("@mailcraft")))
        .find(|t| code.contains(&format!(".{t}@mailcraft")))
        .unwrap_or_default()
        .to_string();
    let theirs: Vec<mime::InlinePicture> = c.inline.iter().filter(|p| code.contains(&format!("cid:{}", p.cid))).cloned().collect();
    c.inline.retain(|p| !code.contains(&format!("cid:{}", p.cid)));
    let code = mime::embed_pictures(&code, &theirs);
    let (sig_text, sig_styles, pictures) = mime::signature_html(&code);
    let block = signature_block(&sig_text);
    if !text.is_empty() {
        mime::rich::append(&mut text, &mut styles, "\n", &mime::rich::Styles::plain("\n"));
    }
    mime::rich::append(&mut text, &mut styles, &block, &mime::rich::Styles::plain(&block));
    let (rest, rest_styles) = mime::rich::from_rich(&mime::html_to_rich(&after));
    if !rest.trim().is_empty() {
        mime::rich::append(&mut text, &mut styles, "\n", &mime::rich::Styles::plain("\n"));
        mime::rich::append(&mut text, &mut styles, &rest, &rest_styles);
    }
    c.body = text;
    c.styles = styles;
    c.signature = block;
    c.signature_code = code;
    c.signature_tag = tag.clone();
    style_signature(c, &(sig_text, sig_styles), &pictures, &tag);
}

/// The line over the original of a reply (an empty paragraph drawn as a rule, which a saved
/// message's HTML has as `<hr>`): above who sent it to whom and when.
fn rule_over_quote(c: &mut Compose) {
    let lines: Vec<&str> = c.body.split('\n').collect();
    let at = (1..lines.len()).find(|&i| {
        lines.get(i).is_some_and(|l| l.starts_with("From: "))
            && lines.get(i + 1).is_some_and(|l| l.starts_with("Sent: "))
            && lines.get(i - 1).is_some_and(|l| l.trim().is_empty())
    });
    let Some(at) = at else { return };
    c.styles.fit(&c.body);
    if let Some(p) = c.styles.paras.get_mut(at - 1) {
        p.rule = true;
    }
}
