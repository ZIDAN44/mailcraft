//! UI tests: a new-mail notification clicked (issue #273) brings MailCraft forward at the
//! message it was about.

use std::sync::{Arc, Mutex};

use super::*;

/// Services that remember what each notification would open, and report a click when told.
#[derive(Default, Clone)]
struct Clicks {
    opens: Arc<Mutex<Vec<Option<MessageKey>>>>,
    clicked: Arc<Mutex<Option<Option<MessageKey>>>>,
}

impl crate::Services for Clicks {
    stub_services!();
    fn notify_about(&mut self, _title: &str, _body: &str, opens: Option<MessageKey>) {
        self.opens.lock().unwrap().push(opens);
    }
    fn notification_clicked(&mut self) -> Option<Option<MessageKey>> {
        self.clicked.lock().unwrap().take()
    }
}

/// The notification of new mail names its newest message; clicked, with the window in the
/// notification area and another folder open, MailCraft comes back at that message: its folder,
/// the Other tab it's on, it shown.
#[test]
fn a_clicked_notification_shows_its_message() {
    let (mut a, _) = app("notified");
    let clicks = Clicks::default();
    a.services = Box::new(clicks.clone());
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    a.run("view.focused", json!({"on": true}));
    a.run("list.focusTab", json!({"value": "other"}));
    let m = a.session.list.iter().find(|m| m.subject == "Open call: winter group show").map(|m| (**m).clone()).expect("in Other");
    let older = Summary { key: MessageKey { uid: 1, ..m.key.clone() }, date: m.date - 60, ..m.clone() };
    a.run("list.focusTab", json!({"value": "focused"}));
    a.run("folder.open", json!({"account": "sample", "folder": "Sent"}));
    a.session.new_mail = vec![m.clone(), older];
    frame(&ctx, &mut a, vec![]);
    assert_eq!(*clicks.opens.lock().unwrap(), [Some(m.key.clone())], "the newest of them");
    // Away in the notification area: only `logic` runs.
    a.ui.in_tray = true;
    *clicks.clicked.lock().unwrap() = Some(Some(m.key.clone()));
    a.logic(&ctx);
    assert!(!a.ui.in_tray, "back from the notification area");
    assert!(matches!(&a.session.view, mailcraft_engine::View::Folder { folder, .. } if folder == "INBOX"), "{:?}", a.session.view);
    assert!(a.session.focus_other, "the Other tab, where it is");
    assert!(a.session.reading.as_ref().is_some_and(|r| r.key == m.key));
    // One about nothing in particular: the window alone.
    a.ui.in_tray = true;
    *clicks.clicked.lock().unwrap() = Some(None);
    a.logic(&ctx);
    assert!(!a.ui.in_tray);
    assert!(a.session.reading.as_ref().is_some_and(|r| r.key == m.key), "left as it was");
}

/// One clicked in the notification centre reaches the window as its `mailcraft:` link, handed
/// over by a launch: the window comes back at its message, as a click on the toast does. A link
/// that isn't one MailCraft makes opens nothing, and isn't taken for a file.
#[test]
fn a_notifications_link_shows_its_message() {
    let (mut a, _) = app("notified-link");
    let ctx = egui::Context::default();
    frame(&ctx, &mut a, vec![]);
    let m = a.session.list.iter().find(|m| m.subject == "Character turnarounds").map(|m| m.key.clone()).expect("in the Inbox");
    a.run("folder.open", json!({"account": "sample", "folder": "Sent"}));
    a.ui.in_tray = true;
    a.open_handed(&mailcraft_engine::model::app_link::show_link(Some(&m)));
    a.logic(&ctx);
    assert!(!a.ui.in_tray, "back from the notification area");
    assert!(a.session.reading.as_ref().is_some_and(|r| r.key == m), "{:?}", a.session.reading.as_ref().map(|r| &r.key));
    for not_ours in ["mailcraft:delete?key=x", "mailcraft:show?key=nobody%3A1%3A2%3AINBOX"] {
        a.ui.in_tray = true;
        a.open_handed(not_ours);
        a.logic(&ctx);
        assert!(a.ui.dialog.is_none() && a.session.reading.as_ref().is_some_and(|r| r.key == m), "{not_ours}: nothing opened");
    }
    assert!(a.session.notices.iter().all(|n| !n.text.contains("Couldn't open")), "not taken for a file");
}
