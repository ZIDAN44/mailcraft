//! The command registry: every user-visible action, by id.
//!
//! Shortcuts follow the ones mail users already know: Ctrl+N new, Ctrl+R reply, Ctrl+Shift+R
//! reply all, Ctrl+F forward, Ctrl+Q / Ctrl+U read / unread, Insert flag, Delete, Backspace
//! archive, Ctrl+Shift+V move, F9 send/receive, Ctrl+Enter send.

use mailcraft_mime::{OutAttachment, ReplyMode, mime_from_name, sanitize_filename};
use mailcraft_model::{AccountConfig, Flag, FolderRole, Security, ServerConfig};
use serde_json::{Value, json};

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

pub fn registry() -> Registry {
    Registry::new(vec![
        // Home › New
        CommandSpec::new("mail.new", "New mail", "Home › New", new_mail).key("Mod+N").params(r#"{"account"?: string}"#).when(has_account),
        // Home › Delete
        CommandSpec::new("mail.delete", "Delete", "Home › Delete", delete).key("Delete").params(r#"{"permanent"?: bool}"#).when(has_selection),
        CommandSpec::new("mail.archive", "Archive", "Home › Delete", |s, _| s.move_selection(MoveTarget::Role(FolderRole::Archive)))
            .key("Backspace")
            .when(has_selection),
        CommandSpec::new("mail.ignore", "Ignore conversation", "Home › Delete", |s, _| s.ignore_selection()).when(has_selection),
        CommandSpec::new("mail.junk", "Report junk", "Home › Delete", |s, _| s.move_selection(MoveTarget::Role(FolderRole::Junk))).when(has_selection),
        CommandSpec::new("mail.notJunk", "Not junk", "Home › Delete", |s, _| s.move_selection(MoveTarget::Role(FolderRole::Inbox))).when(has_selection),
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
        CommandSpec::new("rules.run", "Run rules now", "Home › Rules", |s, v| {
            s.run_rules_now(p::u64(v, "index").and_then(|i| usize::try_from(i).ok()), p::bool(v, "inbox").unwrap_or(false))
        })
        .params(r#"{"index"?: number, "inbox"?: bool (the mail already in the Inbox, not the list on screen)}"#)
            .when(has_account),
        // Send / Receive
        CommandSpec::new("mail.sync", "Send / Receive all folders", "Send / Receive", |s, _| Ok(json!({"accounts": s.sync_all()}))).key("F9").when(can_connect),
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
        CommandSpec::new("list.filter", "Filter", "Message list", filter).params(r#"{"value": "all"|"unread"|"flagged"|"attachments"|"categorized"|"important"|"toMe"}"#),
        CommandSpec::new("list.messages", "List messages", "Message list", list_messages).params(r#"{"limit"?: number}"#),
        CommandSpec::new("message.read", "Read message", "Reading pane", read_message).params(r#"{"key"?: string}"#),
        CommandSpec::new("message.popOut", "Open in new window", "Message list › Open", |s, v| {
            let k = match parse_key(v) {
                Some(_) => key_param(s, v)?,
                None => s.focus.clone().or_else(|| s.selection.first().cloned()).or_else(|| s.reading.as_ref().map(|r| r.key.clone())).ok_or_else(|| CmdError::Disabled("select a message first".into()))?,
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
        CommandSpec::new("message.blockPictures", "Don't show pictures from this sender", "Reading pane", |s, _| s.block_pictures()).when(has_reading),
        CommandSpec::new("view.html", "Show HTML formatted", "View › Messages", |s, v| {
            s.prefs.show_html = p::bool(v, "on").unwrap_or(!s.prefs.show_html);
            Ok(json!({"on": s.prefs.show_html}))
        })
        .params(r#"{"on"?: bool}"#),
        // Search
        CommandSpec::new("search.run", "Search", "Search box", search).key("Mod+E").params(r#"{"query": string, "scope"?: "all"|"current"}"#),
        CommandSpec::new("search.clear", "Clear search", "Search box", |s, _| Ok(json!({"results": s.run_search("", SearchScope::All)}))).key("Escape"),
        // Compose
        CommandSpec::new("compose.update", "Edit message", "Message", compose_update)
            .params(r#"{"to"?: string, "cc"?: string, "bcc"?: string, "subject"?: string, "body"?: string, "marks"?: bool (the body uses **bold**, *italic*, - lists…), "account"?: string, "importance"?: "high"|"normal"|"low", "readReceipt"?: bool, "replyTo"?: string (where replies go), "sendAfter"?: number (Unix seconds: held in the Outbox until then; 0 sends at once)}"#)
            .when(has_compose),
        CommandSpec::new("compose.attach", "Attach file", "Message › Insert", attach).params(r#"{"path": string} | {"name": string, "data": [bytes]}"#).when(has_compose),
        CommandSpec::new("compose.insertPicture", "Insert picture", "Message › Insert", insert_picture)
            .params(r#"{"path": string} | {"name": string, "data": [bytes]}, "insert"?: bool (default true), "at"?: number (character; default the end)"#)
            .when(has_compose),
        CommandSpec::new("compose.format", "Plain text / Formatted", "Message › Format", |s, v| {
            let c = s.compose.as_mut().ok_or_else(|| CmdError::Disabled("no message is being written".into()))?;
            c.plain = p::bool(v, "plain").unwrap_or(!c.plain);
            Ok(json!({"plain": c.plain}))
        })
        .params(r#"{"plain"?: bool}"#)
        .when(has_compose),
        CommandSpec::new("compose.removeAttachment", "Remove attachment", "Message › Insert", remove_attachment).params(r#"{"index": number}"#).when(has_compose),
        CommandSpec::new("compose.send", "Send", "Message", |s, v| s.send_compose(p::bool(v, "allowEmptySubject").unwrap_or(false)))
            .key("Mod+Enter / Alt+S")
            .params(r#"{"allowEmptySubject"?: bool}"#)
            .when(has_compose),
        CommandSpec::new("compose.saveDraft", "Save draft", "Message", |s, _| s.save_draft()).key("Mod+S").when(has_compose),
        CommandSpec::new("compose.discard", "Discard", "Message", discard).when(has_compose),
        CommandSpec::new("compose.popOut", "Pop out", "Message", |s, _| {
            let c = s.compose.as_mut().ok_or_else(|| CmdError::Disabled("no message is being written".into()))?;
            c.window = true;
            Ok(json!({"window": true}))
        })
        .when(has_compose),
        CommandSpec::new("compose.popIn", "Pop in", "Message", |s, _| {
            let c = s.compose.as_mut().ok_or_else(|| CmdError::Disabled("no message is being written".into()))?;
            c.window = false;
            Ok(json!({"window": false}))
        })
        .when(has_compose),
        CommandSpec::new("compose.close", "Close", "Message", close_compose).params(r#"{"save"?: bool (keep it in Drafts; default true)}"#).when(has_compose),
        CommandSpec::new("contacts.suggest", "Suggest recipients", "Message › To", |s, v| {
            let q = p::req_str(v, "query")?.to_string();
            let limit = p::u64(v, "limit").map_or(8, |n| usize::try_from(n).unwrap_or(8).clamp(1, 100));
            let list: Vec<Value> = s
                .suggest_contacts(&q, limit)
                .into_iter()
                .map(|c| json!({"name": c.address.name, "email": c.address.email, "address": c.address.to_string(), "sent": c.sent, "received": c.received}))
                .collect();
            Ok(json!(list))
        })
        .params(r#"{"query": string, "limit"?: number}"#),
        // Attachments
        CommandSpec::new("attachment.save", "Save attachment", "Reading pane", save_attachment).params(r#"{"index": number, "path": string}"#).when(has_reading),
        CommandSpec::new("attachment.saveAll", "Save all attachments", "Reading pane", save_all_attachments).params(r#"{"dir": string}"#).when(has_reading),
        CommandSpec::new("attachment.preview", "Preview attachment", "Reading pane", preview_attachment).params(r#"{"index": number}"#).when(has_reading),
        // View
        CommandSpec::new("view.readingPane", "Reading pane", "View › Layout", reading_pane).params(r#"{"value": "right"|"bottom"|"off"}"#),
        CommandSpec::new("view.conversations", "Show as conversations", "View › Messages", |s, v| {
            s.prefs.conversations = p::bool(v, "on").unwrap_or(!s.prefs.conversations);
            s.refresh_list();
            Ok(json!({"on": s.prefs.conversations}))
        })
        .params(r#"{"on"?: bool}"#),
        CommandSpec::new("view.preview", "Message preview", "View › Messages", |s, v| {
            s.prefs.show_preview = p::bool(v, "on").unwrap_or(!s.prefs.show_preview);
            Ok(json!({"on": s.prefs.show_preview}))
        })
        .params(r#"{"on"?: bool}"#),
        CommandSpec::new("view.compact", "Compact list", "View › Layout", |s, v| {
            s.prefs.compact = p::bool(v, "on").unwrap_or(!s.prefs.compact);
            Ok(json!({"on": s.prefs.compact}))
        })
        .params(r#"{"on"?: bool}"#),
        CommandSpec::new("view.arrange", "Arrange by", "View › Arrangement", |s, v| {
            let by = p::req_str(v, "by")?;
            s.prefs.arrange =
                crate::session::Arrange::from_code(by).ok_or_else(|| CmdError::Params(format!("`{by}` isn't date, from, to, subject, size, flag or attachments")))?;
            // A new arrangement starts in its natural order.
            s.prefs.reverse_sort = p::bool(v, "reverse").unwrap_or(false);
            s.refresh_list();
            Ok(json!({"by": s.prefs.arrange.code(), "reverse": s.prefs.reverse_sort}))
        })
        .params(r#"{"by": "date"|"from"|"to"|"subject"|"size"|"flag"|"attachments", "reverse"?: bool}"#),
        CommandSpec::new("view.reverseSort", "Reverse sort", "View › Arrangement", |s, v| {
            s.prefs.reverse_sort = p::bool(v, "on").unwrap_or(!s.prefs.reverse_sort);
            s.refresh_list();
            Ok(json!({"on": s.prefs.reverse_sort}))
        })
        .params(r#"{"on"?: bool}"#),
        CommandSpec::new("view.folderPane", "Minimize the folder pane", "View › Layout", |s, v| {
            s.prefs.folder_pane_minimized = p::bool(v, "minimized").unwrap_or(!s.prefs.folder_pane_minimized);
            Ok(json!({"minimized": s.prefs.folder_pane_minimized}))
        })
        .params(r#"{"minimized"?: bool}"#),
        CommandSpec::new("view.collapse", "Collapse or expand in the folder pane", "Folder pane", |s, v| {
            let key = p::req_str(v, "key")?.trim();
            if key.is_empty() || key.len() > 600 {
                return Err(CmdError::Params("`key` names what to collapse: favorites, account:<id> or folder:<account>/<path>".into()));
            }
            let collapse = p::bool(v, "collapsed").unwrap_or(!s.prefs.collapsed.contains(key));
            if collapse {
                // A bounded list: the oldest go first when there are very many.
                while s.prefs.collapsed.len() >= 2_000 {
                    let Some(first) = s.prefs.collapsed.iter().next().cloned() else { break };
                    s.prefs.collapsed.remove(&first);
                }
                s.prefs.collapsed.insert(key.to_string());
            } else {
                s.prefs.collapsed.remove(key);
            }
            Ok(json!({"key": key, "collapsed": collapse}))
        })
        .params(r#"{"key": string (favorites, account:<id> or folder:<account>/<path>), "collapsed"?: bool (default: the other way)}"#),
        CommandSpec::new("view.focused", "Focused Inbox", "View › Messages", |s, v| {
            s.prefs.focused_inbox = p::bool(v, "on").unwrap_or(!s.prefs.focused_inbox);
            s.focus_other = false;
            s.refresh_list();
            Ok(json!({"on": s.prefs.focused_inbox}))
        })
        .params(r#"{"on"?: bool}"#),
        CommandSpec::new("list.focusTab", "Focused / Other", "Message list", |s, v| {
            s.focus_other = match p::req_str(v, "value")? {
                x if x.eq_ignore_ascii_case("other") => true,
                x if x.eq_ignore_ascii_case("focused") => false,
                x => return Err(CmdError::Params(format!("unknown tab `{x}`"))),
            };
            s.selection.clear();
            s.reading = None;
            s.refresh_list();
            Ok(json!({"messages": s.list.len()}))
        })
        .params(r#"{"value": "focused"|"other"}"#),
        CommandSpec::new("view.confirmDelete", "Ask before deleting for good", "File › Options › Mail", |s, v| {
            s.prefs.confirm_permanent_delete = p::bool(v, "on").unwrap_or(!s.prefs.confirm_permanent_delete);
            Ok(json!({"on": s.prefs.confirm_permanent_delete}))
        })
        .params(r#"{"on"?: bool}"#),
        CommandSpec::new("view.notifications", "Desktop notifications", "File › Options › Mail", |s, v| {
            s.prefs.notify = p::bool(v, "on").unwrap_or(!s.prefs.notify);
            Ok(json!({"on": s.prefs.notify}))
        })
        .params(r#"{"on"?: bool}"#),
        CommandSpec::new("view.newMailWindow", "New messages in their own window", "File › Options › Mail", |s, v| {
            s.prefs.new_mail_window = p::bool(v, "on").unwrap_or(!s.prefs.new_mail_window);
            Ok(json!({"on": s.prefs.new_mail_window}))
        })
        .params(r#"{"on"?: bool}"#),
        CommandSpec::new("view.markReadOnOpen", "Mark as read when opened", "File › Options › Mail", |s, v| {
            s.prefs.mark_read_on_open = p::bool(v, "on").unwrap_or(!s.prefs.mark_read_on_open);
            Ok(json!({"on": s.prefs.mark_read_on_open}))
        })
        .params(r#"{"on"?: bool}"#),
        CommandSpec::new("view.trustSender", "Show pictures from a sender", "File › Options › Privacy", |s, v| {
            let who = p::req_str(v, "address")?.trim().to_ascii_lowercase();
            if !mailcraft_model::addr::is_valid_email(&who) {
                return Err(CmdError::Params(format!("`{who}` is not an email address")));
            }
            if p::bool(v, "on").unwrap_or(true) {
                s.prefs.trusted_senders.insert(who.clone());
            } else {
                s.prefs.trusted_senders.remove(&who);
            }
            Ok(json!({"trusted": s.prefs.trusted_senders.iter().collect::<Vec<_>>()}))
        })
        .params(r#"{"address": string, "on"?: bool}"#),
        // Accounts
        // Calendar and People
        CommandSpec::new("view.module", "Mail, Calendar, People or Notes", "Navigation", |s, v| {
            s.module = match p::req_str(v, "name")? {
                "mail" => crate::Module::Mail,
                "calendar" => crate::Module::Calendar,
                "people" => crate::Module::People,
                "notes" => crate::Module::Notes,
                other => return Err(CmdError::Params(format!("unknown module `{other}`"))),
            };
            Ok(json!({"module": s.module}))
        })
        .params(r#"{"name": "mail"|"calendar"|"people"|"notes"}"#),
        // Outlook's keys for its modules: Ctrl+1 Mail, Ctrl+2 Calendar, Ctrl+3 People, Ctrl+5 Notes.
        CommandSpec::new("view.mail", "Mail", "Navigation", |s, _| {
            s.module = crate::Module::Mail;
            Ok(json!({"module": s.module}))
        })
        .key("Mod+1"),
        CommandSpec::new("view.calendar", "Calendar", "Navigation", |s, _| {
            s.module = crate::Module::Calendar;
            Ok(json!({"module": s.module}))
        })
        .key("Mod+2"),
        CommandSpec::new("view.people", "People", "Navigation", |s, _| {
            s.module = crate::Module::People;
            Ok(json!({"module": s.module}))
        })
        .key("Mod+3"),
        CommandSpec::new("view.notes", "Notes", "Navigation", |s, _| {
            s.module = crate::Module::Notes;
            Ok(json!({"module": s.module}))
        })
        .key("Mod+5"),
        CommandSpec::new("calendar.view", "Day, week or month", "Calendar › View", |s, v| {
            s.cal_mode = match p::req_str(v, "mode")? {
                "day" => crate::CalMode::Day,
                "week" => crate::CalMode::Week,
                "month" => crate::CalMode::Month,
                other => return Err(CmdError::Params(format!("unknown view `{other}`"))),
            };
            s.cal_goto(None, 0)
        })
        .params(r#"{"mode": "day"|"week"|"month"}"#),
        CommandSpec::new("calendar.goto", "Go to a date", "Calendar › Go to", |s, v| s.cal_goto(p::str(v, "date"), p::i64(v, "step").unwrap_or(0)))
            .params(r#"{"date"?: "YYYY-MM-DD"|"today", "step"?: number (days, weeks or months, as on screen)}"#),
        CommandSpec::new("calendar.events", "Events", "Calendar", |s, v| {
            let off = i64::from(s.clock.offset);
            let (d0, d1) = s.cal_days();
            let from = p::str(v, "from").and_then(|x| parse_local(x, off)).map_or(s.local_midnight(d0), |(t, all_day)| if all_day { t - off } else { t });
            let to = p::str(v, "to").and_then(|x| parse_local(x, off)).map_or(s.local_midnight(d1), |(t, all_day)| if all_day { t - off + 86_400 } else { t });
            Ok(json!(s.agenda(from, to).iter().map(|i| json!({
                "uid": i.uid, "summary": i.summary, "location": i.location, "start": i.start, "end": i.end, "allDay": i.all_day,
                "occurrence": i.occurrence, "recurring": i.recurring, "account": i.account, "calendar": i.calendar,
                "needsReply": i.needs_reply, "status": i.status,
            })).collect::<Vec<_>>()))
        })
        .params(r#"{"from"?: date or date-time, "to"?: …} (default: what the calendar shows)"#),
        CommandSpec::new("event.create", "New event", "Calendar › New event", |s, v| {
            let mut e = event_from(s, v, mailcraft_model::ical::CalEvent::default())?;
            let account = match p::str(v, "account") {
                Some(a) => a.to_string(),
                None => s.current_account().map(|a| a.config.id.clone()).ok_or_else(|| CmdError::Disabled("add an account first".into()))?,
            };
            // With guests, the user organizes it.
            if !e.attendees.is_empty() && e.organizer.is_none() {
                let me = s.account(&account).map(|a| (a.config.name.clone(), a.config.email.clone())).unwrap_or_default();
                e.organizer = Some(mailcraft_model::ical::Attendee { email: me.1, name: me.0, partstat: "ACCEPTED".into(), rsvp: false });
            }
            let r = s.create_event(Some(&account), p::str(v, "calendar"), e.clone())?;
            if let Some(uid) = r.get("uid").and_then(Value::as_str) {
                e.uid = uid.to_string();
                s.send_invitations(&e, &account, "REQUEST")?;
            }
            Ok(r)
        })
        .params(r#"{"summary": string, "start": "YYYY-MM-DD[THH:MM]", "end"?: …, "allDay"?: bool, "location"?: string, "description"?: string, "repeat"?: "daily"|"weekly"|"monthly"|"yearly"|RRULE, "attendees"?: "a@x, Name <b@y>" (invited by mail), "account"?: string, "calendar"?: string}"#),
        CommandSpec::new("event.update", "Edit event", "Calendar › Event", |s, v| {
            let uid = p::req_str(v, "uid")?.to_string();
            let current = s.find_event(&uid).map(|(_, e)| e.event.clone()).ok_or_else(|| CmdError::Params(format!("no event `{uid}`")))?;
            let e = event_from(s, v, current)?;
            let r = s.update_event(&uid, |x| *x = e)?;
            // The user's own meeting: the guests hear of the change.
            if let Some((account, stored)) = s.find_event(&uid).map(|(a, e)| (a.to_string(), e.event.clone()))
                && organized_by_me(s, &stored)
            {
                s.send_invitations(&stored, &account, "REQUEST")?;
            }
            Ok(r)
        })
        .params(r#"{"uid": string, … the fields of event.create to change}"#),
        CommandSpec::new("event.delete", "Delete event", "Calendar › Event", |s, v| {
            let uid = p::req_str(v, "uid")?.to_string();
            let before = s.find_event(&uid).map(|(a, e)| (a.to_string(), e.event.clone()));
            let occurrence = p::i64(v, "occurrence");
            let r = s.delete_event(&uid, occurrence)?;
            if let Some((account, mut e)) = before
                && organized_by_me(s, &e)
            {
                // One occurrence off: the guests are told of that one.
                if let Some(at) = occurrence.filter(|_| e.rrule.is_some()) {
                    let len = e.end - e.start;
                    e.recurrence_id = Some(at);
                    e.rrule = None;
                    e.start = at;
                    e.end = at + len;
                }
                e.sequence = e.sequence.saturating_add(1);
                s.send_invitations(&e, &account, "CANCEL")?;
            }
            Ok(r)
        })
        .params(r#"{"uid": string, "occurrence"?: unix seconds (only that occurrence)}"#),
        CommandSpec::new("invite.respond", "Answer the invitation", "Reading pane › Invitation", |s, v| s.respond_to_invite(p::req_str(v, "answer")?))
            .params(r#"{"answer": "accepted"|"tentative"|"declined"}"#)
            .when(has_reading),
        CommandSpec::new("invite.apply", "Update the calendar", "Reading pane › Invitation", |s, _| s.apply_invite())
            .params("(an answer from a guest, or a cancellation)")
            .when(has_reading),
        CommandSpec::new("event.respond", "Answer an invitation", "Calendar › Event", |s, v| {
            let uid = p::req_str(v, "uid")?.to_string();
            s.respond_to_event(&uid, p::req_str(v, "answer")?)
        })
        .params(r#"{"uid": string, "answer": "accepted"|"tentative"|"declined"}"#),
        CommandSpec::new("contact.list", "People", "People", |s, v| {
            let q = p::str(v, "query").unwrap_or_default();
            Ok(json!(s.people(q).iter().map(|x| json!({"uid": x.card.uid, "name": x.card.display(), "emails": x.card.emails, "phones": x.card.phones, "org": x.card.org, "title": x.card.title, "account": x.account})).collect::<Vec<_>>()))
        })
        .params(r#"{"query"?: string}"#),
        CommandSpec::new("contact.addressBook", "Address book", "Message › Names", |s, v| {
            let list = s.address_book(p::str(v, "source").unwrap_or("all"), p::str(v, "query").unwrap_or_default());
            Ok(json!(list.iter().map(|a| json!({"name": a.name, "email": a.email})).collect::<Vec<_>>()))
        })
        .key("Mod+Shift+B")
        .params(r#"{"query"?: string, "source"?: "all" | "people"}"#),
        CommandSpec::new("contact.create", "New contact", "People › New contact", |s, v| {
            let c = card_from(v, mailcraft_model::vcard::Card::default())?;
            s.create_card(p::str(v, "account"), c)
        })
        .params(r#"{"name": string, "emails"?: [string] | "a, b", "phones"?: [string] | "…", "org"?: string, "title"?: string, "note"?: string, "account"?: string}"#),
        CommandSpec::new("contact.update", "Edit contact", "People › Contact", |s, v| {
            let uid = p::req_str(v, "uid")?.to_string();
            let current = s.find_person(&uid).map(|x| x.card).ok_or_else(|| CmdError::Params(format!("no contact `{uid}`")))?;
            let c = card_from(v, current)?;
            s.update_card(&uid, |x| *x = c)
        })
        .params(r#"{"uid": string, … the fields of contact.create to change}"#),
        CommandSpec::new("contact.delete", "Delete contact", "People › Contact", |s, v| {
            let uid = p::req_str(v, "uid")?.to_string();
            s.delete_card(&uid)
        })
        .params(r#"{"uid": string}"#),
        CommandSpec::new("contact.fromMessage", "Add sender to contacts", "Reading pane", |s, _| {
            let from = s.reading.as_ref().and_then(|r| r.parsed.as_ref()).map(|p| p.from.clone()).ok_or_else(|| CmdError::Disabled("open a message first".into()))?;
            if let Some(existing) = s.people(&from.email).into_iter().find(|x| x.card.emails.iter().any(|e| e.value.eq_ignore_ascii_case(&from.email))) {
                return Ok(json!({"uid": existing.card.uid, "existing": true}));
            }
            let card = mailcraft_model::vcard::Card {
                name: from.name.clone(),
                emails: vec![mailcraft_model::vcard::Typed { value: from.email.clone(), kind: String::new() }],
                ..Default::default()
            };
            let account = s.reading.as_ref().map(|r| r.key.account.clone());
            s.create_card(account.as_deref(), card)
        })
        .when(has_reading),
        CommandSpec::new("calendar.openShared", "Open shared calendar", "Calendar", |s, v| s.open_shared_calendar(p::str(v, "account"), p::req_str(v, "email")?))
            .params(r#"{"email": string (whose calendar), "account"?: string (an Exchange account; else the first)}"#),
        CommandSpec::new("calendar.closeShared", "Close shared calendar", "Calendar", |s, v| s.close_shared_calendar(p::req_str(v, "email")?))
            .params(r#"{"email": string}"#),
        CommandSpec::new("note.list", "Notes", "Notes", |s, v| {
            let list: Vec<Value> = s
                .notes(p::str(v, "query").unwrap_or(""))
                .into_iter()
                .map(|n| json!({"uid": n.note.uid, "text": n.note.text, "color": n.note.color, "modified": n.note.modified, "account": n.account}))
                .collect();
            Ok(json!({"notes": list}))
        })
        .params(r#"{"query"?: string}"#),
        CommandSpec::new("note.create", "New note", "Notes", |s, v| {
            let color = p::u64(v, "color").and_then(|c| u8::try_from(c).ok());
            s.create_note(p::str(v, "account"), p::str(v, "text").unwrap_or(""), color)
        })
        .params(r#"{"text"?: string, "color"?: 0|1|2|3|4 (blue, green, pink, yellow, white), "account"?: string}"#),
        CommandSpec::new("note.update", "Edit note", "Notes", |s, v| {
            let color = p::u64(v, "color").and_then(|c| u8::try_from(c).ok());
            s.update_note(p::req_str(v, "uid")?, p::str(v, "text"), color)
        })
        .params(r#"{"uid": string, "text"?: string, "color"?: 0|1|2|3|4}"#),
        CommandSpec::new("note.delete", "Delete note", "Notes", |s, v| s.delete_note(p::req_str(v, "uid")?)).params(r#"{"uid": string}"#),
        CommandSpec::new("folder.empty", "Empty folder", "Folder › Empty", |s, v| {
            let account = account_param(s, v)?;
            let folder = match (p::str(v, "folder"), p::str(v, "role")) {
                (Some(f), _) => f.to_string(),
                (None, Some(r)) => {
                    let role = FolderRole::from_code(r).ok_or_else(|| CmdError::Params(format!("unknown folder role `{r}`")))?;
                    s.account(&account).and_then(|a| a.folder(role)).map(|f| f.path.clone()).ok_or_else(|| CmdError::Params(format!("the account has no {r} folder")))?
                }
                (None, None) => return Err(CmdError::Params("`folder` or `role` is required".into())),
            };
            s.empty_folder(&account, &folder)
        })
        .params(r#"{"account"?: string, "folder"?: string (Deleted Items and Junk are emptied for good; any other folder's mail goes to Deleted Items), "role"?: "trash"|"junk"}"#),
        CommandSpec::new("account.add", "Add account", "File › Accounts", add_account)
            .params(r#"{"email": string, "password"?: string, "oauth"?: "google"|"microsoft", "deferSignIn"?: bool, "local"?: bool, "name"?: string, "username"?: string, "imapHost"?: string, "imapPort"?: number, "imapSecurity"?: "tls"|"starttls"|"none", "smtpHost"?: string, "smtpPort"?: number, "smtpSecurity"?: …, "jmap"?: string (a JMAP session URL, instead of IMAP and SMTP), "pop"?: true (POP3 instead of IMAP: popHost, popPort, popSecurity, "popDelete"?: bool to remove mail from the server once downloaded, else a copy stays: "popKeepDays"?: number (removed that many days after downloading; 14 when not given, 0 for never), "popDeleteWithTrash"?: bool (removed once deleted here for good)), "pst"?: string (an existing Outlook data file whose mail comes in first), "ews"?: string|true (on-premises Exchange: its EWS address or server name, or true to find it with Autodiscover; username DOMAIN\\user or an address), "graph"?: true|string (Exchange Online through Microsoft Graph; signs in with Microsoft)}"#),
        CommandSpec::new("account.importPst", "Open Outlook data file", "File › Open & Export", |s, v| {
            let path = p::req_str(v, "path")?.to_string();
            s.import_pst(&path)
        })
        .params(r#"{"path": string (a .pst or .mailcraft data file; its mail goes into a new account on this computer)}"#),
        CommandSpec::new("account.exportPst", "Export to Outlook data file", "File › Open & Export", |s, v| {
            let path = p::req_str(v, "path")?.to_string();
            let account = match p::str(v, "account") {
                Some(a) => a.to_string(),
                None => s.current_account().map(|a| a.config.id.clone()).ok_or_else(|| CmdError::Params("there is no account to export".into()))?,
            };
            s.export_pst(&account, &path)
        })
        .params(r#"{"path": string (the .pst file to write; one already there is replaced), "account"?: string (default: the current one)}"#),
        CommandSpec::new("account.discover", "Find server settings", "File › Accounts", |s, v| {
            let email = p::req_str(v, "email")?.trim().to_ascii_lowercase();
            if !mailcraft_model::addr::is_valid_email(&email) {
                return Err(CmdError::Params(format!("`{email}` is not an email address")));
            }
            match s.discovered.get(&email) {
                Some(Some((Some(c), source))) => {
                    Ok(json!({"found": true, "source": source, "imap": c.imap, "smtp": c.smtp, "pop": c.pop, "username": c.username}))
                }
                Some(Some((None, _))) => Ok(json!({"found": false})),
                Some(None) => Ok(json!({"pending": true})),
                None => {
                    s.discovered.insert(email.clone(), None);
                    s.push_job(mailcraft_model::Job::Discover { email });
                    Ok(json!({"pending": true}))
                }
            }
        })
        .params(r#"{"email": string} (asks once; call again for the answer)"#),
        CommandSpec::new("account.move", "Move account", "Folder pane › account menu", |s, v| {
            let id = p::req_str(v, "account")?.to_string();
            let by = p::i64(v, "by").ok_or_else(|| CmdError::Params("`by` is required: -1 (up) or 1 (down)".into()))?;
            s.move_account(&id, by.clamp(-1000, 1000))
        })
        .params(r#"{"account": string, "by": number (-1 moves it up, 1 down)}"#),
        CommandSpec::new("account.remove", "Remove account", "File › Accounts", |s, v| {
            let id = p::req_str(v, "account")?.to_string();
            s.remove_account(&id)
        })
        .params(r#"{"account": string}"#),
        CommandSpec::new("account.signIn", "Sign in with the browser", "File › Accounts", |s, v| {
            let id = p::req_str(v, "account")?;
            let config = s.account(id).map(|a| a.config.clone()).ok_or_else(|| CmdError::Params(format!("no account `{id}`")))?;
            if config.oauth.is_none() {
                return Err(CmdError::Params("this account signs in with a password".into()));
            }
            s.push_job(mailcraft_model::Job::SignIn { account: config });
            Ok(json!({"signingIn": id}))
        })
        .params(r#"{"account": string}"#),
        CommandSpec::new("account.setPassword", "Sign in again", "File › Accounts", set_password).params(r#"{"account": string, "password": string}"#),
        CommandSpec::new("account.setSignature", "Signature", "File › Options › Mail", set_signature).params(r#"{"account": string, "signature": string}"#),
        CommandSpec::new("account.list", "Accounts", "File › Accounts", list_accounts),
        CommandSpec::new("account.update", "Account settings", "File › Info › Account settings", update_account).params(
            r#"{"account": string, "name"?: string, "username"?: string, "imapHost"?: string, "imapPort"?: number, "imapSecurity"?: …, "smtpHost"?: …, "jmap"?: string|null, "popHost"?: …, "popDelete"?: bool, "popKeepDays"?: number, "popDeleteWithTrash"?: bool}"#,
        ),
        CommandSpec::new("account.mailbox", "Mailbox settings", "File › Info › Mailbox settings", |s, v| {
            let id = account_param(s, v)?;
            let a = s.account(&id).ok_or_else(|| CmdError::Params(format!("no account `{id}`")))?;
            let quota = a.quota.map(|(used, limit)| json!({"used": used, "limit": limit}));
            Ok(json!({"account": id, "quota": quota, "cached": s.cache_size(&id)}))
        })
        .params(r#"{"account"?: string}"#),
        CommandSpec::new("account.vacation", "Automatic replies", "File › Info › Automatic replies", |s, v| {
            let id = account_param(s, v)?;
            if p::bool(v, "refresh").unwrap_or(true) {
                s.load_vacation(&id)
            } else {
                Ok(s.vacation_json(&id))
            }
        })
        .params(r#"{"account"?: string, "refresh"?: bool} (asks the server; call with refresh=false for the answer)"#),
        CommandSpec::new("account.setVacation", "Turn automatic replies on or off", "File › Info › Automatic replies", |s, v| {
            let id = account_param(s, v)?;
            let day = |k: &str| p::str(v, k).map(str::trim).filter(|d| !d.is_empty()).map(str::to_string);
            let vacation = mailcraft_model::job::Vacation {
                enabled: p::bool(v, "enabled").unwrap_or(true),
                subject: p::str(v, "subject").unwrap_or_default().chars().take(200).collect(),
                text: p::str(v, "text").unwrap_or_default().chars().take(20_000).collect(),
                from: day("from"),
                until: day("until"),
            };
            s.set_vacation(&id, vacation)
        })
        .params(r#"{"account"?: string, "enabled": bool, "subject"?: string, "text": string, "from"?: "YYYY-MM-DD", "until"?: "YYYY-MM-DD"}"#),
    ])
}

/// `account` from the params, else the account of the folder on screen.
fn account_param(s: &Session, v: &Value) -> Result<String, CmdError> {
    match p::str(v, "account") {
        Some(a) if s.account(a).is_some() => Ok(a.to_string()),
        Some(a) => Err(CmdError::Params(format!("no account `{a}`"))),
        None => s.current_account().map(|a| a.config.id.clone()).ok_or_else(|| CmdError::Disabled("add an account first".into())),
    }
}

fn new_mail(s: &mut Session, v: &Value) -> CmdResult {
    let c = s.new_compose(p::str(v, "account"))?;
    Ok(json!({"compose": c.id}))
}

fn delete(s: &mut Session, v: &Value) -> CmdResult {
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

fn move_to(s: &mut Session, v: &Value) -> CmdResult {
    let folder = p::req_str(v, "folder")?;
    if let Some(role) = folder.strip_prefix("role:").and_then(FolderRole::from_code) {
        return s.move_selection(MoveTarget::Role(role));
    }
    s.move_selection(MoveTarget::Folder(folder.to_string()))
}

fn snooze(s: &mut Session, v: &Value) -> CmdResult {
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
fn flag(s: &mut Session, v: &Value) -> CmdResult {
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

fn mark_all_read(s: &mut Session, _: &Value) -> CmdResult {
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

fn sync_folder(s: &mut Session, _: &Value) -> CmdResult {
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

fn open_folder(s: &mut Session, v: &Value) -> CmdResult {
    let account = p::req_str(v, "account")?.to_string();
    let folder = p::req_str(v, "folder")?.to_string();
    if s.folder(&account, &folder).is_none() {
        return Err(CmdError::Params(format!("no folder `{folder}` in account `{account}`")));
    }
    s.open_view(View::Folder { account, folder });
    Ok(json!({"messages": s.list.len()}))
}

fn unified(s: &mut Session, v: &Value) -> CmdResult {
    let role = match p::str(v, "role") {
        Some(r) => FolderRole::from_code(r).ok_or_else(|| CmdError::Params(format!("unknown folder role `{r}`")))?,
        None => FolderRole::Inbox,
    };
    s.open_view(View::Unified { role });
    Ok(json!({"messages": s.list.len()}))
}

fn key_param(s: &Session, v: &Value) -> Result<mailcraft_model::MessageKey, CmdError> {
    let k = parse_key(v).ok_or_else(|| CmdError::Params("`key` is required (account:validity:uid:folder)".into()))?;
    if s.summary(&k).is_none() {
        return Err(CmdError::Params(format!("`{}` is not in the list", k.to_id())));
    }
    Ok(k)
}

fn open_message(s: &mut Session, v: &Value) -> CmdResult {
    let k = key_param(s, v)?;
    s.open_message(&k);
    Ok(json!({"key": k.to_id()}))
}

fn select(s: &mut Session, v: &Value) -> CmdResult {
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

fn select_all(s: &mut Session, _: &Value) -> CmdResult {
    s.selection = s.list.iter().map(|m| m.key.clone()).collect();
    Ok(json!({"selected": s.selection.len()}))
}

fn step(s: &mut Session, dir: i64) -> CmdResult {
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

fn filter(s: &mut Session, v: &Value) -> CmdResult {
    let value = p::req_str(v, "value")?;
    s.filter = ListFilter::EVERY
        .into_iter()
        .find(|f| f.code().eq_ignore_ascii_case(value))
        .ok_or_else(|| CmdError::Params(format!("unknown filter `{value}`")))?;
    s.refresh_list();
    Ok(json!({"messages": s.list.len()}))
}

fn summary_json(m: &mailcraft_model::Summary) -> Value {
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

fn list_messages(s: &mut Session, v: &Value) -> CmdResult {
    let limit = p::u64(v, "limit").map_or(50, |n| usize::try_from(n).unwrap_or(usize::MAX).min(10_000));
    Ok(json!({
        "view": s.view_title(),
        "total": s.list.len(),
        "messages": s.list.iter().take(limit).map(summary_json).collect::<Vec<_>>(),
    }))
}

fn read_message(s: &mut Session, v: &Value) -> CmdResult {
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

fn search(s: &mut Session, v: &Value) -> CmdResult {
    let q = p::req_str(v, "query")?.to_string();
    let scope = match p::str(v, "scope") {
        Some(x) if x.eq_ignore_ascii_case("current") => SearchScope::Current,
        _ => SearchScope::All,
    };
    let n = s.run_search(&q, scope);
    Ok(json!({"results": n}))
}

fn compose_update(s: &mut Session, v: &Value) -> CmdResult {
    let account = p::str(v, "account").map(str::to_string);
    if let Some(a) = &account
        && s.account(a).is_none()
    {
        return Err(CmdError::Params(format!("no account `{a}`")));
    }
    let c = s.compose.as_mut().ok_or_else(|| CmdError::Disabled("no message is being written".into()))?;
    if c.sending {
        return Err(CmdError::Disabled("the message is being sent".into()));
    }
    let cap = |x: &str| x.chars().take(1 << 20).collect::<String>();
    for (k, field) in [("to", &mut c.to), ("cc", &mut c.cc), ("bcc", &mut c.bcc), ("subject", &mut c.subject)] {
        if let Some(x) = p::str(v, k) {
            *field = cap(x);
        }
    }
    if let Some(x) = p::str(v, "body") {
        if p::bool(v, "marks").unwrap_or(false) {
            let (text, styles) = mailcraft_mime::rich::from_markup(&cap(x));
            c.body = text;
            c.styles = styles;
        } else {
            c.set_text(cap(x));
        }
    }
    if !c.bcc.is_empty() {
        c.show_cc = true;
    }
    if let Some(i) = p::str(v, "importance") {
        c.importance = match i {
            "high" => 1,
            "low" => -1,
            "normal" => 0,
            other => return Err(CmdError::Params(format!("importance `{other}` isn't high, normal or low"))),
        };
    }
    if let Some(r) = p::bool(v, "readReceipt") {
        c.read_receipt = r;
    }
    if let Some(x) = p::str(v, "replyTo") {
        c.reply_to = cap(x);
    }
    if let Some(at) = p::i64(v, "sendAfter") {
        c.send_after = (at > 0).then_some(at);
    }
    if let Some(a) = account {
        c.account = a;
    }
    Ok(json!({"compose": c.id}))
}

/// Largest attachment accepted (most servers refuse messages over 25–35 MB).
const ATTACH_MAX: usize = 25 << 20;

/// A file from `path`, or `name` and `data` (bytes), at most 25 MB.
fn file_param(v: &Value) -> Result<(String, Vec<u8>), CmdError> {
    let (name, data) = if let Some(path) = p::str(v, "path") {
        let pb = std::path::PathBuf::from(path);
        let meta = std::fs::metadata(&pb).map_err(|e| CmdError::Failed(format!("{path}: {e}")))?;
        if meta.len() > ATTACH_MAX as u64 {
            return Err(CmdError::Failed(format!("{path} is larger than 25 MB")));
        }
        let data = std::fs::read(&pb).map_err(|e| CmdError::Failed(format!("{path}: {e}")))?;
        (pb.file_name().and_then(|n| n.to_str()).unwrap_or("attachment").to_string(), data)
    } else {
        let name = p::req_str(v, "name")?.to_string();
        let data: Vec<u8> = v
            .get("data")
            .and_then(Value::as_array)
            .ok_or_else(|| CmdError::Params("`path`, or `name` and `data`, are required".into()))?
            .iter()
            .take(ATTACH_MAX + 1)
            .map(|b| b.as_u64().and_then(|x| u8::try_from(x).ok()).ok_or_else(|| CmdError::Params("`data` must be bytes".into())))
            .collect::<Result<_, _>>()?;
        (name, data)
    };
    if data.len() > ATTACH_MAX {
        return Err(CmdError::Failed("attachments are limited to 25 MB".into()));
    }
    Ok((name, data))
}

/// What a message being written carries, in bytes.
fn compose_size(c: &crate::Compose) -> usize {
    c.attachments.iter().map(|a| a.data.len()).sum::<usize>() + c.inline.iter().map(|p| p.data.len()).sum::<usize>()
}

fn attach(s: &mut Session, v: &Value) -> CmdResult {
    let (name, data) = file_param(v)?;
    let c = s.compose.as_mut().ok_or_else(|| CmdError::Disabled("no message is being written".into()))?;
    let total: usize = compose_size(c) + data.len();
    if total > ATTACH_MAX {
        return Err(CmdError::Failed("attachments are limited to 25 MB in total".into()));
    }
    let name = sanitize_filename(&name);
    let mime = mime_from_name(&name).to_string();
    c.attachments.push(OutAttachment { name: name.clone(), mime, data });
    Ok(json!({"attached": name, "count": c.attachments.len()}))
}

/// Place a picture in the text. With `"insert": false` the mark isn't added to the text (the
/// editor puts it at the cursor itself).
fn insert_picture(s: &mut Session, v: &Value) -> CmdResult {
    let (name, data) = file_param(v)?;
    let name = sanitize_filename(&name);
    let mime = mime_from_name(&name).to_string();
    if !mime.starts_with("image/") {
        return Err(CmdError::Params(format!("{name} isn't a picture")));
    }
    let cid = s.new_cid();
    let c = s.compose.as_mut().ok_or_else(|| CmdError::Disabled("no message is being written".into()))?;
    if compose_size(c) + data.len() > ATTACH_MAX {
        return Err(CmdError::Failed("pictures and attachments are limited to 25 MB in total".into()));
    }
    c.inline.push(mailcraft_mime::InlinePicture { cid: cid.clone(), name, mime, data });
    c.plain = false;
    if p::bool(v, "insert").unwrap_or(true) {
        let n = c.body.chars().count();
        let at = p::u64(v, "at").and_then(|a| usize::try_from(a).ok()).unwrap_or(n).min(n);
        insert_picture_at(c, at, &cid);
    }
    Ok(json!({"cid": cid}))
}

/// Put picture `cid` into the text at character `at`.
pub fn insert_picture_at(c: &mut crate::session::Compose, at: usize, cid: &str) {
    use mailcraft_mime::rich::{PICTURE, Style};
    let old = c.body.clone();
    let byte = old.char_indices().nth(at).map_or(old.len(), |(b, _)| b);
    c.body.insert(byte, PICTURE);
    let mut styles = std::mem::take(&mut c.styles);
    styles.edit_at(&old, &c.body, &Style::default(), Some(at + 1));
    styles.restyle(&c.body, at..at + 1, |s| Style { picture: Some(cid.to_string()), ..s.clone() });
    c.styles = styles;
}

fn remove_attachment(s: &mut Session, v: &Value) -> CmdResult {
    let i = p::u64(v, "index").ok_or_else(|| CmdError::Params("`index` is required".into()))?;
    let c = s.compose.as_mut().ok_or_else(|| CmdError::Disabled("no message is being written".into()))?;
    let i = usize::try_from(i).map_err(|_| CmdError::Params("bad index".into()))?;
    if i >= c.attachments.len() {
        return Err(CmdError::Params(format!("no attachment {i}")));
    }
    let a = c.attachments.remove(i);
    Ok(json!({"removed": a.name}))
}

/// Close the message being written: kept in Drafts (unless `save` is false or nothing was
/// written), as closing a message window in Outlook offers.
fn close_compose(s: &mut Session, v: &Value) -> CmdResult {
    let c = s.compose.clone().ok_or_else(|| CmdError::Disabled("no message is being written".into()))?;
    let written = s.compose_written();
    let saved = if p::bool(v, "save").unwrap_or(true) && written && !c.sending {
        s.save_draft()?;
        true
    } else {
        false
    };
    s.compose = None;
    if saved {
        s.notice("Saved in Drafts", false);
    }
    Ok(json!({"closed": c.id, "saved": saved}))
}

fn discard(s: &mut Session, _: &Value) -> CmdResult {
    let c = s.compose.take();
    Ok(json!({"discarded": c.map(|c| c.id)}))
}

impl Session {
    /// The open message's attachment `index`, as bytes.
    pub fn attachment_bytes(&self, index: usize) -> Result<Vec<u8>, CmdError> {
        let r = self.reading.as_ref().ok_or_else(|| CmdError::Disabled("open a message first".into()))?;
        let raw = self.store.as_ref().and_then(|st| st.read_body(&r.key)).ok_or_else(|| CmdError::Failed("the message isn't downloaded".into()))?;
        mailcraft_mime::attachment_data(&raw, index).ok_or_else(|| CmdError::Params(format!("no attachment {index}")))
    }

    /// The open message's source (`.eml`).
    pub fn message_source(&self) -> Result<Vec<u8>, CmdError> {
        let r = self.reading.as_ref().ok_or_else(|| CmdError::Disabled("open a message first".into()))?;
        if r.key.folder == crate::session::OUTBOX {
            self.outbox_entry(&r.key).and_then(|(a, i)| self.store.as_ref().and_then(|st| st.outbox_raw(&a, i.id)))
        } else {
            self.store.as_ref().and_then(|st| st.read_body(&r.key))
        }
        .ok_or_else(|| CmdError::Failed("the message isn't downloaded".into()))
    }

    /// The open message as a PDF: subject, header, text and the names of its attachments.
    pub fn message_pdf(&self) -> Result<Vec<u8>, CmdError> {
        let r = self.reading.as_ref().ok_or_else(|| CmdError::Disabled("open a message first".into()))?;
        let m = r.parsed.as_ref().ok_or_else(|| CmdError::Disabled("the message is still downloading".into()))?;
        let mut header = vec![("From".to_string(), m.from.to_string())];
        if !m.to.is_empty() {
            header.push(("To".into(), mailcraft_model::addr::format_list(&m.to)));
        }
        if !m.cc.is_empty() {
            header.push(("Cc".into(), mailcraft_model::addr::format_list(&m.cc)));
        }
        if m.date > 0 {
            header.push(("Date".into(), mailcraft_model::dates::full_time(m.date, &self.clock)));
        }
        let files: Vec<String> = m.attachments.iter().filter(|a| !a.inline).map(|a| a.name.clone()).collect();
        if !files.is_empty() {
            header.push(("Attachments".into(), files.join(", ")));
        }
        let doc = mailcraft_print::Doc { title: m.subject.clone(), header, body: m.text.clone() };
        let fonts = self.print_fonts.as_ref().map(|(r, b)| mailcraft_print::Fonts { regular: r, bold: b.as_deref() });
        mailcraft_print::pdf(&doc, fonts).map_err(CmdError::Failed)
    }
}

fn save_attachment(s: &mut Session, v: &Value) -> CmdResult {
    let index = p::u64(v, "index").and_then(|i| usize::try_from(i).ok()).ok_or_else(|| CmdError::Params("`index` is required".into()))?;
    let path = p::req_str(v, "path")?;
    let data = s.attachment_bytes(index)?;
    std::fs::write(path, &data).map_err(|e| CmdError::Failed(format!("{path}: {e}")))?;
    Ok(json!({"saved": path, "bytes": data.len()}))
}

/// Save every attachment (not the pictures inside the text) into a folder; names that are
/// taken get a number.
fn save_all_attachments(s: &mut Session, v: &Value) -> CmdResult {
    let dir = std::path::PathBuf::from(p::req_str(v, "dir")?);
    if !dir.is_dir() {
        return Err(CmdError::Params(format!("{} is not a folder", dir.display())));
    }
    let r = s.reading.as_ref().ok_or_else(|| CmdError::Disabled("open a message first".into()))?;
    let parsed = r.parsed.as_ref().ok_or_else(|| CmdError::Disabled("the message is still downloading".into()))?;
    let raw = s.store.as_ref().and_then(|st| st.read_body(&r.key)).ok_or_else(|| CmdError::Failed("the message isn't downloaded".into()))?;
    let mut saved = Vec::new();
    for a in parsed.attachments.iter().filter(|a| !a.inline) {
        let Some(data) = mailcraft_mime::attachment_data(&raw, a.index) else { continue };
        let name = sanitize_filename(&a.name);
        let (stem, ext) = name.rsplit_once('.').map_or((name.clone(), String::new()), |(s, e)| (s.to_string(), format!(".{e}")));
        let mut path = dir.join(&name);
        let mut n = 2;
        while path.exists() && n < 1000 {
            path = dir.join(format!("{stem} ({n}){ext}"));
            n += 1;
        }
        std::fs::write(&path, &data).map_err(|e| CmdError::Failed(format!("{}: {e}", path.display())))?;
        saved.push(path.to_string_lossy().to_string());
    }
    Ok(json!({"saved": saved}))
}

/// An attachment as text, for previewing (`message` kind: an attached email, parsed).
fn preview_attachment(s: &mut Session, v: &Value) -> CmdResult {
    let index = p::u64(v, "index").and_then(|i| usize::try_from(i).ok()).ok_or_else(|| CmdError::Params("`index` is required".into()))?;
    let r = s.reading.as_ref().ok_or_else(|| CmdError::Disabled("open a message first".into()))?;
    let parsed = r.parsed.as_ref().ok_or_else(|| CmdError::Disabled("the message is still downloading".into()))?;
    let a = parsed.attachments.iter().find(|a| a.index == index).ok_or_else(|| CmdError::Params(format!("no attachment {index}")))?;
    let kind = mailcraft_mime::text_preview_kind(&a.mime, &a.name).ok_or_else(|| CmdError::Failed(format!("{} can't be shown as text", a.name)))?;
    let raw = s.store.as_ref().and_then(|st| st.read_body(&r.key)).ok_or_else(|| CmdError::Failed("the message isn't downloaded".into()))?;
    let data = mailcraft_mime::attachment_data(&raw, index).ok_or_else(|| CmdError::Failed("the attachment can't be read".into()))?;
    if kind == "message" {
        let m = mailcraft_mime::parse(&data).ok_or_else(|| CmdError::Failed("the attached message can't be read".into()))?;
        return Ok(
            json!({"kind": "message", "name": a.name, "subject": m.subject, "from": m.from.to_string(), "to": mailcraft_model::addr::format_list(&m.to), "date": m.date, "text": m.text}),
        );
    }
    // At most 512 KB, cut on a character boundary.
    const MAX: usize = 512 << 10;
    let text = String::from_utf8_lossy(data.get(..MAX).unwrap_or(&data)).to_string();
    Ok(json!({"kind": "text", "name": a.name, "text": text, "truncated": data.len() > MAX}))
}

/// The open message's source, as an `.eml` file.
fn save_message(s: &mut Session, v: &Value) -> CmdResult {
    let path = p::req_str(v, "path")?;
    let raw = s.message_source()?;
    std::fs::write(path, &raw).map_err(|e| CmdError::Failed(format!("{path}: {e}")))?;
    Ok(json!({"saved": path, "bytes": raw.len()}))
}

/// The open message as a PDF.
fn export_pdf(s: &mut Session, v: &Value) -> CmdResult {
    let path = p::req_str(v, "path")?;
    let bytes = s.message_pdf()?;
    std::fs::write(path, &bytes).map_err(|e| CmdError::Failed(format!("{path}: {e}")))?;
    Ok(json!({"saved": path, "bytes": bytes.len()}))
}

fn reading_pane(s: &mut Session, v: &Value) -> CmdResult {
    s.prefs.reading_pane = match p::req_str(v, "value")?.to_ascii_lowercase().as_str() {
        "right" => ReadingPane::Right,
        "bottom" => ReadingPane::Bottom,
        "off" => ReadingPane::Off,
        other => return Err(CmdError::Params(format!("unknown reading pane position `{other}`"))),
    };
    Ok(json!({"value": p::req_str(v, "value")?}))
}

fn server(v: &Value, prefix: &str, guess: &ServerConfig) -> Result<ServerConfig, CmdError> {
    let host = p::str(v, &format!("{prefix}Host")).map(|h| h.trim().to_string()).unwrap_or_else(|| guess.host.clone());
    let port = match p::u64(v, &format!("{prefix}Port")) {
        Some(n) => u16::try_from(n).map_err(|_| CmdError::Params(format!("{prefix}Port: {n} is not a port")))?,
        None => guess.port,
    };
    let security = match p::str(v, &format!("{prefix}Security")) {
        Some(x) => Security::from_code(x).ok_or_else(|| CmdError::Params(format!("{prefix}Security: unknown `{x}`")))?,
        None => guess.security,
    };
    if host.contains(char::is_whitespace) || host.contains('/') {
        return Err(CmdError::Params(format!("{prefix}Host: `{host}` is not a server name")));
    }
    Ok(ServerConfig { host, port, security })
}

fn add_account(s: &mut Session, v: &Value) -> CmdResult {
    let email = p::req_str(v, "email")?;
    let guess = AccountConfig::guess(email);
    let local = p::bool(v, "local").unwrap_or(false);
    let jmap = p::str(v, "jmap").map(str::trim).filter(|u| !u.is_empty()).map(str::to_string);
    // `graph`: true for Exchange Online through Microsoft Graph (or the API's address).
    let graph = match v.get("graph") {
        Some(Value::Bool(true)) => Some(mailcraft_model::GRAPH_URL.to_string()),
        Some(Value::String(u)) if !u.trim().is_empty() => Some(u.trim().to_string()),
        _ => None,
    };
    // `ews`: on-premises Exchange through Exchange Web Services, at this address (a server name
    // alone means its usual `/EWS/Exchange.asmx`).
    // `true`: found with Autodiscover when the account connects.
    let ews = match v.get("ews") {
        Some(Value::Bool(true)) => Some(mailcraft_model::EWS_AUTODISCOVER.to_string()),
        Some(Value::String(u)) if !u.trim().is_empty() => Some(ews_url(u)),
        _ => None,
    };
    if s.jmap_only && jmap.is_none() {
        return Err(CmdError::Params("in the browser, MailCraft works with JMAP accounts only".into()));
    }
    // `pop`: true to download mail over POP3 (popHost, popPort, popSecurity) and send over SMTP.
    let pop = if p::bool(v, "pop").unwrap_or(false) {
        let host = match guess.imap.host.strip_prefix("imap.") {
            Some(rest) => format!("pop.{rest}"),
            None => guess.imap.host.clone(),
        };
        Some(server(v, "pop", &ServerConfig { host, port: 995, security: Security::Tls })?)
    } else {
        None
    };
    let plain = local || jmap.is_some() || graph.is_some() || ews.is_some();
    let password = p::str(v, "password").map(str::to_string);
    // Graph signs in with Microsoft, unless given a token to use as it is.
    let oauth = p::str(v, "oauth")
        .map(str::to_string)
        .or_else(|| (graph.is_some() && password.as_deref().is_none_or(str::is_empty)).then(|| "microsoft".to_string()));
    let config = AccountConfig {
        id: String::new(),
        name: p::str(v, "name").map(str::to_string).unwrap_or(guess.name.clone()),
        email: email.to_string(),
        username: p::str(v, "username").map(str::to_string).unwrap_or(guess.username.clone()),
        imap: if plain || pop.is_some() { ServerConfig::default() } else { server(v, "imap", &guess.imap)? },
        smtp: if plain { ServerConfig::default() } else { server(v, "smtp", &guess.smtp)? },
        signature: p::str(v, "signature").unwrap_or_default().to_string(),
        color: 0,
        oauth,
        jmap,
        graph,
        ews,
        pop,
        pop_delete: p::bool(v, "popDelete").unwrap_or(false),
        // A copy stays on the server for 14 days unless asked otherwise (0: for good).
        pop_keep_days: match v.get("popKeepDays").and_then(serde_json::Value::as_u64) {
            None => Some(14),
            Some(0) => None,
            Some(n) => Some(u32::try_from(n).unwrap_or(u32::MAX)),
        },
        pop_delete_with_trash: p::bool(v, "popDeleteWithTrash").unwrap_or(false),
    };
    if let Some(o) = &config.oauth
        && !s.oauth_available.contains(o)
    {
        return Err(CmdError::Params(format!("signing in with {o} isn't set up in this copy of MailCraft")));
    }
    // `deferSignIn`: add now, ask for the password when the account first connects.
    let defer = p::bool(v, "deferSignIn").unwrap_or(false);
    if !local && !defer && config.oauth.is_none() && password.as_deref().is_none_or(str::is_empty) {
        return Err(CmdError::Params("`password` is required".into()));
    }
    let id = s.add_account(config, password)?;
    // `pst`: an existing data file whose mail comes into the account first.
    if let Some(path) = p::str(v, "pst").map(str::trim).filter(|p| !p.is_empty()) {
        s.attach_data_file(&id, path)?;
    }
    Ok(json!({"account": id}))
}

/// An Exchange Web Services address from what the user typed: a server name gets `https://` and
/// `/EWS/Exchange.asmx`.
fn ews_url(typed: &str) -> String {
    let t = typed.trim().trim_end_matches('/');
    let with_scheme = if t.contains("://") { t.to_string() } else { format!("https://{t}") };
    let after_host = with_scheme.split_once("://").map_or("", |(_, rest)| rest);
    if after_host.contains('/') { with_scheme } else { format!("{with_scheme}/EWS/Exchange.asmx") }
}

fn set_password(s: &mut Session, v: &Value) -> CmdResult {
    let id = p::req_str(v, "account")?;
    let pw = p::req_str(v, "password")?;
    if pw.is_empty() {
        return Err(CmdError::Params("`password` is empty".into()));
    }
    let config = s.account(id).map(|a| a.config.clone()).ok_or_else(|| CmdError::Params(format!("no account `{id}`")))?;
    s.update_account(config, Some(pw.to_string()))
}

fn update_account(s: &mut Session, v: &Value) -> CmdResult {
    let id = p::req_str(v, "account")?;
    let mut c = s.account(id).map(|a| a.config.clone()).ok_or_else(|| CmdError::Params(format!("no account `{id}`")))?;
    if c.is_local() {
        return Err(CmdError::Params("the sample mailbox has no server settings".into()));
    }
    if let Some(n) = p::str(v, "name") {
        c.name = n.trim().chars().take(200).collect();
    }
    if let Some(u) = p::str(v, "username").map(str::trim).filter(|u| !u.is_empty()) {
        c.username = u.to_string();
    }
    match v.get("jmap") {
        Some(Value::String(u)) if !u.trim().is_empty() => c.jmap = Some(u.trim().to_string()),
        Some(Value::String(_) | Value::Null) => c.jmap = None,
        _ => {}
    }
    if let Some(u) = p::str(v, "ews").map(str::trim).filter(|u| !u.is_empty())
        && c.ews.is_some()
    {
        c.ews = Some(ews_url(u));
    }
    if let Some(pop) = c.pop.clone() {
        c.pop = Some(server(v, "pop", &pop)?);
        c.smtp = server(v, "smtp", &c.smtp)?;
        if let Some(d) = p::bool(v, "popDelete") {
            c.pop_delete = d;
        }
        if let Some(n) = v.get("popKeepDays").and_then(serde_json::Value::as_u64) {
            c.pop_keep_days = (n > 0).then(|| u32::try_from(n).unwrap_or(u32::MAX));
        }
        if let Some(d) = p::bool(v, "popDeleteWithTrash") {
            c.pop_delete_with_trash = d;
        }
    } else if !c.is_api() {
        c.imap = server(v, "imap", &c.imap)?;
        c.smtp = server(v, "smtp", &c.smtp)?;
        if c.imap.host.is_empty() || c.imap.port == 0 || c.smtp.host.is_empty() || c.smtp.port == 0 {
            return Err(CmdError::Params("incoming and outgoing server names and ports are required".into()));
        }
    }
    s.update_account(c, None)
}

fn set_signature(s: &mut Session, v: &Value) -> CmdResult {
    let id = p::req_str(v, "account")?;
    let mut config = s.account(id).map(|a| a.config.clone()).ok_or_else(|| CmdError::Params(format!("no account `{id}`")))?;
    config.signature = p::req_str(v, "signature")?.chars().take(10_000).collect();
    s.update_account(config, None)
}

fn list_accounts(s: &mut Session, _: &Value) -> CmdResult {
    Ok(json!(s
        .accounts
        .iter()
        .map(|a| json!({
            "account": a.config.id,
            "email": a.config.email,
            "name": a.config.name,
            "graph": a.config.graph,
            "pop": a.config.pop.as_ref().map(|p| format!("{}:{} ({})", p.host, p.port, p.security.code())),
            "ews": a.config.ews,
            "imap": format!("{}:{} ({})", a.config.imap.host, a.config.imap.port, a.config.imap.security.code()),
            "smtp": format!("{}:{} ({})", a.config.smtp.host, a.config.smtp.port, a.config.smtp.security.code()),
            "status": a.status,
            "folders": a.folders.iter().map(|f| json!({"path": f.path, "name": f.display_name(), "role": f.role.code(), "unread": f.unread, "total": f.total})).collect::<Vec<_>>(),
        }))
        .collect::<Vec<_>>()))
}

/// An event from command params, starting from `e` (what's not given stays).
fn event_from(s: &Session, v: &Value, mut e: mailcraft_model::ical::CalEvent) -> Result<mailcraft_model::ical::CalEvent, CmdError> {
    let off = i64::from(s.clock.offset);
    let when = |k: &str| -> Result<Option<(i64, bool)>, CmdError> {
        match p::str(v, k) {
            Some(x) => parse_local(x, off).map(Some).ok_or_else(|| CmdError::Params(format!("`{x}` is not a date (YYYY-MM-DD or YYYY-MM-DDTHH:MM)"))),
            None => Ok(v.get(k).and_then(Value::as_i64).map(|n| (n, false))),
        }
    };
    let length = e.end - e.start;
    if let Some((t, all_day)) = when("start")? {
        e.start = t;
        e.all_day = p::bool(v, "allDay").unwrap_or(all_day);
        e.end = t + length.max(if e.all_day { 86_400 } else { 3600 });
    } else if let Some(a) = p::bool(v, "allDay") {
        e.all_day = a;
    }
    if let Some((t, all_day)) = when("end")? {
        // An all-day end date is the last day shown: the event ends the day after.
        e.end = if all_day && e.all_day { t + 86_400 } else { t };
    }
    if e.all_day {
        e.start -= e.start.rem_euclid(86_400);
        e.end = (e.end - e.end.rem_euclid(86_400)).max(e.start + 86_400);
    }
    for (k, field) in [("summary", &mut e.summary), ("location", &mut e.location), ("description", &mut e.description)] {
        if let Some(x) = p::str(v, k) {
            *field = x.chars().take(10_000).collect();
        }
    }
    if let Some(list) = p::str(v, "attendees") {
        let people = mailcraft_model::addr::parse_list(list).map_err(|bad| CmdError::Params(format!("not an email address: {bad}")))?;
        let before = std::mem::take(&mut e.attendees);
        e.attendees = people
            .into_iter()
            .take(200)
            .map(|a| {
                let known = before.iter().find(|b| b.email.eq_ignore_ascii_case(&a.email));
                mailcraft_model::ical::Attendee {
                    partstat: known.map_or_else(|| "NEEDS-ACTION".to_string(), |k| k.partstat.clone()),
                    rsvp: known.is_none_or(|k| k.rsvp),
                    email: a.email,
                    name: a.name,
                }
            })
            .collect();
    }
    match p::str(v, "repeat").map(str::trim) {
        Some("" | "none" | "never") => e.rrule = None,
        Some(r @ ("daily" | "weekly" | "monthly" | "yearly")) => e.rrule = Some(format!("FREQ={}", r.to_ascii_uppercase())),
        Some(r) if r.to_ascii_uppercase().starts_with("FREQ=") && r.len() < 500 => e.rrule = Some(r.to_string()),
        Some(r) => return Err(CmdError::Params(format!("`{r}` is not a repeat"))),
        None => {}
    }
    Ok(e)
}

/// A card from command params, starting from `c`.
fn card_from(v: &Value, mut c: mailcraft_model::vcard::Card) -> Result<mailcraft_model::vcard::Card, CmdError> {
    let list = |k: &str| -> Option<Vec<String>> {
        match v.get(k)? {
            Value::String(s) => Some(s.split([',', ';', '\n']).map(str::trim).filter(|x| !x.is_empty()).map(str::to_string).collect()),
            Value::Array(a) => Some(a.iter().filter_map(Value::as_str).map(str::trim).filter(|x| !x.is_empty()).map(str::to_string).collect()),
            _ => None,
        }
    };
    if let Some(n) = p::str(v, "name") {
        c.name = n.trim().chars().take(300).collect();
        c.given.clear();
        c.family.clear();
    }
    if let Some(e) = list("emails") {
        c.emails = e.into_iter().take(20).map(|value| mailcraft_model::vcard::Typed { value, kind: String::new() }).collect();
    }
    if let Some(ph) = list("phones") {
        c.phones = ph.into_iter().take(20).map(|value| mailcraft_model::vcard::Typed { value, kind: String::new() }).collect();
    }
    for (k, field) in [("org", &mut c.org), ("title", &mut c.title), ("note", &mut c.note)] {
        if let Some(x) = p::str(v, k) {
            *field = x.chars().take(10_000).collect();
        }
    }
    Ok(c)
}

/// The user organizes this event and it has guests to tell.
fn organized_by_me(s: &Session, e: &mailcraft_model::ical::CalEvent) -> bool {
    !e.attendees.is_empty()
        && e.organizer.as_ref().is_some_and(|o| {
            s.accounts.iter().any(|a| a.config.email.eq_ignore_ascii_case(&o.email) || a.config.username.eq_ignore_ascii_case(&o.email))
        })
}
