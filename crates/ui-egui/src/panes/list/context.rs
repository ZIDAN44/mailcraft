//! A message's full action menu, shared by touch and desktop input.

use super::*;

pub(super) fn message_menu(response: &egui::Response, s: &Session, t: &theme::Tokens, m: &Summary, selected: bool, acts: &mut Vec<Action>) {
    response.context_menu(|ui| crate::bidi::menu_contents(ui, |ui| message_menu_contents(ui, s, t, m, selected, acts)));
}

pub(super) fn touch_message_menu(response: &egui::Response, s: &Session, t: &theme::Tokens, m: &Summary, selected: bool, acts: &mut Vec<Action>) {
    crate::bidi::popup_menu(response).show(|ui| {
        ui.spacing_mut().interact_size.y = 44.0;
        touch_message_menu_contents(ui, s, t, m, selected, acts);
    });
}

fn touch_message_menu_contents(ui: &mut Ui, s: &Session, t: &theme::Tokens, m: &Summary, selected: bool, acts: &mut Vec<Action>) {
    let key = m.key.clone();
    let flags = m.flags.clone();
    let (subject, sender) = (m.subject.clone(), m.from.email.clone());
    if !s.selection.contains(&key) {
        acts.push(Action::Run("list.select", json!({"key": key.to_id()})));
    }
    let item = |ui: &mut Ui, acts: &mut Vec<Action>, label: &str, act: Action| {
        if ui.button(label).clicked() {
            acts.push(act);
            ui.close();
        }
    };
    let several = selected && s.selection.len() > 1;
    if flags.draft || s.view_role() == Some(mailcraft_engine::model::FolderRole::Drafts) {
        if !several {
            item(ui, acts, tl!("Edit"), Action::Run("draft.edit", json!({"key": key.to_id()})));
        }
        item(ui, acts, tl!("Open in new window"), Action::Run("message.popOut", json!({"key": key.to_id()})));
        crate::bidi::menu_button(ui, tl!("Move and delete"), |ui| {
            crate::bidi::menu_contents(ui, |ui| {
                item(ui, acts, tl!("Move to…"), Action::Dialog(Dialog::move_to()));
                item(ui, acts, tl!("Delete"), Action::Run("mail.delete", json!({})));
            })
        });
        item(ui, acts, tl!("View message source"), Action::Run("message.source", json!({"key": key.to_id()})));
        return;
    }
    item(ui, acts, tl!("Open in new window"), Action::Run("message.popOut", json!({"key": key.to_id()})));
    crate::bidi::menu_button(ui, tl!("Respond"), |ui| {
        crate::bidi::menu_contents(ui, |ui| {
            for (label, act) in [
                (tl!("Reply"), Action::Run("mail.reply", json!({}))),
                (tl!("Reply all"), Action::Run("mail.replyAll", json!({}))),
                (tl!("Forward"), Action::Run("mail.forward", json!({}))),
                (tl!("Quick print"), Action::Print),
            ] {
                if ui.add_enabled(!several, egui::Button::new(label)).clicked() {
                    acts.push(Action::Open(m.key.clone()));
                    acts.push(act);
                    ui.close();
                }
            }
        })
    });
    item(
        ui,
        acts,
        tl!(if flags.seen { "Mark as unread" } else { "Mark as read" }),
        Action::Run(if flags.seen { "mail.markUnread" } else { "mail.markRead" }, json!({})),
    );
    crate::bidi::menu_button(ui, tl!("Follow up"), |ui| crate::organize::follow_up_menu(ui, s, t, acts));
    crate::bidi::menu_button(ui, tl!("Move and delete"), |ui| {
        crate::bidi::menu_contents(ui, |ui| {
            item(ui, acts, tl!("Move to…"), Action::Dialog(Dialog::move_to()));
            item(ui, acts, tl!("Archive"), Action::Run("mail.archive", json!({})));
            item(ui, acts, tl!("Report junk"), Action::Run("mail.junk", json!({})));
            crate::bidi::menu_button(ui, tl!("Junk"), |ui| crate::bidi::menu_contents(ui, |ui| crate::junk_menu(ui, s, acts)));
            item(ui, acts, tl!("Ignore"), Action::Run("mail.ignore", json!({})));
            item(ui, acts, tl!("Delete"), Action::Run("mail.delete", json!({})));
        })
    });
    crate::bidi::menu_button(ui, tl!("More options"), |ui| crate::organize::context_items(ui, s, t, acts, flags.pinned()));
    crate::bidi::menu_button(ui, tl!("Find related"), |ui| {
        crate::bidi::menu_contents(ui, |ui| {
            let topic = mailcraft_engine::model::thread::base_subject(&subject);
            for (label, query) in [
                ("Messages in this conversation", format!("subject:\"{}\"", topic.replace('"', ""))),
                ("Messages from sender", format!("from:{sender}")),
            ] {
                if ui.button(tl!(label)).clicked() {
                    acts.push(Action::Search(query));
                    ui.close();
                }
            }
        })
    });
    item(ui, acts, tl!("View message source"), Action::Run("message.source", json!({"key": key.to_id()})));
}

fn message_menu_contents(ui: &mut Ui, s: &Session, t: &theme::Tokens, m: &Summary, selected: bool, acts: &mut Vec<Action>) {
    let key = m.key.clone();
    let flags = m.flags.clone();
    let (subject, sender) = (m.subject.clone(), m.from.email.clone());
    if !s.selection.contains(&key) {
        acts.push(Action::Run("list.select", json!({"key": key.to_id()})));
    }
    let item = |ui: &mut Ui, acts: &mut Vec<Action>, label: &str, act: Action| {
        if ui.button(label).clicked() {
            acts.push(act);
            ui.close();
        }
    };
    item(ui, acts, tl!("Open in new window"), Action::Run("message.popOut", json!({"key": key.to_id()})));
    ui.separator();
    // One of several selected: answering and printing are for one message.
    let several = selected && s.selection.len() > 1;
    for (label, act) in [
        (tl!("Reply"), Action::Run("mail.reply", json!({}))),
        (tl!("Reply all"), Action::Run("mail.replyAll", json!({}))),
        (tl!("Forward"), Action::Run("mail.forward", json!({}))),
        (tl!("Quick print"), Action::Print),
    ] {
        if ui.add_enabled(!several, egui::Button::new(label)).clicked() {
            acts.push(act);
            ui.close();
        }
    }
    ui.separator();
    item(
        ui,
        acts,
        tl!(if flags.seen { "Mark as unread" } else { "Mark as read" }),
        Action::Run(if flags.seen { "mail.markUnread" } else { "mail.markRead" }, json!({})),
    );
    crate::bidi::menu_button(ui, tl!("Follow up"), |ui| crate::organize::follow_up_menu(ui, s, t, acts));
    ui.separator();
    item(ui, acts, tl!("Move to…"), Action::Dialog(Dialog::move_to()));
    item(ui, acts, tl!("Archive"), Action::Run("mail.archive", json!({})));
    item(ui, acts, tl!("Report junk"), Action::Run("mail.junk", json!({})));
    crate::bidi::menu_button(ui, tl!("Junk"), |ui| crate::bidi::menu_contents(ui, |ui| crate::junk_menu(ui, s, acts)));
    item(ui, acts, tl!("Ignore"), Action::Run("mail.ignore", json!({})));
    item(ui, acts, tl!("Delete"), Action::Run("mail.delete", json!({})));
    ui.separator();
    // Find related: the conversation, or everything from the sender.
    crate::bidi::menu_button(ui, tl!("Find related"), |ui| {
        crate::bidi::menu_contents(ui, |ui| {
            let topic = mailcraft_engine::model::thread::base_subject(&subject);
            let searches = [
                ("Messages in this conversation", format!("subject:\"{}\"", topic.replace('"', ""))),
                ("Messages from sender", format!("from:{sender}")),
            ];
            for (label, query) in searches {
                if ui.button(tl!(label)).clicked() {
                    acts.push(Action::Search(query));
                    ui.close();
                }
            }
        })
    });
    crate::organize::context_items(ui, s, t, acts, flags.pinned());
    ui.separator();
    item(ui, acts, tl!("View message source"), Action::Run("message.source", json!({"key": key.to_id()})));
}
