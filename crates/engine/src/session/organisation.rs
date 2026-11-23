//! An organisation's profile in the session (issue #245): its defaults put into the settings
//! once each, its locks held through every command, and a command that would change a locked
//! setting refused, said as set by the organisation. Without a profile, nothing here does a
//! thing: no setting is touched, no command refused, and no work is done after each.

use std::sync::Arc;

use serde_json::Value;

use super::*;
use crate::policy::{Organisation, domain_of};

mod accounts;
mod signature;

pub use accounts::{ENCRYPTION_REQUIRED, NO_EXCEPTIONS};
pub use signature::DETAILS;

/// Said of a setting the organisation locked.
pub const BY_ORGANISATION: &str = "Set by your organisation";

/// The settings an organisation may set: every one MailCraft has but the person's own state
/// (what's open, folded, ordered or searched lately, and what names their accounts' folders).
pub const ORG_SETTINGS: [&str; 82] = [
    "updates",
    "readingPane",
    "conversations",
    "markReadOnOpen",
    "readInPane",
    "notesOrder",
    "tasksOrder",
    "tasksHideDone",
    "calendarsSideBySide",
    "scheduleWorkHours",
    "calendarMonths",
    "workTime",
    "freeTimeMinutes",
    "addressBook",
    "keepMessages",
    "keepOffline",
    "readAfterSecs",
    "showPreview",
    "compact",
    "folderDensity",
    "groups",
    "singleLine",
    "classicRibbon",
    "ribbonHidden",
    "openSendReceive",
    "accent",
    "plainTopBar",
    "lightMessages",
    "systemTitleBar",
    "dateStyle",
    "confirmPermanentDelete",
    "notify",
    "notifyOther",
    "spelling",
    "dictionary",
    "composeFont",
    "closeToTray",
    "startInTray",
    "trayUnread",
    "categories",
    "focusedInbox",
    "showHtml",
    "trustedSenders",
    "blockedSenders",
    "safeSenders",
    "newMailWindow",
    "replyWindow",
    "ctrlEnter",
    "undoSendSecs",
    "startIn",
    "showFavorites",
    "tentativeInvites",
    "recordAnswers",
    "agents",
    "agentsRun",
    "agentsAlways",
    "answeredInvites",
    "answeredReplies",
    "arrange",
    "reverseSort",
    "workOffline",
    "todoBar",
    "followUp",
    "followUpDone",
    "mailReminders",
    "quickSteps",
    "animations",
    "compactOnClose",
    "autoArchive",
    "searchFolders",
    "reminders",
    "searchScope",
    "keepPhotoLocation",
    "cleanLinks",
    "autoPictures",
    "checkRevocation",
    "senderAddresses",
    "externalTag",
    "orgDomains",
    "meetingRooms",
    "meetingHosts",
    "meetingForAll",
];

/// A field's name as the settings name it: `auto_pictures` → `autoPictures`.
pub fn camel(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut up = false;
    for c in name.chars() {
        if c == '_' {
            up = true;
        } else if up {
            out.extend(c.to_uppercase());
            up = false;
        } else {
            out.push(c);
        }
    }
    out
}

impl Session {
    /// The organisation's profile, as the app read it at start (none: an empty one).
    pub fn set_organisation(&mut self, o: Organisation) {
        self.organisation = Arc::new(o);
        self.refresh_organisation();
    }

    /// What of the profile applies, worked out again: once it's set, and as accounts come and
    /// go (their domains pick the profiles of several institutions).
    pub fn refresh_organisation(&mut self) {
        if self.organisation.is_empty() {
            self.org = crate::policy::Effective::default();
            return;
        }
        let mut domains: Vec<String> = self.accounts.iter().map(|a| domain_of(&a.config.email)).filter(|d| !d.is_empty()).collect();
        domains.sort();
        domains.dedup();
        let mut e = self.organisation.effective(&domains);
        // Updates are a setting like the others: preset, or locked.
        if let Some((u, locked)) = e.updates {
            e.settings.insert("updates".into(), crate::policy::Given { value: serde_json::json!(u), locked });
        }
        let now = serde_json::to_value(&self.prefs).unwrap_or(Value::Null);
        let mut notes = Vec::new();
        let mut held = Vec::new();
        e.settings.retain(|k, g| {
            if !ORG_SETTINGS.contains(&k.as_str()) {
                notes.push(format!("settings.{k}: not a setting an organisation sets"));
                return false;
            }
            // Taken as the settings take it, and kept as they write it (a list left empty isn't
            // written at all), for the lock to be checked against.
            let mut probe = now.clone();
            if let Some(o) = probe.as_object_mut() {
                o.insert(k.clone(), g.value.clone());
            }
            match serde_json::from_value::<Prefs>(probe) {
                Ok(p) => {
                    if g.locked {
                        let written = serde_json::to_value(&p).ok().and_then(|v| v.get(k.as_str()).cloned());
                        held.push((k.clone(), written));
                    }
                    true
                }
                Err(_) => {
                    notes.push(format!("settings.{k}: `{}` isn't a value it takes", g.value));
                    false
                }
            }
        });
        e.notes = notes;
        e.held = held;
        self.org = e;
        self.apply_org_defaults();
        self.hold_org_locks();
        self.apply_org_signatures();
        self.ask_own_details();
    }

    /// Whether the setting `key` (`autoPictures`, or a field's name; several, comma-separated:
    /// any of them) is locked.
    pub fn setting_locked(&self, key: &str) -> bool {
        key.split(',').any(|k| self.org.locked(&camel(k.trim())))
    }

    /// Why the command `id` can't run as the organisation set things: it changes a locked
    /// setting.
    pub fn command_locked(&self, id: &str) -> Option<&'static str> {
        if self.org.held.is_empty() {
            return None;
        }
        let registry = self.registry();
        let setting = registry.get(id)?.setting;
        (!setting.is_empty() && self.setting_locked(setting)).then_some(BY_ORGANISATION)
    }

    /// The organisation's defaults not yet put in, put in: each value once, so one the person
    /// changed stays until the organisation gives another.
    fn apply_org_defaults(&mut self) {
        let due: Vec<(String, Value)> = self
            .org
            .settings
            .iter()
            .filter(|(k, g)| !g.locked && self.prefs.org_applied.get(*k) != Some(&g.value))
            .map(|(k, g)| (k.clone(), g.value.clone()))
            .collect();
        if due.is_empty() {
            return;
        }
        self.set_settings(&due);
        for (k, v) in due {
            self.prefs.org_applied.insert(k, v);
        }
    }

    /// The locked settings put back as the organisation locked them, where anything changed
    /// them: after every command, and once the profile is set. Whether any was.
    /// Whether command `id` may run as the organisation locked things: `Ok(None)`, as any other;
    /// `Ok(Some(jobs queued so far))` when it sets settings that are all locked themselves, so it
    /// runs to read them, and a change is refused after ([`Session::org_refuses_change`]); an
    /// error when a lock governs something else it changes (trusting a sender, with web pictures
    /// locked off).
    pub(crate) fn org_locked_run(&self, id: &str) -> Result<Option<usize>, CmdError> {
        let Some(why) = self.command_locked(id) else { return Ok(None) };
        let setting = self.registry().get(id).map_or("", |c| c.setting);
        if setting.split(',').all(|k| self.setting_locked(k.trim())) { Ok(Some(self.jobs.len())) } else { Err(CmdError::Disabled(why.to_string())) }
    }

    /// After a command that sets a locked setting has run (`jobs`: how many jobs were queued
    /// before it, `None` for any other command): a read of it stands; a change is put back, what
    /// the command queued is dropped, and it's refused, said.
    pub(crate) fn org_refuses_change(&mut self, jobs: Option<usize>) -> Option<CmdError> {
        let before = jobs?;
        if !self.hold_org_locks() {
            return None;
        }
        self.jobs.truncate(before);
        Some(CmdError::Disabled(BY_ORGANISATION.to_string()))
    }

    pub(crate) fn hold_org_locks(&mut self) -> bool {
        if self.org.held.is_empty() {
            return false;
        }
        let now = serde_json::to_value(&self.prefs).unwrap_or(Value::Null);
        let off: Vec<(String, Value)> = self
            .org
            .held
            .iter()
            .filter(|(k, written)| now.get(k.as_str()) != written.as_ref())
            .filter_map(|(k, _)| self.org.settings.get(k).map(|g| (k.clone(), g.value.clone())))
            .collect();
        if off.is_empty() {
            return false;
        }
        self.set_settings(&off);
        true
    }

    /// Settings by their keys, with what follows from them as the window's own changes do.
    fn set_settings(&mut self, kv: &[(String, Value)]) {
        let Ok(mut v) = serde_json::to_value(&self.prefs) else { return };
        if let Some(o) = v.as_object_mut() {
            for (k, val) in kv {
                o.insert(k.clone(), val.clone());
            }
        }
        let Ok(p) = serde_json::from_value::<Prefs>(v) else { return };
        let keep = (p.keep_messages, p.keep_offline) != (self.prefs.keep_messages, self.prefs.keep_offline);
        let offline = (p.work_offline != self.prefs.work_offline).then_some(p.work_offline);
        self.prefs = p;
        if keep {
            self.apply_sync_prefs();
        }
        if let Some(on) = offline {
            self.set_work_offline(on);
        }
    }

    /// Whether `email` may be an account here: any, unless the organisation keeps accounts to
    /// its domains. Why not, said.
    pub fn org_allows_account(&self, email: &str) -> Result<(), String> {
        if self.org.allows_address(email) {
            return Ok(());
        }
        let domains = self.org.allowed_domains.as_ref().map(|d| d.join(", ")).unwrap_or_default();
        Err(format!("your organisation allows accounts of its own mail domains only ({domains})"))
    }
}
