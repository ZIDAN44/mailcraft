//! Rules and categories to a file and back. On a computer, a file where the user says; in the
//! browser, which has no paths, a download, and a file its picker hands over later.

use crate::*;
use mailcraft_engine::CmdResult;

const RULES: &str = "MailCraft rules.json";
const CATEGORIES: &str = "MailCraft categories.json";

impl MailApp {
    pub(crate) fn export_rules(&mut self) {
        if self.services.downloads() {
            match self.session.rules_file() {
                Ok(bytes) => {
                    self.services.download(RULES, &bytes);
                    let n = self.session.rules.len();
                    self.session.notice(tf!("Rules saved in {file}: {n}", n = n, file = RULES), false);
                }
                Err(e) => self.session.notice(e.to_string(), true),
            }
        } else if let Some(path) = self.services.save_path(RULES) {
            match self.session.run("rules.export", &json!({"path": path.to_string_lossy()})) {
                Ok(v) => {
                    let n = v.get("rules").and_then(Value::as_u64).unwrap_or(0);
                    let file = path.file_name().unwrap_or_default().to_string_lossy().to_string();
                    self.session.notice(tf!("Rules saved in {file}: {n}", n = n, file = file), false);
                }
                Err(e) => self.session.notice(e.to_string(), true),
            }
        }
    }

    pub(crate) fn import_rules(&mut self) {
        // The browser's picker answers later, with the file ([`MailApp::rules_picked`]).
        if self.services.pick_later("rules") {
            return;
        }
        if let Some(path) = self.services.pick_rules_file() {
            let r = self.session.run("rules.import", &json!({"path": path.to_string_lossy()}));
            self.rules_imported(r);
        }
    }

    /// A rules file the browser's picker handed over.
    pub(crate) fn rules_picked(&mut self, data: &[u8]) {
        let r = self.session.import_rules(data, false);
        self.rules_imported(r);
    }

    fn rules_imported(&mut self, r: CmdResult) {
        match r {
            Ok(v) => {
                let n = v.get("added").and_then(Value::as_u64).unwrap_or(0);
                let off = v.get("turnedOff").and_then(Value::as_u64).unwrap_or(0);
                let text = if off > 0 {
                    tf!("Rules imported: {n} ({off} turned off, as their account isn't on this computer)", n = n, off = off)
                } else {
                    tf!("Rules imported: {n}", n = n)
                };
                self.session.notice(text, false);
            }
            Err(e) => self.session.notice(e.to_string(), true),
        }
    }

    pub(crate) fn export_categories(&mut self) {
        if self.services.downloads() {
            match self.session.categories_file() {
                Ok(bytes) => {
                    self.services.download(CATEGORIES, &bytes);
                    let n = self.session.prefs.categories.len();
                    self.session.notice(tf!("Categories saved in {file}: {n}", n = n, file = CATEGORIES), false);
                }
                Err(e) => self.session.notice(e.to_string(), true),
            }
        } else if let Some(path) = self.services.save_path(CATEGORIES) {
            match self.session.run("category.export", &json!({"path": path.to_string_lossy()})) {
                Ok(v) => {
                    let n = v.get("categories").and_then(Value::as_u64).unwrap_or(0);
                    let file = path.file_name().unwrap_or_default().to_string_lossy().to_string();
                    self.session.notice(tf!("Categories saved in {file}: {n}", n = n, file = file), false);
                }
                Err(e) => self.session.notice(e.to_string(), true),
            }
        }
    }

    pub(crate) fn import_categories(&mut self) {
        // As the rules: the browser's picker answers later ([`MailApp::categories_picked`]).
        if self.services.pick_later("categories") {
            return;
        }
        if let Some(path) = self.services.pick_categories_file() {
            let r = self.session.run("category.import", &json!({"path": path.to_string_lossy()}));
            self.categories_imported(r);
        }
    }

    /// A categories file the browser's picker handed over.
    pub(crate) fn categories_picked(&mut self, data: &[u8]) {
        let r = self.session.import_categories(data);
        self.categories_imported(r);
    }

    fn categories_imported(&mut self, r: CmdResult) {
        match r {
            Ok(v) => {
                let n = v.get("added").and_then(Value::as_u64).unwrap_or(0);
                let changed = v.get("changed").and_then(Value::as_u64).unwrap_or(0);
                let skipped = v.get("skipped").and_then(Value::as_array).map_or(0, Vec::len);
                let text = if skipped > 0 {
                    tf!(
                        "Categories imported: {n} new, {changed} changed, {skipped} left out (their keyword is another category's here)",
                        n = n,
                        changed = changed,
                        skipped = skipped
                    )
                } else {
                    tf!("Categories imported: {n} new, {changed} changed", n = n, changed = changed)
                };
                self.session.notice(text, skipped > 0);
            }
            Err(e) => self.session.notice(e.to_string(), true),
        }
    }
}
