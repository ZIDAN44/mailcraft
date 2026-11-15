//! What the window keeps between runs, for the app to save and give back at the next start: the
//! preferences, the theme and language, where the windows were, and how People lays contacts out.

use super::*;

impl MailApp {
    /// Preferences and theme for the app to save.
    pub fn saved_state(&self) -> Value {
        json!({ "prefs": self.session.prefs, "dark": self.ui.dark, "language": self.ui.language, "windows": self.ui.places, "peopleView": self.ui.people_view, "peopleList": self.ui.people_list })
    }

    /// What [`MailApp::saved_state`] saved, back. A setting another version wrote otherwise
    /// starts as it does for everyone, the rest kept (said in the log, by name).
    pub fn restore_state(&mut self, v: &Value) {
        if let Some(saved) = v.get("prefs") {
            match mailcraft_engine::Prefs::from_saved(saved) {
                Some((p, unread)) => {
                    if !unread.is_empty() {
                        log::warn!("settings that couldn't be read start as they do for everyone: {}", unread.join(", "));
                    }
                    self.take_prefs(p);
                }
                None => log::warn!("the saved settings couldn't be read: they start as they do for everyone"),
            }
        }
        self.ui.dark = v.get("dark").and_then(Value::as_bool);
        if let Some(pv) = read(v, "peopleView") {
            self.ui.people_view = pv;
        }
        if let Some(pl) = read(v, "peopleList") {
            self.ui.people_list = pl;
        }
        if let Some(w) = read(v, "windows") {
            self.ui.places = w;
        }
        if let Some(l) = v.get("language").and_then(Value::as_str).filter(|l| *l == i18n::AUTO || i18n::Lang::from_code(l).is_some()) {
            self.ui.language = l.to_string();
        }
    }

    /// The preferences restored, and what follows from them before any account connects.
    fn take_prefs(&mut self, p: mailcraft_engine::Prefs) {
        let offline = p.work_offline;
        let keep = (p.keep_messages, p.keep_offline);
        self.session.prefs = p;
        self.session.refresh_list();
        // How much mail to keep, before any account connects (when it isn't the default).
        let d = mailcraft_engine::Prefs::default();
        if keep != (d.keep_messages, d.keep_offline) {
            self.session.apply_sync_prefs();
        }
        // Still working offline from last time: before any account connects.
        if offline {
            self.session.set_work_offline(true);
        }
    }
}

/// The value saved as `key`, when there is one and it can be read (one that can't is said in the
/// log, and left as it is).
fn read<T: serde::de::DeserializeOwned>(v: &Value, key: &str) -> Option<T> {
    let saved = v.get(key)?;
    serde_json::from_value(saved.clone()).map_err(|e| log::warn!("the saved {key} couldn't be read: {e}")).ok()
}
