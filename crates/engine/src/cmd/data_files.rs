//! Commands: data files (Outlook's `.pst`, MailCraft's own) looked at, imported, stopped, gone on
//! with, kept as they are or started again.

use mailcraft_model::job::StopHow;

use super::*;

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
            s.import_pst(&path, p::bool(v, "again").unwrap_or(false))
        })
        .params(r#"{"path": string (a .pst or .mailcraft data file; its mail goes into a new account on this computer), "again"?: bool (import it though it's already open as another account)}"#),
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
    ]
}
