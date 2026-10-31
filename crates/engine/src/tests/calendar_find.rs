//! Engine tests: private events and Show as (issue #32).

use mailcraft_model::ical::{Privacy, ShowAs, privacy, show_as};

use super::*;

#[test]
fn private_and_show_as_are_set_kept_and_shown() {
    let mut s = sample_session("calendar-private");
    let uid = s
        .run(
            "event.create",
            &json!({"summary": "Doctor", "start": "2026-10-20T08:00", "end": "2026-10-20T09:00", "privacy": "private", "showAs": "oof"}),
        )
        .unwrap()["uid"]
        .as_str()
        .unwrap()
        .to_string();
    let e = s.find_event(&uid).unwrap().1.event.clone();
    assert_eq!((privacy(&e), show_as(&e)), (Privacy::Private, ShowAs::OutOfOffice));
    let day = s.run("calendar.events", &json!({"from": "2026-10-20", "to": "2026-10-20"})).unwrap();
    let doc = day.as_array().unwrap().iter().find(|x| x["uid"] == uid.as_str()).cloned().unwrap();
    assert_eq!((doc["privacy"].as_str(), doc["showAs"].as_str()), (Some("private"), Some("oof")));
    // Changing something else leaves both; "show time only" and free change them.
    s.run("event.update", &json!({"uid": uid, "location": "Clinic"})).unwrap();
    let e = s.find_event(&uid).unwrap().1.event.clone();
    assert_eq!((privacy(&e), show_as(&e)), (Privacy::Private, ShowAs::OutOfOffice));
    s.run("event.update", &json!({"uid": uid, "privacy": "confidential", "showAs": "free"})).unwrap();
    let e = s.find_event(&uid).unwrap().1.event.clone();
    assert_eq!((privacy(&e), show_as(&e)), (Privacy::Confidential, ShowAs::Free));
    let text = mailcraft_model::ical::write(std::slice::from_ref(&e), None, NOW);
    assert!(text.contains("CLASS:CONFIDENTIAL") && text.contains("TRANSP:TRANSPARENT") && text.contains("X-MICROSOFT-CDO-BUSYSTATUS:FREE"), "{text}");
    assert_eq!(text.matches("CLASS:").count(), 1);
    // Words that aren't one are refused, and change nothing.
    for bad in [json!({"uid": uid, "privacy": "secret"}), json!({"uid": uid, "showAs": "maybe"}), json!({"uid": uid, "privacy": ""})] {
        assert!(matches!(s.run("event.update", &bad), Err(CmdError::Params(_))), "{bad}");
    }
    assert_eq!(privacy(&s.find_event(&uid).unwrap().1.event), Privacy::Confidential);
    // A classification written elsewhere that MailCraft doesn't know is kept through an edit.
    let mut odd = s.find_event(&uid).unwrap().1.event.clone();
    odd.extra.retain(|l| !l.starts_with("CLASS"));
    odd.extra.insert(0, "CLASS:X-TEAM".into());
    s.update_event(&uid, |x| *x = odd).unwrap();
    s.run("event.update", &json!({"uid": uid, "summary": "Doctor (moved)", "privacy": "private"})).unwrap();
    assert!(s.find_event(&uid).unwrap().1.event.extra.iter().any(|l| l == "CLASS:X-TEAM"), "already private: kept as written");
    done(s);
}
