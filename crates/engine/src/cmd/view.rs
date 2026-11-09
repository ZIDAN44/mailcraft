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
        CommandSpec::new("view.classicRibbon", "Classic ribbon", "View › Layout", |s, v| {
            s.prefs.classic_ribbon = p::bool(v, "on").unwrap_or(!s.prefs.classic_ribbon);
            Ok(json!({"on": s.prefs.classic_ribbon}))
        })
        .params(r#"{"on"?: bool}"#),
        CommandSpec::new("view.accent", "Accent colour", "File › Options › Personalize", |s, v| {
            let given = p::req_str(v, "color")?;
            s.prefs.accent = crate::session::accent_code(given)
                .ok_or_else(|| CmdError::Params(format!("`{given}` isn't one of the accent colours or a colour as #rrggbb")))?;
            Ok(json!({"color": s.prefs.accent}))
        })
        .params(r##"{"color": "teal"|"blue"|"indigo"|"violet"|"rose"|"red"|"orange"|"green"|"#rrggbb" ("" or "teal": MailCraft's own)}"##),
        CommandSpec::new("view.plainTopBar", "Plain top bar", "File › Options › Personalize", |s, v| {
            s.prefs.plain_top_bar = p::bool(v, "on").unwrap_or(!s.prefs.plain_top_bar);
            Ok(json!({"on": s.prefs.plain_top_bar}))
        })
        .params(r#"{"on"?: bool}"#),
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
        CommandSpec::new("view.replyWindow", "Replies and forwards in their own window", "File › Options › Mail", |s, v| {
            s.prefs.reply_window = p::bool(v, "on").unwrap_or(!s.prefs.reply_window);
            Ok(json!({"on": s.prefs.reply_window}))
        })
        .params(r#"{"on"?: bool}"#),
        CommandSpec::new("calendar.tentativeInvites", "Invitations into the calendar as tentative", "File › Options › Calendar", |s, v| {
            s.prefs.tentative_invites = p::bool(v, "on").unwrap_or(!s.prefs.tentative_invites);
            Ok(json!({"on": s.prefs.tentative_invites}))
        })
        .params(r#"{"on"?: bool}"#),
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
        CommandSpec::new("view.favorites", "Show Favorites", "View › Layout", |s, v| {
            s.prefs.show_favorites = p::bool(v, "on").unwrap_or(!s.prefs.show_favorites);
            Ok(json!({"on": s.prefs.show_favorites}))
        })
        .params(r#"{"on"?: bool}"#),
        CommandSpec::new("view.markReadOnOpen", "Mark as read when opened", "File › Options › Mail", |s, v| {
            s.prefs.mark_read_on_open = p::bool(v, "on").unwrap_or(!s.prefs.mark_read_on_open);
            Ok(json!({"on": s.prefs.mark_read_on_open}))
        })
        .params(r#"{"on"?: bool}"#),
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
        CommandSpec::new("calendar.sideBySide", "Side by side", "Calendar › View", |s, v| {
            s.prefs.calendars_side_by_side = p::bool(v, "on").unwrap_or(!s.prefs.calendars_side_by_side);
            Ok(json!({"on": s.prefs.calendars_side_by_side}))
        })
        .params(r#"{"on"?: bool (each calendar in a column of its own in day and week)}"#),
        CommandSpec::new("calendar.months", "Months in the side pane", "Calendar › Side pane", |s, v| {
            let n = p::u64(v, "count").ok_or_else(|| CmdError::Params("`count` is required (1 to 6)".into()))?;
            s.prefs.calendar_months = u8::try_from(n.clamp(1, 6)).unwrap_or(1);
            Ok(json!({"count": s.prefs.calendar_months}))
        })
        .params(r#"{"count": number (1 to 6)}"#),
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
            let mut c = s.account(id).map(|a| a.config.clone()).ok_or_else(|| CmdError::Params(format!("no account `{id}`")))?;
            c.online_only = p::bool(v, "on").unwrap_or(!c.online_only);
            s.update_account(c, None)?;
            let on = s.account(id).is_some_and(|a| a.config.keeps_online());
            Ok(json!({"account": id, "on": on}))
        })
        .params(r#"{"account": string, "on"?: bool (default: the other way; keep none of its mail on this computer, only in memory while MailCraft runs)}"#),
        CommandSpec::new("account.checkEvery", "Check for new mail every", "File › Options › Mail", |s, v| {
            let id = p::req_str(v, "account")?;
            let minutes = p::u64(v, "minutes").ok_or_else(|| CmdError::Params("`minutes` is required (0: automatically)".into()))?;
            let minutes = u32::try_from(minutes).ok().filter(|m| *m <= mailcraft_model::MAX_CHECK_MINUTES);
            let Some(minutes) = minutes else {
                return Err(CmdError::Params(format!("`minutes` can be at most {}", mailcraft_model::MAX_CHECK_MINUTES)));
            };
            let mut c = s.account(id).map(|a| a.config.clone()).ok_or_else(|| CmdError::Params(format!("no account `{id}`")))?;
            if c.is_local() {
                return Err(CmdError::Params("that account has no server to look at".into()));
            }
            c.check_every = (minutes > 0).then_some(minutes);
            s.update_account(c, None)?;
            Ok(json!({"account": id, "minutes": minutes}))
        })
        .params(r#"{"account": string, "minutes": number (0: as soon as the server says, or else every two minutes; up to a day)}"#),
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
        CommandSpec::new("view.animations", "Play animations", "File › Options › General", |s, v| {
            s.prefs.animations = p::bool(v, "on").unwrap_or(!s.prefs.animations);
            Ok(json!({ "on": s.prefs.animations }))
        })
        .params(r#"{"on"?: bool (none: switch)}"#),
        CommandSpec::new("view.photoLocation", "Send where photos were taken", "File › Options › Safety", |s, v| {
            s.prefs.keep_photo_location = p::bool(v, "on").unwrap_or(!s.prefs.keep_photo_location);
            Ok(json!({ "on": s.prefs.keep_photo_location }))
        })
        .params(r#"{"on"?: bool (none: switch; off, the place is taken out of attached photos as they're sent)}"#),
        CommandSpec::new("view.cleanLinks", "Open links without their tracking", "File › Options › Safety", |s, v| {
            s.prefs.clean_links = p::bool(v, "on").unwrap_or(!s.prefs.clean_links);
            Ok(json!({ "on": s.prefs.clean_links }))
        })
        .params(r#"{"on"?: bool (none: switch; on, a link opens out of its redirect wrapper and without the tags naming the reader)}"#),
        CommandSpec::new("view.autoPictures", "Show web pictures unless a message looks suspicious", "File › Options › Safety", |s, v| {
            s.prefs.auto_pictures = p::bool(v, "on").unwrap_or(!s.prefs.auto_pictures);
            Ok(json!({ "on": s.prefs.auto_pictures }))
        })
        .params(r#"{"on"?: bool (none: switch; on, a message whose sender the mail server confirmed shows its web pictures as it opens)}"#),
        CommandSpec::new("view.checkRevocation", "Check whether certificates were withdrawn", "File › Options › Safety", |s, v| {
            s.prefs.check_revocation = p::bool(v, "on").unwrap_or(!s.prefs.check_revocation);
            Ok(json!({ "on": s.prefs.check_revocation }))
        })
        .params(r#"{"on"?: bool (none: switch; on, a signed message's certificate is checked with its authority as it's shown, which tells the authority)}"#),
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
