//! Commands: data files (Outlook's `.pst`, MailCraft's own) looked at, imported, stopped, gone on
//! with, kept as they are or started again; and an account's own data file measured and made
//! smaller.

use mailcraft_model::job::{CompactMode, StopHow};

use super::*;

fn compacting(s: &Session) -> Option<&'static str> {
    if s.compaction.running.is_empty() { Some("nothing is being compacted") } else { None }
}

fn importing(s: &Session) -> Option<&'static str> {
    if s.import_running() { None } else { Some("nothing is being imported") }
}

/// `account`, else the account on screen.
fn account_or_current(s: &Session, v: &Value) -> Result<String, CmdError> {
    match p::str(v, "account") {
        Some(a) => Ok(a.to_string()),
        None => s.current_account().map(|a| a.config.id.clone()).ok_or_else(|| CmdError::Params("`account` is required".into())),
    }
}

pub(super) fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("account.inspectDataFile", "Look at a data file", "File › Open & Export", |s, v| {
            s.inspect_data_file(p::req_str(v, "path")?, p::bool(v, "again").unwrap_or(false))
        })
        .params(r#"{"path": string, "again"?: bool (look once more)} (asks once; call again for what it holds, and the account it's already open as)"#),
        CommandSpec::new("account.importPst", "Open Outlook data file", "File › Open & Export", |s, v| {
            let path = p::req_str(v, "path")?.to_string();
            let changes = crate::session::Changes::parse(p::str(v, "changes"))?;
            s.import_pst(&path, p::bool(v, "again").unwrap_or(false), changes)
        })
        .params(r#"{"path": string (a .pst or .mailcraft data file; its mail goes into a new account on this computer, an Outlook file's copy beside it: archive.pst's in archive.mailcraft, gone on with when it's opened again), "again"?: bool (import it though it's already open as another account), "changes"?: "read" (the file changed since its copy was read, or it's open already: what it has new is added to the copy) | "keep" (the copy opened as it was read)}"#),
        CommandSpec::new("account.stopImport", "Stop importing", "Folder pane › account menu", |s, v| {
            let how = match p::str(v, "how") {
                None | Some("cancel") => StopHow::Cancel,
                Some("pause") => StopHow::Pause,
                Some(h) => return Err(CmdError::Params(format!("`how` is \"cancel\" or \"pause\", not `{h}`"))),
            };
            let accounts: Vec<String> = if p::bool(v, "all").unwrap_or(false) {
                s.accounts.iter().map(|a| a.config.id.clone()).filter(|a| s.import_state(a).is_some_and(|st| st.live)).collect()
            } else {
                vec![account_or_current(s, v)?]
            };
            s.stop_import(&accounts, how)
        })
        .params(r#"{"account"?: string (default: the current one), "all"?: bool (every import), "how"?: "cancel" (kept, to go on when asked; the default) | "pause" (kept, to go on the next time MailCraft opens)}"#)
        .when(importing),
        CommandSpec::new("account.resumeImport", "Continue importing", "Folder pane › account menu", |s, v| {
            let account = account_or_current(s, v)?;
            s.resume_import(&account)
        })
        .params(r#"{"account"?: string} (an import stopped or interrupted goes on)"#),
        CommandSpec::new("account.restartImport", "Import again", "Folder pane › account menu", |s, v| {
            let account = account_or_current(s, v)?;
            s.restart_import(&account, p::str(v, "path"))
        })
        .params(r#"{"account"?: string, "path"?: string (default: the file it came from)} (what a stopped import brought in goes, and the file is read from the start)"#),
        CommandSpec::new("account.keepImported", "Keep what's imported", "Folder pane › account menu", |s, v| {
            let account = account_or_current(s, v)?;
            s.keep_imported(&account)
        })
        .params(r#"{"account"?: string} (a stopped import won't go on)"#),
        CommandSpec::new("account.importStatus", "Import status", "Status bar", |s, v| s.import_status(p::str(v, "account")))
            .params(r#"{"account"?: string} (every import running or waiting, or that account's, with its report)"#),
        CommandSpec::new("dataFile.health", "Data file health", "File › Info › Mailbox settings", |s, v| {
            let account = account_or_current(s, v)?;
            s.data_file_health(&account)
        })
        .params(r#"{"account"?: string} (measures the account's data file; answers with what's known: its health, a compaction running, the last one's report)"#),
        CommandSpec::new("dataFile.compact", "Compact now", "File › Info › Mailbox settings", |s, v| {
            let account = account_or_current(s, v)?;
            let mode = match p::str(v, "mode") {
                None | Some("manual") => CompactMode::Manual,
                Some("close") => CompactMode::Close,
                Some(m) => return Err(CmdError::Params(format!("`mode` is \"manual\" or \"close\", not `{m}`"))),
            };
            s.compact_data_file(&account, mode)
        })
        .params(r#"{"account"?: string, "mode"?: "manual" (as long as it takes; the default) | "close" (MailCraft closing: only when that's short)} (makes the account's data file smaller: in place, or by a copy when it's big and kept raw by an earlier build)"#),
        CommandSpec::new("dataFile.compactOnClose", "Compact data files on close", "File › Options › Data files", |s, v| {
            s.prefs.compact_on_close = p::bool(v, "on").unwrap_or(!s.prefs.compact_on_close);
            Ok(json!({ "on": s.prefs.compact_on_close }))
        })
        .params(r#"{"on"?: bool (none: switch; on, a data file with room to give back is compacted as MailCraft closes when that's short)}"#),
        CommandSpec::new("dataFile.cancelCompact", "Stop compacting", "Status bar", |s, v| {
            let account = account_or_current(s, v)?;
            s.cancel_compact(&account)
        })
        .params(r#"{"account"?: string} (stops where it can: compressing again or copying; the part that has the file to itself can't be stopped)"#)
        .when(compacting),
        CommandSpec::new("dataFile.idle", "MailCraft is idle", "Window", |s, v| {
            let seconds = p::u64(v, "seconds").ok_or_else(|| CmdError::Params("`seconds` (number) is required".into()))?;
            Ok(serde_json::json!({ "measuring": s.data_files_idle(seconds) }))
        })
        .params(r#"{"seconds": number (since the user last did anything)} (the window calls it now and then: data files that need it are compacted when MailCraft is left alone)"#),
    ]
}
