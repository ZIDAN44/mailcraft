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
                egui::ComboBox::from_id_salt(("check-every", c.id.as_str()))
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
            });
        }
    });
}
