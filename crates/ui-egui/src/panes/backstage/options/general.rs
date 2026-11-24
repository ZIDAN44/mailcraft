//! Options' General card (the look, dates, closing, the language) and Data files card.

use super::*;

/// The theme, the accent colour and top bar, dates and the clock, closing the window, the
/// language.
pub(super) fn general(s: &Session, st: &UiState, ui: &mut Ui, acts: &mut Vec<Action>) {
    let t = st.tokens;
    card(ui, &t, tl!("General"), None, |ui| {
        let theme_locked = s.org.theme.as_ref().is_some_and(|g| g.locked);
        setting_held(theme_locked, ui, &t, true, tl!("Theme"), None, |ui| {
            let options = [tl!("System"), tl!("Light"), tl!("Dark")];
            let values = [None, Some(false), Some(true)];
            let now = values.iter().position(|v| *v == st.dark).unwrap_or(0);
            if let Some(i) = widgets::segmented(ui, &t, "theme", &options, now)
                && let Some(v) = values.get(i)
            {
                acts.push(Action::SetDark(*v));
            }
        });
        text_size(s, &t, ui, acts);
        // Issue #109: messages on a light background in the dark theme, as they were written.
        let light = Switch {
            label: tl!("Messages on a light background"),
            about: Some(tl!("In the dark theme, messages keep the light page and colours they were written in")),
            on: s.prefs.light_messages,
            cmd: "view.lightMessages",
        };
        switch_row(s, ui, &t, false, light, acts);
        // Issue #8: an accent colour of one's own (a named one, or any colour), made to read in
        // either theme, and the top bar in it or plain.
        setting_by(s, "view.accent", ui, &t, false, tl!("Accent colour"), Some(tl!("Buttons, links, the selection and the top bar")), |ui| {
            accent_choice(ui, &t, &s.prefs.accent, acts);
        });
        let plain = Switch {
            label: tl!("Plain top bar"),
            about: Some(tl!("The top bar in the panes' colours instead of the accent colour")),
            on: s.prefs.plain_top_bar,
            cmd: "view.plainTopBar",
        };
        switch_row(s, ui, &t, false, plain, acts);
        let ribbon = Switch {
            label: tl!("Hide the ribbon"),
            about: Some(tl!("Its tabs stay on the top bar: click one for its buttons, twice to keep them")),
            on: s.prefs.ribbon_hidden,
            cmd: "view.hideRibbon",
        };
        switch_row(s, ui, &t, false, ribbon, acts);
        // Windows, Linux and the BSDs: the top bar is the window's title bar (issue #22), or the
        // system's is kept.
        if cfg!(any(windows, all(unix, not(target_os = "macos")))) {
            let system = Switch {
                label: tl!("Use the system title bar"),
                about: Some(tl!("Applies when MailCraft starts again")),
                on: s.prefs.system_title_bar,
                cmd: "view.systemTitleBar",
            };
            switch_row(s, ui, &t, false, system, acts);
        }
        // Issue #46: dates as the user writes them (each choice shown as today's date), and the
        // clock, as Windows' region settings offer them to Outlook.
        let style = s.prefs.date_style;
        setting_by(
            s,
            "view.dateFormat",
            ui,
            &t,
            false,
            tl!("Dates"),
            Some(tl!("In the message list, the reading pane, the header of a reply, and the event and task forms")),
            |ui| {
                let (y, m, d) = mailcraft_engine::model::dates::civil_from_days(mailcraft_engine::model::dates::today(&s.clock));
                let r = egui::ComboBox::from_id_salt("date-order")
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
                let mut order = style.order;
                crate::drop_down::keys(ui, &r, &mut order, &mailcraft_engine::model::dates::DateOrder::ALL);
                if order != style.order {
                    acts.push(Action::Run("view.dateFormat", json!({"order": order.code()})));
                }
            },
        );
        setting_by(s, "view.dateFormat", ui, &t, false, tl!("Time"), None, |ui| {
            let options = [tl!("24-hour"), tl!("12-hour")];
            if let Some(i) = widgets::segmented(ui, &t, "clock", &options, usize::from(style.hours12)) {
                acts.push(Action::Run("view.dateFormat", json!({"hours12": i == 1})));
            }
        });
        setting_by(s, "view.dateFormat", ui, &t, false, tl!("Whole date and time in the list"), None, |ui| {
            let mut v = style.full_in_list;
            if widgets::switch(ui, &t, &mut v, tl!("Whole date and time in the list")).changed() {
                acts.push(Action::Run("view.dateFormat", json!({"fullInList": v})));
            }
        });
        // Issue #87: as the system has it, read back after each change (not a setting of MailCraft's).
        if let Some(on) = st.at_sign_in {
            let label = tl!("Start MailCraft when you sign in");
            let about = tl!(
                "It starts as you sign in to this computer and checks for mail. Its window comes up too, unless it's kept running when closed and started put away (below)."
            );
            setting(ui, &t, false, label, Some(about), |ui| {
                let mut v = on;
                if widgets::switch(ui, &t, &mut v, label).changed() {
                    acts.push(Action::StartAtSignIn(v));
                }
            });
        }
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
            switch_row(s, ui, &t, false, keep, acts);
            if s.prefs.close_to_tray {
                // Issue #116: started at sign-in, MailCraft needn't bring its window up.
                let label = match kept {
                    crate::KeptOnClose::Hidden => tl!("Start MailCraft in the notification area"),
                    crate::KeptOnClose::Minimized => tl!("Start MailCraft minimized"),
                };
                let about = tl!(
                    "It checks for mail and announces it with its window put away, as when it's closed. Started to open a link or a file, it opens the window."
                );
                switch_row(s, ui, &t, false, Switch { label, about: Some(about), on: s.prefs.start_in_tray, cmd: "view.startInTray" }, acts);
                // Issue #117: the icon there says how much is unread (the Dock's does on a Mac).
                if !cfg!(target_os = "macos") {
                    let unread = Switch {
                        label: tl!("Show the unread mail on its icon"),
                        about: Some(tl!("How many unread messages the Inboxes have, in the corner of MailCraft's icon in the notification area")),
                        on: s.prefs.tray_unread,
                        cmd: "view.trayUnread",
                    };
                    switch_row(s, ui, &t, false, unread, acts);
                }
            }
        }
        let language_locked = s.org.language.as_ref().is_some_and(|g| g.locked);
        setting_held(language_locked, ui, &t, false, tl!("Language"), None, |ui| {
            let language = st.language.as_str();
            let current = crate::i18n::Lang::from_code(language).map_or_else(|| tl!("Same as the system").to_string(), |l| l.name().to_string());
            let r = egui::ComboBox::from_id_salt("language")
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
            let codes: Vec<&str> = std::iter::once(crate::i18n::AUTO).chain(crate::i18n::Lang::for_picker().into_iter().map(|l| l.code())).collect();
            let mut at = language;
            crate::drop_down::keys(ui, &r, &mut at, &codes);
            if at != language {
                acts.push(Action::SetLanguage(at.into()));
            }
        });
        // Issue #207: MailCraft updating itself, as installed, with releases only, or never.
        let about = tl!("A copy installed from MailCraft's releases looks for a newer one and installs it as it closes");
        setting_by(s, "options.updates", ui, &t, false, tl!("Update MailCraft by itself"), Some(about), |ui| {
            use mailcraft_engine::policy::Updates;
            let options = [tl!("Yes"), tl!("Releases only"), tl!("No")];
            let values = [Updates::On, Updates::Stable, Updates::Off];
            let now = values.iter().position(|v| *v == s.prefs.updates).unwrap_or(0);
            if let Some(i) = widgets::segmented(ui, &t, "updates", &options, now)
                && let Some(v) = values.get(i)
            {
                acts.push(Action::Run("options.updates", json!({"value": v})));
            }
        });
    });
}

/// Where the data files are (one per account, as Outlook's .pst files); movable as those are.
/// The interface's text size (the comment on issue #267): the whole window larger or smaller,
/// as a browser zooms; the keys say the same.
fn text_size(s: &Session, t: &Tokens, ui: &mut Ui, acts: &mut Vec<Action>) {
    use mailcraft_engine::TEXT_SIZES;
    let about = tl!("Everything in the window, larger or smaller. Ctrl and + or − change it too, and Ctrl+0 puts it back.");
    setting_by(s, "view.textSize", ui, t, false, tl!("Text size"), Some(about), |ui| {
        let now = s.prefs.text_size;
        let shown = |p: u16| tf!("{percent}%", percent = p);
        let r = egui::ComboBox::from_id_salt("text-size")
            .width(90.0)
            .selected_text(shown(now))
            .show_ui(ui, |ui| {
                for p in TEXT_SIZES {
                    if ui.selectable_label(now == p, shown(p)).clicked() {
                        acts.push(Action::Run("view.textSize", json!({"percent": p})));
                    }
                }
            })
            .response
            // The setting's name is beside it in the row, which doesn't hand it on.
            .accessible_name(tl!("Text size"));
        let mut at = now;
        crate::drop_down::keys(ui, &r, &mut at, &TEXT_SIZES);
        if at != now {
            acts.push(Action::Run("view.textSize", json!({"percent": at})));
        }
    });
}

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
            // AutoArchive (issue #180), as Outlook's AutoArchive Settings; its file suggested
            // beside the data files, as Outlook's archive.pst is in Outlook Files.
            let every = s.prefs.auto_archive.every_days;
            let about = if every == 0 {
                tl!("Off: old mail stays where it is until you clean it up.").to_string()
            } else {
                tf!("Every {days} days, old mail is archived by each folder's settings.", days = every)
            };
            setting(ui, &t, false, tl!("AutoArchive"), Some(&about), |ui| {
                if ui.button(tl!("AutoArchive settings…")).clicked() {
                    let suggested = dir.join("archive.pst").display().to_string();
                    acts.push(Action::Dialog(Dialog::AutoArchive(Box::new(crate::dialogs::AutoArchiveForm::new(s, &suggested)))));
                }
            });
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
            switch_row(s, ui, &t, false, compact, acts);
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
