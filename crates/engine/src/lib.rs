//! MailCraft engine: the session and its commands.
//!
//! **Everything is a command.** Each user-visible action is a [`CommandSpec`] with a stable id
//! (`mail.reply`, `folder.open`), a label, its place in the UI, a default shortcut, a params
//! description and a `run` function. The toolbar, menus, shortcuts, the CLI and agents all call
//! [`Session::run`].
//!
//! **Sans-I/O.** The session reads the local cache ([`Store`]) but never touches the network: it
//! queues [`Job`]s ([`Session::take_jobs`]) and applies the [`Event`]s that come back
//! ([`Session::apply`]). The desktop app connects the two to `mailcraft-net`'s worker pool; tests
//! script the events directly.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod agents;
pub mod backup;
pub mod cmd;
pub mod policy;
pub mod sample;
mod session;

use std::collections::HashMap;

use serde_json::Value;

pub use mailcraft_mime as mime;
pub use mailcraft_model as model;
pub use mailcraft_pgp as pgp;
pub use mailcraft_smime as smime;
pub use mailcraft_store as store;
pub use session::camel as setting_key;
pub use session::{
    ACCENTS, ARRANGE_HEADINGS, AccountState, AgeUnit, ArchiveRule, Arrange, AutoArchive, BY_ORGANISATION, BodyWork, CardSearch, Changes, Compacting,
    Compaction, Compose, CtrlEnter, DETAILS, Drafted, Filed, FolderArchive, FolderDensity, FolderRef, IDLE_AFTER_S, ImportState, Imports, IndexPass,
    KeySource, KnownCert, KnownKey, ListFilter, Lock, MoveTarget, NORMAL_TEXT, NotesOrder, Notice, ORG_SETTINGS, PANICS_CAUGHT, PGP_DOC,
    PINNED_HEADING, PaneSection, PersonActivity, PersonFile, PersonLink, PersonMeeting, PersonPeer, PersonThread, PgpSettings, Picture, Prefs,
    PreparedBody, Quote, ROW_ACTIONS_MAX, Reach, Reactions, ReadInPane, Reading, ReadingPane, RevocationCheck, Row, RowAction, SMIME_DOC,
    SOURCE_SHOWN, Scheme, Seal, SearchProgress, SearchScope, SearchStats, SearchWork, Security, SendBlock, SendCheck, SendReceive, SenderAddresses,
    SenderMarks, ServerSearch, Session, SignedBy, SmimeSettings, SourceView, StartIn, TEXT_SIZE_RANGE, TEXT_SIZES, TransferRow, TrustedCertificate,
    UNDO_SEND_MAX, UndoHold, UpdateCheck, VacationState, View, accent_code, accent_rgb,
    invite::{AnsweredInvites, Fit, Invite, InviteDay, InviteProblem, Route, WaitingInvite, forwarding_allowed, proposals_allowed},
    junk::{JunkList, junk_entry},
    newer_release, parse_key,
    pim::{
        ACCOUNT_COLOR, AgendaItem, CalMode, DueReminder, EVENT_HITS_MAX, EventHit, EventWhen, FREE_LENGTHS, FreeBusyState, ListedTask, LocalEvents,
        Module, NoteRef, Person, TASKS_CALENDAR, TODO_BAR_SPLIT, TODO_BAR_WIDTH, TaskRef, TasksOrder, TodoBar, TodoGroup, TodoItem, TodoWhat,
        parse_local,
    },
    quick_steps::QuickStep,
    search_folders::SearchFolder,
    server_rules::{OUTLOOK_LIST, ServerRulesState},
    signature_parts,
};

/// What `--version` prints for the program `name` (`mailcraft`, `mailcraft-cli`): its version,
/// plus the commit and date of release builds (packaging/common.sh).
pub fn version_line(name: &str) -> String {
    build_line(name, env!("CARGO_PKG_VERSION"), option_env!("MAILCRAFT_BUILD_SHA"), option_env!("MAILCRAFT_BUILD_DATE"))
}

/// [`version_line`] of a build: the commit shortened to 9 characters.
fn build_line(name: &str, version: &str, sha: Option<&str>, date: Option<&str>) -> String {
    let v = format!("{name} {version}");
    match (sha, date) {
        (Some(sha), Some(date)) if !sha.is_empty() => format!("{v} ({}, {date})", sha.get(..9).unwrap_or(sha)),
        _ => v,
    }
}

#[derive(Debug, thiserror::Error, Clone, PartialEq)]
pub enum CmdError {
    #[error("unknown command `{0}`")]
    Unknown(String),
    #[error("bad parameters: {0}")]
    Params(String),
    #[error("not available: {0}")]
    Disabled(String),
    #[error("{0}")]
    Failed(String),
}

impl From<mailcraft_store::StoreError> for CmdError {
    fn from(e: mailcraft_store::StoreError) -> Self {
        CmdError::Failed(e.to_string())
    }
}

pub type CmdResult = Result<Value, CmdError>;

/// A registered command.
#[derive(Clone)]
pub struct CommandSpec {
    pub id: &'static str,
    pub label: &'static str,
    /// Where it lives in the UI: `Home › Respond`, `View › Layout`…
    pub location: &'static str,
    /// Default shortcut in a portable form (`Mod+R`, `Mod+Shift+R`, `F9`). `Mod` = Ctrl, or ⌘ on macOS.
    pub shortcut: &'static str,
    /// Parameter description for agents (`{"folder": string}`).
    pub params: &'static str,
    /// `None` = enabled; `Some(reason)` = disabled.
    pub enabled: fn(&Session) -> Option<&'static str>,
    pub run: fn(&mut Session, &Value) -> CmdResult,
    /// It leaves the open message's body being got ready on another thread, where every other
    /// command waits for it: it only moves about the list (or opens another message), changes the
    /// list or the view without acting on the open message, or is about a data file (the window
    /// tells the idle policy how long it's been every half minute).
    pub leaves_body: bool,
    /// The setting it changes, as the settings name it (`autoPictures`, or the field's own name,
    /// `auto_pictures`); `""` for none. Locked by the organisation (issue #245), it's refused.
    pub setting: &'static str,
    /// It sends mail or invitations to other people: a message, a reaction, a meeting's
    /// invitations, changes or cancellation, an answer to one. Agents are asked about it even
    /// when they may run every other command without asking (issue #281).
    pub sends: bool,
}

fn always(_: &Session) -> Option<&'static str> {
    None
}

impl CommandSpec {
    pub const fn new(id: &'static str, label: &'static str, location: &'static str, run: fn(&mut Session, &Value) -> CmdResult) -> Self {
        CommandSpec { id, label, location, shortcut: "", params: "{}", enabled: always, run, leaves_body: false, setting: "", sends: false }
    }
    pub const fn key(mut self, s: &'static str) -> Self {
        self.shortcut = s;
        self
    }
    pub const fn params(mut self, p: &'static str) -> Self {
        self.params = p;
        self
    }
    /// It leaves the open message's body waiting ([`CommandSpec::leaves_body`]).
    pub const fn leaves_body(mut self) -> Self {
        self.leaves_body = true;
        self
    }
    /// It sends to other people ([`CommandSpec::sends`]).
    pub const fn sends(mut self) -> Self {
        self.sends = true;
        self
    }
    pub const fn when(mut self, f: fn(&Session) -> Option<&'static str>) -> Self {
        self.enabled = f;
        self
    }
    /// The setting it changes ([`CommandSpec::setting`]).
    pub const fn setting(mut self, key: &'static str) -> Self {
        self.setting = key;
        self
    }
}

/// All commands by id.
pub struct Registry {
    specs: Vec<CommandSpec>,
    by_id: HashMap<&'static str, usize>,
}

impl Registry {
    pub fn new(specs: Vec<CommandSpec>) -> Self {
        let by_id = specs.iter().enumerate().map(|(i, s)| (s.id, i)).collect();
        Registry { specs, by_id }
    }
    pub fn get(&self, id: &str) -> Option<&CommandSpec> {
        self.by_id.get(id).and_then(|i| self.specs.get(*i))
    }
    pub fn all(&self) -> &[CommandSpec] {
        &self.specs
    }
    /// The command list as JSON (for agents and `mailcraft-cli commands`).
    pub fn describe(&self) -> Value {
        Value::Array(
            self.specs
                .iter()
                .map(|s| serde_json::json!({"id": s.id, "label": s.label, "location": s.location, "shortcut": s.shortcut, "params": s.params}))
                .collect(),
        )
    }
    /// The command bound to a shortcut, if any.
    pub fn by_shortcut(&self, key: &str) -> Option<&CommandSpec> {
        self.specs.iter().find(|s| !s.shortcut.is_empty() && s.shortcut.split(" / ").any(|k| k.eq_ignore_ascii_case(key)))
    }
}

/// Parameter helpers.
pub mod p {
    use serde_json::Value;

    use crate::CmdError;

    pub fn str<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
        v.get(k).and_then(Value::as_str)
    }
    pub fn req_str<'a>(v: &'a Value, k: &str) -> Result<&'a str, CmdError> {
        str(v, k).ok_or_else(|| CmdError::Params(format!("`{k}` (string) is required")))
    }
    pub fn u64(v: &Value, k: &str) -> Option<u64> {
        v.get(k).and_then(|x| x.as_u64().or_else(|| x.as_f64().filter(|f| *f >= 0.0 && f.is_finite()).map(|f| f as u64)))
    }
    pub fn bool(v: &Value, k: &str) -> Option<bool> {
        v.get(k).and_then(Value::as_bool)
    }
    pub fn i64(v: &Value, k: &str) -> Option<i64> {
        v.get(k).and_then(|x| x.as_i64().or_else(|| x.as_f64().filter(|f| f.is_finite() && f.abs() < 9.0e15).map(|f| f as i64)))
    }
    /// A file's bytes given as `k` (a list of numbers, 0 to 255), at most `max` of them: `None`
    /// when there's no list, "too big" as for a file named by its path (`read_capped`), and read
    /// where it is (each caller decoded its own way: one without the cap, two after a copy).
    pub fn bytes(v: &Value, k: &str, max: usize) -> Result<Option<Vec<u8>>, CmdError> {
        let Some(list) = v.get(k).and_then(Value::as_array) else { return Ok(None) };
        if list.len() > max {
            return Err(CmdError::Params(format!("`{k}` is too big")));
        }
        let not_bytes = || CmdError::Params(format!("`{k}` must be bytes"));
        list.iter().map(|b| b.as_u64().and_then(|x| u8::try_from(x).ok()).ok_or_else(not_bytes)).collect::<Result<_, _>>().map(Some)
    }
}

#[cfg(test)]
mod bench;
#[cfg(test)]
mod tests;
