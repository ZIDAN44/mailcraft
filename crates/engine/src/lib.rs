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

pub mod cmd;
pub mod sample;
mod session;

use std::collections::HashMap;

use serde_json::Value;

pub use mailcraft_mime as mime;
pub use mailcraft_model as model;
pub use mailcraft_store as store;
pub use session::{
    ARRANGE_HEADINGS, AccountState, Arrange, Compose, FolderRef, ListFilter, MoveTarget, NotesOrder, Notice, Picture, Prefs, ReadInPane, Reading,
    ReadingPane, Row, SearchScope, Session, VacationState, View,
    invite::Invite,
    parse_key,
    pim::{AgendaItem, CalMode, Module, NoteRef, Person, parse_local},
};

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
}

fn always(_: &Session) -> Option<&'static str> {
    None
}

impl CommandSpec {
    pub const fn new(id: &'static str, label: &'static str, location: &'static str, run: fn(&mut Session, &Value) -> CmdResult) -> Self {
        CommandSpec { id, label, location, shortcut: "", params: "{}", enabled: always, run }
    }
    pub const fn key(mut self, s: &'static str) -> Self {
        self.shortcut = s;
        self
    }
    pub const fn params(mut self, p: &'static str) -> Self {
        self.params = p;
        self
    }
    pub const fn when(mut self, f: fn(&Session) -> Option<&'static str>) -> Self {
        self.enabled = f;
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
}

#[cfg(test)]
mod tests;
