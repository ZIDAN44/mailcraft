//! Commands: writing messages, and attachments.

use super::*;

pub(super) fn commands() -> Vec<CommandSpec> {
    vec![
        // Compose
        CommandSpec::new("compose.list", "Messages being written", "Message", |s, _| {
            let active = s.compose.as_ref().map(|c| c.id);
            Ok(json!(s.all_composes().map(|c| json!({"id": c.id, "subject": c.subject, "to": c.to, "active": Some(c.id) == active})).collect::<Vec<_>>()))
        }),
        CommandSpec::new("compose.activate", "Switch to a message being written", "Message", |s, v| {
            let id = p::u64(v, "id").ok_or_else(|| CmdError::Params("`id` is required (from compose.list)".into()))?;
            if s.activate_compose(id) { Ok(json!({"active": id})) } else { Err(CmdError::Params(format!("no message {id} is being written"))) }
        })
        .params(r#"{"id": number (from compose.list)}"#),
        CommandSpec::new("compose.keepAll", "Keep the messages being written", "Closing MailCraft", |s, _| {
            let kept = s.keep_drafts();
            Ok(json!({ "kept": kept }))
        }),
        CommandSpec::new("compose.update", "Edit message", "Message", compose_update).leaves_body()
            .params(r#"{"to"?: string, "cc"?: string, "bcc"?: string, "subject"?: string, "body"?: string, "marks"?: bool (the body uses **bold**, *italic*, - lists…), "account"?: string, "from"?: string (one of the account's addresses; "" for its own), "importance"?: "high"|"normal"|"low", "readReceipt"?: bool, "replyTo"?: string (where replies go), "sendAfter"?: number (Unix seconds: held in the Outbox until then; 0 sends at once)}"#)
            .when(has_compose),
        CommandSpec::new("compose.checkNames", "Check Names", "Message", |s, _| s.check_names())
            .params(r#"{} (the names typed in To, Cc and Bcc that aren't addresses, looked up: one person is put in; answers {"resolved": n, "several": [{"field", "typed", "people": [{"name", "email"}]}] to pick from with compose.pickName, "none": [{"field", "typed"}], "pending": [{"field", "typed"}] (a directory still asked)})"#)
            .when(has_compose),
        CommandSpec::new("compose.pickName", "Pick a name", "Message › Check Names", |s, v| {
            let field = p::req_str(v, "field")?;
            let typed = p::req_str(v, "typed")?;
            let person = mailcraft_model::EmailAddress { name: p::str(v, "name").unwrap_or_default().to_string(), email: p::req_str(v, "email")?.to_string() };
            s.pick_name(field, typed, &person)
        })
        .params(r#"{"field": "to"|"cc"|"bcc", "typed": string (as Check Names said it), "email": string, "name"?: string}"#)
        .when(has_compose),
        CommandSpec::new("compose.attach", "Attach file", "Message › Insert", attach).params(r#"{"path": string} | {"name": string, "handed": number (a file handed over: Session::hand)} | {"name": string, "data": [bytes]}"#).when(has_compose),
        CommandSpec::new("compose.insertPicture", "Insert picture", "Message › Insert", insert_picture)
            .params(r#"{"path": string} | {"name": string, "handed": number (a file handed over: Session::hand)} | {"name": string, "data": [bytes]}, "insert"?: bool (default true), "at"?: number (character; default the end), "width"?: number (CSS pixels it's shown and sent at; default its own width), "line"?: bool (on a line of its own), "quoteCell"?: number (into that block of text of the quoted original, `at` counted in it)"#)
            .when(has_compose),
        CommandSpec::new("compose.resizePicture", "Picture size", "Message › Picture", resize_picture)
            .params(r#"{"cid": string, "width"?: number (CSS pixels; 0 or none: its own width)}"#)
            .when(has_compose),
        CommandSpec::new("compose.signaturePictures", "Show the signature's pictures", "Message › Signature", |s, _| s.show_signature_pictures())
            .when(has_compose),
        CommandSpec::new("compose.format", "Plain text / Formatted", "Message › Format", |s, v| {
            let c = s.composing_mut()?;
            c.plain = p::bool(v, "plain").unwrap_or(!c.plain);
            Ok(json!({"plain": c.plain}))
        })
        .params(r#"{"plain"?: bool}"#)
        .when(has_compose),
        CommandSpec::new("draft.edit", "Edit draft", "Drafts", |s, v| {
            let key = parse_key(v).ok_or_else(|| CmdError::Params("`key` is required (account:validity:uid:folder)".into()))?;
            s.edit_draft(&key)
        })
        .params(r#"{"key": string (a message in Drafts)}"#),
        CommandSpec::new("compose.removeAttachment", "Remove attachment", "Message › Insert", remove_attachment).params(r#"{"index": number}"#).when(has_compose),
        CommandSpec::new("compose.send", "Send", "Message", |s, v| {
            s.send_compose(p::bool(v, "allowEmptySubject").unwrap_or(false), p::bool(v, "checked").unwrap_or(false))
        })
        .key("Mod+Enter / Alt+S")
        .params(r#"{"allowEmptySubject"?: bool, "checked"?: bool (without it, a message to a lookalike address or showing many people each other's addresses is held, and the answer is {"check": [...]})}"#)
        .when(can_send_compose),
        CommandSpec::new("compose.hideRecipients", "Put everyone in Bcc", "Message", |s, _| s.hide_recipients()).when(has_compose),
        CommandSpec::new("compose.saveDraft", "Save draft", "Message", |s, _| s.save_draft()).key("Mod+S").when(has_compose),
        CommandSpec::new("compose.discard", "Discard", "Message", discard).when(has_compose),
        CommandSpec::new("compose.popOut", "Pop out", "Message", |s, _| pop_out(s, true)).when(has_compose),
        CommandSpec::new("compose.popIn", "Pop in", "Message", |s, _| pop_out(s, false)).when(has_compose),
        CommandSpec::new("compose.close", "Close", "Message", close_compose).params(r#"{"save"?: bool (keep it in Drafts; default true)}"#).when(has_compose),
        CommandSpec::new("contacts.suggest", "Suggest recipients", "Message › To", |s, v| {
            let q = p::req_str(v, "query")?.to_string();
            let limit = p::u64(v, "limit").map_or(8, |n| usize::try_from(n).unwrap_or(8).clamp(1, 100));
            let list: Vec<Value> = s
                .suggest_contacts(&q, limit)
                .into_iter()
                .map(|c| json!({"name": c.address.name, "email": c.address.email, "address": c.address.to_string(), "sent": c.sent, "received": c.received}))
                .collect();
            Ok(json!(list))
        })
        .params(r#"{"query": string, "limit"?: number}"#),
        // Attachments
        CommandSpec::new("attachment.save", "Save attachment", "Reading pane", save_attachment)
            .params(r#"{"index": number, "path": string, "open"?: bool (saved to be opened by another app: never a file that runs code)}"#)
            .when(has_reading),
        CommandSpec::new("attachment.saveAll", "Save all attachments", "Reading pane", save_all_attachments).params(r#"{"dir": string, "indices"?: [number] (only these attachments)}"#).when(has_reading),
        CommandSpec::new("attachment.preview", "Preview attachment", "Reading pane", preview_attachment).params(r#"{"index": number}"#).when(has_reading),
        CommandSpec::new("picture.save", "Save picture", "Reading pane › Picture", save_picture)
            .params(r#"{"key": string (an open message), "src": string (the picture as the message names it), "path": string}"#),
    ]
}

/// The message being written in a window of its own (`window`), or back in the main one.
fn pop_out(s: &mut Session, window: bool) -> CmdResult {
    s.composing_mut()?.window = window;
    Ok(json!({ "window": window }))
}

pub(super) fn compose_update(s: &mut Session, v: &Value) -> CmdResult {
    let account = p::str(v, "account").map(str::to_string);
    if let Some(a) = &account
        && s.account(a).is_none()
    {
        return Err(CmdError::Params(format!("no account `{a}`")));
    }
    // Which of the account's addresses it's from (issue #81): the account's own is none.
    let from = match p::str(v, "from") {
        Some(address) => {
            let of = account.clone().or_else(|| s.compose.as_ref().map(|c| c.account.clone())).unwrap_or_default();
            let config = s.account(&of).map(|a| &a.config).ok_or_else(|| CmdError::Disabled(NOT_WRITING.into()))?;
            match config.identity(address) {
                Some(i) => Some(i.email.clone()),
                None if address.trim().is_empty() || config.email.eq_ignore_ascii_case(address.trim()) => Some(String::new()),
                None => return Err(CmdError::Params(format!("`{}` isn't one of this account's addresses", address.trim()))),
            }
        }
        None => None,
    };
    // The address another account writes its new mail from (issue #88).
    let new_mail_from = account.as_deref().and_then(|a| s.account(a)).map(|a| a.config.new_mail_from().to_string()).unwrap_or_default();
    let c = s.composing_mut()?;
    if c.sending {
        return Err(CmdError::Disabled("the message is being sent".into()));
    }
    // Every param that can be refused is read before anything changes: a refusal changes nothing.
    let importance = match p::str(v, "importance") {
        None => None,
        Some("high") => Some(1),
        Some("low") => Some(-1),
        Some("normal") => Some(0),
        Some(other) => return Err(CmdError::Params(format!("importance `{other}` isn't high, normal or low"))),
    };
    let cap = |x: &str| x.chars().take(1 << 20).collect::<String>();
    for (k, field) in [("to", &mut c.to), ("cc", &mut c.cc), ("bcc", &mut c.bcc), ("subject", &mut c.subject)] {
        if let Some(x) = p::str(v, k) {
            *field = cap(x);
        }
    }
    if let Some(x) = p::str(v, "body") {
        if p::bool(v, "marks").unwrap_or(false) {
            let (text, styles) = mailcraft_mime::rich::from_markup(&cap(x));
            c.body = text;
            c.styles = styles;
        } else {
            c.set_text(cap(x));
        }
    }
    if !c.bcc.is_empty() {
        c.show_bcc = true;
    }
    if let Some(i) = importance {
        c.importance = i;
    }
    if let Some(r) = p::bool(v, "readReceipt") {
        c.read_receipt = r;
    }
    if let Some(x) = p::str(v, "replyTo") {
        c.reply_to = cap(x);
    }
    if let Some(at) = p::i64(v, "sendAfter") {
        c.send_after = (at > 0).then_some(at);
    }
    let id = c.id;
    // The signature follows the account it's sent from (issue #11), and the address.
    if account.is_some() || from.is_some() {
        let (was, was_from) = (c.account.clone(), c.from.clone());
        if let Some(a) = account {
            // Another account's addresses aren't this one's: the one that account writes new
            // mail from (an answer goes from its own), unless one is picked.
            if a != c.account {
                c.from = if c.mode.is_none() { new_mail_from } else { String::new() };
            }
            c.account = a;
        }
        if let Some(f) = from {
            c.from = f;
        }
        s.follow_from(&was, &was_from);
        s.signing_follows_from(&was, &was_from);
    }
    Ok(json!({"compose": id}))
}

/// Largest attachment accepted (most servers refuse messages over 25–35 MB).
pub(super) const ATTACH_MAX: usize = 25 << 20;
/// [`ATTACH_MAX`] in MB, as the messages say it.
const ATTACH_MAX_MB: usize = ATTACH_MAX >> 20;

/// Most files kept handed over at once ([`Session::hand`]): each command takes its own.
const HANDED_KEPT: usize = 4;

impl Session {
    /// A file's bytes handed over by the window (one picked in the browser, a picture pasted)
    /// for the command that follows, which names it `"handed": n` (and takes it, or it's let go
    /// of when that command ends). Given as `"data"`, a JSON list
    /// with a value per byte (24 bytes each on wasm32), a 25 MB file took about 600 MB.
    pub fn hand(&mut self, data: Vec<u8>) -> u64 {
        self.last_handed = self.last_handed.wrapping_add(1);
        if self.handed.len() >= HANDED_KEPT {
            self.handed.remove(0);
        }
        self.handed.push((self.last_handed, data));
        self.last_handed
    }

    /// The file `v` names with `"handed"`, taken (each is used once); an error when it names one
    /// that isn't here, `None` when it names none.
    pub(crate) fn take_handed(&mut self, v: &Value) -> Result<Option<Vec<u8>>, CmdError> {
        let Some(n) = v.get("handed") else { return Ok(None) };
        let i = n.as_u64().and_then(|n| self.handed.iter().position(|(k, _)| *k == n));
        let i = i.ok_or_else(|| CmdError::Params("`handed`: no such file was handed over".into()))?;
        Ok(Some(self.handed.remove(i).1))
    }

    /// Let go of the file `v` names with `"handed"`, when it's still here: its command ended
    /// without taking it (unknown, not available now, or refused before it read its file), and
    /// nothing else will. It stayed, up to 25 MB of it, until four more were handed over.
    pub(crate) fn drop_handed(&mut self, v: &Value) {
        if let Some(n) = v.get("handed").and_then(Value::as_u64) {
            self.handed.retain(|(k, _)| *k != n);
        }
    }
}

/// A file from `path`, or `name` and the bytes handed over (`handed`) or given (`data`), at most
/// [`ATTACH_MAX`].
pub(super) fn file_param(s: &mut Session, v: &Value) -> Result<(String, Vec<u8>), CmdError> {
    let too_big = || CmdError::Failed(format!("attachments are limited to {ATTACH_MAX_MB} MB"));
    let (name, data) = if let Some(path) = p::str(v, "path") {
        let pb = std::path::PathBuf::from(path);
        let data = read_within(path, ATTACH_MAX as u64, || CmdError::Failed(format!("{path} is larger than {ATTACH_MAX_MB} MB")))?;
        (pb.file_name().and_then(|n| n.to_str()).unwrap_or("attachment").to_string(), data)
    } else if let Some(data) = s.take_handed(v)? {
        (p::req_str(v, "name")?.to_string(), data)
    } else {
        let name = p::req_str(v, "name")?.to_string();
        if v.get("data").and_then(Value::as_array).is_some_and(|list| list.len() > ATTACH_MAX) {
            return Err(too_big());
        }
        let data = p::bytes(v, "data", ATTACH_MAX)?.ok_or_else(|| CmdError::Params("`path`, or `name` and `data`, are required".into()))?;
        (name, data)
    };
    if data.len() > ATTACH_MAX {
        return Err(too_big());
    }
    Ok((name, data))
}

/// What a message being written carries, in bytes.
pub(super) fn compose_size(c: &crate::Compose) -> usize {
    c.attachments.iter().map(|a| a.data.len()).sum::<usize>() + c.inline.iter().map(|p| p.data.len()).sum::<usize>()
}

pub(super) fn attach(s: &mut Session, v: &Value) -> CmdResult {
    let (name, data) = file_param(s, v)?;
    let c = s.composing_mut()?;
    let total: usize = compose_size(c) + data.len();
    if total > ATTACH_MAX {
        return Err(CmdError::Failed(format!("attachments are limited to {ATTACH_MAX_MB} MB in total")));
    }
    let name = sanitize_filename(&name);
    let mime = mime_from_name(&name).to_string();
    c.attachments.push(OutAttachment { name: name.clone(), mime, data, ..Default::default() });
    Ok(json!({"attached": name, "count": c.attachments.len()}))
}

/// Place a picture in the text. With `"insert": false` the mark isn't added to the text (the
/// editor puts it at the cursor itself).
pub(super) fn insert_picture(s: &mut Session, v: &Value) -> CmdResult {
    let (name, data) = file_param(s, v)?;
    let name = sanitize_filename(&name);
    let mime = mime_from_name(&name).to_string();
    if !mime.starts_with("image/") {
        return Err(CmdError::Params(format!("{name} isn't a picture")));
    }
    let cid = s.new_cid();
    let c = s.composing_mut()?;
    if compose_size(c) + data.len() > ATTACH_MAX {
        return Err(CmdError::Failed(format!("pictures and attachments are limited to {ATTACH_MAX_MB} MB in total")));
    }
    c.inline.push(mailcraft_mime::InlinePicture { cid: cid.clone(), name, mime, data });
    c.plain = false;
    if !p::bool(v, "insert").unwrap_or(true) {
        return Ok(json!({"cid": cid}));
    }
    let place = |c: &mut crate::session::Compose| {
        let n = c.body.chars().count();
        let mut at = p::u64(v, "at").and_then(|a| usize::try_from(a).ok()).unwrap_or(n).min(n);
        // On a line of its own: a line break before it and after it, where there's text there.
        if p::bool(v, "line").unwrap_or(false) {
            if at > 0 && c.body.chars().nth(at - 1) != Some('\n') {
                insert_text_at(c, at, "\n");
                at += 1;
            }
            if c.body.chars().nth(at).is_some_and(|ch| ch != '\n') {
                insert_text_at(c, at, "\n");
            }
        }
        insert_picture_at(c, at, &cid, picture_width(v));
        at
    };
    // Into a block of text of the original under a reply, where its cursor is.
    let at = match p::u64(v, "quoteCell").and_then(|n| usize::try_from(n).ok()) {
        Some(n) => c.in_quote_cell(n, place).ok_or_else(|| CmdError::Params(format!("the quote has no block of text {n}")))?,
        None => place(c),
    };
    Ok(json!({"cid": cid, "end": at + 1}))
}

/// Text into the message at character `at`, in the look of what's around it.
fn insert_text_at(c: &mut crate::session::Compose, at: usize, text: &str) {
    let old = c.body.clone();
    let byte = old.char_indices().nth(at).map_or(old.len(), |(b, _)| b);
    c.body.insert_str(byte, text);
    let mut styles = std::mem::take(&mut c.styles);
    let around = styles.style(at.saturating_sub(1)).for_typing();
    styles.edit_at(&old, &c.body, &around, Some(at + text.chars().count()));
    c.styles = styles;
}

/// Narrowest and widest a picture is set to, in CSS pixels.
pub const PICTURE_WIDTHS: std::ops::RangeInclusive<u16> = 16..=4000;

/// The `width` asked for a picture, within [`PICTURE_WIDTHS`]; `None` (its own width) when
/// there's none, or 0.
fn picture_width(v: &Value) -> Option<u16> {
    let w = p::u64(v, "width").filter(|w| *w > 0)?;
    Some(u16::try_from(w).unwrap_or(u16::MAX).clamp(*PICTURE_WIDTHS.start(), *PICTURE_WIDTHS.end()))
}

/// Show and send picture `cid` at another width (Outlook's picture handles and Size), or at its
/// own again.
pub(super) fn resize_picture(s: &mut Session, v: &Value) -> CmdResult {
    use mailcraft_mime::rich::{PICTURE, Style};
    let cid = p::req_str(v, "cid")?;
    let width = picture_width(v);
    let c = s.composing_mut()?;
    c.styles.fit(&c.body);
    let at: Vec<usize> =
        c.body.chars().enumerate().filter(|(i, ch)| *ch == PICTURE && c.styles.style(*i).picture.as_deref() == Some(cid)).map(|(i, _)| i).collect();
    if at.is_empty() {
        return Err(CmdError::Params(format!("no picture {cid} in the message")));
    }
    let mut styles = std::mem::take(&mut c.styles);
    for i in at {
        styles.restyle(&c.body, i..i + 1, |s| Style { picture_width: width, ..s.clone() });
    }
    c.styles = styles;
    Ok(json!({"cid": cid, "width": width}))
}

/// Put picture `cid` into the text at character `at`, `width` wide (`None`: its own width).
pub fn insert_picture_at(c: &mut crate::session::Compose, at: usize, cid: &str, width: Option<u16>) {
    use mailcraft_mime::rich::{PICTURE, Style};
    let old = c.body.clone();
    let byte = old.char_indices().nth(at).map_or(old.len(), |(b, _)| b);
    c.body.insert(byte, PICTURE);
    let mut styles = std::mem::take(&mut c.styles);
    styles.edit_at(&old, &c.body, &Style::default(), Some(at + 1));
    styles.restyle(&c.body, at..at + 1, |s| Style { picture: Some(cid.to_string()), picture_width: width, ..s.clone() });
    c.styles = styles;
}

pub(super) fn remove_attachment(s: &mut Session, v: &Value) -> CmdResult {
    let i = p::u64(v, "index").ok_or_else(|| CmdError::Params("`index` is required".into()))?;
    let c = s.composing_mut()?;
    let i = usize::try_from(i).map_err(|_| CmdError::Params("bad index".into()))?;
    if i >= c.attachments.len() {
        return Err(CmdError::Params(format!("no attachment {i}")));
    }
    let a = c.attachments.remove(i);
    Ok(json!({"removed": a.name}))
}

/// Close the message being written: kept in Drafts (unless `save` is false or nothing was
/// written), as closing a message window in Outlook offers.
pub(super) fn close_compose(s: &mut Session, v: &Value) -> CmdResult {
    // What's needed of it, not a copy of it all (attachments and pictures, up to 25 MB).
    let (id, account, sending) = s.composing().map(|c| (c.id, c.account.clone(), c.sending))?;
    let written = s.compose_written();
    let saved = if p::bool(v, "save").unwrap_or(true) && written && !sending {
        s.save_draft()?;
        true
    } else {
        false
    };
    if saved {
        // Said saved once it is: on this computer at once; a server's save has only begun, and
        // the message is held until it ends (#53), coming back with the reason if it fails.
        if s.account(&account).is_none_or(|a| a.is_local()) {
            s.notice("Saved in Drafts", false);
        }
        s.let_go_saved();
    } else {
        s.let_go_compose();
    }
    Ok(json!({"closed": id, "saved": saved}))
}

pub(super) fn discard(s: &mut Session, _: &Value) -> CmdResult {
    let c = s.let_go_compose();
    Ok(json!({"discarded": c.map(|c| c.id)}))
}

impl Session {
    /// The open message's attachment `index`, as bytes.
    pub fn attachment_bytes(&self, index: usize) -> Result<Vec<u8>, CmdError> {
        let r = self.reading.as_ref().ok_or_else(|| CmdError::Disabled("open a message first".into()))?;
        if r.text_first {
            return Err(CmdError::Failed(crate::session::STILL_COMING.into()));
        }
        // A signed or encrypted message's attachments are the ones inside it.
        let raw = self.readable_raw(&r.key).ok_or_else(|| CmdError::Failed("the message isn't downloaded".into()))?;
        mailcraft_mime::attachment_data(&raw, index).ok_or_else(|| CmdError::Params(format!("no attachment {index}")))
    }

    /// The open message's source (`.eml`).
    pub fn message_source(&self) -> Result<Vec<u8>, CmdError> {
        let r = self.reading.as_ref().ok_or_else(|| CmdError::Disabled("open a message first".into()))?;
        self.body_raw(&r.key).ok_or_else(|| CmdError::Failed("the message isn't downloaded".into()))
    }
}

pub(super) fn save_attachment(s: &mut Session, v: &Value) -> CmdResult {
    let index = p::u64(v, "index").and_then(|i| usize::try_from(i).ok()).ok_or_else(|| CmdError::Params("`index` is required".into()))?;
    let path = p::req_str(v, "path")?;
    if p::bool(v, "open").unwrap_or(false) {
        refuse_runnable(path)?;
    }
    let data = s.attachment_bytes(index)?;
    std::fs::write(path, &data).map_err(|e| CmdError::Failed(format!("{path}: {e}")))?;
    mark_from_internet(std::path::Path::new(path), &data);
    Ok(json!({"saved": path, "bytes": data.len()}))
}

/// A picture a message open shows, saved (issue #110): one it carries, or one from the web loaded.
fn save_picture(s: &mut Session, v: &Value) -> CmdResult {
    let key = parse_key(v).ok_or_else(|| CmdError::Params("`key` is required (account:validity:uid:folder)".into()))?;
    let src = p::req_str(v, "src")?;
    let path = p::req_str(v, "path")?;
    let data = s.picture_bytes(&key, src).ok_or_else(|| CmdError::Disabled("that picture isn't here".into()))?;
    std::fs::write(path, &data).map_err(|e| CmdError::Failed(format!("{path}: {e}")))?;
    mark_from_internet(std::path::Path::new(path), &data);
    Ok(json!({"saved": path, "bytes": data.len()}))
}

/// A file saved at `path` to be opened by another app: refused when it could run a program (a
/// program, script or shortcut opened from a message would run), never handed on.
pub(super) fn refuse_runnable(path: &str) -> Result<(), CmdError> {
    let name = std::path::Path::new(path).file_name().unwrap_or_default().to_string_lossy().into_owned();
    if mailcraft_mime::runs_code(&name) {
        return Err(CmdError::Disabled(format!("{name} could run a program, so it isn't opened from here; save it and check it first")));
    }
    Ok(())
}

/// Windows' mark that a file came from the internet (its `Zone.Identifier` stream, zone 3), as
/// browsers and mail programs add: SmartScreen checks a program before it runs, and Office opens
/// the file in Protected View. Not on a picture that is one, or plain text: there it guards
/// nothing, and Windows' Photos asks "Is this file from a trusted source?" before every picture
/// that has it (when MailCraft hands it over). Writing a file keeps the streams it had, so on such
/// a file a mark an earlier copy at its name left (an older MailCraft marked pictures too) is
/// removed. Best effort; nothing elsewhere.
pub(super) fn mark_from_internet(path: &std::path::Path, data: &[u8]) {
    if !cfg!(windows) {
        return;
    }
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let mut stream = path.as_os_str().to_owned();
    stream.push(":Zone.Identifier");
    if mailcraft_mime::harmless_to_open(&name, data) {
        let _ = std::fs::remove_file(stream);
    } else {
        let _ = std::fs::write(
            stream,
            "[ZoneTransfer]
ZoneId=3
",
        );
    }
}

/// Save every attachment (not the pictures inside the text) into a folder; names that are
/// taken get a number.
pub(super) fn save_all_attachments(s: &mut Session, v: &Value) -> CmdResult {
    let dir = std::path::PathBuf::from(p::req_str(v, "dir")?);
    if !dir.is_dir() {
        return Err(CmdError::Params(format!("{} is not a folder", dir.display())));
    }
    let r = s.reading.as_ref().ok_or_else(|| CmdError::Disabled("open a message first".into()))?;
    let parsed = r.parsed.as_ref().ok_or_else(|| CmdError::Disabled("the message is still downloading".into()))?;
    if r.text_first {
        return Err(CmdError::Failed(crate::session::STILL_COMING.into()));
    }
    let raw = s.readable_raw(&r.key).ok_or_else(|| CmdError::Failed("the message isn't downloaded".into()))?;
    // Only some of them, when asked (the ones selected).
    let only: Option<Vec<u64>> = v.get("indices").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_u64).take(10_000).collect());
    let names: std::collections::HashMap<usize, &str> = parsed
        .attachments
        .iter()
        .filter(|a| !a.inline && only.as_ref().is_none_or(|o| o.contains(&(a.index as u64))))
        .map(|a| (a.index, a.name.as_str()))
        .collect();
    let wanted: Vec<usize> = names.keys().copied().collect();
    let (mut saved, mut failed) = (Vec::new(), None);
    // The message parsed once for all of them (it was once per file: a 25 MB message with 30
    // files read 30 times), each file written as it's decoded.
    mailcraft_mime::attachments_moved(&raw, &wanted, &mut |index, data| {
        let Some(name) = names.get(&index).filter(|_| failed.is_none()) else { return };
        match save_beside(&dir, name, &data) {
            Ok(path) => saved.push(path),
            Err(e) => failed = Some(e),
        }
    });
    match failed {
        Some(e) => Err(e),
        None => Ok(json!({"saved": saved})),
    }
}

/// Highest number a saved file's name is given when the name is taken (`name (999).txt`).
const NUMBERED_MAX: u32 = 999;

/// `data` saved in `dir` as `name` (made safe), or with a number when that's taken; refused when
/// every number is taken too, so that no file there is written over.
fn save_beside(dir: &std::path::Path, name: &str, data: &[u8]) -> Result<String, CmdError> {
    let name = sanitize_filename(name);
    let (stem, ext) = name.rsplit_once('.').map_or((name.clone(), String::new()), |(s, e)| (s.to_string(), format!(".{e}")));
    let path = std::iter::once(dir.join(&name))
        .chain((2..=NUMBERED_MAX).map(|n| dir.join(format!("{stem} ({n}){ext}"))))
        .find(|p| !p.exists())
        .ok_or_else(|| CmdError::Failed(format!("{}: {name} and every numbered copy up to ({NUMBERED_MAX}) are there already", dir.display())))?;
    std::fs::write(&path, data).map_err(|e| CmdError::Failed(format!("{}: {e}", path.display())))?;
    mark_from_internet(&path, data);
    Ok(path.to_string_lossy().to_string())
}

/// An attachment as text, for previewing (`message` kind: an attached email, parsed).
pub(super) fn preview_attachment(s: &mut Session, v: &Value) -> CmdResult {
    let index = p::u64(v, "index").and_then(|i| usize::try_from(i).ok()).ok_or_else(|| CmdError::Params("`index` is required".into()))?;
    let r = s.reading.as_ref().ok_or_else(|| CmdError::Disabled("open a message first".into()))?;
    let parsed = r.parsed.as_ref().ok_or_else(|| CmdError::Disabled("the message is still downloading".into()))?;
    let a = parsed.attachments.iter().find(|a| a.index == index).ok_or_else(|| CmdError::Params(format!("no attachment {index}")))?;
    let kind = mailcraft_mime::text_preview_kind(&a.mime, &a.name).ok_or_else(|| CmdError::Failed(format!("{} can't be shown as text", a.name)))?;
    if r.text_first {
        return Err(CmdError::Failed(crate::session::STILL_COMING.into()));
    }
    let raw = s.readable_raw(&r.key).ok_or_else(|| CmdError::Failed("the message isn't downloaded".into()))?;
    let unreadable = || CmdError::Failed("the attachment can't be read".into());
    if kind == "message" {
        let data = mailcraft_mime::attachment_data(&raw, index).ok_or_else(unreadable)?;
        let name = a.name.clone();
        return super::message_preview(s, &name, &data);
    }
    // Read in the charset the file names (its saved bytes are as they came); at most 512 KB, cut
    // on a character boundary.
    const MAX: usize = 512 << 10;
    let mut text = mailcraft_mime::attachment_text(&raw, index).ok_or_else(unreadable)?;
    let truncated = text.len() > MAX;
    if truncated {
        let cut = (0..=MAX).rev().find(|i| text.is_char_boundary(*i)).unwrap_or(0);
        text.truncate(cut);
    }
    Ok(json!({"kind": "text", "name": a.name, "text": text, "truncated": truncated}))
}

/// The open message's source, as an `.eml` file.
pub(super) fn save_message(s: &mut Session, v: &Value) -> CmdResult {
    let path = p::req_str(v, "path")?;
    let raw = s.message_source()?;
    std::fs::write(path, &raw).map_err(|e| CmdError::Failed(format!("{path}: {e}")))?;
    Ok(json!({"saved": path, "bytes": raw.len()}))
}

/// The open message as a PDF.
pub(super) fn export_pdf(s: &mut Session, v: &Value) -> CmdResult {
    let path = p::req_str(v, "path")?;
    let bytes = s.message_pdf()?;
    std::fs::write(path, &bytes).map_err(|e| CmdError::Failed(format!("{path}: {e}")))?;
    Ok(json!({"saved": path, "bytes": bytes.len()}))
}
