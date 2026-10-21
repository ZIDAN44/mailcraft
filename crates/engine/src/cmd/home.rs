//! Commands: the Home tab (new, delete, junk, respond, move, tags, categories, rules).

use super::*;

pub(super) fn commands() -> Vec<CommandSpec> {
    vec![
        // Home › New
        CommandSpec::new("mail.new", "New mail", "Home › New", new_mail)
            .key("Mod+N")
            .params(r#"{"account"?: string, "to"?: string (who it's to, "Name <address>")}"#)
            .when(has_account),
        // Home › Delete
        CommandSpec::new("mail.delete", "Delete", "Home › Delete", delete).key("Delete").params(r#"{"permanent"?: bool}"#).when(has_selection),
        CommandSpec::new("mail.archive", "Archive", "Home › Delete", |s, _| s.move_selection(MoveTarget::Role(FolderRole::Archive)))
            .key("Backspace")
            .when(has_selection),
        CommandSpec::new("mail.ignore", "Ignore conversation", "Home › Delete", |s, _| s.ignore_selection()).when(has_selection),
        CommandSpec::new("mail.cleanUp", "Clean up conversation", "Home › Delete › Clean Up", |s, _| {
            let sel = s.selected();
            let keys: Vec<mailcraft_model::MessageKey> = sel.iter().map(|m| m.key.clone()).collect();
            let mut folders: Vec<(String, String)> = sel.iter().map(|m| (m.key.account.clone(), m.key.folder.clone())).collect();
            folders.sort();
            folders.dedup();
            s.clean_up(&folders, Some(&keys))
        })
        .when(has_selection),
        CommandSpec::new("folder.cleanUp", "Clean up folder", "Home › Delete › Clean Up", |s, v| {
            let folders = match p::str(v, "folder") {
                Some(f) => {
                    let account = account_param(s, v)?;
                    if s.folder(&account, f).is_none() {
                        return Err(CmdError::Params(format!("no folder `{f}`")));
                    }
                    vec![(account, f.to_string())]
                }
                None => match &s.view {
                    View::Folder { account, folder } => vec![(account.clone(), folder.clone())],
                    View::Unified { role } => s.accounts.iter().filter_map(|a| a.folder(*role).map(|f| (a.config.id.clone(), f.path.clone()))).collect(),
                    _ => return Err(CmdError::Disabled("open a folder first".into())),
                },
            };
            s.clean_up(&folders, None)
        })
        .params(r#"{"account"?: string, "folder"?: string (none: the folder on screen)}"#),
        CommandSpec::new("mail.junk", "Report junk", "Home › Delete", |s, _| s.move_selection(MoveTarget::Role(FolderRole::Junk))).when(has_selection),
        CommandSpec::new("mail.notJunk", "Not junk", "Home › Delete", |s, v| {
            // "Always trust mail from this sender", as Outlook asks.
            if p::bool(v, "always").unwrap_or(false) {
                s.junk_list_selection(JunkList::Safe, false)?;
                return Ok(json!({"trusted": true}));
            }
            s.move_selection(MoveTarget::Role(FolderRole::Inbox))
        })
        .params(r#"{"always"?: bool}"#)
        .when(has_selection),
        // Home › Delete › Junk
        CommandSpec::new("junk.blockSender", "Block sender", "Home › Delete › Junk", |s, _| s.junk_list_selection(JunkList::Blocked, false))
            .when(has_selection),
        CommandSpec::new("junk.neverBlockSender", "Never block sender", "Home › Delete › Junk", |s, _| s.junk_list_selection(JunkList::Safe, false))
            .when(has_selection),
        CommandSpec::new("junk.neverBlockDomain", "Never block sender's domain", "Home › Delete › Junk", |s, _| {
            s.junk_list_selection(JunkList::Safe, true)
        })
        .when(has_selection),
        CommandSpec::new("junk.lists", "Junk Email options", "Home › Delete › Junk", |s, _| {
            Ok(json!({"blocked": s.prefs.blocked_senders, "safe": s.prefs.safe_senders}))
        }),
        CommandSpec::new("junk.edit", "Edit a junk list", "Home › Delete › Junk › Junk Email options", |s, v| {
            let which = match p::req_str(v, "list")? {
                "blocked" => JunkList::Blocked,
                "safe" => JunkList::Safe,
                other => return Err(CmdError::Params(format!("no list `{other}` (blocked or safe)"))),
            };
            let entry = s.set_junk_entry(which, p::req_str(v, "entry")?, p::bool(v, "on").unwrap_or(true))?;
            Ok(json!({"entry": entry, "blocked": s.prefs.blocked_senders, "safe": s.prefs.safe_senders}))
        })
        .params(r#"{"list": "blocked"|"safe", "entry": string (an address or @domain), "on"?: bool}"#),
        // Home › Respond
        CommandSpec::new("mail.reply", "Reply", "Home › Respond", |s, _| s.reply_compose(ReplyMode::Reply)).key("Mod+R").when(has_reading),
        CommandSpec::new("mail.replyAll", "Reply all", "Home › Respond", |s, _| s.reply_compose(ReplyMode::ReplyAll)).key("Mod+Shift+R").when(has_reading),
        CommandSpec::new("mail.forward", "Forward", "Home › Respond", |s, _| s.reply_compose(ReplyMode::Forward)).key("Mod+F").when(has_reading),
        CommandSpec::new("mail.forwardAsAttachment", "Forward as attachment", "Reading pane › More", |s, _| s.forward_as_attachment()).when(has_reading),
        // Home › Move
        CommandSpec::new("mail.move", "Move to", "Home › Move", move_to).key("Mod+Shift+V").params(r#"{"folder": string}"#).when(has_selection),
        // Home › Tags
        CommandSpec::new("mail.markRead", "Mark as read", "Home › Tags", |s, _| s.flag_selection(Flag::Seen, Some(true))).key("Mod+Q").when(has_selection),
        CommandSpec::new("mail.markUnread", "Mark as unread", "Home › Tags", |s, _| s.flag_selection(Flag::Seen, Some(false))).key("Mod+U").when(has_selection),
        CommandSpec::new("mail.toggleRead", "Read / Unread", "Home › Tags", |s, _| s.flag_selection(Flag::Seen, None)).when(has_selection),
        CommandSpec::new("mail.flag", "Flag / Unflag", "Home › Tags", flag)
            .key("Insert")
            .params(r#"{"on"?: bool, "due"?: "today"|"tomorrow"|"thisWeek"|"nextWeek"|"none"|"YYYY-MM-DD" (Follow Up: flag, due then), "complete"?: true (Mark Complete: unflag, forget the date)}"#)
            .when(has_selection),
        CommandSpec::new("mail.markAllRead", "Mark all as read", "Folder", mark_all_read).when(has_account),
        CommandSpec::new("mail.categorize", "Categorize", "Home › Tags", |s, v| s.categorize_selection(p::req_str(v, "category")?, p::bool(v, "on")))
            .params(r#"{"category": string, "on"?: bool}"#)
            .when(has_selection),
        CommandSpec::new("mail.pin", "Pin / Unpin", "Home › Tags", |s, v| s.pin_selection(p::bool(v, "on"))).params(r#"{"on"?: bool}"#).when(has_selection),
        CommandSpec::new("mail.snooze", "Snooze", "Home › Tags", snooze)
            .params(r#"{"preset": "later"|"tomorrow"|"weekend"|"nextWeek"} | {"until": unix seconds}"#)
            .when(has_selection),
        CommandSpec::new("mail.unsnooze", "Unsnooze", "Home › Tags", |s, _| s.unsnooze_selection()).when(has_selection),
        CommandSpec::new("mail.moveToFocused", "Move to Focused", "Home › Move", |s, _| s.set_focus_for_selection(true)).when(has_selection),
        CommandSpec::new("mail.moveToOther", "Move to Other", "Home › Move", |s, _| s.set_focus_for_selection(false)).when(has_selection),
        // Categories
        CommandSpec::new("category.list", "Categories", "Home › Tags › Categorize", |s, _| {
            Ok(json!(s.prefs.categories.iter().map(|c| json!({"name": c.name, "keyword": c.keyword, "color": c.color})).collect::<Vec<_>>()))
        }),
        CommandSpec::new("category.create", "New category", "Home › Tags › Categorize", |s, v| {
            s.add_category(p::req_str(v, "name")?, p::u64(v, "color").and_then(|c| u8::try_from(c).ok()))
        })
        .params(r#"{"name": string, "color"?: number}"#),
        CommandSpec::new("category.delete", "Delete category", "Home › Tags › Categorize", |s, v| s.remove_category(p::req_str(v, "name")?))
            .params(r#"{"name": string}"#),
        // Rules
        CommandSpec::new("rules.list", "Rules", "Home › Rules", |s, _| Ok(json!(s.rules))),
        CommandSpec::new("rules.set", "Replace rules", "Home › Rules", |s, v| {
            let rules = v.get("rules").cloned().ok_or_else(|| CmdError::Params("`rules` is required".into()))?;
            let rules = serde_json::from_value(rules).map_err(|e| CmdError::Params(format!("rules: {e}")))?;
            s.set_rules(rules)
        })
        .params(r#"{"rules": [Rule]} (see rules.add)"#),
        CommandSpec::new("rules.add", "New rule", "Home › Rules", |s, v| {
            let rule = v.get("rule").cloned().ok_or_else(|| CmdError::Params("`rule` is required".into()))?;
            let rule = serde_json::from_value(rule).map_err(|e| CmdError::Params(format!("rule: {e}")))?;
            let mut rules = s.rules.clone();
            rules.push(rule);
            s.set_rules(rules)
        })
        .params(
            r#"{"rule": {"name": string, "conditions": [{"if": "from"|"to"|"subject"|"anyText", "value": string} | {"if": "hasAttachment"|"bulk"}], "matchAll"?: bool, "actions": [{"do": "moveTo", "value": folder} | {"do": "moveToRole", "value": "archive"|…} | {"do": "categorize", "value": name} | {"do": "markRead"|"flag"|"pin"|"delete"}], "account"?: string, "stop"?: bool, "enabled"?: bool}}"#,
        ),
        CommandSpec::new("rules.delete", "Delete rule", "Home › Rules", |s, v| {
            let i = p::u64(v, "index").and_then(|i| usize::try_from(i).ok()).ok_or_else(|| CmdError::Params("`index` is required".into()))?;
            if i >= s.rules.len() {
                return Err(CmdError::Params(format!("no rule {i}")));
            }
            let mut rules = s.rules.clone();
            rules.remove(i);
            s.set_rules(rules)
        })
        .params(r#"{"index": number}"#),
        CommandSpec::new("rules.export", "Export rules", "Home › Rules › Options", |s, v| {
            let path = p::req_str(v, "path")?;
            let bytes = s.rules_file()?;
            std::fs::write(path, bytes).map_err(|e| CmdError::Failed(format!("{path}: {e}")))?;
            Ok(json!({ "rules": s.rules.len(), "path": path }))
        })
        .params(r#"{"path": string}"#),
        CommandSpec::new("rules.import", "Import rules", "Home › Rules › Options", |s, v| {
            let path = p::req_str(v, "path")?;
            let bytes = read_capped(path, 4 << 20)?;
            s.import_rules(&bytes, p::bool(v, "replace").unwrap_or(false))
        })
        .params(r#"{"path": string, "replace"?: bool (in place of the rules here; else added after them)}"#),
        CommandSpec::new("rules.run", "Run rules now", "Home › Rules", |s, v| {
            s.run_rules_now(p::u64(v, "index").and_then(|i| usize::try_from(i).ok()), p::bool(v, "inbox").unwrap_or(false))
        })
        .params(r#"{"index"?: number, "inbox"?: bool (the mail already in the Inbox, not the list on screen)}"#)
            .when(has_account),
    ]
}

pub(super) fn new_mail(s: &mut Session, v: &Value) -> CmdResult {
    // Who it's to, checked before anything opens.
    let to = match p::str(v, "to").map(str::trim).filter(|t| !t.is_empty()) {
        Some(t) => mailcraft_model::addr::parse_list(t).map_err(|bad| CmdError::Params(format!("not an email address: {bad}")))?,
        None => Vec::new(),
    };
    let c = s.new_compose(p::str(v, "account"))?;
    if !to.is_empty() {
        c.to = mailcraft_model::addr::format_field(&to);
    }
    Ok(json!({"compose": c.id}))
}

pub(super) fn delete(s: &mut Session, v: &Value) -> CmdResult {
    if s.view == View::Outbox && s.search.is_none() {
        return s.delete_outbox_selection();
    }
    let in_trash = s.view_role() == Some(FolderRole::Trash) && s.search.is_none();
    let all_in_trash =
        s.selected().iter().all(|m| s.account(&m.key.account).and_then(|a| a.folder(FolderRole::Trash)).is_some_and(|t| t.path == m.key.folder));
    if p::bool(v, "permanent").unwrap_or(false) || in_trash || all_in_trash {
        return s.expunge_selection();
    }
    s.move_selection(MoveTarget::Role(FolderRole::Trash))
}

pub(super) fn move_to(s: &mut Session, v: &Value) -> CmdResult {
    let folder = p::req_str(v, "folder")?;
    if let Some(role) = folder.strip_prefix("role:").and_then(FolderRole::from_code) {
        return s.move_selection(MoveTarget::Role(role));
    }
    s.move_selection(MoveTarget::Folder(folder.to_string()))
}

pub(super) fn snooze(s: &mut Session, v: &Value) -> CmdResult {
    let until = match (p::str(v, "preset"), v.get("until").and_then(Value::as_i64)) {
        (Some(code), _) => {
            mailcraft_model::organize::SnoozeUntil::from_code(code).ok_or_else(|| CmdError::Params(format!("unknown preset `{code}`")))?.at(&s.clock)
        }
        (None, Some(t)) => t,
        (None, None) => return Err(CmdError::Params("`preset` or `until` is required".into())),
    };
    s.snooze_selection(until)
}

/// Flag or unflag; or Follow Up by a day, or Mark Complete.
pub(super) fn flag(s: &mut Session, v: &Value) -> CmdResult {
    if p::bool(v, "complete") == Some(true) {
        return s.follow_up_selection(None, true);
    }
    match p::str(v, "due") {
        None => s.flag_selection(Flag::Flagged, p::bool(v, "on")),
        Some(d) if d.eq_ignore_ascii_case("none") => s.follow_up_selection(None, false),
        Some(d) => {
            let day = match mailcraft_model::dates::FollowUp::from_code(d) {
                Some(f) => f.due(&s.clock),
                None if mailcraft_model::job::is_day(d) => {
                    let num = |a: usize, z: usize| d.get(a..z).and_then(|x| x.parse::<u32>().ok()).unwrap_or(1);
                    mailcraft_model::dates::days_from_civil(i64::from(num(0, 4)), num(5, 7), num(8, 10))
                }
                None => return Err(CmdError::Params(format!("`due`: not today, tomorrow, thisWeek, nextWeek, none or a date: `{d}`"))),
            };
            s.follow_up_selection(Some(day), false)
        }
    }
}

pub(super) fn mark_all_read(s: &mut Session, _: &Value) -> CmdResult {
    let keys: Vec<_> = s.list.iter().filter(|m| !m.flags.seen).map(|m| m.key.clone()).collect();
    let n = keys.len();
    let mut by_account: std::collections::BTreeMap<String, Vec<_>> = Default::default();
    for k in keys {
        by_account.entry(k.account.clone()).or_default().push(k);
    }
    for (a, keys) in by_account {
        s.apply_op(&a, mailcraft_model::Op::SetFlag { keys, flag: Flag::Seen, on: true });
    }
    Ok(json!({"count": n}))
}
