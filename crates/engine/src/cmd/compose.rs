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
        CommandSpec::new("compose.update", "Edit message", "Message", compose_update)
            .params(r#"{"to"?: string, "cc"?: string, "bcc"?: string, "subject"?: string, "body"?: string, "marks"?: bool (the body uses **bold**, *italic*, - lists…), "account"?: string, "importance"?: "high"|"normal"|"low", "readReceipt"?: bool, "replyTo"?: string (where replies go), "sendAfter"?: number (Unix seconds: held in the Outbox until then; 0 sends at once)}"#)
            .when(has_compose),
        CommandSpec::new("compose.attach", "Attach file", "Message › Insert", attach).params(r#"{"path": string} | {"name": string, "data": [bytes]}"#).when(has_compose),
        CommandSpec::new("compose.insertPicture", "Insert picture", "Message › Insert", insert_picture)
            .params(r#"{"path": string} | {"name": string, "data": [bytes]}, "insert"?: bool (default true), "at"?: number (character; default the end)"#)
            .when(has_compose),
        CommandSpec::new("compose.format", "Plain text / Formatted", "Message › Format", |s, v| {
            let c = s.compose.as_mut().ok_or_else(|| CmdError::Disabled("no message is being written".into()))?;
            c.plain = p::bool(v, "plain").unwrap_or(!c.plain);
            Ok(json!({"plain": c.plain}))
        })
        .params(r#"{"plain"?: bool}"#)
        .when(has_compose),
        CommandSpec::new("compose.removeAttachment", "Remove attachment", "Message › Insert", remove_attachment).params(r#"{"index": number}"#).when(has_compose),
        CommandSpec::new("compose.send", "Send", "Message", |s, v| s.send_compose(p::bool(v, "allowEmptySubject").unwrap_or(false)))
            .key("Mod+Enter / Alt+S")
            .params(r#"{"allowEmptySubject"?: bool}"#)
            .when(has_compose),
        CommandSpec::new("compose.saveDraft", "Save draft", "Message", |s, _| s.save_draft()).key("Mod+S").when(has_compose),
        CommandSpec::new("compose.discard", "Discard", "Message", discard).when(has_compose),
        CommandSpec::new("compose.popOut", "Pop out", "Message", |s, _| {
            let c = s.compose.as_mut().ok_or_else(|| CmdError::Disabled("no message is being written".into()))?;
            c.window = true;
            Ok(json!({"window": true}))
        })
        .when(has_compose),
        CommandSpec::new("compose.popIn", "Pop in", "Message", |s, _| {
            let c = s.compose.as_mut().ok_or_else(|| CmdError::Disabled("no message is being written".into()))?;
            c.window = false;
            Ok(json!({"window": false}))
        })
        .when(has_compose),
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
    ]
}

pub(super) fn compose_update(s: &mut Session, v: &Value) -> CmdResult {
    let account = p::str(v, "account").map(str::to_string);
    if let Some(a) = &account
        && s.account(a).is_none()
    {
        return Err(CmdError::Params(format!("no account `{a}`")));
    }
    let c = s.compose.as_mut().ok_or_else(|| CmdError::Disabled("no message is being written".into()))?;
    if c.sending {
        return Err(CmdError::Disabled("the message is being sent".into()));
    }
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
        c.show_cc = true;
    }
    if let Some(i) = p::str(v, "importance") {
        c.importance = match i {
            "high" => 1,
            "low" => -1,
            "normal" => 0,
            other => return Err(CmdError::Params(format!("importance `{other}` isn't high, normal or low"))),
        };
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
    if let Some(a) = account {
        c.account = a;
    }
    Ok(json!({"compose": c.id}))
}

/// Largest attachment accepted (most servers refuse messages over 25–35 MB).
pub(super) const ATTACH_MAX: usize = 25 << 20;

/// A file from `path`, or `name` and `data` (bytes), at most 25 MB.
pub(super) fn file_param(v: &Value) -> Result<(String, Vec<u8>), CmdError> {
    let (name, data) = if let Some(path) = p::str(v, "path") {
        let pb = std::path::PathBuf::from(path);
        let data = read_capped(path, ATTACH_MAX as u64).map_err(|e| match e {
            CmdError::Params(m) if m.ends_with("too big") => CmdError::Failed(format!("{path} is larger than 25 MB")),
            other => other,
        })?;
        (pb.file_name().and_then(|n| n.to_str()).unwrap_or("attachment").to_string(), data)
    } else {
        let name = p::req_str(v, "name")?.to_string();
        let data: Vec<u8> = v
            .get("data")
            .and_then(Value::as_array)
            .ok_or_else(|| CmdError::Params("`path`, or `name` and `data`, are required".into()))?
            .iter()
            .take(ATTACH_MAX + 1)
            .map(|b| b.as_u64().and_then(|x| u8::try_from(x).ok()).ok_or_else(|| CmdError::Params("`data` must be bytes".into())))
            .collect::<Result<_, _>>()?;
        (name, data)
    };
    if data.len() > ATTACH_MAX {
        return Err(CmdError::Failed("attachments are limited to 25 MB".into()));
    }
    Ok((name, data))
}

/// What a message being written carries, in bytes.
pub(super) fn compose_size(c: &crate::Compose) -> usize {
    c.attachments.iter().map(|a| a.data.len()).sum::<usize>() + c.inline.iter().map(|p| p.data.len()).sum::<usize>()
}

pub(super) fn attach(s: &mut Session, v: &Value) -> CmdResult {
    let (name, data) = file_param(v)?;
    let c = s.compose.as_mut().ok_or_else(|| CmdError::Disabled("no message is being written".into()))?;
    let total: usize = compose_size(c) + data.len();
    if total > ATTACH_MAX {
        return Err(CmdError::Failed("attachments are limited to 25 MB in total".into()));
    }
    let name = sanitize_filename(&name);
    let mime = mime_from_name(&name).to_string();
    c.attachments.push(OutAttachment { name: name.clone(), mime, data });
    Ok(json!({"attached": name, "count": c.attachments.len()}))
}

/// Place a picture in the text. With `"insert": false` the mark isn't added to the text (the
/// editor puts it at the cursor itself).
pub(super) fn insert_picture(s: &mut Session, v: &Value) -> CmdResult {
    let (name, data) = file_param(v)?;
    let name = sanitize_filename(&name);
    let mime = mime_from_name(&name).to_string();
    if !mime.starts_with("image/") {
        return Err(CmdError::Params(format!("{name} isn't a picture")));
    }
    let cid = s.new_cid();
    let c = s.compose.as_mut().ok_or_else(|| CmdError::Disabled("no message is being written".into()))?;
    if compose_size(c) + data.len() > ATTACH_MAX {
        return Err(CmdError::Failed("pictures and attachments are limited to 25 MB in total".into()));
    }
    c.inline.push(mailcraft_mime::InlinePicture { cid: cid.clone(), name, mime, data });
    c.plain = false;
    if p::bool(v, "insert").unwrap_or(true) {
        let n = c.body.chars().count();
        let at = p::u64(v, "at").and_then(|a| usize::try_from(a).ok()).unwrap_or(n).min(n);
        insert_picture_at(c, at, &cid);
    }
    Ok(json!({"cid": cid}))
}

/// The account's signature into the message being written, at character `at` (else the end),
/// on a line of its own after the usual `-- ` line (Outlook's Insert › Signature).
pub(super) fn insert_signature(s: &mut Session, v: &Value) -> CmdResult {
    use mailcraft_mime::rich::Style;
    let account = s.compose.as_ref().map(|c| c.account.clone()).ok_or_else(|| CmdError::Disabled("no message is being written".into()))?;
    let rich = s.signature_rich(&account);
    let sig = rich.0.trim_end().to_string();
    if sig.trim().is_empty() {
        return Err(CmdError::Params("this account has no signature yet: make one with Signatures…".into()));
    }
    let c = s.compose.as_mut().ok_or_else(|| CmdError::Disabled("no message is being written".into()))?;
    let old = c.body.clone();
    let block = crate::session::signature_block(&sig);
    // The signature put in before, still there as it was, is replaced (as choosing a signature
    // in Outlook replaces the one there); otherwise it goes in at the cursor.
    let there = if c.signature.is_empty() { None } else { old.rfind(&c.signature).map(|b| (b, b + c.signature.len())) };
    let (start, stop, text) = match there {
        Some((b, e)) => (b, e, block.clone()),
        None => {
            let n = old.chars().count();
            let at = p::u64(v, "at").and_then(|a| usize::try_from(a).ok()).unwrap_or(n).min(n);
            let b = old.char_indices().nth(at).map_or(old.len(), |(b, _)| b);
            let before = if at > 0 && old.chars().nth(at - 1) != Some('\n') { "\n" } else { "" };
            (b, b, format!("{before}{block}\n"))
        }
    };
    c.signature = block;
    c.body.replace_range(start..stop, &text);
    let at = old.get(..start).unwrap_or("").chars().count();
    let end = at + text.chars().count();
    let mut styles = std::mem::take(&mut c.styles);
    styles.edit_at(&old, &c.body, &Style::default(), Some(end));
    c.styles = styles;
    crate::session::style_signature(c, &rich);
    Ok(json!({"inserted": at, "end": end}))
}

/// Put picture `cid` into the text at character `at`.
pub fn insert_picture_at(c: &mut crate::session::Compose, at: usize, cid: &str) {
    use mailcraft_mime::rich::{PICTURE, Style};
    let old = c.body.clone();
    let byte = old.char_indices().nth(at).map_or(old.len(), |(b, _)| b);
    c.body.insert(byte, PICTURE);
    let mut styles = std::mem::take(&mut c.styles);
    styles.edit_at(&old, &c.body, &Style::default(), Some(at + 1));
    styles.restyle(&c.body, at..at + 1, |s| Style { picture: Some(cid.to_string()), ..s.clone() });
    c.styles = styles;
}

pub(super) fn remove_attachment(s: &mut Session, v: &Value) -> CmdResult {
    let i = p::u64(v, "index").ok_or_else(|| CmdError::Params("`index` is required".into()))?;
    let c = s.compose.as_mut().ok_or_else(|| CmdError::Disabled("no message is being written".into()))?;
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
    let c = s.compose.clone().ok_or_else(|| CmdError::Disabled("no message is being written".into()))?;
    let written = s.compose_written();
    let saved = if p::bool(v, "save").unwrap_or(true) && written && !c.sending {
        s.save_draft()?;
        true
    } else {
        false
    };
    s.compose = None;
    if saved {
        s.notice("Saved in Drafts", false);
    }
    Ok(json!({"closed": c.id, "saved": saved}))
}

pub(super) fn discard(s: &mut Session, _: &Value) -> CmdResult {
    let c = s.compose.take();
    Ok(json!({"discarded": c.map(|c| c.id)}))
}

impl Session {
    /// The open message's attachment `index`, as bytes.
    pub fn attachment_bytes(&self, index: usize) -> Result<Vec<u8>, CmdError> {
        let r = self.reading.as_ref().ok_or_else(|| CmdError::Disabled("open a message first".into()))?;
        let raw = self.store.as_ref().and_then(|st| st.read_body(&r.key)).ok_or_else(|| CmdError::Failed("the message isn't downloaded".into()))?;
        mailcraft_mime::attachment_data(&raw, index).ok_or_else(|| CmdError::Params(format!("no attachment {index}")))
    }

    /// The open message's source (`.eml`).
    pub fn message_source(&self) -> Result<Vec<u8>, CmdError> {
        let r = self.reading.as_ref().ok_or_else(|| CmdError::Disabled("open a message first".into()))?;
        if r.key.folder == crate::session::OUTBOX {
            self.outbox_entry(&r.key).and_then(|(a, i)| self.store.as_ref().and_then(|st| st.outbox_raw(&a, i.id)))
        } else {
            self.store.as_ref().and_then(|st| st.read_body(&r.key))
        }
        .ok_or_else(|| CmdError::Failed("the message isn't downloaded".into()))
    }

    /// The open message as a PDF: subject, header, text and the names of its attachments.
    pub fn message_pdf(&self) -> Result<Vec<u8>, CmdError> {
        let r = self.reading.as_ref().ok_or_else(|| CmdError::Disabled("open a message first".into()))?;
        let m = r.parsed.as_ref().ok_or_else(|| CmdError::Disabled("the message is still downloading".into()))?;
        let mut header = vec![("From".to_string(), m.from.to_string())];
        if !m.to.is_empty() {
            header.push(("To".into(), mailcraft_model::addr::format_list(&m.to)));
        }
        if !m.cc.is_empty() {
            header.push(("Cc".into(), mailcraft_model::addr::format_list(&m.cc)));
        }
        if m.date > 0 {
            header.push(("Date".into(), mailcraft_model::dates::full_time(m.date, &self.clock)));
        }
        let files: Vec<String> = m.attachments.iter().filter(|a| !a.inline).map(|a| a.name.clone()).collect();
        if !files.is_empty() {
            header.push(("Attachments".into(), files.join(", ")));
        }
        let doc = mailcraft_print::Doc { title: m.subject.clone(), header, body: m.text.clone() };
        let fonts = self.print_fonts.as_ref().map(|(r, b)| mailcraft_print::Fonts { regular: r, bold: b.as_deref() });
        mailcraft_print::pdf(&doc, fonts).map_err(CmdError::Failed)
    }
}

pub(super) fn save_attachment(s: &mut Session, v: &Value) -> CmdResult {
    let index = p::u64(v, "index").and_then(|i| usize::try_from(i).ok()).ok_or_else(|| CmdError::Params("`index` is required".into()))?;
    let path = p::req_str(v, "path")?;
    // Opened from a message, a program, script or shortcut would run: never handed on.
    if p::bool(v, "open").unwrap_or(false) {
        let leaf = std::path::Path::new(path).file_name().unwrap_or_default();
        let name = leaf.to_string_lossy().into_owned();
        if mailcraft_mime::runs_code(&name) {
            return Err(CmdError::Disabled(format!("{name} could run a program, so it isn't opened from here; save it and check it first")));
        }
    }
    let data = s.attachment_bytes(index)?;
    std::fs::write(path, &data).map_err(|e| CmdError::Failed(format!("{path}: {e}")))?;
    mark_from_internet(std::path::Path::new(path));
    Ok(json!({"saved": path, "bytes": data.len()}))
}

/// Windows' mark that a file came from the internet (its `Zone.Identifier` stream, zone 3), as
/// browsers and mail programs add: SmartScreen checks a program before it runs, and Office opens
/// the file in Protected View. Best effort; nothing elsewhere.
fn mark_from_internet(path: &std::path::Path) {
    if cfg!(windows) {
        let mut stream = path.as_os_str().to_owned();
        stream.push(":Zone.Identifier");
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
    let raw = s.store.as_ref().and_then(|st| st.read_body(&r.key)).ok_or_else(|| CmdError::Failed("the message isn't downloaded".into()))?;
    // Only some of them, when asked (the ones selected).
    let only: Option<Vec<u64>> = v.get("indices").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_u64).take(10_000).collect());
    let mut saved = Vec::new();
    for a in parsed.attachments.iter().filter(|a| !a.inline && only.as_ref().is_none_or(|o| o.contains(&(a.index as u64)))) {
        let Some(data) = mailcraft_mime::attachment_data(&raw, a.index) else { continue };
        let name = sanitize_filename(&a.name);
        let (stem, ext) = name.rsplit_once('.').map_or((name.clone(), String::new()), |(s, e)| (s.to_string(), format!(".{e}")));
        let mut path = dir.join(&name);
        let mut n = 2;
        while path.exists() && n < 1000 {
            path = dir.join(format!("{stem} ({n}){ext}"));
            n += 1;
        }
        std::fs::write(&path, &data).map_err(|e| CmdError::Failed(format!("{}: {e}", path.display())))?;
        mark_from_internet(&path);
        saved.push(path.to_string_lossy().to_string());
    }
    Ok(json!({"saved": saved}))
}

/// An attachment as text, for previewing (`message` kind: an attached email, parsed).
pub(super) fn preview_attachment(s: &mut Session, v: &Value) -> CmdResult {
    let index = p::u64(v, "index").and_then(|i| usize::try_from(i).ok()).ok_or_else(|| CmdError::Params("`index` is required".into()))?;
    let r = s.reading.as_ref().ok_or_else(|| CmdError::Disabled("open a message first".into()))?;
    let parsed = r.parsed.as_ref().ok_or_else(|| CmdError::Disabled("the message is still downloading".into()))?;
    let a = parsed.attachments.iter().find(|a| a.index == index).ok_or_else(|| CmdError::Params(format!("no attachment {index}")))?;
    let kind = mailcraft_mime::text_preview_kind(&a.mime, &a.name).ok_or_else(|| CmdError::Failed(format!("{} can't be shown as text", a.name)))?;
    let raw = s.store.as_ref().and_then(|st| st.read_body(&r.key)).ok_or_else(|| CmdError::Failed("the message isn't downloaded".into()))?;
    let data = mailcraft_mime::attachment_data(&raw, index).ok_or_else(|| CmdError::Failed("the attachment can't be read".into()))?;
    if kind == "message" {
        let m = mailcraft_mime::parse(&data).ok_or_else(|| CmdError::Failed("the attached message can't be read".into()))?;
        return Ok(
            json!({"kind": "message", "name": a.name, "subject": m.subject, "from": m.from.to_string(), "to": mailcraft_model::addr::format_list(&m.to), "date": m.date, "text": m.text}),
        );
    }
    // At most 512 KB, cut on a character boundary.
    const MAX: usize = 512 << 10;
    let text = String::from_utf8_lossy(data.get(..MAX).unwrap_or(&data)).to_string();
    Ok(json!({"kind": "text", "name": a.name, "text": text, "truncated": data.len() > MAX}))
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
