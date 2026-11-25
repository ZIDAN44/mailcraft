//! Engine tests: meeting mail in the window's language (issue #235), and a meeting's cancellation
//! sent when asked to, with the user's own words, or not at all.

use super::*;

/// A sent message's subject and text, by its subject's start.
fn sent_text(s: &Session, starts: &str) -> (String, String) {
    let st = s.store.as_ref().unwrap().load_folder("sample", "Sent").unwrap();
    let m = st
        .messages
        .iter()
        .filter(|m| m.subject.starts_with(starts))
        .max_by_key(|m| m.key.uid)
        .unwrap_or_else(|| panic!("no {starts}: {:?}", st.messages.iter().map(|m| &m.subject).collect::<Vec<_>>()));
    let raw = s.store.as_ref().unwrap().read_body(&m.key).unwrap();
    let p = mailcraft_mime::parse(&raw).unwrap();
    (p.subject.clone(), p.text.clone())
}

/// German words, as the window would give them; dates with a German weekday.
fn german() -> crate::MailWords {
    fn date(s: &str) -> String {
        s.replace("Wed", "Mi").replace("Thu", "Do")
    }
    crate::MailWords {
        invitation: "Einladung: {summary}".into(),
        updated: "Aktualisiert: {summary}".into(),
        cancelled: "Abgesagt: {summary}".into(),
        cancelled_body: "{summary} wurde abgesagt.".into(),
        when: "Wann: {when}".into(),
        place: "Wo: {place}".into(),
        date,
        ..Default::default()
    }
}

/// The invitation, its update and its cancellation say so in the window's language, the date
/// with its weekday as that language has it; an update says it's one ("Invitation:" again, it
/// was).
#[test]
fn meeting_mail_is_in_the_windows_language() {
    let mut s = sample_session("mail-words");
    s.mail_words = german();
    let p = json!({"summary": "Schnitt", "start": "2026-10-21T15:00", "location": "Raum 2", "attendees": "Zoe Park <zoe@studio.example>"});
    let uid = s.run("event.create", &p).unwrap()["uid"].as_str().unwrap().to_string();
    let (subject, text) = sent_text(&s, "Einladung: ");
    assert_eq!(subject, "Einladung: Schnitt");
    assert!(text.contains("Wann: Mi 2026-10-21 15:00") && text.contains("Wo: Raum 2"), "{text}");
    s.run("event.update", &json!({"uid": uid, "start": "2026-10-22T15:00"})).unwrap();
    let (subject, text) = sent_text(&s, "Aktualisiert: ");
    assert_eq!(subject, "Aktualisiert: Schnitt");
    assert!(text.contains("Wann: Do 2026-10-22 15:00"), "{text}");
    s.run("event.delete", &json!({"uid": uid})).unwrap();
    let (subject, text) = sent_text(&s, "Abgesagt: ");
    assert_eq!((subject.as_str(), text.trim()), ("Abgesagt: Schnitt", "Schnitt wurde abgesagt."));
}

/// Issue #235: deleting one's own meeting asks first, as Outlook: the cancellation goes with the
/// user's words, or, asked not to, none goes.
#[test]
fn a_cancellation_goes_with_the_users_words_or_not_at_all() {
    let mut s = sample_session("cancel-asked");
    let made = |s: &mut Session, summary: &str| {
        let p = json!({"summary": summary, "start": "2026-10-21T15:00", "attendees": "Zoe Park <zoe@studio.example>"});
        s.run("event.create", &p).unwrap()["uid"].as_str().unwrap().to_string()
    };
    let uid = made(&mut s, "Grading");
    s.run("event.delete", &json!({"uid": uid, "message": "Sorry, the room is gone. Next week instead."})).unwrap();
    let (_, text) = sent_text(&s, "Cancelled: Grading");
    assert!(text.contains("Grading has been cancelled.") && text.contains("Sorry, the room is gone. Next week instead."), "{text}");
    let uid = made(&mut s, "Pitch");
    s.run("event.delete", &json!({"uid": uid, "notify": false})).unwrap();
    assert!(s.find_event(&uid).is_none(), "deleted");
    let st = s.store.as_ref().unwrap().load_folder("sample", "Sent").unwrap();
    assert!(!st.messages.iter().any(|m| m.subject == "Cancelled: Pitch"), "nobody told");
}

/// Issue #235: the default categories, untouched, take the window's language ("Important, Work…"
/// stayed English in German), keeping their keywords; once the user changed them, they're theirs.
#[test]
fn untouched_default_categories_take_the_windows_language() {
    let mut s = sample_session("category-words");
    s.prefs.categories = mailcraft_model::organize::default_categories();
    s.mail_words.categories = ["Wichtig", "Arbeit", "Privat", "Zu erledigen", "Später"].map(String::from);
    assert!(s.name_default_categories());
    let named: Vec<(&str, &str)> = s.prefs.categories.iter().map(|c| (c.name.as_str(), c.keyword.as_str())).collect();
    assert_eq!(named, [("Wichtig", "$label1"), ("Arbeit", "$label2"), ("Privat", "$label3"), ("Zu erledigen", "$label4"), ("Später", "$label5")]);
    // Named already, or changed by the user: left alone.
    s.mail_words.categories = ["Importante", "Trabajo", "Personal", "Por hacer", "Más tarde"].map(String::from);
    assert!(!s.name_default_categories(), "not English defaults any more");
    s.prefs.categories = mailcraft_model::organize::default_categories();
    s.prefs.categories[0].color = 7;
    assert!(!s.name_default_categories(), "one changed");
    assert_eq!(s.prefs.categories[1].name, "Work");
}
