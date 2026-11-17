use crate::*;

impl MailApp {
    /// Let a supporting platform read clipboard images in the background.
    pub(crate) fn clipboard_picture_later(&mut self, target: crate::PendingClipboardPicture) -> bool {
        if self.pending_clipboard_picture.is_some() {
            return true;
        }
        match self.services.clipboard_picture_later() {
            Some(true) => self.pending_clipboard_picture = Some(target),
            Some(false) => {}
            None => return false,
        }
        true
    }

    /// Finish inserting a clipboard picture at the compose position or signature selected when
    /// its read began.
    pub(crate) fn finish_clipboard_picture(&mut self, png: Vec<u8>) {
        let Some(target) = self.pending_clipboard_picture.take() else { return };
        match target {
            crate::PendingClipboardPicture::Compose { compose_id, mut params }
                if self.session.compose.as_ref().is_some_and(|compose| compose.id == compose_id) =>
            {
                let size = rich_edit::own_size(&png);
                let (width, line) = (rich_edit::fitted_width(size), rich_edit::own_line(size));
                params["dataBase64"] = json!(crate::binary_base64(&png));
                params["width"] = json!(width);
                params["line"] = json!(line);
                let result = self.session.run("compose.insertPicture", &params);
                self.place_picture(result);
            }
            crate::PendingClipboardPicture::Signature { account, editing, name } if matches!(&self.ui.dialog, Some(Dialog::Signature(form)) if form.account == account && form.editing == editing && form.name == name) =>
            {
                self.add_signature_picture("pasted.png".to_string(), png);
            }
            _ => {}
        }
    }

    /// Put a picture's mark at the body's cursor.
    /// Where a picture goes: at the cursor in the message.
    pub(crate) fn picture_at(&self) -> usize {
        let n = self.session.compose.as_ref().map_or(0, |c| c.body.chars().count());
        self.ui.editor.as_ref().filter(|e| self.session.compose.as_ref().is_some_and(|c| c.id == e.id)).map_or(n, |e| e.sel.start.min(n))
    }

    /// Where a picture goes, put into `params`: the text's cursor, or that of the block of the
    /// quoted original being changed (`quoteCell`).
    pub(crate) fn picture_place(&self, mut params: serde_json::Value) -> serde_json::Value {
        match self.quote_cell() {
            Some((n, at)) => {
                params["at"] = json!(at);
                params["quoteCell"] = json!(n);
            }
            None => params["at"] = json!(self.picture_at()),
        }
        params
    }

    /// The block of the quoted original with the cursor, and where in it the cursor is.
    fn quote_cell(&self) -> Option<(usize, usize)> {
        let c = self.session.compose.as_ref().filter(|c| c.quote.is_some())?;
        let (id, n) = self.ui.quote_focus.filter(|(id, _)| *id == c.id)?;
        Some((n, self.ui.quote_editors.get(&(id, n)).map_or(0, |e| e.sel.start)))
    }

    /// A picture into the signature being edited, at its cursor: pasted, or chosen from files.
    pub(crate) fn signature_picture(&mut self, paste: bool) {
        use mailcraft_engine::mime::SIGNATURE_PICTURE_MAX;
        if !matches!(self.ui.dialog, Some(Dialog::Signature(_))) {
            return;
        }
        if paste {
            let target = match self.ui.dialog.as_ref() {
                Some(Dialog::Signature(form)) => Some(crate::PendingClipboardPicture::Signature {
                    account: form.account.clone(),
                    editing: form.editing.clone(),
                    name: form.name.clone(),
                }),
                _ => None,
            };
            if let Some(target) = target
                && self.clipboard_picture_later(target)
            {
                return;
            }
        }
        let pictures: Vec<(String, Vec<u8>)> = if paste {
            self.services.clipboard_picture().map(|png| ("pasted.png".to_string(), png)).into_iter().collect()
        } else {
            let read = |p: &std::path::PathBuf| {
                let f = std::fs::File::open(p).ok()?;
                let mut v = Vec::new();
                std::io::Read::read_to_end(&mut std::io::Read::take(f, SIGNATURE_PICTURE_MAX as u64 + 1), &mut v).ok()?;
                Some((p.file_name()?.to_string_lossy().to_string(), v))
            };
            self.services.pick_pictures().iter().filter_map(read).collect()
        };
        for (name, data) in pictures {
            self.add_signature_picture(name, data);
        }
    }

    /// Add picture bytes returned by a platform picker or the clipboard to the open signature.
    pub(crate) fn add_signature_picture(&mut self, name: String, data: Vec<u8>) {
        use mailcraft_engine::mime::{InlinePicture, SIGNATURE_PICTURE_MAX, SIGNATURE_PICTURE_TYPES, SIGNATURE_PICTURES, mime_from_name};
        let Some(Dialog::Signature(form)) = self.ui.dialog.as_mut() else { return };
        let mime = mime_from_name(&name);
        let size = rich_edit::own_size(&data);
        if data.len() > SIGNATURE_PICTURE_MAX
            || form.compose.inline.len() >= SIGNATURE_PICTURES
            || size.is_none()
            || !SIGNATURE_PICTURE_TYPES.contains(&mime)
        {
            self.session.notice(tf!("{name} can't go in a signature: pictures of up to 1 MB, 10 at most (PNG, JPEG, GIF, WebP)", name = name), true);
            return;
        }
        let cid = (1..).map(|n| format!("signature{n}@mailcraft")).find(|c| !form.compose.inline.iter().any(|p| &p.cid == c)).unwrap_or_default();
        form.compose.inline.push(InlinePicture { cid: cid.clone(), name, mime: mime.to_string(), data });
        let n = form.compose.body.chars().count();
        let at = form.editor.0.sel.start.min(n);
        let width = rich_edit::fitted_width(size).and_then(|w| u16::try_from(w).ok());
        mailcraft_engine::cmd::insert_picture_at(&mut form.compose, at, &cid, width);
        form.editor.0.sel = at + 1..at + 1;
        form.editor.0.refocus = Some(at + 1..at + 1);
    }

    /// A signature saved as a web page, as the one being edited's HTML code (issue #11): an
    /// Outlook `.htm` signature, its pictures read from the folder beside it (that folder only,
    /// pictures only, each 1 MB at most).
    pub(crate) fn signature_file(&mut self) {
        use mailcraft_engine::mime;
        if !matches!(self.ui.dialog, Some(Dialog::Signature(_))) {
            return;
        }
        let Some(path) = self.services.pick_files().into_iter().next() else { return };
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let read = std::fs::File::open(&path).and_then(|f| {
            let mut v = Vec::new();
            std::io::Read::read_to_end(&mut std::io::Read::take(f, mime::SIGNATURE_CODE_MAX as u64 + 1), &mut v).map(|_| v)
        });
        let bytes = match read {
            Ok(b) if b.len() <= mime::SIGNATURE_CODE_MAX => b,
            _ => {
                self.session.notice(tf!("{name} can't be read as a signature", name = name), true);
                return;
            }
        };
        let folder = path.parent().and_then(|p| p.canonicalize().ok());
        let html = mime::local_pictures(&mime::pictures_beside(&mime::signature_file(&bytes)), |src| {
            let folder = folder.as_ref()?;
            let rel = percent_decoded(src.trim().get(5..)?);
            let file = folder.join(rel.trim_start_matches(['/', '\\'])).canonicalize().ok()?;
            let meta = std::fs::metadata(&file).ok()?;
            if !file.starts_with(folder) || !meta.is_file() || meta.len() > mime::SIGNATURE_PICTURE_MAX as u64 {
                return None;
            }
            let kind = mime::mime_from_name(&file.file_name()?.to_string_lossy()).to_string();
            mime::SIGNATURE_PICTURE_TYPES.contains(&kind.as_str()).then_some(())?;
            Some((kind, std::fs::read(&file).ok()?))
        });
        if let Some(Dialog::Signature(f)) = self.ui.dialog.as_mut() {
            f.take_code(&html);
        }
    }

    pub(crate) fn place_picture(&mut self, r: mailcraft_engine::CmdResult) {
        match r {
            // In place: the cursor goes after it.
            Ok(v) => {
                let cell = self.quote_cell().and_then(|(n, _)| self.session.compose.as_ref().map(|c| (c.id, n)));
                let ed = match cell {
                    Some(k) => self.ui.quote_editors.get_mut(&k),
                    None => self.ui.editor.as_mut(),
                };
                if let Some(e) = ed {
                    let end = v.get("end").and_then(serde_json::Value::as_u64).and_then(|e| usize::try_from(e).ok()).unwrap_or(e.sel.start + 1);
                    e.sel = end..end;
                }
            }
            Err(e) => self.session.notice(e.to_string(), true),
        }
    }

    /// Follow a link from a message: only web pages and mail addresses, never local files or
    /// other schemes.
    pub(crate) fn open_link(&mut self, url: &str) {
        // Where it really goes, without what says who clicked (File › Options › Safety).
        let cleaned = if self.session.prefs.clean_links { mailcraft_engine::mime::clean_link(url) } else { url.to_string() };
        let u = cleaned.trim();
        let lower = u.to_ascii_lowercase();
        if lower.starts_with("mailto:") {
            self.open_handed(u);
        } else if lower.starts_with("tel:") && self.services.can_dial_phone() {
            const MAX_TEL_LINK_BYTES: usize = 512;
            let value = u.get(4..).unwrap_or_default();
            if value.len() > MAX_TEL_LINK_BYTES {
                self.session.notice(tf!("Links like {link} aren't opened", link = widgets::shorten(u, 40)), true);
                return;
            }
            let number = mailcraft_engine::model::mailto::percent_decode(value);
            if let Err(error) = self.services.dial_phone(&number) {
                self.session.notice(tf!("The link couldn't be opened: {error}", error = error), true);
            }
        } else if lower.starts_with("https://") || lower.starts_with("http://") {
            if let Err(e) = self.services.open_url(u) {
                self.session.notice(tf!("The link couldn't be opened: {error}", error = e), true);
            }
        } else {
            self.session.notice(tf!("Links like {link} aren't opened", link = widgets::shorten(u, 40)), true);
        }
    }

    /// A file name for the open message (its subject, made safe).
    pub(crate) fn message_file_name(&self) -> String {
        let subject = self.session.reading.as_ref().and_then(|r| r.parsed.as_ref()).map(|p| p.subject.clone()).unwrap_or_default();
        let name = mailcraft_engine::mime::sanitize_filename(if subject.trim().is_empty() { "message" } else { &subject });
        name.chars().take(80).collect()
    }

    /// Write an attachment to a temporary file to hand to another app: dragged out (a copy, as
    /// saving), or to be opened (never a program, script or shortcut: the engine refuses).
    pub(crate) fn attachment_to_temp(&mut self, i: usize, open: bool) -> Option<PathBuf> {
        if !self.ui.has_files {
            return None;
        }
        let name = self.attachment_name(i);
        let Some(dir) = mailcraft_engine::store::scratch_dir("open") else {
            self.session.notice(tl!("There's no folder to put the attachment in for another app"), true);
            return None;
        };
        let path = dir.join(mailcraft_engine::mime::sanitize_filename(&name));
        match self.session.run("attachment.save", &json!({"index": i, "path": path.to_string_lossy(), "open": open})) {
            Ok(_) => Some(path),
            Err(e) => {
                self.session.notice(e.to_string(), true);
                None
            }
        }
    }

    /// A note's file `index`, opened in another app (saved for it first, never when it could run
    /// a program) or saved where asked.
    pub(super) fn note_file(&mut self, uid: &str, index: usize, open: bool) {
        let name = self
            .session
            .notes("")
            .iter()
            .find(|n| n.note.uid == uid)
            .and_then(|n| n.note.kept_files().get(index).map(|f| mailcraft_engine::mime::sanitize_filename(f.file_name())))
            .unwrap_or_else(|| "attachment".into());
        let path = if open {
            let Some(dir) = mailcraft_engine::store::scratch_dir("open") else {
                self.session.notice(tl!("There's no folder to put the attachment in for another app"), true);
                return;
            };
            dir.join(&name)
        } else {
            let Some(path) = self.services.save_path(&name) else { return };
            path
        };
        match self.session.run("note.saveFile", &json!({"uid": uid, "index": index, "path": path.to_string_lossy(), "open": open})) {
            Ok(_) if open => {
                if let Err(e) = self.services.open_path(&path) {
                    self.session.notice(tf!("{name} could not be opened: {error}", name = name, error = e), true);
                }
            }
            Ok(_) => self.session.notice(tf!("Saved {path}", path = path.display()), false),
            Err(e) => self.session.notice(e.to_string(), true),
        }
    }

    pub(crate) fn attachment_name(&self, i: usize) -> String {
        self.session
            .reading
            .as_ref()
            .and_then(|r| r.parsed.as_ref())
            .and_then(|p| p.attachments.iter().find(|a| a.index == i))
            .map(|a| a.name.clone())
            .unwrap_or_else(|| "attachment".into())
    }

    pub(crate) fn attachment_mime(&self, i: usize) -> String {
        self.session
            .reading
            .as_ref()
            .and_then(|r| r.parsed.as_ref())
            .and_then(|p| p.attachments.iter().find(|a| a.index == i))
            .map(|a| a.mime.clone())
            .unwrap_or_else(|| "application/octet-stream".into())
    }

    pub(super) fn queue_attachment_downloads(&mut self, files: Vec<(usize, String, String)>) {
        let Some(key) = self.session.reading.as_ref().map(|reading| reading.key.clone()) else { return };
        self.ui
            .attachment_downloads
            .extend(files.into_iter().map(|(index, name, mime)| (key.clone(), index, mailcraft_engine::mime::sanitize_filename(&name), mime)));
        self.download_next_attachment();
    }

    pub(crate) fn download_next_attachment(&mut self) {
        while let Some((key, index, name, mime)) = self.ui.attachment_downloads.pop_front() {
            match self.session.attachment_bytes_for(&key, index) {
                Ok(bytes) => {
                    self.services.download_owned_with_mime(&name, &mime, bytes);
                    break;
                }
                Err(error) => self.session.notice(error.to_string(), true),
            }
        }
    }

    /// Every attachment of the message being read, selected (Ctrl+A in its attachments).
    pub(crate) fn select_all_attachments(&mut self) {
        let Some(r) = self.session.reading.as_ref() else { return };
        let all: Vec<usize> = r.parsed.as_ref().map(|p| p.attachments.iter().filter(|a| !a.inline).map(|a| a.index).collect()).unwrap_or_default();
        self.ui.attachment_selected = Some((r.key.clone(), all));
        self.ui.attachment_selection_mode = true;
    }
}

/// `%20` and the like decoded (an address in a saved web page), as UTF-8, else in Windows'
/// Western characters, which Outlook writes a name with an accent in (`Andr%E9_files`).
fn percent_decoded(s: &str) -> String {
    let raw = s.as_bytes();
    let mut out = Vec::with_capacity(raw.len());
    let mut i = 0;
    while let Some(&b) = raw.get(i) {
        let hex = |k: usize| raw.get(k).and_then(|h| char::from(*h).to_digit(16));
        match (b, hex(i + 1), hex(i + 2)) {
            (b'%', Some(a), Some(c)) => {
                out.push(u8::try_from(a * 16 + c).unwrap_or(b'%'));
                i += 3;
            }
            _ => {
                out.push(b);
                i += 1;
            }
        }
    }
    mailcraft_engine::mime::signature_file(&out)
}
