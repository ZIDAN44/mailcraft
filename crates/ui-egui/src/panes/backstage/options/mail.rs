//! Options' cards about mail: Reading (the panes, the list, marking read, where messages are
//! written), Mail kept on this computer, and Checking for new mail.

use super::*;

/// The reading pane and the list, marking mail read, the windows messages are written in, and
/// plain text.
pub(super) fn reading(s: &Session, t: &Tokens, ui: &mut Ui, acts: &mut Vec<Action>) {
    card(ui, t, tl!("Reading"), None, |ui| {
        setting(ui, t, true, tl!("Reading pane"), None, |ui| {
            let values = [(ReadingPane::Right, "right"), (ReadingPane::Bottom, "bottom"), (ReadingPane::Off, "off")];
            let now = values.iter().position(|(v, _)| *v == s.prefs.reading_pane).unwrap_or(0);
            if let Some(i) = widgets::segmented(ui, t, "reading-pane", &[tl!("Right"), tl!("Bottom"), tl!("Off")], now)
                && let Some((_, code)) = values.get(i)
            {
                acts.push(Action::Run("view.readingPane", json!({"value": code})));
            }
        });
        start_in(s, t, ui, acts);
        let switches = [
            (tl!("Group messages by conversation"), None, s.prefs.conversations, "view.conversations"),
            (tl!("Show a line of each message in the list"), None, s.prefs.show_preview, "view.preview"),
            (tl!("Compact list"), None, s.prefs.compact, "view.compact"),
            (tl!("Single-line list"), Some(tl!("One line a message: the sender, the subject and the time")), s.prefs.single_line, "view.singleLine"),
            (
                tl!("Classic ribbon"),
                Some(tl!("Big buttons with their names under them, in named groups")),
                s.prefs.classic_ribbon,
                "view.classicRibbon",
            ),
        ];
        for (label, about, on, cmd) in switches {
            switch_row(ui, t, false, Switch { label, about, on, cmd }, acts);
        }
        // When the reading pane marks mail read, as Outlook's Reading Pane options: when shown
        // (after a wait), once another message is chosen, or not at all (only a message opened
        // in its own window is then).
        let on = s.prefs.mark_read_on_open;
        setting(
            ui,
            t,
            false,
            tl!("Mark messages as read in the reading pane"),
            Some(tl!("A message opened in its own window (double-click) is always marked as read.")),
            |ui| {
                let mut v = on;
                if widgets::switch(ui, t, &mut v, tl!("Mark messages as read in the reading pane")).changed() {
                    acts.push(Action::Run("view.readingOptions", json!({"markRead": v})));
                }
            },
        );
        if on {
            ui.indent("read-when", |ui| {
                let shown = s.prefs.read_in_pane == mailcraft_engine::ReadInPane::Shown;
                ui.horizontal(|ui| {
                    if ui.radio(shown, tl!("When it's shown, after")).clicked() {
                        acts.push(Action::Run("view.readingOptions", json!({"when": "shown"})));
                    }
                    let mut secs = s.prefs.read_after_secs;
                    if ui.add_enabled(shown, egui::DragValue::new(&mut secs).range(0..=999).speed(0.2)).changed() {
                        acts.push(Action::Run("view.readingOptions", json!({"after": secs})));
                    }
                    ui.label(tl!("seconds"));
                });
                if ui.radio(!shown, tl!("When I choose another message")).clicked() {
                    acts.push(Action::Run("view.readingOptions", json!({"when": "left"})));
                }
            });
            ui.add_space(8.0);
        }
        let windows = [
            (tl!("Write new messages in a window of their own"), s.prefs.new_mail_window, "view.newMailWindow"),
            (tl!("Write replies and forwards in a window of their own"), s.prefs.reply_window, "view.replyWindow"),
        ];
        for (label, on, cmd) in windows {
            switch_row(ui, t, false, Switch { label, about: None, on, cmd }, acts);
        }
        // Issue #174: in the interface's language, which says when it has no dictionary here.
        let lang = crate::rich_edit::spelling::language();
        let missing = (!crate::rich_edit::spelling::has_dictionary(lang))
            .then(|| tf!("There's no spelling dictionary for {language} on this computer.", language = crate::i18n::current().name()));
        let about = missing.as_deref().unwrap_or(tl!("Misspelled words are underlined in red. Right-click one for suggestions."));
        switch_row(
            ui,
            t,
            false,
            Switch { label: tl!("Check spelling as you type"), about: Some(about), on: s.prefs.spelling, cmd: "options.spelling" },
            acts,
        );
        compose_font(s, t, ui, acts);
        // Issue #115: Ctrl+Enter sends at once only when that's chosen.
        setting(ui, t, false, tl!("Ctrl+Enter in a message"), None, |ui| {
            use mailcraft_engine::CtrlEnter;
            let ways = [CtrlEnter::Ask, CtrlEnter::Send, CtrlEnter::Nothing];
            let now = ways.iter().position(|w| *w == s.prefs.ctrl_enter).unwrap_or(0);
            if let Some(i) = widgets::segmented(ui, t, "ctrl-enter", &[tl!("Ask first"), tl!("Send"), tl!("Do nothing")], now)
                && let Some(w) = ways.get(i)
            {
                acts.push(Action::Run("options.ctrlEnter", json!({"does": w})));
            }
        });
        // Issue #135: a message sent waits this long in the Outbox, so it can be taken back.
        setting(ui, t, false, tl!("Undo Send"), Some(tl!("A message you send waits this long in the Outbox, with Undo, before it goes.")), |ui| {
            const WAITS: [u32; 5] = [0, 5, 10, 20, 30];
            let labels: Vec<String> = WAITS.iter().map(|n| if *n == 0 { tl!("Off").to_string() } else { tf!("{n} s", n = n) }).collect();
            let shown: Vec<&str> = labels.iter().map(String::as_str).collect();
            let now = WAITS.iter().position(|n| *n == s.prefs.undo_send_secs).unwrap_or(0);
            if let Some(i) = widgets::segmented(ui, t, "undo-send", &shown, now)
                && let Some(n) = WAITS.get(i)
            {
                acts.push(Action::Run("options.undoSend", json!({"seconds": n})));
            }
        });
        // Every message as plain text, as Outlook's own setting for all mail: here, out of the
        // way, and View › Plain text; one message's More actions (⋯) and Show formatted switch
        // that message alone.
        setting(
            ui,
            t,
            false,
            tl!("Read all mail as plain text"),
            Some(tl!("Formatting, colours and pictures are left out. Show formatted, above a message, shows that one as it was sent.")),
            |ui| {
                let mut v = !s.prefs.show_html;
                if widgets::switch(ui, t, &mut v, tl!("Read all mail as plain text")).changed() {
                    acts.push(Action::Run("view.html", json!({"on": !v})));
                }
            },
        );
    });
}

/// How much mail is kept on this computer, as Outlook's "Mail to keep offline".
pub(super) fn kept(s: &Session, t: &Tokens, ui: &mut Ui, acts: &mut Vec<Action>) {
    card(
        ui,
        t,
        tl!("Mail kept on this computer"),
        Some(tl!("New mail always comes in. Older mail past these stays on the server; the rest of each message is downloaded when it's opened.")),
        |ui| {
            setting(ui, t, true, tl!("Of each folder, keep"), None, |ui| {
                let values = [1000u32, 5000, 20000, 0];
                let now = values.iter().position(|v| *v == s.prefs.keep_messages).unwrap_or(0);
                if let Some(i) = widgets::segmented(ui, t, "keep", &["1,000", "5,000", "20,000", tl!("All")], now)
                    && let Some(n) = values.get(i)
                {
                    acts.push(Action::Run("account.keepMail", json!({"messages": n})));
                }
            });
            setting(ui, t, false, tl!("Download in full for reading offline"), None, |ui| {
                let values = [40u32, 200, 1000];
                let now = values.iter().position(|v| *v == s.prefs.keep_offline).unwrap_or(0);
                if let Some(i) = widgets::segmented(ui, t, "offline", &["40", "200", "1,000"], now)
                    && let Some(n) = values.get(i)
                {
                    acts.push(Action::Run("account.keepMail", json!({"offline": n})));
                }
            });
            // Issue #4: an account kept online only has none of its mail here.
            let servers: Vec<&mailcraft_engine::model::AccountConfig> =
                s.accounts.iter().map(|a| &a.config).filter(|c| c.can_be_online_only()).collect();
            for (i, c) in servers.iter().enumerate() {
                let label = tf!("Online only: {email}", email = c.email);
                setting(ui, t, false, &label, (i == 0).then(crate::dialogs::online_only_about), |ui| {
                    let mut on = c.keeps_online();
                    if widgets::switch(ui, t, &mut on, &label).changed() {
                        acts.push(Action::Run("account.onlineOnly", json!({"account": c.id, "on": on})));
                    }
                });
            }
        },
    );
}

/// How often each account looks for new mail (issue #58: each account its own). Not in the
/// browser, which waits on its server's own news.
pub(super) fn checking(s: &Session, t: &Tokens, ui: &mut Ui, acts: &mut Vec<Action>) {
    let checked: Vec<&mailcraft_engine::model::AccountConfig> = s.accounts.iter().map(|a| &a.config).filter(|c| !c.is_local()).collect();
    if s.jmap_only || checked.is_empty() {
        return;
    }
    let about = tl!(
        "Automatically gets new mail as soon as the server says there's some, where a server can, and otherwise looks every two minutes. With a set time, an account looks only then. Mail you send goes at once, and Send / Receive looks right away."
    );
    card(ui, t, tl!("Checking for new mail"), Some(about), |ui| {
        for (i, c) in checked.iter().enumerate() {
            setting(ui, t, i == 0, &c.email, None, |ui| {
                let now = c.check_every.unwrap_or(0);
                let word = |m: u32| if m == 0 { tl!("Automatically").to_string() } else { tf!("Every {n} minutes", n = m) };
                // Only when asked (issue #104): Send / Receive on the account.
                let shown = if c.check_by_hand { tl!("Only when I ask").to_string() } else { word(now) };
                let r = egui::ComboBox::from_id_salt(("check-every", c.id.as_str()))
                    .width(170.0)
                    .selected_text(shown)
                    .show_ui(ui, |ui| {
                        for m in [0u32, 5, 10, 15, 30, 60] {
                            if ui.selectable_label(m == now && !c.check_by_hand, word(m)).clicked() && (m != now || c.check_by_hand) {
                                acts.push(Action::Run("account.checkEvery", json!({"account": c.id, "minutes": m})));
                            }
                        }
                        if ui
                            .selectable_label(c.check_by_hand, tl!("Only when I ask"))
                            .on_hover_text(tl!("Send / Receive on the account looks; Send / Receive for all leaves it out"))
                            .clicked()
                            && !c.check_by_hand
                        {
                            acts.push(Action::Run("account.checkEvery", json!({"account": c.id, "byHand": true})));
                        }
                    })
                    .response
                    // The row's name, the account's address, under the card's Checking for new mail.
                    .accessible_name(c.email.as_str());
                // Minutes, then by hand (None).
                let choices = [Some(0u32), Some(5), Some(10), Some(15), Some(30), Some(60), None];
                let was = if c.check_by_hand { None } else { Some(now) };
                let mut at = was;
                crate::drop_down::keys(ui, &r, &mut at, &choices);
                if at != was {
                    let p = match at {
                        Some(m) => json!({"account": c.id, "minutes": m}),
                        None => json!({"account": c.id, "byHand": true}),
                    };
                    acts.push(Action::Run("account.checkEvery", p));
                }
            });
        }
    });
}

/// What opens at the start (issue #172), as Outlook's "Start Outlook in this folder": the first
/// account's Inbox, All Inboxes, the folder open last, or any account's folder.
fn start_in(s: &Session, t: &Tokens, ui: &mut Ui, acts: &mut Vec<Action>) {
    use mailcraft_engine::StartIn;
    let folder_name = |account: &str, folder: &str| -> Option<String> {
        let a = s.account(account)?;
        let f = a.folders.iter().find(|f| f.path == folder)?;
        let who = if a.config.email.is_empty() { a.config.name.as_str() } else { a.config.email.as_str() };
        Some(format!("{who} › {}", crate::i18n::folder(f)))
    };
    let shown = match &s.prefs.start_in {
        StartIn::FirstInbox => tl!("The first account's Inbox").to_string(),
        StartIn::AllInboxes => tl!("All Inboxes").to_string(),
        StartIn::Last => tl!("The folder open last").to_string(),
        StartIn::Folder { account, folder } => folder_name(account, folder).unwrap_or_else(|| tl!("The first account's Inbox").to_string()),
    };
    setting(ui, t, false, tl!("Start in"), Some(tl!("What opens when MailCraft starts")), |ui| {
        let r = egui::ComboBox::from_id_salt("start-in")
            .width(260.0)
            .height(400.0)
            .selected_text(shown)
            .show_ui(ui, |ui| {
                for (open, label) in [("firstInbox", "The first account's Inbox"), ("allInboxes", "All Inboxes"), ("last", "The folder open last")] {
                    let on = matches!(
                        (&s.prefs.start_in, open),
                        (StartIn::FirstInbox, "firstInbox") | (StartIn::AllInboxes, "allInboxes") | (StartIn::Last, "last")
                    );
                    if ui.selectable_label(on, tl!(label)).clicked() {
                        acts.push(Action::Run("options.startIn", json!({"open": open})));
                    }
                }
                for a in s.accounts.iter().filter(|a| !a.folders.is_empty()) {
                    ui.separator();
                    for f in &a.folders {
                        let on = matches!(&s.prefs.start_in, StartIn::Folder { account, folder } if *account == a.config.id && *folder == f.path);
                        let Some(name) = folder_name(&a.config.id, &f.path) else { continue };
                        if ui.selectable_label(on, name).clicked() {
                            acts.push(Action::Run("options.startIn", json!({"open": "folder", "account": a.config.id, "folder": f.path})));
                        }
                    }
                }
            })
            .response
            // The setting's name is beside it in the row, which doesn't hand it on.
            .accessible_name(tl!("Start in"));
        let mut choices = vec![StartIn::FirstInbox, StartIn::AllInboxes, StartIn::Last];
        for a in s.accounts.iter().filter(|a| !a.folders.is_empty()) {
            choices.extend(
                a.folders
                    .iter()
                    .filter(|f| folder_name(&a.config.id, &f.path).is_some())
                    .map(|f| StartIn::Folder { account: a.config.id.clone(), folder: f.path.clone() }),
            );
        }
        let mut at = s.prefs.start_in.clone();
        crate::drop_down::keys(ui, &r, &mut at, &choices);
        if at != s.prefs.start_in
            && let Ok(p) = serde_json::to_value(&at)
        {
            acts.push(Action::Run("options.startIn", p));
        }
    });
}

/// The font new messages are written in (issue #166), as Outlook's "New mail messages" font:
/// MailCraft's own, or any installed on the computer (none offered in the browser, nor the row).
fn compose_font(s: &Session, t: &Tokens, ui: &mut Ui, acts: &mut Vec<Action>) {
    let families = crate::mail_fonts::families();
    if families.is_empty() {
        return;
    }
    let about = tl!("New messages, replies and forwards are written in it. Text you set a font on keeps its own.");
    setting(ui, t, false, tl!("Font for new messages"), Some(about), |ui| {
        let now = s.prefs.compose_font.clone();
        let r = egui::ComboBox::from_id_salt("compose-font")
            .width(220.0)
            .height(360.0)
            .selected_text(now.clone().unwrap_or_else(|| tl!("Default font").to_string()))
            .truncate()
            .show_ui(ui, |ui| {
                if ui.selectable_label(now.is_none(), tl!("Default font")).clicked() {
                    acts.push(Action::Run("options.composeFont", json!({"font": null})));
                }
                for f in families.iter() {
                    let on = now.as_deref().is_some_and(|n| n.eq_ignore_ascii_case(f));
                    if ui.selectable_label(on, f.as_str()).clicked() {
                        acts.push(Action::Run("options.composeFont", json!({"font": f})));
                    }
                }
            })
            .response
            // The setting's name is beside it in the row, which doesn't hand it on.
            .accessible_name(tl!("Font for new messages"));
        let choices: Vec<Option<String>> = std::iter::once(None).chain(families.iter().cloned().map(Some)).collect();
        let mut at = now.clone();
        crate::drop_down::keys(ui, &r, &mut at, &choices);
        if at != now {
            acts.push(Action::Run("options.composeFont", json!({"font": at})));
        }
    });
}
