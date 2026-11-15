//! What goes into the message being written, or the signature being edited: pictures, a
//! signature, files attached.

use crate::*;

impl MailApp {
    pub(super) fn insert_signature(&mut self, name: Option<String>) {
        let mut p = json!({"at": self.picture_at()});
        if let Some(name) = name {
            p["name"] = json!(name);
        }
        let r = self.session.run("compose.insertSignature", &p);
        match r {
            Ok(v) => {
                // The cursor after it.
                let end = v.get("end").and_then(serde_json::Value::as_u64).and_then(|e| usize::try_from(e).ok()).unwrap_or(0);
                if let Some(ed) = self.ui.editor.as_mut() {
                    ed.refocus = Some(end..end);
                }
            }
            Err(e) => self.session.notice(e.to_string(), true),
        }
    }

    /// Pictures from files, at the cursor (the browser's picker answers later).
    pub(super) fn insert_picture(&mut self) {
        if self.services.pick_later(&crate::Picked::Picture) {
            return;
        }
        for p in self.services.pick_pictures() {
            // Wider than a page: in at a page's width (its header read, nothing decoded).
            let head = read_capped(&p, 1 << 20);
            let size = head.ok().as_deref().and_then(rich_edit::own_size);
            let (width, line) = (rich_edit::fitted_width(size), rich_edit::own_line(size));
            let r =
                self.session.run("compose.insertPicture", &self.picture_place(json!({"path": p.to_string_lossy(), "width": width, "line": line})));
            self.place_picture(r);
        }
    }

    pub(super) fn paste_picture(&mut self) {
        // Formatted text that came with a picture (from Word or a web page) was pasted
        // as text.
        let text = clipboard_html().is_some_and(|h| !mailcraft_engine::mime::pasted_html(&h).0.trim().is_empty());
        if !text && let Some(png) = self.services.clipboard_picture() {
            let size = rich_edit::own_size(&png);
            let (width, line) = (rich_edit::fitted_width(size), rich_edit::own_line(size));
            let handed = self.session.hand(png);
            let params = self.picture_place(json!({"name": "pasted.png", "handed": handed, "width": width, "line": line}));
            let r = self.session.run("compose.insertPicture", &params);
            self.place_picture(r);
        }
    }

    /// Files attached (the browser's picker answers later).
    pub(super) fn attach_files(&mut self) {
        if self.services.pick_later(&crate::Picked::Attach) {
            return;
        }
        for p in self.services.pick_files() {
            self.run("compose.attach", json!({"path": p.to_string_lossy()}));
        }
    }

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
        use mailcraft_engine::mime::{InlinePicture, SIGNATURE_PICTURE_MAX, SIGNATURE_PICTURE_TYPES, SIGNATURE_PICTURES, mime_from_name};
        if !matches!(self.ui.dialog, Some(Dialog::Signature(_))) {
            return;
        }
        let pictures: Vec<(String, Vec<u8>)> = if paste {
            self.services.clipboard_picture().map(|png| ("pasted.png".to_string(), png)).into_iter().collect()
        } else {
            let read = |p: &std::path::PathBuf| {
                let v = read_capped(p, SIGNATURE_PICTURE_MAX as u64 + 1).ok()?;
                Some((p.file_name()?.to_string_lossy().to_string(), v))
            };
            self.services.pick_pictures().iter().filter_map(read).collect()
        };
        let Some(Dialog::Signature(f)) = self.ui.dialog.as_mut() else { return };
        for (name, data) in pictures {
            let mime = mime_from_name(&name);
            let size = rich_edit::own_size(&data);
            if data.len() > SIGNATURE_PICTURE_MAX
                || f.compose.inline.len() >= SIGNATURE_PICTURES
                || size.is_none()
                || !SIGNATURE_PICTURE_TYPES.contains(&mime)
            {
                self.session
                    .notice(tf!("{name} can't go in a signature: pictures of up to 1 MB, 10 at most (PNG, JPEG, GIF, WebP)", name = name), true);
                continue;
            }
            let cid = (1..).map(mailcraft_engine::mime::signature_cid).find(|c| !f.compose.inline.iter().any(|p| &p.cid == c)).unwrap_or_default();
            f.compose.inline.push(InlinePicture { cid: cid.clone(), name, mime: mime.to_string(), data });
            let n = f.compose.body.chars().count();
            let at = f.editor.0.sel.start.min(n);
            let width = rich_edit::fitted_width(size).and_then(|w| u16::try_from(w).ok());
            mailcraft_engine::cmd::insert_picture_at(&mut f.compose, at, &cid, width);
            f.editor.0.sel = at + 1..at + 1;
            f.editor.0.refocus = Some(at + 1..at + 1);
        }
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
        let read = read_capped(&path, mime::SIGNATURE_CODE_MAX as u64 + 1);
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
}

/// A file's first `max` bytes (all of it, when it's no bigger): what's read of a file picked,
/// which can be of any size.
fn read_capped(path: &std::path::Path, max: u64) -> std::io::Result<Vec<u8>> {
    let mut v = Vec::new();
    std::io::Read::read_to_end(&mut std::io::Read::take(std::fs::File::open(path)?, max), &mut v)?;
    Ok(v)
}

/// `%20` and the like decoded (an address in a saved web page), as UTF-8, else in Windows'
/// Western characters, which Outlook writes a name with an accent in (`Andr%E9_files`).
fn percent_decoded(s: &str) -> String {
    mailcraft_engine::mime::signature_file(&mailcraft_engine::model::mailto::percent_decode_bytes(s))
}
