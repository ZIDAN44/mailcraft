//! The ribbon's tabs in Mail: Home, Send / Receive, Search and View.

use super::quick_steps_menu;
use crate::*;

/// Whether a command can run now (its button is greyed out when it can't).
fn enabled(s: &Session, id: &str) -> bool {
    s.registry().get(id).is_some_and(|c| (c.enabled)(s).is_none())
}

/// Home: new items, deleting, moving, responding, tags and finding.
pub(super) fn ribbon_home(ui: &mut Ui, s: &Session, t: Tokens, acts: &mut Vec<Action>) {
    if widgets::accent_button(ui, Icon::NewMail, "New mail", &t, enabled(s, "mail.new")).on_hover_text(tl!("New mail (Ctrl+N)")).clicked() {
        acts.push(Action::Run("mail.new", json!({})));
    }
    // New items of other kinds, as Outlook's New Email drop-down.
    let r = widgets::tiny_icon_button(ui, Icon::ChevronDown, "New items", &t);
    egui::Popup::menu(&r).show(|ui| {
        if ui.button(tl!("Email message")).clicked() {
            acts.push(Action::Run("mail.new", json!({})));
            ui.close();
        }
        if ui.button(tl!("Event")).clicked() {
            acts.push(Action::Dialog(Dialog::new_event(s, None)));
            ui.close();
        }
        if ui.button(tl!("Contact")).clicked() {
            acts.push(Action::Dialog(Dialog::new_contact()));
            ui.close();
        }
    });
    widgets::group(ui, &t, "New", false);
    if widgets::tool_button(ui, Icon::Delete, "Delete", &t, enabled(s, "mail.delete")).on_hover_text(tl!("Delete (Del)")).clicked() {
        // Asked first when it's for good (`act_run`).
        acts.push(Action::Run("mail.delete", json!({})));
    }
    // Ignore the conversation, as Outlook's Delete drop-down.
    let r = widgets::tiny_icon_button(ui, Icon::ChevronDown, "More delete options", &t);
    egui::Popup::menu(&r).show(|ui| {
        let ignoring = !s.selection.is_empty() && s.selected_iter().all(|m| s.is_ignored(m));
        let label = if ignoring { "Stop ignoring the conversation" } else { "Ignore the conversation" };
        if ui
            .add_enabled(enabled(s, "mail.ignore"), egui::Button::new(tl!(label)))
            .on_hover_text(tl!("Its messages, and replies that arrive later, go to Deleted Items"))
            .clicked()
        {
            acts.push(Action::Run("mail.ignore", json!({})));
            ui.close();
        }
        // Clean Up: what a later reply quotes in full goes to Deleted Items.
        ui.separator();
        let tip = tl!("Messages quoted in full by a later reply go to Deleted Items");
        if ui.add_enabled(enabled(s, "mail.cleanUp"), egui::Button::new(tl!("Clean up conversation"))).on_hover_text(tip).clicked() {
            acts.push(Action::Run("mail.cleanUp", json!({})));
            ui.close();
        }
        if ui.add_enabled(enabled(s, "folder.cleanUp"), egui::Button::new(tl!("Clean up folder"))).on_hover_text(tip).clicked() {
            acts.push(Action::Run("folder.cleanUp", json!({})));
            ui.close();
        }
    });
    if widgets::tool_button(ui, Icon::Archive, "Archive", &t, enabled(s, "mail.archive")).on_hover_text(tl!("Archive (Backspace)")).clicked() {
        acts.push(Action::Run("mail.archive", json!({})));
    }
    let in_junk = s.view_role() == Some(mailcraft_engine::model::FolderRole::Junk);
    let (jid, jl) = if in_junk { ("mail.notJunk", "Not junk") } else { ("mail.junk", "Report junk") };
    if widgets::tool_button(ui, Icon::Junk, jl, &t, enabled(s, jid)).clicked() {
        acts.push(Action::Run(jid, json!({})));
    }
    // The Junk drop-down: the sender's lists, as Outlook's.
    let r = widgets::tiny_icon_button(ui, Icon::ChevronDown, "Junk options", &t);
    egui::Popup::menu(&r).show(|ui| junk_menu(ui, s, acts));
    widgets::group(ui, &t, "Delete", false);
    if widgets::tool_button(ui, Icon::Move, "Move to", &t, enabled(s, "mail.move")).on_hover_text(tl!("Move to a folder (Ctrl+Shift+V)")).clicked() {
        acts.push(Action::Dialog(Dialog::move_to()));
    }
    // Quick Steps: each done to the selection in one click (Ctrl+Shift+1…9).
    let r = widgets::tool_button(ui, Icon::Steps, "Quick steps", &t, true);
    egui::Popup::menu(&r).show(|ui| quick_steps_menu(ui, s, acts));
    widgets::group(ui, &t, "Move", false);
    for (id, icon, label, tip) in [
        ("mail.reply", Icon::Reply, "Reply", "Reply (Ctrl+R)"),
        ("mail.replyAll", Icon::ReplyAll, "Reply all", "Reply all (Ctrl+Shift+R)"),
        ("mail.forward", Icon::Forward, "Forward", "Forward (Ctrl+F)"),
    ] {
        if widgets::tool_button(ui, icon, label, &t, enabled(s, id)).on_hover_text(tl!(tip)).clicked() {
            acts.push(Action::Run(id, json!({})));
        }
    }
    widgets::group(ui, &t, "Respond", false);
    let first = s.first_selected();
    let first_unread = first.is_some_and(|m| !m.flags.seen);
    let (ricon, rlabel) = if first_unread { (Icon::Read, "Mark read") } else { (Icon::Unread, "Mark unread") };
    if widgets::tool_button(ui, ricon, rlabel, &t, enabled(s, "mail.toggleRead")).on_hover_text(tl!("Read / Unread (Ctrl+Q / Ctrl+U)")).clicked() {
        acts.push(Action::Run("mail.toggleRead", json!({})));
    }
    if widgets::tool_button(ui, Icon::Flag, "Flag", &t, enabled(s, "mail.flag")).on_hover_text(tl!("Flag / Unflag (Insert)")).clicked() {
        acts.push(Action::Run("mail.flag", json!({})));
    }
    // Follow Up: when it's due, as Outlook's flag drop-down.
    let r = widgets::tiny_icon_button(ui, Icon::ChevronDown, "Follow up", &t);
    egui::Popup::menu(&r).show(|ui| organize::follow_up_menu(ui, s, &t, acts));
    let sel = !s.selection.is_empty();
    let r = widgets::tool_button(ui, Icon::Tag, "Categorize", &t, sel);
    egui::Popup::menu(&r).show(|ui| organize::categorize_menu(ui, s, &t, acts));
    let pinned = first.is_some_and(|m| m.flags.pinned());
    if widgets::tool_button(ui, Icon::Pin, if pinned { "Unpin" } else { "Pin" }, &t, sel).clicked() {
        acts.push(Action::Run("mail.pin", json!({"on": !pinned})));
    }
    let r = widgets::tool_button(ui, Icon::Clock, "Snooze", &t, sel);
    egui::Popup::menu(&r).show(|ui| organize::snooze_menu(ui, s, &t, acts));
    widgets::group(ui, &t, "Tags", false);
    // Show only some mail, as Outlook's Filter Email.
    let r = widgets::tool_button(ui, Icon::Funnel, "Filter", &t, true);
    egui::Popup::menu(&r).show(|ui| {
        for f in mailcraft_engine::ListFilter::EVERY {
            if ui.selectable_label(s.filter == f, tl!(f.label())).clicked() {
                acts.push(Action::Run("list.filter", json!({"value": f.code()})));
                ui.close();
            }
        }
    });
    widgets::group(ui, &t, "Find", true);
}

/// Send / Receive: every folder, this one, the Outbox, and working offline (`has_files`: not
/// in the browser).
pub(super) fn ribbon_send_receive(ui: &mut Ui, s: &Session, t: Tokens, has_files: bool, acts: &mut Vec<Action>) {
    if widgets::tool_button(ui, Icon::Sync, "Send / Receive all folders", &t, enabled(s, "mail.sync")).on_hover_text("F9").clicked() {
        acts.push(Action::Run("mail.sync", json!({})));
    }
    if widgets::tool_button(ui, Icon::Folder, "Update folder", &t, enabled(s, "folder.sync")).on_hover_text("Shift+F9").clicked() {
        acts.push(Action::Run("folder.sync", json!({})));
    }
    if widgets::tool_button(ui, Icon::Send, "Send all", &t, enabled(s, "outbox.send") && !s.prefs.work_offline)
        .on_hover_text(tl!("Send what's waiting in the Outbox"))
        .clicked()
    {
        acts.push(Action::Run("outbox.send", json!({})));
    }
    widgets::group(ui, &t, "Send & Receive", !has_files);
    // Working offline: no connections; mail you send waits in the Outbox.
    if has_files {
        let off = s.prefs.work_offline;
        let mut r = ui
            .selectable_label(off, tl!("Work offline"))
            .on_hover_text(tl!("Close every connection: changes, mail you send and drafts wait on this computer"));
        crate::keytips::apply(ui, "Work offline", &mut r, true, &t);
        if r.clicked() {
            acts.push(Action::Run("mail.workOffline", json!({"on": !off})));
        }
        widgets::group(ui, &t, "Preferences", true);
    }
}

/// Search, while searching: where it looks, terms to narrow it, Close Search.
pub(super) fn ribbon_search(ui: &mut Ui, s: &Session, ust: &mut UiState, t: Tokens, acts: &mut Vec<Action>) {
    let query = s.search.as_ref().map(|(q, _)| q.clone()).unwrap_or_else(|| ust.search_text.clone());
    let current = s.search.as_ref().map_or(s.prefs.search_scope, |(_, scope)| *scope);
    ui.label(RichText::new(tl!("Look in")).color(t.dim));
    for scope in SearchScope::ALL {
        if ui.selectable_label(scope == current, scope_label(scope)).clicked() && scope != current {
            acts.push(Action::Run("search.run", json!({"query": query, "scope": scope.code()})));
        }
    }
    widgets::group(ui, &t, "Scope", false);
    // Terms to type a name after: put in the box, ready for it.
    for (icon, label, term) in [(Icon::Person, "From", "from:"), (Icon::Send, "Sent To", "to:"), (Icon::Read, "Subject", "subject:")] {
        if widgets::tool_button(ui, icon, label, &t, true).clicked() {
            let text = &mut ust.search_text;
            if !text.is_empty() && !text.ends_with(' ') {
                text.push(' ');
            }
            text.push_str(term);
            ust.focus_search = true;
            ust.search_cursor_end = true;
        }
    }
    // Terms that narrow at once: on, or off again.
    let words: Vec<&str> = query.split_whitespace().collect();
    for (icon, label, term) in [
        (Icon::Attach, "Has Attachments", "has:attachment"),
        (Icon::Unread, "Unread", "is:unread"),
        (Icon::Tag, "Categorized", "is:categorized"),
        (Icon::Flag, "Flagged", "is:flagged"),
        (Icon::Warning, "Important", "is:important"),
        (Icon::Send, "Awaiting reply", "is:awaiting"),
    ] {
        let on = words.iter().any(|w| w.eq_ignore_ascii_case(term));
        if widgets::tool_toggle(ui, icon, label, &t, on).clicked() {
            let q = if on {
                words.iter().filter(|w| !w.eq_ignore_ascii_case(term)).copied().collect::<Vec<_>>().join(" ")
            } else if query.trim().is_empty() {
                term.to_string()
            } else {
                format!("{} {term}", query.trim_end())
            };
            ust.search_text = q.clone();
            if q.trim().is_empty() {
                acts.push(Action::Run("search.clear", json!({})));
            } else {
                acts.push(Action::Run("search.run", json!({"query": q, "scope": current.code()})));
            }
        }
    }
    widgets::group(ui, &t, "Refine", false);
    if widgets::tool_button(ui, Icon::Close, "Close Search", &t, true).clicked() {
        acts.push(Action::Run("search.clear", json!({})));
        ust.search_text.clear();
        ust.tab = RibbonTab::Home;
    }
    widgets::group(ui, &t, "Close", true);
}

/// View: the reading pane, the list's look and arrangement, the message, the theme (`dark`: the
/// one chosen, `None` the system's).
pub(super) fn ribbon_view(ui: &mut Ui, s: &Session, t: Tokens, dark: Option<bool>, acts: &mut Vec<Action>) {
    // Buttons as Home's are, so the row folds as Home's does in a narrow
    // window (labels go from the end) instead of scrolling: the reading pane's
    // place and the theme in a menu each, the rest turned on or off.
    let r = widgets::tool_button(ui, Icon::Layout, "Reading pane", &t, true);
    egui::Popup::menu(&r).show(|ui| {
        for (v, label, code) in [(ReadingPane::Right, "Right", "right"), (ReadingPane::Bottom, "Bottom", "bottom"), (ReadingPane::Off, "Off", "off")]
        {
            if ui.selectable_label(s.prefs.reading_pane == v, tl!(label)).clicked() {
                acts.push(Action::Run("view.readingPane", json!({"value": code})));
                ui.close();
            }
        }
    });
    widgets::group(ui, &t, "Reading pane", false);
    for (on, icon, label, tip, id) in [
        (s.prefs.focused_inbox, Icon::Inbox, "Focused Inbox", "Put newsletters and automatic mail in Other", "view.focused"),
        (s.prefs.conversations, Icon::Conversation, "Conversations", "Group messages by conversation", "view.conversations"),
        (s.prefs.show_preview, Icon::Preview, "Message preview", "", "view.preview"),
        (s.prefs.compact, Icon::Compact, "Compact", "", "view.compact"),
        (s.prefs.single_line, Icon::ListBullet, "Single line", "One line a message: the sender, the subject and the time", "view.singleLine"),
        (!s.prefs.folder_pane_minimized, Icon::Folder, "Folder pane", "", "view.folderPane"),
        (s.prefs.show_favorites, Icon::Star, "Favorites", "Show Favorites at the top of the folder pane", "view.favorites"),
    ] {
        let r = widgets::tool_toggle(ui, icon, label, &t, on);
        let r = if tip.is_empty() { r } else { r.on_hover_text(tl!(tip)) };
        if r.clicked() {
            acts.push(Action::Run(id, json!({})));
        }
    }
    // The To-Do Bar beside the mail: its parts, each on or off, as Outlook's menu.
    let bar = s.prefs.todo_bar;
    let r = widgets::tool_button(ui, Icon::Task, "To-Do Bar", &t, true)
        .on_hover_text(tl!("Show the calendar, what's coming up and your tasks beside the mail"));
    egui::Popup::menu(&r).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        for (on, label, part) in
            [(bar.calendar, "Calendar", "calendar"), (bar.appointments, "Appointments", "appointments"), (bar.tasks, "Tasks", "tasks")]
        {
            let mut now = on;
            if ui.checkbox(&mut now, tl!(label)).changed() {
                acts.push(Action::Run("view.todoBar", json!({"part": part, "on": now})));
            }
        }
        ui.separator();
        if ui.selectable_label(!bar.shown(), tl!("Off")).clicked() {
            acts.push(Action::Run("view.todoBar", json!({"off": true})));
            ui.close();
        }
    });
    widgets::group(ui, &t, "Layout", false);
    let arrange = tf!("Arrange by: {field}", field = tl!(s.prefs.arrange.label()));
    let r = widgets::tool_button(ui, Icon::ListBullet, widgets::Label::shown("Arrange by", &arrange), &t, true);
    egui::Popup::menu(&r).show(|ui| list::arrange_menu(ui, s, acts));
    if widgets::tool_toggle(ui, Icon::Sort, "Reverse sort", &t, s.prefs.reverse_sort).clicked() {
        acts.push(Action::Run("view.reverseSort", json!({})));
    }
    widgets::group(ui, &t, "Arrangement", false);
    // The message's source, headers and all, in a window of its own (issue #27).
    let one = s.reading.is_some() || s.focus.is_some() || !s.selection.is_empty();
    if widgets::tool_button(ui, Icon::Source, "Message source", &t, one).on_hover_text(tl!("View the message as it came, headers and all")).clicked()
        && one
    {
        acts.push(Action::Run("message.source", json!({})));
    }
    if widgets::tool_toggle(ui, Icon::ClearFormat, "Plain text", &t, !s.prefs.show_html)
        .on_hover_text(tl!("Show mail without its formatting"))
        .clicked()
    {
        acts.push(Action::Run("view.html", json!({})));
    }
    widgets::group(ui, &t, "Message", false);
    let theme_icon = if dark == Some(true) { Icon::Moon } else { Icon::Sun };
    let r = widgets::tool_button(ui, theme_icon, "Theme", &t, true);
    egui::Popup::menu(&r).show(|ui| {
        for (v, label) in [(None, "System"), (Some(false), "Light"), (Some(true), "Dark")] {
            if ui.selectable_label(dark == v, tl!(label)).clicked() {
                acts.push(Action::SetDark(v));
                ui.close();
            }
        }
    });
    // Big icons over their labels, in named groups (Outlook's classic
    // ribbon), or one row of small ones.
    if widgets::tool_toggle(ui, Icon::Ribbon, "Classic ribbon", &t, s.prefs.classic_ribbon)
        .on_hover_text(tl!("Big buttons with their names under them, in named groups"))
        .clicked()
    {
        acts.push(Action::Run("view.classicRibbon", json!({})));
    }
    widgets::group(ui, &t, "Appearance", true);
}
