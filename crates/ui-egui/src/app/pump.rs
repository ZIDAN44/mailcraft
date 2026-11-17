use crate::*;
use mailcraft_engine::model::Event;
use std::path::PathBuf;

impl MailApp {
    pub(super) fn pump(&mut self) {
        let (now, offset) = self.services.clock();
        let before = self.session.clock.offset;
        self.session.set_clock(now, offset);
        // The offset changed (summer time, or the system's zone did): which zone it is, again.
        if offset != before {
            self.session.set_zone(self.services.zone().as_deref());
        }
        for e in self.backend.poll() {
            self.services.event(&e);
            if let Event::MessageFileRead { path, .. } = &e {
                self.services.discard_picked_file(std::path::Path::new(path));
            }
            self.session.apply(e);
        }
        self.files_filed();
        if let Some(result) = self.services.take_clipboard_files() {
            match result {
                Ok(files) => self.attach_clipboard_files(files),
                Err(error) => self.session.notice(error, true),
            }
        }
        if let Some(picture) = self.services.take_clipboard_picture() {
            if let Some(picture) = picture {
                self.finish_clipboard_picture(picture);
            } else {
                self.pending_clipboard_picture = None;
            }
        }
        for error in self.services.take_pick_errors() {
            self.session.notice(error, true);
        }
        for name in self.services.take_downloaded() {
            self.session.notice(tf!("Saved {path}", path = name), false);
        }
        if self.services.downloads_files() {
            let completed = self.services.take_download_completions();
            for _ in 0..completed {
                self.download_next_attachment();
            }
        }
        // Sign-in pages go to the browser.
        for url in std::mem::take(&mut self.session.open_urls) {
            if let Err(e) = self.services.open_url(&url) {
                self.session.notice(tf!("The sign-in page couldn't be opened: {error}", error = e), true);
            }
        }
        // Files chosen in the browser's picker.
        for (purpose, name, data) in self.services.take_picked() {
            if purpose == "account-data-file" {
                if let Some(Dialog::AddAccount(form)) = self.ui.dialog.as_mut() {
                    form.pst = name;
                }
            } else if purpose == "data-file" {
                if is_message_or_pim_file(std::path::Path::new(&name)) {
                    self.open_handed(&name);
                    if !is_outlook_message_file(std::path::Path::new(&name)) {
                        self.services.discard_picked_file(std::path::Path::new(&name));
                    }
                } else {
                    match self.session.run("account.inspectDataFile", &json!({"path": name, "again": true})) {
                        Ok(_) => {
                            self.ui.backstage = None;
                            self.ui.dialog = Some(Dialog::OpenDataFile { path: name });
                        }
                        Err(error) => self.session.notice(error.to_string(), true),
                    }
                }
            } else if purpose == "rules" {
                match self.session.run("rules.import", &json!({"dataBase64": binary_base64(&data)})) {
                    Ok(result) => {
                        let n = result.get("added").and_then(Value::as_u64).unwrap_or(0);
                        let off = result.get("turnedOff").and_then(Value::as_u64).unwrap_or(0);
                        let text = if off > 0 {
                            tf!("Rules imported: {n} ({off} turned off, as their account isn't on this computer)", n = n, off = off)
                        } else {
                            tf!("Rules imported: {n}", n = n)
                        };
                        self.session.notice(text, false);
                    }
                    Err(error) => self.session.notice(error.to_string(), true),
                }
            } else if purpose == "categories" {
                match self.session.run("category.import", &json!({"dataBase64": binary_base64(&data)})) {
                    Ok(result) => {
                        let added = result.get("added").and_then(Value::as_u64).unwrap_or(0);
                        let changed = result.get("changed").and_then(Value::as_u64).unwrap_or(0);
                        let skipped = result.get("skipped").and_then(Value::as_array).map_or(0, Vec::len);
                        let text = if skipped > 0 {
                            tf!(
                                "Categories imported: {n} new, {changed} changed, {skipped} left out (their keyword is another category's here)",
                                n = added,
                                changed = changed,
                                skipped = skipped
                            )
                        } else {
                            tf!("Categories imported: {n} new, {changed} changed", n = added, changed = changed)
                        };
                        self.session.notice(text, skipped > 0);
                    }
                    Err(error) => self.session.notice(error.to_string(), true),
                }
            } else if let Some(account) = purpose.strip_prefix("pgp-secret:") {
                self.ui.dialog = Some(Dialog::PgpKey { account: account.to_string(), path: name, data: Some(data), passphrase: String::new() });
            } else if let Some(account) = purpose.strip_prefix("pgp-public:") {
                if let Err(error) = self.session.run("pgp.importPublic", &json!({"account": account, "dataBase64": binary_base64(&data)})) {
                    self.session.notice(error.to_string(), true);
                }
            } else if purpose == "signature" {
                let html = mailcraft_engine::mime::signature_file(&data);
                if let Some(Dialog::Signature(form)) = self.ui.dialog.as_mut() {
                    form.take_code(&html);
                }
            } else if purpose == "signature-picture" {
                self.add_signature_picture(name, data);
            } else if let Some(account) = purpose.strip_prefix("certificate:") {
                self.ui.dialog = Some(Dialog::Certificate { account: account.to_string(), path: name, data: Some(data), password: String::new() });
            } else if purpose == "picture" {
                let size = rich_edit::own_size(&data);
                let (width, line) = (rich_edit::fitted_width(size), rich_edit::own_line(size));
                let r = self.session.run(
                    "compose.insertPicture",
                    &self.picture_place(json!({"name": name, "dataBase64": binary_base64(&data), "width": width, "line": line})),
                );
                self.place_picture(r);
            } else if matches!(purpose.as_str(), "attach-path" | "attach-path-batch" | "attach-shared-path") {
                // Android stages the selected file privately, so the engine can read it directly
                // without making a large Base64 copy on the UI thread.
                let reopen_picker = purpose == "attach-path";
                let path = PathBuf::from(&name);
                let result = self.session.run("compose.attach", &json!({"path": path.to_string_lossy()}));
                self.services.discard_picked_file(&path);
                match result {
                    Ok(_) if self.session.compose.is_some() && reopen_picker => {
                        self.services.pick_later("attach");
                    }
                    Ok(_) | Err(mailcraft_engine::CmdError::Disabled(_)) => {}
                    Err(mailcraft_engine::CmdError::Params(message) | mailcraft_engine::CmdError::Failed(message))
                        if self.session.compose.is_some() =>
                    {
                        self.ui.compose_alert = Some(message);
                    }
                    Err(error) => self.session.notice(error.to_string(), true),
                }
            } else {
                self.run("compose.attach", json!({"name": name, "dataBase64": binary_base64(&data)}));
            }
        }
        let staged_dialog_files: Vec<PathBuf> = match &self.ui.dialog {
            Some(Dialog::AddAccount(form)) if !form.pst.trim().is_empty() => vec![PathBuf::from(form.pst.trim())],
            Some(Dialog::OpenDataFile { path, .. }) => vec![PathBuf::from(path)],
            _ => Vec::new(),
        };
        self.services.keep_staged_files(&staged_dialog_files);
        // Snoozed mail that is due comes back.
        self.session.tick();
    }
}
