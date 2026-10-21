//! Commands: Send / Receive, moving around, folders, search, Quick Steps and Search Folders.

use super::compose::{export_pdf, save_message};
use super::*;

pub(super) fn commands() -> Vec<CommandSpec> {
    vec![
        // Send / Receive
        CommandSpec::new("mail.sync", "Send / Receive all folders", "Send / Receive", |s, _| Ok(json!({"accounts": s.sync_all()})))
            .key("F9")
            .when(can_connect),
        CommandSpec::new("folder.sync", "Update folder", "Send / Receive", sync_folder).key("Shift+F9").when(can_connect),
        CommandSpec::new("edit.undo", "Undo", "Quick access", |s, _| s.undo_last())
            .key("Mod+Z")
            .when(|s| if s.undo.is_empty() { Some("nothing to undo") } else { None }),
        CommandSpec::new("mail.workOffline", "Work offline", "Send / Receive", |s, v| {
            let on = p::bool(v, "on").unwrap_or(!s.prefs.work_offline);
            s.set_work_offline(on);
            Ok(json!({"on": on}))
        })
        .params(r#"{"on"?: bool}"#),
        // Navigation
        CommandSpec::new("folder.open", "Open folder", "Folder pane", open_folder).params(r#"{"account": string, "folder": string}"#),
        CommandSpec::new("folder.unified", "All inboxes", "Folder pane › Favorites", unified).params(r#"{"role"?: "inbox"|"sent"|"drafts"|…}"#),
        CommandSpec::new("folder.outbox", "Outbox", "Folder pane", |s, _| {
            s.refresh_outbox();
            s.open_view(View::Outbox);
            Ok(json!({"messages": s.list.len()}))
        }),
        CommandSpec::new("outbox.send", "Send now", "Outbox", |s, v| match parse_key(v) {
            Some(k) => s.send_outbox_now(&k),
            None => s.send_outbox(),
        })
        .params(r#"{"key"?: string (one message, even one held back to be sent later; else all that are due)}"#)
        .when(|s| if s.outbox.is_empty() { Some("the Outbox is empty") } else { None }),
        CommandSpec::new("outbox.edit", "Edit queued message", "Outbox", |s, v| {
            let k = parse_key(v).or_else(|| s.reading.as_ref().map(|r| r.key.clone())).ok_or_else(|| CmdError::Params("`key` is required".into()))?;
            s.edit_outbox(&k)
        })
        .params(r#"{"key"?: string}"#),
        // Folder › Actions
        CommandSpec::new("folder.create", "New folder", "Folder › New", |s, v| {
            let account = account_param(s, v)?;
            s.create_folder(&account, p::str(v, "parent").filter(|x| !x.is_empty()), p::req_str(v, "name")?)
        })
        .params(r#"{"name": string, "parent"?: string, "account"?: string}"#)
        .when(has_account),
        CommandSpec::new("folder.rename", "Rename folder", "Folder › Actions", |s, v| {
            let account = account_param(s, v)?;
            s.rename_folder(&account, p::req_str(v, "folder")?, p::req_str(v, "name")?)
        })
        .params(r#"{"folder": string, "name": string, "account"?: string}"#)
        .when(has_account),
        CommandSpec::new("folder.move", "Move folder", "Folder › Actions", |s, v| {
            let account = account_param(s, v)?;
            s.move_folder(&account, p::req_str(v, "folder")?, p::str(v, "parent").filter(|x| !x.is_empty()))
        })
        .params(r#"{"folder": string, "parent"?: string (empty or absent: top level), "account"?: string}"#)
        .when(has_account),
        CommandSpec::new("folder.delete", "Delete folder", "Folder › Actions", |s, v| {
            let account = account_param(s, v)?;
            s.delete_folder(&account, p::req_str(v, "folder")?, p::bool(v, "permanent").unwrap_or(false))
        })
        .params(r#"{"folder": string, "permanent"?: bool, "account"?: string}"#)
        .when(has_account),
        CommandSpec::new("folder.favorite", "Show in Favorites", "Folder › Favorites", |s, v| {
            let account = account_param(s, v)?;
            s.set_favorite(&account, p::req_str(v, "folder")?, p::bool(v, "on"))
        })
        .params(r#"{"folder": string, "on"?: bool, "account"?: string}"#)
        .when(has_account),
        CommandSpec::new("list.open", "Open message", "Message list", open_message).params(r#"{"key": "account:validity:uid:folder"}"#),
        CommandSpec::new("list.select", "Select", "Message list", select).params(r#"{"key": string, "add"?: bool}"#),
        CommandSpec::new("list.selectAll", "Select all", "Message list", select_all).key("Mod+A"),
        CommandSpec::new("list.next", "Next message", "Message list", |s, _| step(s, 1)).key("Down / J"),
        CommandSpec::new("list.prev", "Previous message", "Message list", |s, _| step(s, -1)).key("Up / K"),
        CommandSpec::new("list.filter", "Filter", "Message list", filter)
            .params(r#"{"value": "all"|"unread"|"flagged"|"attachments"|"categorized"|"important"|"toMe"}"#),
        CommandSpec::new("list.messages", "List messages", "Message list", list_messages).params(r#"{"limit"?: number}"#),
        CommandSpec::new("message.read", "Read message", "Reading pane", read_message).params(r#"{"key"?: string}"#),
        CommandSpec::new("message.popOut", "Open in new window", "Message list › Open", |s, v| {
            let k = match parse_key(v) {
                Some(_) => key_param(s, v)?,
                None => s
                    .focus
                    .clone()
                    .or_else(|| s.selection.first().cloned())
                    .or_else(|| s.reading.as_ref().map(|r| r.key.clone()))
                    .ok_or_else(|| CmdError::Disabled("select a message first".into()))?,
            };
            s.pop_out(&k)
        })
        .params(r#"{"key"?: string}"#),
        CommandSpec::new("message.closeWindow", "Close the message window", "Message window", |s, v| {
            let k = parse_key(v).ok_or_else(|| CmdError::Params("`key` is required".into()))?;
            s.close_window(&k);
            Ok(json!({"windows": s.windows.len()}))
        })
        .params(r#"{"key": string}"#),
        CommandSpec::new("message.saveAs", "Save as", "File › Save As", save_message).params(r#"{"path": string}"#).when(has_reading),
        CommandSpec::new("message.exportPdf", "Save as PDF", "File › Save As", export_pdf).params(r#"{"path": string}"#).when(has_reading),
        CommandSpec::new("message.showPictures", "Show pictures", "Reading pane", |s, v| s.show_pictures(p::bool(v, "always").unwrap_or(false)))
            .params(r#"{"always"?: bool}"#)
            .when(has_reading),
        CommandSpec::new("message.blockPictures", "Don't show pictures from this sender", "Reading pane", |s, _| s.block_pictures())
            .when(has_reading),
        CommandSpec::new("view.html", "Show HTML formatted", "View › Messages", |s, v| {
            s.prefs.show_html = p::bool(v, "on").unwrap_or(!s.prefs.show_html);
            Ok(json!({"on": s.prefs.show_html}))
        })
        .params(r#"{"on"?: bool}"#),
        // Search
        CommandSpec::new("search.run", "Search", "Search box", search).key("Mod+E").params(
            r#"{"query": string, "scope"?: "current"|"subfolders"|"mailbox"|"all" (kept for the next searches; default: the last one), "remember"?: bool (among the recent searches)}"#,
        ),
        CommandSpec::new("search.remember", "Remember a search", "Search box (left with a search in it)", |s, v| {
            s.remember_search(p::req_str(v, "query")?);
            Ok(json!({"recent": s.recent_searches().len()}))
        })
        .params(r#"{"query": string}"#),
        CommandSpec::new("search.forgetRecent", "Clear recent searches", "Search box › recent searches", |s, _| {
            s.forget_recent_searches();
            Ok(json!({}))
        })
        .when(|s| s.recent_searches().is_empty().then_some("no recent searches")),
        // Home › Quick Steps
        CommandSpec::new("quickStep.list", "Quick Steps", "Home › Quick Steps", |s, _| Ok(json!(s.prefs.quick_steps))),
        CommandSpec::new("quickStep.create", "New Quick Step", "Home › Quick Steps", |s, v| {
            let actions = quick_actions(v)?.ok_or_else(|| CmdError::Params("`actions` is required".into()))?;
            s.create_quick_step(p::req_str(v, "name")?, actions)
        })
        .params(QUICK_STEP_PARAMS),
        CommandSpec::new("quickStep.edit", "Edit Quick Step", "Home › Quick Steps", |s, v| {
            let actions = quick_actions(v)?;
            s.edit_quick_step(p::req_str(v, "name")?, p::str(v, "newName"), actions)
        })
        .params(r#"{"name": string, "newName"?: string, "actions"?: […] (as quickStep.create)}"#),
        CommandSpec::new("quickStep.delete", "Delete Quick Step", "Home › Quick Steps", |s, v| s.delete_quick_step(p::req_str(v, "name")?))
            .params(r#"{"name": string}"#),
        CommandSpec::new("quickStep.run", "Do a Quick Step", "Home › Quick Steps", |s, v| {
            let index = p::u64(v, "index").and_then(|i| usize::try_from(i).ok());
            s.run_quick_step(p::str(v, "name"), index)
        })
        .params(r#"{"name"?: string, "index"?: number (from 0)}"#)
        .when(has_selection),
        CommandSpec::new("quickStep.1", "Quick Step 1", "Home › Quick Steps", |s, _| s.run_quick_step(None, Some(0)))
            .key("Mod+Shift+1")
            .when(has_selection),
        CommandSpec::new("quickStep.2", "Quick Step 2", "Home › Quick Steps", |s, _| s.run_quick_step(None, Some(1)))
            .key("Mod+Shift+2")
            .when(has_selection),
        CommandSpec::new("quickStep.3", "Quick Step 3", "Home › Quick Steps", |s, _| s.run_quick_step(None, Some(2)))
            .key("Mod+Shift+3")
            .when(has_selection),
        CommandSpec::new("quickStep.4", "Quick Step 4", "Home › Quick Steps", |s, _| s.run_quick_step(None, Some(3)))
            .key("Mod+Shift+4")
            .when(has_selection),
        CommandSpec::new("quickStep.5", "Quick Step 5", "Home › Quick Steps", |s, _| s.run_quick_step(None, Some(4)))
            .key("Mod+Shift+5")
            .when(has_selection),
        CommandSpec::new("quickStep.6", "Quick Step 6", "Home › Quick Steps", |s, _| s.run_quick_step(None, Some(5)))
            .key("Mod+Shift+6")
            .when(has_selection),
        CommandSpec::new("quickStep.7", "Quick Step 7", "Home › Quick Steps", |s, _| s.run_quick_step(None, Some(6)))
            .key("Mod+Shift+7")
            .when(has_selection),
        CommandSpec::new("quickStep.8", "Quick Step 8", "Home › Quick Steps", |s, _| s.run_quick_step(None, Some(7)))
            .key("Mod+Shift+8")
            .when(has_selection),
        CommandSpec::new("quickStep.9", "Quick Step 9", "Home › Quick Steps", |s, _| s.run_quick_step(None, Some(8)))
            .key("Mod+Shift+9")
            .when(has_selection),
        // Search Folders
        CommandSpec::new("searchFolder.list", "Search Folders", "Folder pane › Search Folders", |s, _| Ok(json!(s.prefs.search_folders))),
        CommandSpec::new("searchFolder.create", "New Search Folder", "Folder pane › Search Folders", |s, v| {
            s.create_search_folder(p::req_str(v, "name")?, p::req_str(v, "query")?)
        })
        .params(r#"{"name": string, "query": string (as typed in the search box: "is:unread", "from:ana,jun", "larger:1MB")}"#),
        CommandSpec::new("searchFolder.edit", "Customize Search Folder", "Folder pane › Search Folders", |s, v| {
            s.edit_search_folder(p::req_str(v, "name")?, p::str(v, "newName"), p::str(v, "query"))
        })
        .params(r#"{"name": string, "newName"?: string, "query"?: string}"#),
        CommandSpec::new("searchFolder.delete", "Delete Search Folder", "Folder pane › Search Folders", |s, v| {
            s.delete_search_folder(p::req_str(v, "name")?)
        })
        .params(r#"{"name": string (its messages stay where they are)}"#),
        CommandSpec::new("searchFolder.open", "Open Search Folder", "Folder pane › Search Folders", |s, v| {
            s.open_search_folder(p::req_str(v, "name")?)
        })
        .params(r#"{"name": string}"#),
        CommandSpec::new("search.clear", "Clear search", "Search box", |s, _| Ok(json!({"results": s.run_search("", SearchScope::All)})))
            .key("Escape"),
        CommandSpec::new("search.server", "Search the server", "Search › end of the results", |s, _| s.search_server())
            .when(|s| s.search.is_none().then_some("search for something first")),
        CommandSpec::new("folder.more", "Show more from the server", "Folder › end of the list", |s, v| {
            let (account, folder) = match (p::str(v, "account"), p::str(v, "folder")) {
                (Some(a), Some(f)) => (a.to_string(), f.to_string()),
                _ => match &s.view {
                    crate::View::Folder { account, folder } => (account.clone(), folder.clone()),
                    _ => return Err(CmdError::Disabled("open a folder first".into())),
                },
            };
            let step = if s.prefs.keep_messages == 0 { 1000 } else { s.prefs.keep_messages };
            let more = p::u64(v, "more").map_or(step, |n| u32::try_from(n.clamp(1, 100_000)).unwrap_or(step));
            s.show_more(&account, &folder, more)
        })
        .params(r#"{"account"?: string, "folder"?: string (default: the folder on view), "more"?: number (default: as many as Options keeps)}"#),
    ]
}

pub(super) fn sync_folder(s: &mut Session, _: &Value) -> CmdResult {
    let mut n = 0;
    let folders: Vec<(String, String)> = match &s.view {
        View::Folder { account, folder } => vec![(account.clone(), folder.clone())],
        View::Unified { role } => s.accounts.iter().filter_map(|a| a.folder(*role).map(|f| (a.config.id.clone(), f.path.clone()))).collect(),
        View::Empty | View::Outbox => Vec::new(),
    };
    for (account, folder) in folders {
        if s.account(&account).is_some_and(|a| !a.is_local()) {
            s.push_job(mailcraft_model::Job::SyncFolder { account, folder });
            n += 1;
        }
    }
    Ok(json!({"folders": n}))
}

pub(super) fn open_folder(s: &mut Session, v: &Value) -> CmdResult {
    let account = p::req_str(v, "account")?.to_string();
    let folder = p::req_str(v, "folder")?.to_string();
    if s.folder(&account, &folder).is_none() {
        return Err(CmdError::Params(format!("no folder `{folder}` in account `{account}`")));
    }
    s.open_view(View::Folder { account, folder });
    Ok(json!({"messages": s.list.len()}))
}

pub(super) fn unified(s: &mut Session, v: &Value) -> CmdResult {
    let role = match p::str(v, "role") {
        Some(r) => FolderRole::from_code(r).ok_or_else(|| CmdError::Params(format!("unknown folder role `{r}`")))?,
        None => FolderRole::Inbox,
    };
    s.open_view(View::Unified { role });
    Ok(json!({"messages": s.list.len()}))
}

pub(super) const QUICK_STEP_PARAMS: &str = r#"{"name": string, "actions": [{"do": "markRead"|"flag"|"pin"} | {"do": "categorize", "value": name} | {"do": "moveTo", "value": folder} | {"do": "moveToRole", "value": "archive"|"trash"|…} | {"do": "delete"}] (a move comes last; one at most)}"#;

/// A Quick Step's `actions`, when given.
pub(super) fn quick_actions(v: &Value) -> Result<Option<Vec<mailcraft_model::organize::RuleAction>>, CmdError> {
    match v.get("actions") {
        None | Some(Value::Null) => Ok(None),
        Some(a) => serde_json::from_value(a.clone()).map(Some).map_err(|e| CmdError::Params(format!("actions: {e}"))),
    }
}

pub(super) fn open_message(s: &mut Session, v: &Value) -> CmdResult {
    let k = key_param(s, v)?;
    s.open_message(&k);
    Ok(json!({"key": k.to_id()}))
}

pub(super) fn select(s: &mut Session, v: &Value) -> CmdResult {
    let k = key_param(s, v)?;
    if p::bool(v, "add").unwrap_or(false) {
        if let Some(i) = s.selection.iter().position(|x| *x == k) {
            s.selection.remove(i);
        } else {
            s.selection.push(k);
        }
    } else {
        s.selection = vec![k];
    }
    Ok(json!({"selected": s.selection.len()}))
}

pub(super) fn select_all(s: &mut Session, _: &Value) -> CmdResult {
    s.selection = s.list.iter().map(|m| m.key.clone()).collect();
    Ok(json!({"selected": s.selection.len()}))
}

pub(super) fn step(s: &mut Session, dir: i64) -> CmdResult {
    let rows: Vec<usize> = s.rows().iter().filter_map(|r| if let crate::Row::Message { index, .. } = r { Some(*index) } else { None }).collect();
    if rows.is_empty() {
        return Ok(json!({"key": null}));
    }
    let cur = s.focus.as_ref().and_then(|k| s.list.iter().position(|m| &m.key == k)).and_then(|i| rows.iter().position(|&r| r == i));
    let next = match cur {
        None => 0,
        Some(c) => {
            let n = i64::try_from(c).unwrap_or(0).saturating_add(dir).clamp(0, i64::try_from(rows.len()).unwrap_or(1) - 1);
            usize::try_from(n).unwrap_or(0)
        }
    };
    let Some(key) = rows.get(next).and_then(|&i| s.list.get(i)).map(|m| m.key.clone()) else { return Ok(json!({"key": null})) };
    s.open_message(&key);
    Ok(json!({"key": key.to_id()}))
}

pub(super) fn filter(s: &mut Session, v: &Value) -> CmdResult {
    let value = p::req_str(v, "value")?;
    s.filter = ListFilter::EVERY
        .into_iter()
        .find(|f| f.code().eq_ignore_ascii_case(value))
        .ok_or_else(|| CmdError::Params(format!("unknown filter `{value}`")))?;
    s.refresh_list();
    Ok(json!({"messages": s.list.len()}))
}

pub(super) fn summary_json(m: &mailcraft_model::Summary) -> Value {
    json!({
        "key": m.key.to_id(),
        "date": m.date,
        "from": m.from.to_string(),
        "subject": m.subject,
        "preview": m.preview,
        "unread": !m.flags.seen,
        "flagged": m.flags.flagged,
        "attachments": m.has_attachments,
    })
}

pub(super) fn list_messages(s: &mut Session, v: &Value) -> CmdResult {
    let limit = p::u64(v, "limit").map_or(50, |n| usize::try_from(n).unwrap_or(usize::MAX).min(10_000));
    Ok(json!({
        "view": s.view_title(),
        "total": s.list.len(),
        "messages": s.list.iter().take(limit).map(summary_json).collect::<Vec<_>>(),
    }))
}

pub(super) fn read_message(s: &mut Session, v: &Value) -> CmdResult {
    if parse_key(v).is_some() {
        let k = key_param(s, v)?;
        if s.reading.as_ref().is_none_or(|r| r.key != k) {
            s.open_message(&k);
        }
    }
    let r = s.reading.as_ref().ok_or_else(|| CmdError::Disabled("open a message first".into()))?;
    match &r.parsed {
        Some(m) => Ok(json!({
            "key": r.key.to_id(),
            "subject": m.subject,
            "from": m.from.to_string(),
            "to": mailcraft_model::addr::format_list(&m.to),
            "cc": mailcraft_model::addr::format_list(&m.cc),
            "date": m.date,
            "text": m.text,
            "attachments": m.attachments.iter().map(|a| json!({"index": a.index, "name": a.name, "mime": a.mime, "size": a.size})).collect::<Vec<_>>(),
        })),
        None => Ok(json!({"key": r.key.to_id(), "pending": true, "error": r.error})),
    }
}

pub(super) fn search(s: &mut Session, v: &Value) -> CmdResult {
    let q = p::req_str(v, "query")?.to_string();
    // A scope asked for is where the box looks from now on (as Outlook's menu); none: that one.
    let scope = match p::str(v, "scope") {
        Some(x) => {
            let scope = SearchScope::from_code(x).ok_or_else(|| CmdError::Params(format!("no search scope `{x}`")))?;
            s.prefs.search_scope = scope;
            scope
        }
        None => s.prefs.search_scope,
    };
    if p::bool(v, "remember").unwrap_or(false) {
        s.remember_search(&q);
    }
    let n = s.run_search(&q, scope);
    Ok(json!({"results": n, "scope": scope.code()}))
}
