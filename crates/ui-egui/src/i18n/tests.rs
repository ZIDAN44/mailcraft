use std::collections::BTreeSet;

use super::*;

/// The UI crate's own Rust files, read from disk.
fn ui_sources() -> Vec<String> {
    fn walk(dir: &std::path::Path, out: &mut Vec<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        let mut paths: Vec<std::path::PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
        paths.sort();
        for p in paths {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if p.is_dir() {
                if name != "i18n" && name != "tests" {
                    walk(&p, out);
                }
            } else if name.ends_with(".rs") && name != "tests.rs" {
                out.push(std::fs::read_to_string(&p).unwrap_or_default());
            }
        }
    }
    let mut out = Vec::new();
    walk(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut out);
    assert!(out.len() > 20, "the UI's sources weren't found");
    out
}

/// Every interface string: literals in the UI code (`tl!`, `tf!` and the helpers that translate
/// their label), the names the model hands up, and whatever a tour of the interface looks up.
pub(crate) fn interface_strings() -> BTreeSet<String> {
    // Every UI source file, wherever it sits (a file split into a folder stays covered); tests
    // and the catalogs' own code left out.
    let sources = ui_sources();
    // (prefix, which string literals after it are interface text)
    let prefixes: [(&str, &[usize]); 21] = [
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
        // Labels in tables of buttons drawn in a loop: `(Icon::Send, "Sent To", "to:")`.
        ("(Icon::", &[0]),
    ];
    let mut out = BTreeSet::new();
    for src in sources.iter().map(String::as_str) {
        for (p, which) in prefixes {
            let mut rest = src;
            while let Some(i) = rest.find(p) {
                rest = &rest[i + p.len()..];
                // String literals up to the end of the line (the next line, when rustfmt put the
                // text there).
                let mut line = &rest[..rest.find('\n').unwrap_or(rest.len())];
                if line.trim().is_empty()
                    && let Some(next) = rest.get(line.len() + 1..)
                {
                    line = &next[..next.find('\n').unwrap_or(next.len())];
                }
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
    for f in mailcraft_engine::ListFilter::EVERY {
        out.insert(f.label().to_string());
    }
    for p in SnoozeUntil::ALL {
        out.insert(p.label().to_string());
    }
    for a in mailcraft_engine::Arrange::ALL {
        out.insert(a.label().to_string());
    }
    out.extend(mailcraft_engine::ARRANGE_HEADINGS.map(str::to_string));
    // The message editor's ribbon and dialogs; labels chosen at run time.
    for s in [
        "Ignore the conversation",
        "Minimize the folder pane",
        "Save As",
        "Save Attachments",
        "Exit",
        "Show the folder pane",
        "Stop ignoring the conversation",
        "Bold (Ctrl+B)",
        "Italic (Ctrl+I)",
        "Underline (Ctrl+U)",
        "Strikethrough",
        "Text highlight color",
        "Font color",
        "No color",
        "Automatic",
        "Bullets (Ctrl+Shift+L)",
        "Numbering",
        "Decrease indent (Ctrl+Shift+T)",
        "Increase indent (Ctrl+T)",
        "Align left (Ctrl+L)",
        "Center (Ctrl+E)",
        "Align right (Ctrl+R)",
        "Clear all formatting (Ctrl+Space)",
        "Address Book",
        "Attach File",
        "Link",
        "Pictures",
        "To ->",
        "Cc ->",
        "Bcc ->",
        "All addresses",
        "Contacts",
        "Yes",
        "No",
    ] {
        out.insert(s.to_string());
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
        "Public Folders",
        "Blue",
        "Green",
        "Pink",
        "Yellow",
        "White",
        "Cards",
        "Phone list",
        "Full name",
        "Company",
        "Job title",
        "Email",
        "Phone",
        "Cut",
        "Copy",
        "Paste",
        "Select all",
        "Link…",
        "Edit link…",
        "Open link",
        "Remove link",
        "Clear formatting",
        "Edit link",
        "Insert link",
        "In 1 hour",
        "Tomorrow morning",
        "Monday morning",
        "Today",
        "Tomorrow",
        "This week",
        "Next week",
        "Always move to Focused",
        "Always move to Other",
        "Messages in this conversation",
        "Messages from sender",
        "Delete all",
        "More actions",
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
        "You accepted.",
        "You said maybe.",
        "You declined.",
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

/// The word written just before byte `at` of a sentence (spaces and an opening « skipped),
/// lower case.
fn word_before(sentence: &str, at: usize) -> String {
    let head = sentence.get(..at).unwrap_or("").trim_end_matches(|c: char| c.is_whitespace() || c == '«');
    let start = head.rfind(|c: char| !c.is_alphabetic()).map_or(0, |i| i + head.get(i..).and_then(|r| r.chars().next()).map_or(1, char::len_utf8));
    head.get(start..).unwrap_or("").to_lowercase()
}

/// In French « de », « le », « que » and a few more lose their vowel before one (« d’Ana »), so
/// a value filled in when the app runs can't come straight after them: the catalog says
/// « du dossier {folder} », « du compte {account} » instead. Values that are dates begin with the
/// day's name and may follow « le ».
#[test]
fn french_keeps_filled_in_values_away_from_words_that_elide() {
    const ELIDING: [&str; 10] = ["ce", "de", "je", "la", "le", "me", "ne", "que", "se", "te"];
    let Some(fr) = LANGUAGES.iter().find(|l| l.code == "fr") else { panic!("no French catalog") };
    let (entries, _) = parse(fr.source);
    let mut wrong: Vec<String> = Vec::new();
    for (source, french) in &entries {
        for (at, _) in french.match_indices('{') {
            let value: String = french.get(at + 1..).unwrap_or("").chars().take_while(|c| *c != '}').collect();
            let dated = matches!(value.as_str(), "date" | "when");
            if !dated && ELIDING.contains(&word_before(french, at).as_str()) {
                wrong.push(format!("{source} | {french}"));
            }
        }
    }
    assert!(wrong.is_empty(), "a value straight after a word French elides: {wrong:#?}");
    assert_eq!(word_before("Mise à jour de « {x}", 19), "de");
    assert_eq!(word_before("{x}", 0), "");
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

/// Values filled into a template where right-to-left and left-to-right text meet are isolated,
/// so each keeps its own order: a server's error in an Arabic sentence was drawn scrambled
/// ("bob@x.example>: … rejected< 5.1.1 550").
#[test]
fn values_in_right_to_left_text_keep_their_order() {
    let error = "550 5.1.1 <bob@x.example>: Recipient address rejected";
    let arabic = fmt("لم تُرسل: {error}", &[("error", error)]);
    assert_eq!(arabic, format!("لم تُرسل: \u{2068}{error}\u{2069}"));
    assert_eq!(fmt("Not sent: {error}", &[("error", error)]), format!("Not sent: {error}"), "left to right only: as it was");
    assert_eq!(fmt("Organizer: {name}", &[("name", "דנה")]), "Organizer: \u{2068}דנה\u{2069}");
    assert_eq!(fmt("לא נשלח: {error}", &[("error", "")]), "לא נשלח: ", "nothing to isolate");
    // As egui draws it: the error reads as written, left to right, beside the Arabic.
    let ctx = egui::Context::default();
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
        let job = egui::text::LayoutJob::simple(arabic.clone(), egui::FontId::proportional(14.0), egui::Color32::WHITE, f32::INFINITY);
        let g = ui.fonts_mut(|f| f.layout_job(job));
        let mut glyphs: Vec<(f32, char)> = g.rows.iter().flat_map(|r| r.row.glyphs.iter().map(|x| (x.pos.x, x.chr))).collect();
        glyphs.sort_by(|a, b| a.0.total_cmp(&b.0));
        let seen: String = glyphs.iter().map(|(_, c)| *c).filter(|c| !matches!(c, '\u{2068}' | '\u{2069}')).collect();
        assert!(seen.contains(error), "{seen}");
    });
    out.textures_delta.clear();
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
    // Short labels too (the filter above passes "To:" and "Sent To", looked up by a variable):
    // what was drawn and is written as a literal in the interface's code is wording, and must
    // be in the catalogs.
    let sources = ui_sources().concat();
    let catalog = Lang::from_code("de").unwrap_or(Lang::EN).0.catalog();
    let literal: Vec<&String> =
        unknown.iter().filter(|s| !s.is_empty() && !catalog.contains_key(*s) && sources.contains(&format!("\"{s}\""))).collect();
    assert!(literal.is_empty(), "drawn, written in the code, but not in the catalogs: {literal:?}");
}
