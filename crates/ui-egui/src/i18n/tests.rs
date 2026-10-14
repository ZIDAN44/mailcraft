use std::collections::BTreeSet;

use super::*;

/// Every interface string: literals in the UI code (`tl!`, `tf!` and the helpers that translate
/// their label), the names the model hands up, and whatever a tour of the interface looks up.
pub(crate) fn interface_strings() -> BTreeSet<String> {
    let sources = [
        include_str!("../lib.rs"),
        include_str!("../list.rs"),
        include_str!("../nav.rs"),
        include_str!("../reading.rs"),
        include_str!("../dialogs.rs"),
        include_str!("../widgets.rs"),
        include_str!("../organize.rs"),
        include_str!("../editor.rs"),
        include_str!("../backstage.rs"),
        include_str!("../calendar.rs"),
        include_str!("../people.rs"),
        include_str!("../pim_dialogs.rs"),
    ];
    // (prefix, which string literals after it are interface text)
    let prefixes: [(&str, &[usize]); 20] = [
        ("tl!(", &[0]),
        ("tf!(", &[0]),
        ("tool_button(ui, Icon::", &[0]),
        ("accent_button(ui, Icon::", &[0]),
        ("pill_button(ui, Icon::", &[0]),
        ("bar_icon_button(ui, ", &[0]),
        ("tiny_icon_button(ui, Icon::", &[0]),
        ("title(ui, t, ", &[0]),
        ("buttons(ui, t, ", &[0]),
        ("row(ui, t, Icon::", &[0]),
        ("section(ui, t, ", &[0]),
        ("empty(ui, &t, ", &[0]),
        ("label(ui, ", &[0]),
        ("field(ui, &t, ", &[0, 1]),
        ("recipient_field(ui, st, ", &[1, 2]),
        ("tile(ui, t, Icon::", &[0, 1, 2]),
        ("rail_item(ui, &t, Icon::", &[0]),
        ("heading(ui, ", &[0]),
        ("group(ui, ", &[0]),
        ("check(ui, s.prefs.", &[0]),
    ];
    let mut out = BTreeSet::new();
    for src in sources {
        for (p, which) in prefixes {
            let mut rest = src;
            while let Some(i) = rest.find(p) {
                rest = &rest[i + p.len()..];
                // String literals up to the end of the line.
                let line = &rest[..rest.find('\n').unwrap_or(rest.len())];
                let mut lits: Vec<String> = Vec::new();
                let mut chars = line.chars();
                while let Some(c) = chars.next() {
                    if c != '"' {
                        continue;
                    }
                    let mut lit = String::new();
                    while let Some(c) = chars.next() {
                        match c {
                            '\\' => {
                                if let Some(e) = chars.next() {
                                    lit.push(if e == 'n' { '\n' } else { e });
                                }
                            }
                            '"' => break,
                            c => lit.push(c),
                        }
                    }
                    lits.push(lit);
                }
                for &w in which {
                    if let Some(l) = lits.get(w)
                        && l.chars().filter(|c| c.is_alphabetic()).count() >= 2
                        && !l.starts_with("mail.")
                        && !l.starts_with("compose.")
                    {
                        out.insert(l.clone());
                    }
                }
            }
        }
    }
    use mailcraft_engine::model::dates::DateGroup;
    use mailcraft_engine::model::organize::SnoozeUntil;
    for g in [
        DateGroup::Today,
        DateGroup::Yesterday,
        DateGroup::ThisWeek(0),
        DateGroup::ThisWeek(1),
        DateGroup::ThisWeek(2),
        DateGroup::ThisWeek(3),
        DateGroup::ThisWeek(4),
        DateGroup::ThisWeek(5),
        DateGroup::ThisWeek(6),
        DateGroup::LastWeek,
        DateGroup::TwoWeeksAgo,
        DateGroup::ThreeWeeksAgo,
        DateGroup::LastMonth,
        DateGroup::Older,
        DateGroup::Future,
    ] {
        out.insert(g.label().to_string());
    }
    use mailcraft_engine::model::FolderRole;
    for r in [FolderRole::Inbox, FolderRole::Drafts, FolderRole::Sent, FolderRole::Archive, FolderRole::Junk, FolderRole::Trash] {
        out.extend(r.label().map(str::to_string));
    }
    for f in mailcraft_engine::ListFilter::ALL {
        out.insert(f.label().to_string());
    }
    for p in SnoozeUntil::ALL {
        out.insert(p.label().to_string());
    }
    for f in crate::editor::Fmt::ALL {
        out.insert(f.label().to_string());
        out.insert(f.tip().to_string());
    }
    for c in mailcraft_engine::model::organize::default_categories() {
        out.insert(c.name);
    }
    for w in [
        "Mon",
        "Tue",
        "Wed",
        "Thu",
        "Fri",
        "Sat",
        "Sun",
        "Jan",
        "Feb",
        "Mar",
        "Apr",
        "May",
        "Jun",
        "Jul",
        "Aug",
        "Sep",
        "Oct",
        "Nov",
        "Dec",
        "All Inboxes",
        "Outbox",
        "Picture",
        "Video",
        "Audio",
        "Text",
        "Archive",
        "Document",
        "File",
        "Home",
        "View",
        "Right",
        "Bottom",
        "Off",
        "System",
        "Light",
        "Dark",
        "Focused",
        "Other",
        "Message sent",
        "Draft saved",
        "Saved in the Outbox: it will be sent when you're back online",
        "Opened as a PDF: print it from the viewer",
        "Reply",
        "Reply all",
        "Forward",
        "Reply (Ctrl+R)",
        "Reply all (Ctrl+Shift+R)",
        "Forward (Ctrl+F)",
        "Mark read",
        "Mark unread",
        "Not junk",
        "Report junk",
        "Unpin",
        "Pin",
        "Sending…",
        "Send",
        "Rename",
        "Create",
        "Rename folder",
        "New folder",
        "Edit rule",
        "New rule",
        "Remove from Favorites",
        "Show in Favorites",
        "Mark as unread",
        "Mark as read",
        "Unflag",
        "Flag",
        "Sign in again…",
        "Password…",
        "From contains",
        "To or Cc contains",
        "Subject contains",
        "Sender, recipients or subject contain",
        "Has attachments",
        "MailCraft – Sample mailbox",
        // The File page: lists and calls laid out over several lines.
        "Info",
        "Options",
        "About",
        "Back",
        "Sample mailbox",
        "JMAP",
        "Google (IMAP)",
        "Microsoft 365 (IMAP)",
        "IMAP and SMTP",
        "Same as the system",
        "Empty Deleted Items…",
        "Empty Junk Email…",
        "Automatic replies",
        "Account settings",
        "Mailbox",
        "Mailbox settings",
        "Manage rules",
        "Rules",
        "Change this account's name, servers and sign-in, or its signature.",
        "Tell people you're away: each sender gets one reply while they're on, even when this computer is off.",
        "See how much space your mail takes, and empty Deleted Items and Junk Email.",
        "Sort new mail as it arrives: move it, flag it, categorize it.",
        "Local-first: mail is cached on this computer for fast offline search. Passwords stay in the system keychain.",
        "MailCraft talks only to your own mail servers, over encrypted connections. Pictures from the web load only when you ask.",
        "Built in the open from the email standards: no code, icons or assets from other mail apps.",
        // Calendar and People: names in lists.
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
        "Mail",
        "Calendar",
        "People",
        "Day",
        "Week",
        "Month",
        "Accept",
        "Tentative",
        "Decline",
        "Accepted",
        "Declined",
        "No answer yet",
        "Event",
        "New event",
        "Edit contact",
        "New contact",
        "Never",
        "Every day",
        "Every week",
        "Every month",
        "Every year",
        "Custom",
        "No contacts yet.",
        "No one matches.",
        "Close",
        "Cancel",
        "Is a newsletter or list mail",
        "Move to folder",
        "Delete",
        "Categorize",
    ] {
        out.insert(w.to_string());
    }
    out
}

#[test]
fn catalogs_parse_cleanly() {
    for l in LANGUAGES.iter().skip(1) {
        let (entries, errors) = parse(l.source);
        assert!(errors.is_empty(), "{}: {errors:?}", l.code);
        assert!(entries.len() > 100, "{}: only {} entries", l.code, entries.len());
    }
}

#[test]
fn every_interface_string_is_translated() {
    let strings = interface_strings();
    if let Ok(path) = std::env::var("MAILCRAFT_DUMP_STRINGS") {
        let _ = std::fs::write(path, strings.iter().map(|s| s.replace('\n', "\\n")).collect::<Vec<_>>().join("\n"));
    }
    assert!(strings.len() > 200, "{} strings found", strings.len());
    for l in Lang::all().skip(1) {
        let missing: Vec<&String> = strings.iter().filter(|s| l.0.catalog().get(s.as_str()).is_none()).collect();
        assert!(missing.is_empty(), "{}: {} untranslated, e.g. {:?}", l.code(), missing.len(), missing.iter().take(12).collect::<Vec<_>>());
    }
}

#[test]
fn tags_templates_and_dates() {
    assert_eq!(lang_from_tag("pt_BR.UTF-8").map(Lang::code), Some("pt-br"));
    assert_eq!(lang_from_tag("pt-PT"), None);
    assert_eq!(lang_from_tag("zh-Hans-CN").map(Lang::code), Some("zh-hans"));
    assert_eq!(lang_from_tag("zh_TW").map(Lang::code), Some("zh-hant"));
    assert_eq!(lang_from_tag("zh-Hant-HK").map(Lang::code), Some("zh-hant"));
    assert_eq!(lang_from_tag("zh-Hans-HK").map(Lang::code), Some("zh-hans"), "the script wins over the region");
    assert_eq!(lang_from_tag("ko-KR").map(Lang::code), Some("ko"));
    assert_eq!(lang_from_tag("it_IT.UTF-8").map(Lang::code), Some("it"));
    assert_eq!(lang_from_tag("ru").map(Lang::code), Some("ru"));
    assert_eq!(lang_from_tag("de-AT").map(Lang::code), Some("de"));
    assert_eq!(lang_from_tag("C"), Some(Lang::EN));
    assert_eq!(lang_from_tag("xx"), None);
    assert_eq!(fmt("Saved {n} in {folder}", &[("n", "3"), ("folder", "{n}")]), "Saved 3 in {n}", "values aren't templates");
    assert_eq!(fmt("{missing} ok", &[]), "{missing} ok");
    set_current(Lang::from_code("de").unwrap_or(Lang::EN));
    assert_ne!(date("Wed 21:51"), "Wed 21:51");
    assert_eq!(t("Delete"), "Löschen");
    assert_eq!((t("Archive"), tc("folder", "Archive")), ("Archivieren", "Archiv"), "the command, then the folder");
    assert_eq!(tc("folder", "Projects"), "Projects");
    set_current(Lang::EN);
    assert_eq!(date("Wed 21:51"), "Wed 21:51");
}

#[test]
fn a_tour_in_the_pseudo_language_only_draws_known_strings() {
    let known = interface_strings();
    pseudo::set(true);
    crate::tests::tour();
    pseudo::set(false);
    let unknown: Vec<String> = pseudo::seen().into_iter().filter(|s| !known.contains(s)).collect();
    // Dynamic text (names, subjects, notices with values) also goes through `t`; only fixed
    // interface wording must be known. Anything here that is wording needs adding to the scan.
    let wording: Vec<&String> = unknown
        .iter()
        .filter(|s| {
            s.len() < 120
                && !s.contains('@')
                && s.chars().next().is_some_and(char::is_uppercase)
                && s.split(' ').count() > 2
                && !s.contains("Storyboard")
        })
        .collect();
    assert!(wording.len() < 40, "{wording:?}");
}
