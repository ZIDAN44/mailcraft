//! A message saved before, opened to carry on writing it, as Outlook opens a draft from Drafts:
//! as it was saved. Its recipients, subject, importance and read receipt; its text with its
//! formatting, the line over a quote and its signature as it went in (a signature written as
//! HTML code is that code again, to be sent as written); its pictures in place and its files;
//! the original under a reply as it came, with what was changed in it; and the message it
//! answers, which gets its Replied flag once it's sent. Saving it again replaces the saved copy.

use mailcraft_model::thread::norm_id;

use super::signatures::{signature_block, style_signature};
use super::*;
use crate::session::security::is_sealed;

impl Session {
    /// Carry on writing draft `key`. One being written already is brought forward instead.
    pub fn edit_draft(&mut self, key: &MessageKey) -> CmdResult {
        // By the copy it was opened from, or the one it was saved as since (a save on a server
        // is another copy, with the same Message-ID): opened a second time from that, sending
        // one left the other to be sent again, or kept as a draft of mail already sent.
        let id = self.summary(key).map(|m| norm_id(&m.message_id)).filter(|i| !i.is_empty());
        let saved_as = |c: &Compose| c.account == key.account && id.as_ref().is_some_and(|i| norm_id(&c.draft_id) == *i);
        let open = self.all_composes().find(|c| c.draft.as_ref() == Some(key) || saved_as(c)).map(|c| c.id);
        if let Some(id) = open {
            self.activate_compose(id);
            return Ok(serde_json::json!({ "compose": id }));
        }
        let raw = self.readable_raw(key).ok_or_else(|| CmdError::Disabled("the draft is still downloading".into()))?;
        // One kept encrypted (a message to be sent encrypted is, on the server) opens once
        // it's been decrypted, never as what encrypts it.
        if is_sealed(&raw) {
            return Err(CmdError::Disabled("the draft is encrypted: it opens once it has been decrypted".into()));
        }
        // From an account that can send (not the data file it's in, whose copy stays as it is).
        let account = self.sending_account(key.account.clone());
        let mut c = self.compose_from_saved(&account, key, &raw)?;
        if account == key.account {
            c.draft = Some(key.clone());
        }
        // Kept encrypted, it was to be sent encrypted: it still is, whatever the account does
        // by itself (opened again and sent, it went in the clear, with the text it quoted).
        if self.body_raw(key).is_some_and(|stored| is_sealed(&stored)) {
            c.encrypt = true;
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
        // All its files from one parse of it.
        let wanted: Vec<usize> = p.attachments.iter().map(|a| a.index).collect();
        let mut outs: HashMap<usize, OutAttachment> = mime::attachments_out(raw, &wanted).into_iter().collect();
        for a in &p.attachments {
            let Some(out) = outs.remove(&a.index) else { continue };
            let shown =
                a.content_id.clone().filter(|c| a.inline && !c.is_empty() && html.as_deref().is_some_and(|h| h.contains(&format!("cid:{c}"))));
            match shown {
                Some(cid) => inline.push(mime::InlinePicture { cid, name: a.name.clone(), mime: out.mime, data: out.data }),
                None => attachments.push(OutAttachment { name: a.name.clone(), ..out }),
            }
        }
        let (importance, read_receipt, source) = mime::sending_options(raw);
        // From the address it was written from, when that's one of the account's others.
        let from = self.account(account).and_then(|a| a.config.identity(&p.from.email)).map(|i| i.email.clone()).unwrap_or_default();
        let mut c = Compose {
            id: self.fresh_id(),
            account: account.to_string(),
            from,
            to: addr::format_field(&p.to),
            cc: addr::format_field(&p.cc),
            bcc: addr::format_field(&p.bcc),
            show_bcc: !p.bcc.is_empty(),
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

    /// The message with Message-ID `id` in one of `account`'s folders (Drafts first): looked for
    /// in the lists read already, then in the lists as this computer keeps them, read from disk
    /// (a sync may have listed it there first). Every list was read from disk, each time.
    pub(crate) fn find_message(&self, account: &str, id: &str) -> Option<MessageKey> {
        let a = self.account(account)?;
        let id = norm_id(id);
        if id.is_empty() {
            return None;
        }
        let drafts = a.folder(FolderRole::Drafts).map(|f| f.path.as_str());
        let order: Vec<&str> = drafts.into_iter().chain(a.folders.iter().map(|f| f.path.as_str()).filter(|p| Some(*p) != drafts)).collect();
        let read = |folder: &str| self.cache.get(&(account.to_string(), folder.to_string()));
        order.iter().find_map(|f| read(f).and_then(|st| with_id(st, &id))).or_else(|| order.iter().find_map(|f| self.kept_with_id(account, f, &id)))
    }

    /// The copy of `c` saved in Drafts, when there is one: the one it was opened from or last
    /// saved as, or (saved to a server, which gives the copy its place) the one with the Message-ID
    /// it was last saved with, as this computer keeps Drafts (a sync may have listed it there
    /// before the list read is told).
    pub(super) fn saved_copy(&self, c: &Compose) -> Option<MessageKey> {
        c.draft.clone().or_else(|| {
            let folder = &self.account(&c.account)?.folder(FolderRole::Drafts)?.path;
            let id = norm_id(&c.draft_id);
            if id.is_empty() {
                return None;
            }
            self.kept_with_id(&c.account, folder, &id)
        })
    }

    /// The message with Message-ID `id` (as [`norm_id`] gives it) in `folder` as this computer
    /// keeps its list.
    fn kept_with_id(&self, account: &str, folder: &str, id: &str) -> Option<MessageKey> {
        with_id(&self.store.as_ref()?.load_folder(account, folder).ok()?, id)
    }
}

/// The message in `st` with Message-ID `id` (as [`norm_id`] gives it), compared the same way.
fn with_id(st: &FolderState, id: &str) -> Option<MessageKey> {
    st.messages.iter().find(|m| same_id(&m.message_id, id)).map(|m| m.key.clone())
}

/// `norm_id(found) == id`, without making a string of each of a folder's ids: one with nothing
/// for [`norm_id`] to take out (as a list keeps them) is compared as it is.
fn same_id(found: &str, id: &str) -> bool {
    let kept_whole = found.len() <= 998 && !found.chars().any(|c| matches!(c, '<' | '>') || c.is_control() || c.is_whitespace());
    if kept_whole { found == id } else { norm_id(found) == id }
}

/// The text of a saved message (`own`: its HTML without the original under it) into `c`, its
/// formatting kept, and its signature as it went in when it was written as HTML code.
fn own_text(c: &mut Compose, own: &str) {
    let mut tables = Vec::new();
    let Some((before, code, after)) = mime::split_marked(own, mime::SIGNATURE_CLASS) else {
        let (text, styles) = super::tables::text_with_tables(own, &mut tables);
        c.body = text;
        c.styles = styles;
        c.tables = tables;
        return;
    };
    let (mut text, mut styles) = super::tables::text_with_tables(&before, &mut tables);
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
        .filter(|p| code.contains(&format!("cid:{}", p.cid)))
        .find_map(|p| mime::signature_cid_tag(&p.cid))
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
    let (rest, rest_styles) = super::tables::text_with_tables(&after, &mut tables);
    if !rest.trim().is_empty() {
        mime::rich::append(&mut text, &mut styles, "\n", &mime::rich::Styles::plain("\n"));
        mime::rich::append(&mut text, &mut styles, &rest, &rest_styles);
    }
    c.body = text;
    c.styles = styles;
    c.tables = tables;
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
        lines.get(i).is_some_and(|l| is_labelled(l, mime::markup::FROM_LABEL))
            && lines.get(i + 1).is_some_and(|l| is_labelled(l, mime::markup::SENT_LABEL))
            && lines.get(i - 1).is_some_and(|l| l.trim().is_empty())
    });
    let Some(at) = at else { return };
    c.styles.fit(&c.body);
    if let Some(p) = c.styles.paras.get_mut(at - 1) {
        p.rule = true;
    }
}

/// Whether a line starts with a header label (`From:`) and the space after it, as a reply writes them.
fn is_labelled(line: &str, label: &str) -> bool {
    line.strip_prefix(label).is_some_and(|rest| rest.starts_with(' '))
}
