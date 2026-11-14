//! The command registry: every user-visible action, by id.
//!
//! Shortcuts follow the ones mail users already know: Ctrl+N new, Ctrl+R reply, Ctrl+Shift+R
//! reply all, Ctrl+F forward, Ctrl+Q / Ctrl+U read / unread, Insert flag, Delete, Backspace
//! archive, Ctrl+Shift+V move, F9 send/receive, Ctrl+Enter send.

use mailcraft_mime::{OutAttachment, ReplyMode, mime_from_name, sanitize_filename};
use mailcraft_model::{AccountConfig, Flag, FolderRole, Security, ServerConfig};
use serde_json::{Value, json};

use crate::JunkList;
use crate::session::{ListFilter, MoveTarget, ReadingPane, SearchScope, View, parse_key};
use crate::{CmdError, CmdResult, CommandSpec, Registry, Session, p, parse_local};

fn has_selection(s: &Session) -> Option<&'static str> {
    if s.selection.is_empty() { Some("no message selected") } else { None }
}

fn has_reading(s: &Session) -> Option<&'static str> {
    match &s.reading {
        // (One being got ready is, once the command runs: `Session::run` finishes it first.)
        Some(r) if r.parsed.is_some() || s.body_waiting(&r.key) => None,
        Some(_) => Some("the message is still downloading"),
        None => Some("open a message first"),
    }
}

/// Why a command about the message being written can't run.
const NOT_WRITING: &str = "no message is being written";

fn has_compose(s: &Session) -> Option<&'static str> {
    if s.compose.is_some() { None } else { Some(NOT_WRITING) }
}

impl Session {
    /// The message being written, or the error that there's none.
    pub(crate) fn composing(&self) -> Result<&crate::Compose, CmdError> {
        self.compose.as_ref().ok_or_else(|| CmdError::Disabled(NOT_WRITING.into()))
    }

    /// The message being written, to change, or the error that there's none.
    pub(crate) fn composing_mut(&mut self) -> Result<&mut crate::Compose, CmdError> {
        self.compose.as_mut().ok_or_else(|| CmdError::Disabled(NOT_WRITING.into()))
    }
}

/// A message being written, from an account that can send it.
fn can_send_compose(s: &Session) -> Option<&'static str> {
    has_compose(s).or_else(|| s.compose_send_block().map(crate::SendBlock::reason))
}

fn has_account(s: &Session) -> Option<&'static str> {
    if s.accounts.is_empty() { Some("add an account first") } else { None }
}

/// Connecting needs an account, and not working offline.
fn can_connect(s: &Session) -> Option<&'static str> {
    has_account(s).or(if s.prefs.work_offline { Some("working offline") } else { None })
}

/// A command switching one of [`crate::session::Prefs`]' settings on or off: `"on"` says which,
/// none switches it; it answers `{"on": bool}`. `params` when it says more than `{"on"?: bool}`.
macro_rules! toggle {
    ($id:literal, $label:literal, $location:literal, $field:ident) => {
        toggle!($id, $label, $location, $field, r#"{"on"?: bool}"#)
    };
    ($id:literal, $label:literal, $location:literal, $field:ident, $params:literal) => {
        CommandSpec::new($id, $label, $location, |s, v| {
            s.prefs.$field = p::bool(v, "on").unwrap_or(!s.prefs.$field);
            Ok(json!({"on": s.prefs.$field}))
        })
        .params($params)
    };
}

/// Do a Quick Step to the selection, by its number (from 1): Ctrl+Shift+1 to 9.
macro_rules! quick_step {
    ($n:literal) => {
        CommandSpec::new(concat!("quickStep.", $n), concat!("Quick Step ", $n), "Home › Quick Steps", |s, _| s.run_quick_step(None, Some($n - 1)))
            .key(concat!("Mod+Shift+", $n))
            .when(has_selection)
    };
}

/// Show one of Mail, Calendar, People, Tasks and Notes.
fn set_module(s: &mut Session, module: crate::Module) -> CmdResult {
    s.module = module;
    Ok(json!({"module": s.module}))
}

mod accounts;
mod compose;
pub use compose::insert_picture_at;
mod data_files;
mod files;
pub use files::is_message_file;
pub(crate) use files::message_preview;
mod home;
mod meetings;
mod navigation;
mod pgp;
mod pim;
mod printing;
mod security;
mod signatures;
mod tasks;
mod view;

/// Every command, in the order the CLI lists them: the Home tab's first, accounts' last.
pub fn registry() -> Registry {
    let mut all = Vec::new();
    all.extend(home::commands());
    all.extend(navigation::commands());
    all.extend(compose::commands());
    all.extend(view::commands());
    all.extend(pim::commands());
    all.extend(tasks::commands());
    all.extend(meetings::commands());
    all.extend(files::commands());
    all.extend(security::commands());
    all.extend(pgp::commands());
    all.extend(accounts::commands());
    all.extend(data_files::commands());
    all.extend(signatures::commands());
    Registry::new(all)
}

/// `account` from the params, else the account of the folder on screen.
fn account_param(s: &Session, v: &Value) -> Result<String, CmdError> {
    s.account_or_shown(p::str(v, "account"))
}

/// The `categories` of an event or a task (a text split at `,` and `;`, or a list): trimmed,
/// each once whatever its letters (`Vec::dedup` kept a repeat that wasn't next to the first), at
/// most 20 of 100 characters. `None` when none are given.
fn categories_param(v: &Value) -> Option<Vec<String>> {
    let given: Vec<&str> = match v.get("categories")? {
        Value::String(x) => x.split([',', ';']).collect(),
        Value::Array(a) => a.iter().filter_map(Value::as_str).collect(),
        _ => return None,
    };
    let mut seen = std::collections::HashSet::new();
    // Cut before they're compared: two that differ only past the cut are one once kept.
    let cut = |c: &str| c.trim().chars().take(100).collect::<String>().trim_end().to_string();
    let list = given.into_iter().map(cut).filter(|c| !c.is_empty()).filter(|c| seen.insert(c.to_lowercase()));
    Some(list.take(20).collect())
}

/// The `repeat` of an event or a task: `Some(None)` for none, `Some(Some(rule))`, `None` when it
/// isn't given. A rule of its own (`FREQ=…`) can't hold a line break, which would end the line it
/// goes into (an event's was let through, and made safe only as it was written).
fn repeat_param(v: &Value) -> Result<Option<Option<String>>, CmdError> {
    Ok(match p::str(v, "repeat").map(str::trim) {
        None => None,
        Some("" | "none" | "never") => Some(None),
        Some(r @ ("daily" | "weekly" | "monthly" | "yearly")) => Some(Some(format!("FREQ={}", r.to_ascii_uppercase()))),
        Some(r) if r.to_ascii_uppercase().starts_with("FREQ=") && r.len() < 500 && !r.contains(['\r', '\n']) => Some(Some(r.to_string())),
        Some(r) => return Err(CmdError::Params(format!("`{r}` is not a repeat"))),
    })
}

fn key_param(s: &Session, v: &Value) -> Result<mailcraft_model::MessageKey, CmdError> {
    let k = parse_key(v).ok_or_else(|| CmdError::Params("`key` is required (account:validity:uid:folder)".into()))?;
    if s.summary(&k).is_none() {
        return Err(CmdError::Params(format!("`{}` is not in the list", k.to_id())));
    }
    Ok(k)
}

/// A file's bytes, refused when it's bigger than `max` (read no further than that).
pub(crate) fn read_capped(path: &str, max: u64) -> Result<Vec<u8>, CmdError> {
    read_within(path, max, || CmdError::Params(format!("{path} is too big")))
}

/// [`read_capped`], refused with `too_big` when the file is bigger than `max`: a caller says so
/// in its own words without reading the words of another error.
pub(crate) fn read_within(path: &str, max: u64, too_big: impl Fn() -> CmdError) -> Result<Vec<u8>, CmdError> {
    if plain_file_size(path)? > max {
        return Err(too_big());
    }
    read_more(path, open_file(path)?, Vec::new(), max, too_big)
}

/// The size of the file at `path`, refused when it isn't a plain file: a device (`/dev/zero`)
/// reads forever and a pipe blocks on opening, whatever size they report.
fn plain_file_size(path: &str) -> Result<u64, CmdError> {
    match std::fs::metadata(path) {
        Err(e) => Err(CmdError::Failed(format!("{path}: {e}"))),
        Ok(m) if !m.is_file() => Err(CmdError::Params(format!("{path} isn't a file"))),
        Ok(m) => Ok(m.len()),
    }
}

/// The file at `path` opened to read (after [`plain_file_size`]).
fn open_file(path: &str) -> Result<std::fs::File, CmdError> {
    std::fs::File::open(path).map_err(|e| CmdError::Failed(format!("{path}: {e}")))
}

/// The rest of `f` after `bytes`, refused with `too_big` once there's more than `max` in all.
fn read_more(path: &str, f: impl std::io::Read, mut bytes: Vec<u8>, max: u64, too_big: impl Fn() -> CmdError) -> Result<Vec<u8>, CmdError> {
    use std::io::Read;
    let room = max.saturating_add(1).saturating_sub(bytes.len() as u64);
    f.take(room).read_to_end(&mut bytes).map_err(|e| CmdError::Failed(format!("{path}: {e}")))?;
    if bytes.len() as u64 > max {
        return Err(too_big());
    }
    Ok(bytes)
}
