//! Options' General card (the look, dates, closing, the language) and Data files card.

use super::*;

/// The theme, the accent colour and top bar, dates and the clock, closing the window, the
/// language.
pub(super) fn general(s: &Session, st: &UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    let t = st.tokens;
    card(ui, &t, tl!("General"), None, |ui| {
        setting(ui, &t, true, tl!("Theme"), None, |ui| {
            let options = [tl!("System"), tl!("Light"), tl!("Dark")];
            let values = [None, Some(false), Some(true)];
            let now = values.iter().position(|v| *v == st.dark).unwrap_or(0);
            if let Some(i) = widgets::segmented(ui, &t, "theme", &options, now)
                && let Some(v) = values.get(i)
            {
                acts.push(Action::SetDark(*v));
            }
        });
        // Issue #109: messages on a light background in the dark theme, as they were written.
        let light = Switch {
            label: tl!("Messages on a light background"),
            about: Some(tl!("In the dark theme, messages keep the light page and colours they were written in")),
            on: s.prefs.light_messages,
            cmd: "view.lightMessages",
        };
        switch_row(ui, &t, false, light, acts);
        // Issue #8: an accent colour of one's own (a named one, or any colour), made to read in
        // either theme, and the top bar in it or plain.
        setting(ui, &t, false, tl!("Accent colour"), Some(tl!("Buttons, links, the selection and the top bar")), |ui| {
            accent_choice(ui, &t, &s.prefs.accent, acts);
        });
        let plain = Switch {
            label: tl!("Plain top bar"),
            about: Some(tl!("The top bar in the panes' colours instead of the accent colour")),
            on: s.prefs.plain_top_bar,
            cmd: "view.plainTopBar",
        };
        switch_row(ui, &t, false, plain, acts);
        let ribbon = Switch {
            label: tl!("Hide the ribbon"),
            about: Some(tl!("Its tabs stay on the top bar: click one for its buttons, twice to keep them")),
            on: s.prefs.ribbon_hidden,
            cmd: "view.hideRibbon",
        };
        switch_row(ui, &t, false, ribbon, acts);
        // Windows: the top bar is the window's title bar (issue #22), or the system's is kept.
        if cfg!(windows) {
            let system = Switch {
                label: tl!("Use the system title bar"),
                about: Some(tl!("Applies when MailCraft starts again")),
                on: s.prefs.system_title_bar,
                cmd: "view.systemTitleBar",
            };
            switch_row(ui, &t, false, system, acts);
        }
        // Issue #46: dates as the user writes them (each choice shown as today's date), and the
        // clock, as Windows' region settings offer them to Outlook.
        let style = s.prefs.date_style;
        setting(ui, &t, false, tl!("Dates"), Some(tl!("In the message list, the reading pane and the header of a reply")), |ui| {
            let (y, m, d) = mailcraft_engine::model::dates::civil_from_days(mailcraft_engine::model::dates::today(&s.clock));
            egui::ComboBox::from_id_salt("date-order")
                .width(160.0)
                .selected_text(style.order.day(y, m, d))
                .show_ui(ui, |ui| {
                    for o in mailcraft_engine::model::dates::DateOrder::ALL {
                        if ui.selectable_label(style.order == o, o.day(y, m, d)).clicked() {
                            acts.push(Action::Run("view.dateFormat", json!({"order": o.code()})));
                        }
                    }
                })
                .response
                // The setting's name is beside it in the row, which doesn't hand it on.
                .accessible_name(tl!("Dates"));
        });
        setting(ui, &t, false, tl!("Time"), None, |ui| {
            let options = [tl!("24-hour"), tl!("12-hour")];
            if let Some(i) = widgets::segmented(ui, &t, "clock", &options, usize::from(style.hours12)) {
                acts.push(Action::Run("view.dateFormat", json!({"hours12": i == 1})));
            }
        });
        setting(ui, &t, false, tl!("Whole date and time in the list"), None, |ui| {
            let mut v = style.full_in_list;
            if widgets::switch(ui, &t, &mut v, tl!("Whole date and time in the list")).changed() {
                acts.push(Action::Run("view.dateFormat", json!({"fullInList": v})));
            }
        });
        if let Some(kept) = st.keeps_running {
            let about = match kept {
                crate::KeptOnClose::Hidden => {
                    tl!(
                        "It stays in the notification area, checks for mail and announces it. Exit on the File page, or in its icon's menu, closes it."
                    )
                }
                crate::KeptOnClose::Minimized => {
                    tl!("Its window is minimized, and MailCraft goes on checking for mail and announcing it. Exit on the File page closes it.")
                }
            };
            let keep = Switch {
                label: tl!("Keep MailCraft running when its window is closed"),
                about: Some(about),
                on: s.prefs.close_to_tray,
                cmd: "view.closeToTray",
            };
            switch_row(ui, &t, false, keep, acts);
            if s.prefs.close_to_tray {
                // Issue #116: started at sign-in, MailCraft needn't bring its window up.
                let label = match kept {
                    crate::KeptOnClose::Hidden => tl!("Start MailCraft in the notification area"),
                    crate::KeptOnClose::Minimized => tl!("Start MailCraft minimized"),
                };
                let about = tl!(
                    "It checks for mail and announces it with its window put away, as when it's closed. Started to open a link or a file, it opens the window."
                );
                switch_row(ui, &t, false, Switch { label, about: Some(about), on: s.prefs.start_in_tray, cmd: "view.startInTray" }, acts);
                // Issue #117: the icon there says how much is unread (the Dock's does on a Mac).
                if !cfg!(target_os = "macos") {
                    let unread = Switch {
                        label: tl!("Show the unread mail on its icon"),
                        about: Some(tl!("How many unread messages the Inboxes have, in the corner of MailCraft's icon in the notification area")),
                        on: s.prefs.tray_unread,
                        cmd: "view.trayUnread",
                    };
                    switch_row(ui, &t, false, unread, acts);
                }
            }
        }
        setting(ui, &t, false, tl!("Language"), None, |ui| {
            let language = st.language.as_str();
            let current = crate::i18n::Lang::from_code(language).map_or_else(|| tl!("Same as the system").to_string(), |l| l.name().to_string());
            egui::ComboBox::from_id_salt("language")
                .width(200.0)
                .selected_text(current)
                .show_ui(ui, |ui| {
                    if ui.selectable_label(language == crate::i18n::AUTO, tl!("Same as the system")).clicked() {
                        acts.push(Action::SetLanguage(crate::i18n::AUTO.into()));
                    }
                    for l in crate::i18n::Lang::for_picker() {
                        if ui.selectable_label(language == l.code(), l.name()).clicked() {
                            acts.push(Action::SetLanguage(l.code().into()));
                        }
                    }
                })
                .response
                .accessible_name(tl!("Language"));
        });
    });
}

/// Where the data files are (one per account, as Outlook's .pst files); movable as those are.
pub(super) fn data_files(s: &Session, st: &UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    let t = st.tokens;
    let Some((dir, movable)) = &st.data_folder else { return };
    card(
        ui,
        &t,
        tl!("Data files"),
        Some(tl!(
            "Each account's mail, contacts and calendar are kept in a MailCraft data file (.mailcraft) in this folder, as Outlook keeps each account in a .pst in Outlook Files. An Outlook data file you open gets a .mailcraft file of its own here; Outlook's file is left as it was. To have a copy as a .pst, use Save as an Outlook data file under Info."
        )),
        |ui| {
            let path = dir.display().to_string();
            setting(ui, &t, true, tl!("Folder"), Some(&path), |ui| {
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
                setting(ui, &t, false, tl!("Accounts and settings"), Some(&path), |ui| {
                    if ui
                        .button(tl!("Open folder"))
                        .on_hover_text(tl!("The account list (no passwords), the window's settings and the log"))
                        .clicked()
                    {
                        acts.push(Action::OpenFolder(app.clone()));
                    }
                });
            }
            // Compacted as MailCraft closes: off unless chosen (it's done when MailCraft is
            // left alone, or by hand, either way).
            let compact = Switch {
                label: tl!("Compact data files on close"),
                about: Some(tl!(
                    "A data file with room to give back is made smaller as MailCraft closes, when that takes half a minute at most. Otherwise it's done while MailCraft is left alone, or with Compact now under Info."
                )),
                on: s.prefs.compact_on_close,
                cmd: "dataFile.compactOnClose",
            };
            switch_row(ui, &t, false, compact, acts);
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
