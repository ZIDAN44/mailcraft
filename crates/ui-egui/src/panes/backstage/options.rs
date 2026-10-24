//! File › Options: the app's settings, a card per subject.

use super::*;

pub(super) fn options(s: &Session, st: &UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    let t = st.tokens;
    heading(ui, &t, "Options");
    ui.set_max_width(760.0);
    // A card per subject, one setting per row (the helpers Info and Open & Export use too).
    let group = |ui: &mut Ui, title: &str, about: Option<&str>, body: &mut dyn FnMut(&mut Ui)| card(ui, &t, title, about, |ui| body(ui));
    let setting = |ui: &mut Ui, first: bool, label: &str, about: Option<&str>, control: &mut dyn FnMut(&mut Ui)| {
        setting_row(ui, &t, first, name(&t, label), about.map(|a| note(&t, a)), |ui| control(ui));
    };
    let switch = |ui: &mut Ui, on: bool, label: &str, id: &'static str, acts: &mut Vec<Action>| {
        let mut v = on;
        if widgets::switch(ui, &t, &mut v, label).changed() {
            acts.push(Action::Run(id, json!({"on": v})));
        }
    };

    group(ui, tl!("General"), None, &mut |ui| {
        setting(ui, true, tl!("Theme"), None, &mut |ui| {
            let options = [tl!("System"), tl!("Light"), tl!("Dark")];
            let values = [None, Some(false), Some(true)];
            let now = values.iter().position(|v| *v == st.dark).unwrap_or(0);
            if let Some(i) = widgets::segmented(ui, &t, "theme", &options, now)
                && let Some(v) = values.get(i)
            {
                acts.push(Action::SetDark(*v));
            }
        });
        setting(ui, false, tl!("Language"), None, &mut |ui| {
            let language = st.language.as_str();
            let current = crate::i18n::Lang::from_code(language).map_or_else(|| tl!("Same as the system").to_string(), |l| l.name().to_string());
            egui::ComboBox::from_id_salt("language").width(200.0).selected_text(current).show_ui(ui, |ui| {
                if ui.selectable_label(language == crate::i18n::AUTO, tl!("Same as the system")).clicked() {
                    acts.push(Action::SetLanguage(crate::i18n::AUTO.into()));
                }
                for l in crate::i18n::Lang::all() {
                    if ui.selectable_label(language == l.code(), l.name()).clicked() {
                        acts.push(Action::SetLanguage(l.code().into()));
                    }
                }
            });
        });
    });

    // Where the data files are (one per account, as Outlook's .pst files); movable as those are.
    if let Some((dir, movable)) = &st.data_folder {
        group(
            ui,
            tl!("Data files"),
            Some(tl!(
                "Each account's mail, contacts and calendar are kept in a MailCraft data file (.mailcraft) in this folder, as Outlook keeps each account in a .pst in Outlook Files. An Outlook data file you open gets a .mailcraft file of its own here; Outlook's file is left as it was. To have a copy as a .pst, use Save as an Outlook data file under Info."
            )),
            &mut |ui| {
                let path = dir.display().to_string();
                setting(ui, true, tl!("Folder"), Some(&path), &mut |ui| {
                    if *movable && ui.button(tl!("Move…")).clicked() {
                        acts.push(Action::MoveDataFolder);
                    }
                    if ui.button(tl!("Open folder")).clicked() {
                        acts.push(Action::OpenFolder(dir.clone()));
                    }
                });
                // The account list, settings and log are in the app folder, not with the data
                // files (issue #4 looked for them in Documents).
                if let Some(app) = s.store.as_ref().map(|st| st.root().to_path_buf()).filter(|r| r != dir) {
                    let path = app.display().to_string();
                    setting(ui, false, tl!("Accounts and settings"), Some(&path), &mut |ui| {
                        if ui
                            .button(tl!("Open folder"))
                            .on_hover_text(tl!("The account list (no passwords), the window's settings and the log"))
                            .clicked()
                        {
                            acts.push(Action::OpenFolder(app.clone()));
                        }
                    });
                }
                if !*movable && st.pending_move.is_none() {
                    let why = if s.portable {
                        tl!("Portable: everything is kept in MailCraftData beside MailCraft, and passwords only until it closes.")
                    } else {
                        tl!("Chosen when MailCraft was started (--data or MAILCRAFT_DATA), so it can't be moved here.")
                    };
                    ui.label(RichText::new(why).color(t.faint).small());
                }
                if let Some(to) = &st.pending_move {
                    ui.add_space(6.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            RichText::new(tf!("Moves to {folder} the next time MailCraft starts.", folder = to.display().to_string()))
                                .color(t.warn)
                                .small(),
                        );
                        if ui.small_button(tl!("Cancel")).clicked() {
                            acts.push(Action::CancelDataMove);
                        }
                    });
                }
                ui.add_space(8.0);
            },
        );
    }

    group(ui, tl!("Reading"), None, &mut |ui| {
        setting(ui, true, tl!("Reading pane"), None, &mut |ui| {
            let values = [(ReadingPane::Right, "right"), (ReadingPane::Bottom, "bottom"), (ReadingPane::Off, "off")];
            let now = values.iter().position(|(v, _)| *v == s.prefs.reading_pane).unwrap_or(0);
            if let Some(i) = widgets::segmented(ui, &t, "reading-pane", &[tl!("Right"), tl!("Bottom"), tl!("Off")], now)
                && let Some((_, code)) = values.get(i)
            {
                acts.push(Action::Run("view.readingPane", json!({"value": code})));
            }
        });
        setting(ui, false, tl!("Group messages by conversation"), None, &mut |ui| {
            switch(ui, s.prefs.conversations, tl!("Group messages by conversation"), "view.conversations", acts);
        });
        setting(ui, false, tl!("Show a line of each message in the list"), None, &mut |ui| {
            switch(ui, s.prefs.show_preview, tl!("Show a line of each message in the list"), "view.preview", acts);
        });
        setting(ui, false, tl!("Compact list"), None, &mut |ui| {
            switch(ui, s.prefs.compact, tl!("Compact list"), "view.compact", acts);
        });
        // When the reading pane marks mail read, as Outlook's Reading Pane options: when shown
        // (after a wait), once another message is chosen, or not at all (only a message opened
        // in its own window is then).
        let on = s.prefs.mark_read_on_open;
        setting(
            ui,
            false,
            tl!("Mark messages as read in the reading pane"),
            Some(tl!("A message opened in its own window (double-click) is always marked as read.")),
            &mut |ui| {
                let mut v = on;
                if widgets::switch(ui, &t, &mut v, tl!("Mark messages as read in the reading pane")).changed() {
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
        setting(ui, false, tl!("Write new messages in a window of their own"), None, &mut |ui| {
            switch(ui, s.prefs.new_mail_window, tl!("Write new messages in a window of their own"), "view.newMailWindow", acts);
        });
        setting(ui, false, tl!("Show HTML mail formatted (otherwise as plain text)"), None, &mut |ui| {
            switch(ui, s.prefs.show_html, tl!("Show HTML mail formatted (otherwise as plain text)"), "view.html", acts);
        });
    });

    // How much mail is kept on this computer, as Outlook's "Mail to keep offline".
    group(
        ui,
        tl!("Mail kept on this computer"),
        Some(tl!("New mail always comes in. Older mail past these stays on the server; the rest of each message is downloaded when it's opened.")),
        &mut |ui| {
            setting(ui, true, tl!("Of each folder, keep"), None, &mut |ui| {
                let values = [1000u32, 5000, 20000, 0];
                let now = values.iter().position(|v| *v == s.prefs.keep_messages).unwrap_or(0);
                if let Some(i) = widgets::segmented(ui, &t, "keep", &["1,000", "5,000", "20,000", tl!("All")], now)
                    && let Some(n) = values.get(i)
                {
                    acts.push(Action::Run("account.keepMail", json!({"messages": n})));
                }
            });
            setting(ui, false, tl!("Download in full for reading offline"), None, &mut |ui| {
                let values = [40u32, 200, 1000];
                let now = values.iter().position(|v| *v == s.prefs.keep_offline).unwrap_or(0);
                if let Some(i) = widgets::segmented(ui, &t, "offline", &["40", "200", "1,000"], now)
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
                setting(ui, false, &label, (i == 0).then(crate::dialogs::online_only_about), &mut |ui| {
                    let mut on = c.keeps_online();
                    if widgets::switch(ui, &t, &mut on, &label).changed() {
                        acts.push(Action::Run("account.onlineOnly", json!({"account": c.id, "on": on})));
                    }
                });
            }
        },
    );

    group(ui, tl!("Inbox and notifications"), None, &mut |ui| {
        setting(ui, true, tl!("Show a desktop notification when new mail arrives"), None, &mut |ui| {
            switch(ui, s.prefs.notify, tl!("Show a desktop notification when new mail arrives"), "view.notifications", acts);
        });
        setting(ui, false, tl!("Focused Inbox: put newsletters and automatic mail in Other"), None, &mut |ui| {
            switch(ui, s.prefs.focused_inbox, tl!("Focused Inbox: put newsletters and automatic mail in Other"), "view.focused", acts);
        });
        setting(ui, false, tl!("Ask before deleting for good"), None, &mut |ui| {
            switch(ui, s.prefs.confirm_permanent_delete, tl!("Ask before deleting for good"), "view.confirmDelete", acts);
        });
        setting(
            ui,
            false,
            tl!("Play animations"),
            Some(tl!("Hovers, selections and panes ease into place; off, everything changes at once")),
            &mut |ui| {
                switch(ui, s.prefs.animations, tl!("Play animations"), "view.animations", acts);
            },
        );
    });

    // Junk Email: the blocked and safe senders, each list with a box to add to it.
    for (list, title, about) in [
        ("blocked", "Blocked senders", "Mail from these addresses and domains goes to Junk Email as it arrives."),
        ("safe", "Safe senders", "Mail from these addresses and domains is never treated as junk, and its pictures load."),
    ] {
        let entries = if list == "blocked" { &s.prefs.blocked_senders } else { &s.prefs.safe_senders };
        group(ui, tl!(title), Some(tl!(about)), &mut |ui| {
            let id = egui::Id::unique(("junk-add", list));
            let mut text: String = ui.data_mut(|d| d.get_temp::<String>(id)).unwrap_or_default();
            let mut add = false;
            ui.horizontal(|ui| {
                let r = ui.add(egui::TextEdit::singleline(&mut text).hint_text(tl!("name@example.com or @example.com")).desired_width(320.0));
                add |= r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                add |= ui.add_enabled(!text.trim().is_empty(), egui::Button::new(tl!("Add"))).clicked();
            });
            if add && !text.trim().is_empty() {
                acts.push(Action::Run("junk.edit", json!({"list": list, "entry": text.trim()})));
                // One that can't be added stays to be corrected (the reason shows as a notice).
                if mailcraft_engine::junk_entry(&text).is_some() {
                    text.clear();
                }
            }
            ui.data_mut(|d| d.insert_temp(id, text));
            if entries.is_empty() {
                ui.add_space(4.0);
                ui.label(RichText::new(tl!("No senders yet.")).color(t.faint).small());
                ui.add_space(8.0);
            }
            // A long list is shown in part: the first ones, and how many more.
            for (i, who) in entries.iter().take(200).enumerate() {
                setting(ui, i == 0, who, None, &mut |ui| {
                    if ui.button(tl!("Remove")).clicked() {
                        acts.push(Action::Run("junk.edit", json!({"list": list, "entry": who, "on": false})));
                    }
                });
            }
            if entries.len() > 200 {
                ui.label(RichText::new(tf!("And {n} more", n = entries.len() - 200)).color(t.faint).small());
            }
        });
    }

    group(ui, tl!("Safety"), None, &mut |ui| {
        setting(
            ui,
            true,
            tl!("Send where photos were taken"),
            Some(tl!("Off, the place a phone writes into a photo is taken out of JPEG and PNG files as they're sent")),
            &mut |ui| {
                switch(ui, s.prefs.keep_photo_location, tl!("Send where photos were taken"), "view.photoLocation", acts);
            },
        );
        setting(
            ui,
            false,
            tl!("Open links without their tracking"),
            Some(tl!("A link opens where it really goes: out of redirects that carry the real address, without the tags that say who clicked")),
            &mut |ui| {
                switch(ui, s.prefs.clean_links, tl!("Open links without their tracking"), "view.cleanLinks", acts);
            },
        );
    });

    group(ui, tl!("Privacy"), Some(tl!("Pictures from the web load only when you ask. These senders' pictures always load:")), &mut |ui| {
        if s.prefs.trusted_senders.is_empty() {
            ui.add_space(4.0);
            ui.label(RichText::new(tl!("No senders yet.")).color(t.faint).small());
            ui.add_space(8.0);
        }
        for (i, who) in s.prefs.trusted_senders.iter().enumerate() {
            setting(ui, i == 0, who, None, &mut |ui| {
                if ui.button(tl!("Remove")).clicked() {
                    acts.push(Action::Run("view.trustSender", json!({"address": who, "on": false})));
                }
            });
        }
    });
}
