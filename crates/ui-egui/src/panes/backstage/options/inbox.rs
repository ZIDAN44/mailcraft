//! Options' Inbox and notifications card: new mail, the Focused Inbox, meeting invitations
//! once answered, deleting for good and animations.

use mailcraft_engine::AnsweredInvites;
use mailcraft_engine::model::FolderRole;

use super::*;

pub(super) fn inbox(s: &Session, t: &Tokens, ui: &mut Ui, acts: &mut Vec<Action>) {
    card(ui, t, tl!("Inbox and notifications"), None, |ui| {
        let switches = [
            (tl!("Show a desktop notification when new mail arrives"), None, s.prefs.notify, "view.notifications"),
            (tl!("Focused Inbox: put newsletters and automatic mail in Other"), None, s.prefs.focused_inbox, "view.focused"),
            (
                tl!("Add meeting invitations to the calendar as tentative when they arrive"),
                Some(tl!("Before they're answered; nothing is sent until you answer")),
                s.prefs.tentative_invites,
                "calendar.tentativeInvites",
            ),
            (
                tl!("Record guests' answers to your meetings as they arrive"),
                Some(tl!("One from someone not on the guest list, or to an earlier version, waits for Record the answer")),
                s.prefs.record_answers,
                "calendar.recordAnswers",
            ),
        ];
        for (i, (label, about, on, cmd)) in switches.into_iter().enumerate() {
            switch_row(ui, t, i == 0, Switch { label, about, on, cmd }, acts);
            // With both on, Other's mail is announced only when asked (issue #199).
            if cmd == "view.focused" && s.prefs.focused_inbox && s.prefs.notify {
                let (label, about) = (tl!("Notify of mail in Other too"), Some(tl!("Off, only mail that goes to Focused is announced")));
                switch_row(ui, t, false, Switch { label, about, on: s.prefs.notify_other, cmd: "view.notifyOther" }, acts);
            }
        }
        answered_invites(s, t, ui, acts);
        let switches = [
            (tl!("Ask before deleting for good"), None, s.prefs.confirm_permanent_delete, "view.confirmDelete"),
            (
                tl!("Play animations"),
                Some(tl!("Hovers, selections and panes ease into place; off, everything changes at once")),
                s.prefs.animations,
                "view.animations",
            ),
        ];
        for (label, about, on, cmd) in switches {
            switch_row(ui, t, false, Switch { label, about, on, cmd }, acts);
        }
    });
}

/// Issue #31: Outlook's "Delete meeting requests and notifications from Inbox after
/// responding", with a folder of the user's as a third way; and guests' answers too.
fn answered_invites(s: &Session, t: &Tokens, ui: &mut Ui, acts: &mut Vec<Action>) {
    setting(
        ui,
        t,
        false,
        tl!("After answering a meeting invitation"),
        Some(tl!(
            "Answered in the message or in the calendar, the invitation and its earlier versions leave the Inbox; a cancellation too, once it's taken in"
        )),
        |ui| {
            let modes = [AnsweredInvites::Keep, AnsweredInvites::Delete, AnsweredInvites::Move];
            let now = modes.iter().position(|m| *m == s.prefs.answered_invites).unwrap_or(0);
            if let Some(i) = widgets::segmented(ui, t, "answered", &[tl!("Leave it"), tl!("Deleted Items"), tl!("A folder")], now)
                && let Some(m) = modes.get(i)
            {
                acts.push(Action::Run("options.answeredInvites", json!({"after": m})));
            }
        },
    );
    if s.prefs.answered_invites == AnsweredInvites::Move {
        for a in &s.accounts {
            let label = tf!("Folder for {email}", email = a.config.email);
            setting(ui, t, false, &label, None, |ui| {
                let chosen = s.prefs.answered_folders.get(&a.config.id);
                let fits = |f: &&mailcraft_engine::model::Folder| {
                    f.selectable && !matches!(f.role, FolderRole::Inbox | FolderRole::Sent | FolderRole::Drafts)
                };
                let current = chosen
                    .and_then(|c| a.folders.iter().find(|f| &f.path == c))
                    .map_or_else(|| tl!("Choose a folder").to_string(), crate::i18n::folder);
                let r = egui::ComboBox::from_id_salt(("answered-folder", &a.config.id))
                    .selected_text(current)
                    .show_ui(ui, |ui| {
                        for f in a.folders.iter().filter(fits) {
                            if ui.selectable_label(chosen == Some(&f.path), crate::i18n::folder(f)).clicked() {
                                acts.push(Action::Run("options.answeredInvites", json!({"account": a.config.id, "folder": f.path})));
                            }
                        }
                    })
                    .response
                    .accessible_name(label.as_str());
                let paths: Vec<Option<&String>> = a.folders.iter().filter(fits).map(|f| Some(&f.path)).collect();
                let mut at = chosen;
                crate::drop_down::keys(ui, &r, &mut at, &paths);
                if at != chosen
                    && let Some(folder) = at
                {
                    acts.push(Action::Run("options.answeredInvites", json!({"account": a.config.id, "folder": folder})));
                }
            });
        }
    }
    if s.prefs.answered_invites != AnsweredInvites::Keep {
        setting(ui, t, false, tl!("Guests' answers to my meetings go the same way, once recorded"), None, |ui| {
            let mut on = s.prefs.answered_replies;
            if widgets::switch(ui, t, &mut on, tl!("Guests' answers to my meetings go the same way, once recorded")).changed() {
                acts.push(Action::Run("options.answeredInvites", json!({"replies": on})));
            }
        });
    }
}
