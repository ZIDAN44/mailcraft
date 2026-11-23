//! Options' cards about who and what to trust: the blocked and safe senders, Safety (photos'
//! places, links, web pictures, certificates, who mail is from) and Privacy (senders whose
//! pictures always load).

use super::*;

/// Junk Email: the blocked and safe senders, each list with a box to add to it.
pub(super) fn senders(s: &Session, t: &Tokens, ui: &mut Ui, acts: &mut Vec<Action>) {
    for (list, title, about) in [
        ("blocked", "Blocked senders", "Mail from these addresses and domains goes to Junk Email as it arrives."),
        ("safe", "Safe senders", "Mail from these addresses and domains is never treated as junk, and its pictures load."),
    ] {
        let entries = if list == "blocked" { &s.prefs.blocked_senders } else { &s.prefs.safe_senders };
        card(ui, t, tl!(title), Some(tl!(about)), |ui| {
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
                setting_by(s, "junk.edit", ui, t, i == 0, who, None, |ui| {
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
}

/// Where photos were taken, links' tracking, web pictures, certificates withdrawn, and who mail
/// is really from (issue #29).
pub(super) fn safety(s: &Session, t: &Tokens, ui: &mut Ui, acts: &mut Vec<Action>) {
    card(ui, t, tl!("Safety"), None, |ui| {
        let switches = [
            (
                tl!("Send where photos were taken"),
                tl!("Off, the place a phone writes into a photo is taken out of JPEG and PNG files as they're sent"),
                s.prefs.keep_photo_location,
                "view.photoLocation",
            ),
            (
                tl!("Open links without their tracking"),
                tl!("A link opens where it really goes: out of redirects that carry the real address, without the tags that say who clicked"),
                s.prefs.clean_links,
                "view.cleanLinks",
            ),
            (
                tl!("Show web pictures unless a message looks suspicious"),
                tl!(
                    "Pictures load as a message opens, when your mail server confirmed who sent it and it isn't in Junk. The server each picture is on can see your IP address and when you opened it. Tracking pictures stay blocked"
                ),
                s.prefs.auto_pictures,
                "view.autoPictures",
            ),
            (
                tl!("Check whether certificates were withdrawn"),
                tl!(
                    "As signed mail is shown, its certificate's authority is asked whether it withdrew it. Its authority then learns whose mail you're reading"
                ),
                s.prefs.check_revocation,
                "view.checkRevocation",
            ),
        ];
        for (i, (label, about, on, cmd)) in switches.into_iter().enumerate() {
            switch_row(s, ui, t, i == 0, Switch { label, about: Some(about), on, cmd }, acts);
        }
        // Issue #29: who mail is really from.
        setting_by(
            s,
            "view.senderAddresses",
            ui,
            t,
            false,
            tl!("Sender addresses in the message list"),
            Some(tl!("The address a message comes from, after the sender's name: a familiar name can come from anyone's address")),
            |ui| {
                let options = [tl!("Always"), tl!("People you don't know"), tl!("Never")];
                let values = {
                    use mailcraft_engine::SenderAddresses as A;
                    [A::Always, A::Strangers, A::Never]
                };
                let now = values.iter().position(|v| *v == s.prefs.sender_addresses).unwrap_or(1);
                if let Some(i) = widgets::segmented(ui, t, "sender-addresses", &options, now)
                    && let Some(v) = values.get(i)
                {
                    acts.push(Action::Run("view.senderAddresses", json!({"show": v.code()})));
                }
            },
        );
        let external = Switch {
            label: tl!("Tag mail from outside your organisation"),
            about: Some(tl!("Messages from other domains are tagged External, in the list and above the message")),
            on: s.prefs.external_tag,
            cmd: "view.externalTag",
        };
        switch_row(s, ui, t, false, external, acts);
        setting_by(
            s,
            "view.externalTag",
            ui,
            t,
            false,
            tl!("Your organisation's domains"),
            Some(tl!("Separated by commas. Left empty, the domains of your accounts, mail services such as Gmail left out")),
            |ui| {
                let id = egui::Id::unique("org-domains");
                let saved = s.prefs.org_domains.join(", ");
                let mut text: String = ui.data_mut(|d| d.get_temp::<String>(id)).unwrap_or_else(|| saved.clone());
                let own: Vec<&str> = s.organisation().into_iter().flatten().map(String::as_str).collect();
                let hint = if own.is_empty() { tl!("None: nothing is tagged").to_string() } else { own.join(", ") };
                let r = ui.add_enabled(s.prefs.external_tag, egui::TextEdit::singleline(&mut text).hint_text(hint).desired_width(260.0));
                if r.lost_focus() && text.trim() != saved {
                    acts.push(Action::Run("view.externalTag", json!({"domains": text.trim()})));
                }
                // Kept while it's typed in; the saved list shows again after.
                if r.has_focus() {
                    ui.data_mut(|d| d.insert_temp(id, text));
                } else {
                    ui.data_mut(|d| d.remove::<String>(id));
                }
            },
        );
    });
}

/// The senders whose pictures always load.
pub(super) fn privacy(s: &Session, t: &Tokens, ui: &mut Ui, acts: &mut Vec<Action>) {
    let privacy = if s.prefs.auto_pictures {
        tl!("These senders' pictures always load, even when a message looks suspicious:")
    } else {
        tl!("Pictures from the web load only when you ask. These senders' pictures always load:")
    };
    card(ui, t, tl!("Privacy"), Some(privacy), |ui| {
        if s.prefs.trusted_senders.is_empty() {
            ui.add_space(4.0);
            ui.label(RichText::new(tl!("No senders yet.")).color(t.faint).small());
            ui.add_space(8.0);
        }
        for (i, who) in s.prefs.trusted_senders.iter().enumerate() {
            setting_by(s, "view.trustSender", ui, t, i == 0, who, None, |ui| {
                if ui.button(tl!("Remove")).clicked() {
                    acts.push(Action::Run("view.trustSender", json!({"address": who, "on": false})));
                }
            });
        }
    });
}
