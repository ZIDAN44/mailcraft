//! Commands: the View tab and the options.

use super::*;

pub(super) fn commands() -> Vec<CommandSpec> {
    vec![
        // View
        CommandSpec::new("view.readingPane", "Reading pane", "View › Layout", reading_pane).params(r#"{"value": "right"|"bottom"|"off"}"#),
        CommandSpec::new("view.conversations", "Show as conversations", "View › Messages", |s, v| {
            s.prefs.conversations = p::bool(v, "on").unwrap_or(!s.prefs.conversations);
            s.refresh_list();
            s.rethread_reading();
            Ok(json!({"on": s.prefs.conversations}))
        })
        .params(r#"{"on"?: bool}"#),
        toggle!("view.preview", "Message preview", "View › Messages", show_preview),
        toggle!("view.compact", "Compact list", "View › Layout", compact),
        CommandSpec::new("view.folderDensity", "Folder density", "View › Layout", |s, v| {
            let code = p::req_str(v, "value")?;
            s.prefs.folder_density = crate::FolderDensity::from_code(code)
                .ok_or_else(|| CmdError::Params(format!("`value` is \"spacious\", \"medium\" or \"compact\", not `{code}`")))?;
            Ok(json!({"value": code}))
        })
        .params(r#"{"value": "spacious"|"medium"|"compact"} (the folder pane's text size, row height and indent)"#),
        CommandSpec::new("view.groups", "Show in groups", "View › Arrangement", |s, v| {
            s.prefs.groups = p::bool(v, "on").unwrap_or(!s.prefs.groups);
            // Headings folded away (issue #189) fold only while they're shown.
            s.rebuild_rows();
            Ok(json!({"on": s.prefs.groups}))
        })
        .params(r#"{"on"?: bool}"#),
        toggle!("view.singleLine", "Single-line list", "View › Layout", single_line),
        toggle!("view.classicRibbon", "Classic ribbon", "View › Layout", classic_ribbon),
        toggle!("view.hideRibbon", "Hide the ribbon", "View › Layout", ribbon_hidden).key("Mod+F1"),
        CommandSpec::new("view.accent", "Accent colour", "File › Options › Personalize", |s, v| {
            let given = p::req_str(v, "color")?;
            s.prefs.accent = crate::session::accent_code(given)
                .ok_or_else(|| CmdError::Params(format!("`{given}` isn't one of the accent colours or a colour as #rrggbb")))?;
            Ok(json!({"color": s.prefs.accent}))
        })
        .params(r##"{"color": "teal"|"blue"|"indigo"|"violet"|"rose"|"red"|"orange"|"green"|"#rrggbb" ("" or "teal": MailCraft's own)}"##),
        toggle!("view.plainTopBar", "Plain top bar", "File › Options › Personalize", plain_top_bar),
        toggle!("view.lightMessages", "Messages on a light background", "File › Options › General", light_messages),
        CommandSpec::new("view.dateFormat", "Dates and times", "File › Options › General", |s, v| {
            let mut style = s.prefs.date_style;
            if let Some(code) = p::str(v, "order") {
                style.order = mailcraft_model::dates::DateOrder::from_code(code)
                    .ok_or_else(|| CmdError::Params(format!("`{code}` isn't one of iso, dmy, mdy and dmy. (with dots)")))?;
            }
            style.hours12 = p::bool(v, "hours12").unwrap_or(style.hours12);
            style.full_in_list = p::bool(v, "fullInList").unwrap_or(style.full_in_list);
            s.prefs.date_style = style;
            s.clock = s.clock.styled(style);
            Ok(json!({"order": style.order.code(), "hours12": style.hours12, "fullInList": style.full_in_list}))
        })
        .params(r#"{"order"?: "iso"|"dmy"|"mdy"|"dmy." (2026-10-28, 28/10/2026, 10/28/2026, 28.10.2026), "hours12"?: bool, "fullInList"?: bool}"#),
        CommandSpec::new("view.systemTitleBar", "Use the system title bar", "File › Options › General", |s, v| {
            s.prefs.system_title_bar = p::bool(v, "on").unwrap_or(!s.prefs.system_title_bar);
            Ok(json!({"on": s.prefs.system_title_bar, "atNextStart": true}))
        })
        .params(r#"{"on"?: bool} (Windows; when MailCraft starts again)"#),
        CommandSpec::new("view.arrange", "Arrange by", "View › Arrangement", |s, v| {
            let by = p::req_str(v, "by")?;
            s.prefs.arrange =
                crate::session::Arrange::from_code(by).ok_or_else(|| CmdError::Params(format!("`{by}` isn't date, from, to, subject, size, flag or attachments")))?;
            // A new arrangement starts in its natural order.
            s.prefs.reverse_sort = p::bool(v, "reverse").unwrap_or(false);
            s.refresh_list();
            Ok(json!({"by": s.prefs.arrange.code(), "reverse": s.prefs.reverse_sort}))
        }).leaves_body()
        .params(r#"{"by": "date"|"from"|"to"|"subject"|"size"|"flag"|"attachments", "reverse"?: bool}"#),
        CommandSpec::new("view.reverseSort", "Reverse sort", "View › Arrangement", |s, v| {
            s.prefs.reverse_sort = p::bool(v, "on").unwrap_or(!s.prefs.reverse_sort);
            s.refresh_list();
            Ok(json!({"on": s.prefs.reverse_sort}))
        }).leaves_body()
        .params(r#"{"on"?: bool}"#),
        CommandSpec::new("view.folderPane", "Minimize the folder pane", "View › Layout", |s, v| {
            s.prefs.folder_pane_minimized = p::bool(v, "minimized").unwrap_or(!s.prefs.folder_pane_minimized);
            Ok(json!({"minimized": s.prefs.folder_pane_minimized}))
        }).leaves_body()
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
        }).leaves_body()
        .params(r#"{"value": "focused"|"other"}"#),
        toggle!("view.confirmDelete", "Ask before deleting for good", "File › Options › Mail", confirm_permanent_delete),
        toggle!("view.notifications", "Desktop notifications", "File › Options › Mail", notify),
        toggle!("view.notifyOther", "Notify of mail in Other too", "File › Options › Mail", notify_other),
        toggle!("options.spelling", "Check spelling as you type", "File › Options › Mail", spelling),
        CommandSpec::new("options.composeFont", "Font for new messages", "File › Options › Mail", |s, v| {
            s.prefs.compose_font = match v.get("font") {
                Some(serde_json::Value::String(name)) => {
                    Some(mailcraft_mime::rich::font_name(name).ok_or_else(|| CmdError::Params(format!("`{name}` isn't a font's name")))?)
                }
                Some(serde_json::Value::Null) | None => None,
                Some(_) => return Err(CmdError::Params("`font` is a font's name or null".into())),
            };
            Ok(json!({ "font": s.prefs.compose_font }))
        })
        .params(r#"{"font"?: string (an installed typeface: new messages, replies and forwards are written in it) | null (MailCraft's own)}"#),
        CommandSpec::new("spelling.addWord", "Add to dictionary", "Message editor › Spelling", |s, v| {
            let word = p::req_str(v, "word")?;
            let w = crate::session::dictionary_word(word).ok_or_else(|| CmdError::Params(format!("`{word}` isn't a word")))?;
            if p::bool(v, "remove") == Some(true) {
                return Ok(json!({ "removed": s.prefs.dictionary.remove(&w) }));
            }
            if s.prefs.dictionary.len() >= crate::session::DICTIONARY_MAX && !s.prefs.dictionary.contains(&w) {
                return Err(CmdError::Failed("the dictionary is full".into()));
            }
            Ok(json!({ "added": s.prefs.dictionary.insert(w) }))
        })
        .params(r#"{"word": string (taken as spelled right in every language), "remove"?: true (taken out of the dictionary)}"#),
        toggle!("view.closeToTray", "Keep MailCraft running when its window is closed", "File › Options › General", close_to_tray),
        toggle!("view.startInTray", "Start MailCraft with its window put away", "File › Options › General", start_in_tray),
        toggle!("view.trayUnread", "The unread count on the notification area's icon", "File › Options › General", tray_unread),
        toggle!("view.newMailWindow", "New messages in their own window", "File › Options › Mail", new_mail_window),
        toggle!("view.replyWindow", "Replies and forwards in their own window", "File › Options › Mail", reply_window),
        CommandSpec::new("options.ctrlEnter", "What Ctrl+Enter does in a message", "File › Options › Mail", |s, v| {
            if let Some(does) = p::str(v, "does") {
                s.prefs.ctrl_enter = match does {
                    "ask" => crate::CtrlEnter::Ask,
                    "send" => crate::CtrlEnter::Send,
                    "nothing" => crate::CtrlEnter::Nothing,
                    other => return Err(CmdError::Params(format!("`{other}`: Ctrl+Enter asks, sends or does nothing"))),
                };
            }
            Ok(json!({"does": s.prefs.ctrl_enter}))
        })
        .params(r#"{"does"?: "ask"|"send"|"nothing"}"#),
        toggle!("calendar.tentativeInvites", "Invitations into the calendar as tentative", "File › Options › Calendar", tentative_invites),
        CommandSpec::new("options.answeredInvites", "After answering a meeting invitation", "File › Options › Mail", |s, v| {
            if let Some(after) = p::str(v, "after") {
                s.prefs.answered_invites = match after {
                    "keep" => crate::AnsweredInvites::Keep,
                    "delete" => crate::AnsweredInvites::Delete,
                    "move" => crate::AnsweredInvites::Move,
                    other => return Err(CmdError::Params(format!("`{other}`: after answering, keep, delete or move"))),
                };
            }
            if let Some(folder) = p::str(v, "folder") {
                let account = p::req_str(v, "account")?.to_string();
                let a = s.account(&account).ok_or_else(|| CmdError::Params(format!("no account `{account}`")))?;
                if folder.is_empty() {
                    s.prefs.answered_folders.remove(&account);
                } else {
                    let f = a.folders.iter().find(|f| f.path == folder).ok_or_else(|| CmdError::Params(format!("no folder `{folder}`")))?;
                    if !f.selectable || matches!(f.role, FolderRole::Inbox | FolderRole::Sent | FolderRole::Drafts) {
                        return Err(CmdError::Params(format!("`{folder}` can't take answered invitations")));
                    }
                    s.prefs.answered_folders.insert(account, folder.to_string());
                }
            }
            if let Some(on) = p::bool(v, "replies") {
                s.prefs.answered_replies = on;
            }
            Ok(json!({"after": s.prefs.answered_invites, "folders": s.prefs.answered_folders, "replies": s.prefs.answered_replies}))
        })
        .params(r#"{"after"?: "keep"|"delete"|"move", "account"?: string, "folder"?: string (with "move": where the account's go; "" for none), "replies"?: bool (guests' answers too)}"#),
        CommandSpec::new("view.todoBar", "To-Do Bar", "View › Layout", |s, v| {
            if p::bool(v, "off") == Some(true) {
                s.todo_bar_part(None, Some(false))?;
            } else if v.get("part").is_some() || v.get("on").is_some() || (v.get("width").is_none() && v.get("split").is_none()) {
                s.todo_bar_part(p::str(v, "part"), p::bool(v, "on"))?;
            }
            if let Some(w) = p::u64(v, "width") {
                let (least, most) = (*crate::TODO_BAR_WIDTH.start(), *crate::TODO_BAR_WIDTH.end());
                s.prefs.todo_bar.width = u16::try_from(w).unwrap_or(most).clamp(least, most);
            }
            // Where the line between the appointments and the tasks is (issue #196).
            if let Some(split) = p::u64(v, "split") {
                let (least, most) = (*crate::TODO_BAR_SPLIT.start(), *crate::TODO_BAR_SPLIT.end());
                s.prefs.todo_bar.split = u8::try_from(split).unwrap_or(most).clamp(least, most);
            }
            let bar = s.prefs.todo_bar;
            Ok(json!({"calendar": bar.calendar, "appointments": bar.appointments, "tasks": bar.tasks, "width": bar.width, "split": bar.split}))
        })
        .params(
            r#"{"part"?: "calendar"|"appointments"|"tasks" (that part; none: all of them), "on"?: bool (shown or hidden; not given: the other way), "off"?: true (all hidden), "width"?: number, "split"?: 15…85 (the appointments' share of the height, in percent, when the tasks are under them)}"#,
        ),
        CommandSpec::new("todo.complete", "Mark complete", "To-Do Bar", |s, v| {
            let key = p::req_str(v, "key")?;
            let key = mailcraft_model::MessageKey::from_id(key).ok_or_else(|| CmdError::Params(format!("`{key}` isn't a message")))?;
            s.todo_complete(&key)
        })
        .params(r#"{"key": string (a flagged message of the To-Do Bar's task list)}"#),
        CommandSpec::new("todo.flag", "Follow up", "To-Do Bar", super::home::todo_flag).params(
            r#"{"key": string (a flagged message of the To-Do Bar's task list, in any folder), "due"?: "today"|"tomorrow"|"thisWeek"|"nextWeek"|"none"|"YYYY-MM-DD" (Follow Up by that day), "reminder"?: "YYYY-MM-DDTHH:MM"|"none" (remind then), "complete"?: true (Mark Complete), "on"?: bool (none of them given: Clear Flag)}"#,
        ),
        CommandSpec::new("todo.open", "Open", "To-Do Bar", |s, v| {
            let key = p::req_str(v, "key")?;
            let key = mailcraft_model::MessageKey::from_id(key).ok_or_else(|| CmdError::Params(format!("`{key}` isn't a message")))?;
            if s.todo_message(&key).is_none() {
                return Err(CmdError::Params(format!("`{}` isn't flagged", key.to_id())));
            }
            s.pop_out(&key)
        })
        .params(r#"{"key": string (a flagged message of the To-Do Bar's task list: opened in a window of its own)}"#),
        toggle!("view.favorites", "Show Favorites", "View › Layout", show_favorites),
        toggle!("view.markReadOnOpen", "Mark as read when opened", "File › Options › Mail", mark_read_on_open),
        CommandSpec::new("calendar.show", "Show or hide a calendar", "Calendar › Calendars", |s, v| {
            let account = p::req_str(v, "account")?.to_string();
            let calendar = p::req_str(v, "calendar")?.to_string();
            if !s.pim.get(&account).is_some_and(|p| p.calendars.iter().any(|c| c.id == calendar)) {
                return Err(CmdError::Params(format!("no calendar `{calendar}` in `{account}`")));
            }
            let key = Session::calendar_key(&account, &calendar);
            let shown = p::bool(v, "shown").unwrap_or(s.prefs.hidden_calendars.contains(&key));
            if shown {
                s.prefs.hidden_calendars.remove(&key);
            } else {
                s.prefs.hidden_calendars.insert(key);
            }
            s.pim_revision = s.pim_revision.wrapping_add(1);
            Ok(json!({"account": account, "calendar": calendar, "shown": shown}))
        })
        .params(r#"{"account": string, "calendar": string, "shown"?: bool (default: the other way)}"#),
        toggle!("calendar.sideBySide", "Side by side", "Calendar › View", calendars_side_by_side, r#"{"on"?: bool (each calendar in a column of its own in day and week)}"#),
        CommandSpec::new("calendar.months", "Months in the side pane", "Calendar › Side pane", |s, v| {
            let n = p::u64(v, "count").ok_or_else(|| CmdError::Params("`count` is required (1 to 6)".into()))?;
            s.prefs.calendar_months = u8::try_from(n.clamp(1, 6)).unwrap_or(1);
            Ok(json!({"count": s.prefs.calendar_months}))
        })
        .params(r#"{"count": number (1 to 6)}"#),
        CommandSpec::new("calendar.workTime", "Work time", "File › Options › Calendar", |s, v| {
            let days: Option<Vec<u32>> = match v.get("days") {
                None | Some(serde_json::Value::Null) => None,
                Some(d) => Some(
                    d.as_array()
                        .ok_or_else(|| CmdError::Params("`days` is a list of days, 0 (Monday) to 6 (Sunday)".into()))?
                        .iter()
                        .map(|x| x.as_u64().and_then(|x| u32::try_from(x).ok()).unwrap_or(u32::MAX))
                        .collect(),
                ),
            };
            let time = |key: &str| -> Result<Option<u16>, CmdError> {
                let Some(t) = p::str(v, key) else { return Ok(None) };
                let minutes = t
                    .trim()
                    .split_once(':')
                    .and_then(|(h, m)| Some((h.parse::<u16>().ok()?, m.parse::<u16>().ok()?)))
                    .filter(|(h, m)| (*h < 24 && *m < 60) || (*h == 24 && *m == 0))
                    .map(|(h, m)| h * 60 + m);
                minutes.map(Some).ok_or_else(|| CmdError::Params(format!("`{key}` is a time of day (HH:MM), not `{t}`")))
            };
            let (start, end) = (time("start")?, time("end")?);
            s.set_work_time(days.as_deref(), start, end)
        })
        .params(r#"{"days"?: [number] (0 Monday … 6 Sunday), "start"?: "HH:MM", "end"?: "HH:MM"}"#),
        CommandSpec::new("account.keepMail", "Mail kept on this computer", "File › Options › Mail", |s, v| {
            if let Some(n) = p::u64(v, "messages") {
                s.prefs.keep_messages = u32::try_from(n.min(1_000_000)).unwrap_or(0);
            }
            if let Some(n) = p::u64(v, "offline") {
                s.prefs.keep_offline = u32::try_from(n.min(100_000)).unwrap_or(0);
            }
            s.apply_sync_prefs();
            Ok(json!({"messages": s.prefs.keep_messages, "offline": s.prefs.keep_offline}))
        })
        .params(r#"{"messages"?: number (the newest of each folder kept; 0: all), "offline"?: number (of those, the newest downloaded in full)}"#),
        CommandSpec::new("account.onlineOnly", "Online only", "File › Options › Mail", |s, v| {
            let id = p::req_str(v, "account")?;
            let mut c = s.config_of(id)?;
            c.online_only = p::bool(v, "on").unwrap_or(!c.online_only);
            s.update_account(c, None)?;
            let on = s.account(id).is_some_and(|a| a.config.keeps_online());
            Ok(json!({"account": id, "on": on}))
        })
        .params(r#"{"account": string, "on"?: bool (default: the other way; keep none of its mail on this computer, only in memory while MailCraft runs)}"#),
        CommandSpec::new("account.checkEvery", "Check for new mail every", "File › Options › Mail", |s, v| {
            let id = p::req_str(v, "account")?;
            // Only when asked (issue #104).
            if p::bool(v, "byHand") == Some(true) {
                let mut c = s.config_of(id)?;
                if !c.has_server() {
                    return Err(CmdError::Params("that account has no server to look at".into()));
                }
                (c.check_every, c.check_by_hand) = (None, true);
                s.update_account(c, None)?;
                return Ok(json!({"account": id, "byHand": true}));
            }
            let minutes = p::u64(v, "minutes").ok_or_else(|| CmdError::Params("`minutes` is required (0: automatically)".into()))?;
            let minutes = u32::try_from(minutes).ok().filter(|m| *m <= mailcraft_model::MAX_CHECK_MINUTES);
            let Some(minutes) = minutes else {
                return Err(CmdError::Params(format!("`minutes` can be at most {}", mailcraft_model::MAX_CHECK_MINUTES)));
            };
            let mut c = s.config_of(id)?;
            if !c.has_server() {
                return Err(CmdError::Params("that account has no server to look at".into()));
            }
            (c.check_every, c.check_by_hand) = ((minutes > 0).then_some(minutes), false);
            s.update_account(c, None)?;
            Ok(json!({"account": id, "minutes": minutes}))
        })
        .params(r#"{"account": string, "minutes": number (0: as soon as the server says, or else every two minutes; up to a day) | "byHand": true (only when asked: Send / Receive on the account)}"#),
        CommandSpec::new("note.arrange", "Arrange notes", "Notes › Arrange by", |s, v| {
            s.prefs.notes_order = match p::req_str(v, "by")? {
                "modified" => crate::session::NotesOrder::Modified,
                "title" => crate::session::NotesOrder::Title,
                "color" => crate::session::NotesOrder::Color,
                other => return Err(CmdError::Params(format!("`by` is modified, title or color, not `{other}`"))),
            };
            Ok(json!({"by": p::req_str(v, "by")?}))
        })
        .params(r#"{"by": "modified"|"title"|"color"}"#),
        CommandSpec::new("view.readingOptions", "Reading pane options", "File › Options › Mail", |s, v| {
            if let Some(on) = p::bool(v, "markRead") {
                s.prefs.mark_read_on_open = on;
            }
            match p::str(v, "when") {
                Some("shown") => s.prefs.read_in_pane = crate::session::ReadInPane::Shown,
                Some("left") => s.prefs.read_in_pane = crate::session::ReadInPane::Left,
                Some(other) => return Err(CmdError::Params(format!("`when` is shown or left, not `{other}`"))),
                None => {}
            }
            if let Some(secs) = p::u64(v, "after") {
                s.prefs.read_after_secs = u32::try_from(secs.min(999)).unwrap_or(999);
            }
            let when = match s.prefs.read_in_pane {
                crate::session::ReadInPane::Shown => "shown",
                crate::session::ReadInPane::Left => "left",
            };
            Ok(json!({"markRead": s.prefs.mark_read_on_open, "when": when, "after": s.prefs.read_after_secs}))
        })
        .params(r#"{"markRead"?: bool (in the reading pane at all; opening a message in its own window always does), "when"?: "shown"|"left" (when it's shown, or once another is chosen), "after"?: number (seconds shown first, with "shown")}"#),
        toggle!("view.animations", "Play animations", "File › Options › General", animations, r#"{"on"?: bool (none: switch)}"#),
        toggle!("view.photoLocation", "Send where photos were taken", "File › Options › Safety", keep_photo_location, r#"{"on"?: bool (none: switch; off, the place is taken out of attached photos as they're sent)}"#),
        toggle!("view.cleanLinks", "Open links without their tracking", "File › Options › Safety", clean_links, r#"{"on"?: bool (none: switch; on, a link opens out of its redirect wrapper and without the tags naming the reader)}"#),
        toggle!("view.autoPictures", "Show web pictures unless a message looks suspicious", "File › Options › Safety", auto_pictures, r#"{"on"?: bool (none: switch; on, a message whose sender the mail server confirmed shows its web pictures as it opens)}"#),
        toggle!("view.checkRevocation", "Check whether certificates were withdrawn", "File › Options › Safety", check_revocation, r#"{"on"?: bool (none: switch; on, a signed message's certificate is checked with its authority as it's shown, which tells the authority)}"#),
        CommandSpec::new("view.senderAddresses", "Show sender addresses in the message list", "File › Options › Safety", |s, v| {
            let show = p::req_str(v, "show")?;
            s.prefs.sender_addresses = crate::session::SenderAddresses::from_code(show)
                .ok_or_else(|| CmdError::Params(format!("`show` is always, strangers or never, not `{show}`")))?;
            Ok(json!({"show": s.prefs.sender_addresses.code()}))
        })
        .params(r#"{"show": "always"|"strangers"|"never" (strangers: for senders who aren't contacts, written to or in the directory)}"#),
        CommandSpec::new("view.externalTag", "Tag mail from outside the organisation", "File › Options › Safety", external_tag).params(
            r#"{"on"?: bool (none: as it is), "domains"?: [string] | string (the organisation's domains, separated by commas or spaces; empty: those of your accounts, mail services left out)}"#,
        ),
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
    ]
}

/// The External tag (issue #29): on or off, and the organisation's domains.
fn external_tag(s: &mut Session, v: &Value) -> CmdResult {
    if let Some(given) = v.get("domains").filter(|d| !d.is_null()) {
        let words: Vec<String> = match given {
            Value::String(text) => text.split(|c: char| c == ',' || c == ';' || c.is_whitespace()).map(str::to_string).collect(),
            Value::Array(list) => list
                .iter()
                .map(|d| d.as_str().map(str::to_string).ok_or_else(|| CmdError::Params("`domains` lists domains as text".into())))
                .collect::<Result<_, _>>()?,
            _ => return Err(CmdError::Params("`domains` is a list of domains or a text".into())),
        };
        let mut domains: Vec<String> = Vec::new();
        for w in words.iter().map(|w| w.trim()).filter(|w| !w.is_empty()) {
            let d = mailcraft_model::lookalike::clean_domain(w)
                .ok_or_else(|| CmdError::Params(format!("`{}` isn't a domain", w.chars().take(80).collect::<String>())))?;
            if !domains.contains(&d) {
                domains.push(d);
            }
        }
        const MOST: usize = 100;
        if domains.len() > MOST {
            return Err(CmdError::Params(format!("at most {MOST} domains")));
        }
        s.prefs.org_domains = domains;
    }
    if let Some(on) = p::bool(v, "on") {
        s.prefs.external_tag = on;
    }
    let effective: Vec<String> = s.org_domains().into_iter().collect();
    Ok(json!({"on": s.prefs.external_tag, "domains": s.prefs.org_domains, "organisation": effective}))
}

pub(super) fn reading_pane(s: &mut Session, v: &Value) -> CmdResult {
    s.prefs.reading_pane = match p::req_str(v, "value")?.to_ascii_lowercase().as_str() {
        "right" => ReadingPane::Right,
        "bottom" => ReadingPane::Bottom,
        "off" => ReadingPane::Off,
        other => return Err(CmdError::Params(format!("unknown reading pane position `{other}`"))),
    };
    Ok(json!({"value": p::req_str(v, "value")?}))
}
