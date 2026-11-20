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

/// A change to an age and what's done with old mail, from a command's params.
fn rule_change(v: &Value) -> crate::session::RuleChange {
    crate::session::RuleChange {
        older_than: p::u64(v, "olderThan"),
        unit: p::str(v, "unit").map(str::to_string),
        delete: p::bool(v, "delete"),
        path: p::str(v, "path").map(str::to_string),
    }
}

fn archiving(s: &Session) -> Option<&'static str> {
    if s.archiving.is_some() { Some("old mail is being archived already") } else { None }
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
            s.import_pst(&path, p::bool(v, "again").unwrap_or(false), changes, p::bool(v, "takeOver").unwrap_or(false))
        })
        .params(r#"{"path": string (a .pst or .mailcraft data file; its mail goes into a new account on this computer, an Outlook file's copy beside it: archive.pst's in archive.mailcraft, gone on with when it's opened again), "again"?: bool (import it though it's already open as another account), "changes"?: "read" (the file changed since its copy was read, or it's open already: what it has new is added to the copy) | "keep" (the copy opened as it was read), "takeOver"?: bool (its copy is an account of another MailCraft data folder: take it over here; it's refused otherwise, naming that folder)}"#),
        CommandSpec::new("account.stopImport", "Stop importing", "Folder pane › account menu", |s, v| {
            let how = match p::str(v, "how") {
                None | Some("cancel") => StopHow::Cancel,
                Some("pause") => StopHow::Pause,
                Some(h) => return Err(CmdError::Params(format!("`how` is \"cancel\" or \"pause\", not `{h}`"))),
            };
            let accounts: Vec<String> = if p::bool(v, "all").unwrap_or(false) {
                s.accounts.iter().map(|a| a.config.id.clone()).filter(|a| s.import_state(a).is_some_and(|st| st.live)).collect()
            } else {
                vec![account_param(s, v)?]
            };
            s.stop_import(&accounts, how)
        })
        .params(r#"{"account"?: string (default: the current one), "all"?: bool (every import), "how"?: "cancel" (kept, to go on when asked; the default) | "pause" (kept, to go on the next time MailCraft opens)}"#)
        .when(importing),
        CommandSpec::new("account.resumeImport", "Continue importing", "Folder pane › account menu", |s, v| {
            let account = account_param(s, v)?;
            s.resume_import(&account)
        })
        .params(r#"{"account"?: string} (an import stopped or interrupted goes on)"#),
        CommandSpec::new("account.restartImport", "Import again", "Folder pane › account menu", |s, v| {
            let account = account_param(s, v)?;
            s.restart_import(&account, p::str(v, "path"))
        })
        .params(r#"{"account"?: string, "path"?: string (default: the file it came from)} (what a stopped import brought in goes, and the file is read from the start)"#),
        CommandSpec::new("account.keepImported", "Keep what's imported", "Folder pane › account menu", |s, v| {
            let account = account_param(s, v)?;
            s.keep_imported(&account)
        })
        .params(r#"{"account"?: string} (a stopped import won't go on)"#),
        CommandSpec::new("account.importStatus", "Import status", "Status bar", |s, v| s.import_status(p::str(v, "account")))
            .params(r#"{"account"?: string} (every import running or waiting, or that account's, with its report)"#),
        CommandSpec::new("dataFile.archive", "Clean up old items", "File › Info › Tools", |s, v| {
            let account = account_param(s, v)?;
            let path = p::req_str(v, "path")?.to_string();
            let before = p::i64(v, "before").ok_or_else(|| CmdError::Params("`before` (Unix seconds) is required".into()))?;
            s.archive_old(&account, p::str(v, "folder"), before, &path)
        })
        .leaves_body()
        .params(r#"{"path": string (the .pst file to archive to: made when it isn't there, added to when it is), "before": number (Unix seconds: mail received before then is archived), "account"?: string (default: the current one), "folder"?: string (that folder and those under it; default: all the account's folders)} (each old message goes into the folder of the same name in the file, and leaves the mailbox once it's there; the file then shows in the folder list)"#)
        .when(archiving),
        CommandSpec::new("autoArchive.set", "AutoArchive settings", "File › Options › Data files", |s, v| {
            let every = p::u64(v, "everyDays").map(|n| u32::try_from(n).unwrap_or(u32::MAX));
            s.set_auto_archive(every, p::bool(v, "prompt"), p::bool(v, "show"), rule_change(v))
        })
        .leaves_body()
        .params(r#"{"everyDays"?: number (runs every so many days, up to 60; 0: never), "prompt"?: bool (asks first), "show"?: bool (the file shown in the folder list), "olderThan"?: number (1 to 999), "unit"?: "days"|"weeks"|"months", "delete"?: bool (deleted for good, not archived), "path"?: string (the .pst file it's archived to)} (the default settings, for folders without their own)"#),
        CommandSpec::new("autoArchive.folder", "Folder's AutoArchive settings", "Folder › Properties", |s, v| {
            let account = account_param(s, v)?;
            let folder = p::req_str(v, "folder")?.to_string();
            let mode = p::req_str(v, "archive")?.to_string();
            s.set_folder_archive(&account, &folder, &mode, rule_change(v))
        })
        .leaves_body()
        .params(r#"{"account"?: string, "folder": string, "archive": "off" (never archived) | "default" (the default settings) | "own" (these: from the default ones, or its own), "olderThan"?: number (1 to 999), "unit"?: "days"|"weeks"|"months", "delete"?: bool (deleted for good, not archived), "path"?: string (the .pst file it's archived to)}"#),
        CommandSpec::new("autoArchive.applyToAll", "Apply these settings to all folders now", "File › Options › Data files › AutoArchive settings", |s, _| {
            s.auto_archive_to_all()
        })
        .leaves_body()
        .params(r#"{} (every folder follows the default settings)"#),
        CommandSpec::new("autoArchive.run", "Run AutoArchive now", "AutoArchive's question", |s, _| {
            let groups = s.auto_archive_now();
            Ok(json!({ "groups": groups }))
        })
        .leaves_body()
        .params(r#"{} (each folder's old mail by its settings, a group at a time)"#)
        .when(archiving),
        CommandSpec::new("autoArchive.later", "Not now", "AutoArchive's question", |s, _| {
            s.auto_archive_later();
            Ok(json!({ "next": s.prefs.auto_archive.last_run.saturating_add(i64::from(s.prefs.auto_archive.every_days) * 86_400) }))
        })
        .leaves_body()
        .params(r#"{} (asked again once as many days have passed)"#),
        CommandSpec::new("dataFile.health", "Data file health", "File › Info › Mailbox settings", |s, v| {
            let account = account_param(s, v)?;
            s.data_file_health(&account)
        }).leaves_body()
        .params(r#"{"account"?: string} (measures the account's data file; answers with what's known: its health, a compaction running, the last one's report)"#),
        CommandSpec::new("dataFile.compact", "Compact now", "File › Info › Mailbox settings", |s, v| {
            let account = account_param(s, v)?;
            let mode = match p::str(v, "mode") {
                None | Some("manual") => CompactMode::Manual,
                Some("close") => CompactMode::Close,
                Some(m) => return Err(CmdError::Params(format!("`mode` is \"manual\" or \"close\", not `{m}`"))),
            };
            s.compact_data_file(&account, mode)
        }).leaves_body()
        .params(r#"{"account"?: string, "mode"?: "manual" (as long as it takes; the default) | "close" (MailCraft closing: only when that's short)} (makes the account's data file smaller: in place, or by a copy when it's big and kept raw by an earlier build)"#),
        toggle!("dataFile.compactOnClose", "Compact data files on close", "File › Options › Data files", compact_on_close, r#"{"on"?: bool (none: switch; on, a data file with room to give back is compacted as MailCraft closes when that's short)}"#).leaves_body(),
        CommandSpec::new("dataFile.cancelCompact", "Stop compacting", "Status bar", |s, v| {
            let account = account_param(s, v)?;
            s.cancel_compact(&account)
        }).leaves_body()
        .params(r#"{"account"?: string} (stops where it can: compressing again or copying; the part that has the file to itself can't be stopped)"#)
        .when(compacting),
        CommandSpec::new("dataFile.idle", "MailCraft is idle", "Window", |s, v| {
            let seconds = p::u64(v, "seconds").ok_or_else(|| CmdError::Params("`seconds` (number) is required".into()))?;
            Ok(serde_json::json!({ "measuring": s.data_files_idle(seconds) }))
        }).leaves_body()
        .params(r#"{"seconds": number (since the user last did anything)} (the window calls it now and then: data files that need it are compacted when MailCraft is left alone)"#),
    ]
}
