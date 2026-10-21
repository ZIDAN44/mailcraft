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
        Some(r) if r.parsed.is_some() => None,
        Some(_) => Some("the message is still downloading"),
        None => Some("open a message first"),
    }
}

fn has_compose(s: &Session) -> Option<&'static str> {
    if s.compose.is_some() { None } else { Some("no message is being written") }
}

fn has_account(s: &Session) -> Option<&'static str> {
    if s.accounts.is_empty() { Some("add an account first") } else { None }
}

/// Connecting needs an account, and not working offline.
fn can_connect(s: &Session) -> Option<&'static str> {
    has_account(s).or(if s.prefs.work_offline { Some("working offline") } else { None })
}

mod accounts;
mod compose;
mod home;
mod navigation;
mod pim;
mod view;

/// Every command, in the order the CLI lists them: the Home tab's first, accounts' last.
pub fn registry() -> Registry {
    let mut all = Vec::new();
    all.extend(home::commands());
    all.extend(navigation::commands());
    all.extend(compose::commands());
    all.extend(view::commands());
    all.extend(pim::commands());
    all.extend(accounts::commands());
    Registry::new(all)
}

/// `account` from the params, else the account of the folder on screen.
fn account_param(s: &Session, v: &Value) -> Result<String, CmdError> {
    match p::str(v, "account") {
        Some(a) if s.account(a).is_some() => Ok(a.to_string()),
        Some(a) => Err(CmdError::Params(format!("no account `{a}`"))),
        None => s.current_account().map(|a| a.config.id.clone()).ok_or_else(|| CmdError::Disabled("add an account first".into())),
    }
}

fn key_param(s: &Session, v: &Value) -> Result<mailcraft_model::MessageKey, CmdError> {
    let k = parse_key(v).ok_or_else(|| CmdError::Params("`key` is required (account:validity:uid:folder)".into()))?;
    if s.summary(&k).is_none() {
        return Err(CmdError::Params(format!("`{}` is not in the list", k.to_id())));
    }
    Ok(k)
}

/// A file's bytes, refused when it's bigger than `max` (read no further than that).
fn read_capped(path: &str, max: u64) -> Result<Vec<u8>, CmdError> {
    use std::io::Read;
    let f = std::fs::File::open(path).map_err(|e| CmdError::Failed(format!("{path}: {e}")))?;
    let mut bytes = Vec::new();
    f.take(max.saturating_add(1)).read_to_end(&mut bytes).map_err(|e| CmdError::Failed(format!("{path}: {e}")))?;
    if bytes.len() as u64 > max {
        return Err(CmdError::Params(format!("{path} is too big")));
    }
    Ok(bytes)
}
