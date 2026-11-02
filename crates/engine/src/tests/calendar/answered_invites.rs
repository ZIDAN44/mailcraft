//! Engine tests: meeting mail out of the Inbox once it's answered (issue #31), as Outlook's
//! "Delete meeting requests and notifications from Inbox after responding".

use mailcraft_model::ical::{Attendee, CalEvent};

use super::invites::{deliver, me, parts};
use super::*;
use crate::{AnsweredInvites, Prefs};

const ZOE: (&str, &str) = ("Zoe Park", "zoe@studio.example");

fn invitation(uid: &str, sequence: u32) -> CalEvent {
    CalEvent {
        uid: uid.into(),
        summary: "Grade review".into(),
        start: NOW + 2 * 86_400,
        end: NOW + 2 * 86_400 + 3600,
        organizer: Some(Attendee { email: ZOE.1.into(), name: ZOE.0.into(), ..Default::default() }),
        attendees: vec![me("NEEDS-ACTION")],
        sequence,
        ..Default::default()
    }
}

/// The subjects in a folder of the sample mailbox (`deliver` names its messages `<METHOD> <n>`).
fn subjects(s: &mut Session, folder: &str) -> Vec<String> {
    s.folder_state("sample", folder).messages.iter().map(|m| m.subject.clone()).collect()
}

fn has(s: &mut Session, folder: &str, subject: &str) -> bool {
    subjects(s, folder).iter().any(|x| x == subject)
}

/// New installs do as Outlook does; settings saved before the option came keep their mail where
/// it was.
#[test]
fn outlooks_way_for_new_installs_and_none_for_old_settings() {
    assert_eq!(Prefs::default().answered_invites, AnsweredInvites::Delete);
    let old: Prefs = serde_json::from_str(r#"{"notify": false}"#).unwrap();
    assert_eq!(old.answered_invites, AnsweredInvites::Keep);
    assert!(!old.notify, "the rest of the old settings read as they were");
    let round: Prefs = serde_json::from_str(&serde_json::to_string(&Prefs::default()).unwrap()).unwrap();
    assert_eq!(round.answered_invites, AnsweredInvites::Delete, "a saved choice stays");
}

/// Accepting in the reading pane moves the invitation and its earlier version from the Inbox to
/// Deleted Items; another meeting's invitation, and plain mail, stay.
#[test]
fn answering_in_the_message_moves_it_and_its_earlier_versions() {
    let mut s = sample_session("answered-message");
    let uid = "grade@studio.example";
    deliver(&mut s, 1, ZOE, "REQUEST", &[invitation(uid, 0)], NOW);
    deliver(&mut s, 2, ZOE, "REQUEST", &[invitation("other@studio.example", 0)], NOW);
    let plain = subjects(&mut s, "INBOX").len();
    deliver(&mut s, 3, ZOE, "REQUEST", &[invitation(uid, 1)], NOW);
    let r = s.run("invite.respond", &json!({"answer": "accepted"})).unwrap();
    assert_eq!(r["moved"], 2, "{r}");
    assert!(has(&mut s, "Trash", "REQUEST 1") && has(&mut s, "Trash", "REQUEST 3"));
    assert!(has(&mut s, "INBOX", "REQUEST 2"), "another meeting's stays");
    assert_eq!(subjects(&mut s, "INBOX").len(), plain - 1);
    // The meeting itself is in the calendar, answered, and the answer went.
    assert!(parts(&s, uid).iter().all(|e| e.event.attendees.iter().any(|a| a.partstat == "ACCEPTED")));
    assert!(has(&mut s, "Sent", "Accepted: Grade review"));
    done(s);
}

/// Answered from the calendar (the event form): its invitations in the Inbox go too. With the
/// option off, nothing moves.
#[test]
fn answering_in_the_calendar_moves_it_too_and_keep_leaves_it() {
    let mut s = sample_session("answered-calendar");
    deliver(&mut s, 1, ZOE, "REQUEST", &[invitation("cal@studio.example", 0)], NOW);
    s.run("folder.open", &json!({"account": "sample", "folder": "Archive"})).unwrap();
    let r = s.run("event.respond", &json!({"uid": "cal@studio.example", "answer": "tentative"})).unwrap();
    assert_eq!(r["moved"], 1, "{r}");
    assert!(has(&mut s, "Trash", "REQUEST 1"));

    s.run("options.answeredInvites", &json!({"after": "keep"})).unwrap();
    deliver(&mut s, 2, ZOE, "REQUEST", &[invitation("kept@studio.example", 0)], NOW);
    let r = s.run("event.respond", &json!({"uid": "kept@studio.example", "answer": "accepted"})).unwrap();
    assert_eq!(r["moved"], 0);
    let r = s.run("invite.respond", &json!({"answer": "declined"})).unwrap();
    assert_eq!(r["moved"], 0);
    assert!(has(&mut s, "INBOX", "REQUEST 2"));
    done(s);
}

/// To a folder of the user's choosing; an account without one leaves it. Folders that can't
/// take them are refused.
#[test]
fn a_folder_of_the_users_choosing() {
    let mut s = sample_session("answered-folder");
    s.run("options.answeredInvites", &json!({"after": "move"})).unwrap();
    deliver(&mut s, 1, ZOE, "REQUEST", &[invitation("one@studio.example", 0)], NOW);
    assert_eq!(s.run("invite.respond", &json!({"answer": "accepted"})).unwrap()["moved"], 0, "no folder chosen yet");
    s.run("options.answeredInvites", &json!({"account": "sample", "folder": "Projects"})).unwrap();
    deliver(&mut s, 2, ZOE, "REQUEST", &[invitation("two@studio.example", 0)], NOW);
    assert_eq!(s.run("invite.respond", &json!({"answer": "accepted"})).unwrap()["moved"], 1);
    assert!(has(&mut s, "Projects", "REQUEST 2") && has(&mut s, "INBOX", "REQUEST 1"));
    for bad in [
        json!({"account": "sample", "folder": "INBOX"}),
        json!({"account": "sample", "folder": "Sent"}),
        json!({"account": "sample", "folder": "Nowhere"}),
        json!({"account": "nobody", "folder": "Projects"}),
        json!({"folder": "Projects"}),
        json!({"after": "shred"}),
    ] {
        assert!(s.run("options.answeredInvites", &bad).is_err(), "{bad}");
    }
    assert_eq!(s.prefs.answered_folders.get("sample").map(String::as_str), Some("Projects"));
    // The folder chosen is gone: nothing moves.
    s.prefs.answered_folders.insert("sample".into(), "Gone".into());
    deliver(&mut s, 3, ZOE, "REQUEST", &[invitation("three@studio.example", 0)], NOW);
    assert_eq!(s.run("invite.respond", &json!({"answer": "accepted"})).unwrap()["moved"], 0);
    done(s);
}

/// Answering one occurrence moves that occurrence's invitation, not the series'; a cancellation
/// taken in moves itself and the meeting's invitations; one of an occurrence only, itself.
#[test]
fn occurrences_and_cancellations() {
    let mut s = sample_session("answered-cancel");
    let uid = "weekly@studio.example";
    let series = CalEvent { rrule: Some("FREQ=WEEKLY;COUNT=4".into()), ..invitation(uid, 0) };
    let week2 = series.start + 7 * 86_400;
    let moved = CalEvent { recurrence_id: Some(week2), start: week2 + 3600, end: week2 + 7200, rrule: None, ..invitation(uid, 1) };
    deliver(&mut s, 1, ZOE, "REQUEST", std::slice::from_ref(&series), NOW);
    s.run("options.answeredInvites", &json!({"after": "keep"})).unwrap();
    s.run("invite.respond", &json!({"answer": "accepted"})).unwrap();
    s.run("options.answeredInvites", &json!({"after": "delete"})).unwrap();
    deliver(&mut s, 2, ZOE, "REQUEST", std::slice::from_ref(&moved), NOW);
    assert_eq!(s.run("invite.respond", &json!({"answer": "accepted"})).unwrap()["moved"], 1);
    assert!(has(&mut s, "Trash", "REQUEST 2") && has(&mut s, "INBOX", "REQUEST 1"), "the series' invitation stays");
    // Week 3 cancelled: that cancellation only.
    let week3 = series.start + 14 * 86_400;
    deliver(&mut s, 3, ZOE, "CANCEL", &[CalEvent { recurrence_id: Some(week3), rrule: None, ..invitation(uid, 2) }], NOW);
    assert_eq!(s.run("invite.apply", &json!({})).unwrap()["moved"], 1);
    assert!(has(&mut s, "INBOX", "REQUEST 1"));
    // The whole meeting cancelled: the cancellation and the invitation.
    deliver(&mut s, 4, ZOE, "CANCEL", &[invitation(uid, 3)], NOW);
    assert_eq!(s.run("invite.apply", &json!({})).unwrap()["moved"], 2);
    assert!(has(&mut s, "Trash", "CANCEL 4") && has(&mut s, "Trash", "REQUEST 1"));
    assert!(parts(&s, uid).is_empty());
    // A cancellation of a meeting no longer in the calendar is taken in too.
    deliver(&mut s, 5, ZOE, "CANCEL", &[invitation("gone@studio.example", 1)], NOW);
    assert_eq!(s.run("invite.apply", &json!({})).unwrap()["moved"], 1);
    done(s);
}

/// A guest's answer to the user's own meeting moves once recorded, only when asked for.
#[test]
fn guests_answers_when_asked_for() {
    let mut s = sample_session("answered-replies");
    let r = s.run("event.create", &json!({"summary": "Mix", "start": NOW + 86_400, "end": NOW + 90_000})).unwrap();
    let own = s.find_event(r["uid"].as_str().unwrap()).unwrap().1.event.clone();
    let reply = |stat: &str| CalEvent {
        organizer: Some(Attendee { email: "riley@studio.example".into(), ..Default::default() }),
        attendees: vec![Attendee { email: ZOE.1.into(), name: ZOE.0.into(), partstat: stat.into(), rsvp: false }],
        ..own.clone()
    };
    deliver(&mut s, 1, ZOE, "REPLY", &[reply("ACCEPTED")], NOW);
    s.run("invite.apply", &json!({})).unwrap();
    assert!(has(&mut s, "INBOX", "REPLY 1"), "off at first");
    s.run("options.answeredInvites", &json!({"replies": true})).unwrap();
    deliver(&mut s, 2, ZOE, "REPLY", &[reply("DECLINED")], NOW);
    s.run("invite.apply", &json!({})).unwrap();
    assert!(has(&mut s, "Trash", "REPLY 2"));
    done(s);
}

/// Trying to break it: mail that only talks about calendars, one with a calendar for another
/// meeting, and the invitation answered after its own body was taken out of the cache: only the
/// meeting's own mail moves, and answering still works.
#[test]
fn only_the_meetings_own_mail_moves() {
    let mut s = sample_session("answered-only");
    let uid = "own@studio.example";
    deliver(&mut s, 1, ZOE, "REQUEST", &[invitation(uid, 0)], NOW);
    deliver(&mut s, 2, ZOE, "PUBLISH", &[invitation(uid, 0)], NOW);
    deliver(&mut s, 3, ZOE, "REQUEST", &[invitation(uid, 1)], NOW);
    // Its body gone from the cache (kept online only): the answer still goes, it moves (it's
    // the one answered), the others are found by their own bodies.
    let store = s.store.clone().unwrap();
    let st = store.load_folder("sample", "INBOX").unwrap();
    let first = st.messages.iter().find(|m| m.subject == "REQUEST 1").map(|m| m.key.clone()).unwrap();
    store.remove_body(&first);
    let r = s.run("invite.respond", &json!({"answer": "accepted"})).unwrap();
    assert_eq!(r["moved"], 1, "only the one answered: the first's body isn't here to tell: {r}");
    assert!(has(&mut s, "INBOX", "REQUEST 1") && has(&mut s, "INBOX", "PUBLISH 2"), "a published copy isn't an invitation");
    done(s);
}

/// An update newer than the version answered (not taken into the calendar: tentative
/// invitations off here, or held while the account's calendars are found) stays in the Inbox, to
/// be seen and answered; it isn't moved to Deleted Items with the one answered.
#[test]
fn a_newer_update_not_taken_in_stays() {
    let mut s = sample_session("answered-newer");
    s.run("calendar.tentativeInvites", &json!({"on": false})).unwrap();
    let uid = "moved@studio.example";
    deliver(&mut s, 1, ZOE, "REQUEST", &[CalEvent { start: NOW + 3 * 86_400, end: NOW + 3 * 86_400 + 3600, ..invitation(uid, 1) }], NOW + 60);
    deliver(&mut s, 2, ZOE, "REQUEST", &[invitation(uid, 0)], NOW);
    let r = s.run("invite.respond", &json!({"answer": "accepted"})).unwrap();
    assert_eq!(r["moved"], 1, "only the one answered: {r}");
    assert!(has(&mut s, "Trash", "REQUEST 2") && has(&mut s, "INBOX", "REQUEST 1"), "the newer update stays");
    // Answered from the calendar: the update still stays, it isn't what was answered.
    let r = s.run("event.respond", &json!({"uid": uid, "answer": "tentative"})).unwrap();
    assert_eq!(r["moved"], 0, "{r}");
    assert!(has(&mut s, "INBOX", "REQUEST 1"));
    done(s);
}

/// The calendar's copy of a meeting is stamped (`DTSTAMP`) when MailCraft writes it to the
/// server; with the organizer's clock ahead, that's before the invitation's own stamp. It's still
/// the same version, so answering it from the calendar moves the invitation.
#[test]
fn the_calendars_own_stamp_isnt_a_version() {
    let mut s = sample_session("answered-stamp");
    let uid = "stamp@studio.example";
    deliver(&mut s, 1, ZOE, "REQUEST", &[invitation(uid, 0)], NOW + 3600);
    for e in s.pim.get_mut("sample").unwrap().events.iter_mut().filter(|e| e.event.uid == uid) {
        e.event.dtstamp = NOW - 60;
    }
    let r = s.run("event.respond", &json!({"uid": uid, "answer": "accepted"})).unwrap();
    assert_eq!(r["moved"], 1, "{r}");
    done(s);
}

/// Answered while read, the invitation leaves the Inbox and the list goes on to the next
/// message, as after a delete: the reading pane, the focus and the Shift anchor (issue #26) are
/// there, not lost with it (Down went back to the top of the list, Shift+Down selected from
/// there). An earlier version further down doesn't take the list with it.
#[test]
fn answered_while_read_the_list_goes_on_to_the_next() {
    let mut s = sample_session("answered-next");
    let uid = "next@studio.example";
    deliver(&mut s, 1, ZOE, "REQUEST", &[invitation(uid, 0)], NOW - 30 * 86_400);
    deliver(&mut s, 2, ZOE, "REQUEST", &[invitation(uid, 1)], NOW);
    // The newest invitation, read in the middle of the list (the list is by date).
    let store = s.store.clone().unwrap();
    let mut inbox = store.load_folder("sample", "INBOX").unwrap();
    let rows_before: Vec<i64> = inbox.messages.iter().map(|m| m.date).collect();
    let mut dates = rows_before.clone();
    dates.sort_unstable();
    let middle = dates.get(dates.len() / 2).copied().unwrap();
    for m in inbox.messages.iter_mut().filter(|m| m.subject.starts_with("REQUEST")).map(std::sync::Arc::make_mut) {
        m.date = if m.subject == "REQUEST 2" { middle - 1 } else { dates[0] - 1 };
    }
    store.save_folder("sample", "INBOX", &inbox).unwrap();
    s.apply(Event::FolderChanged { account: "sample".into(), folder: "INBOX".into() });
    let rows = s.row_keys();
    let read = s.reading.as_ref().map(|r| r.key.clone()).unwrap();
    let at = rows.iter().position(|k| *k == read).unwrap();
    assert!(at > 0 && at + 2 < rows.len(), "in the middle: {at} of {}", rows.len());
    let after = rows[at + 1].clone();
    assert_eq!(s.run("invite.respond", &json!({"answer": "accepted"})).unwrap()["moved"], 2);
    assert!(has(&mut s, "Trash", "REQUEST 1") && has(&mut s, "Trash", "REQUEST 2"));
    assert_eq!(s.reading.as_ref().map(|r| &r.key), Some(&after), "the next one is read");
    assert_eq!(s.selection, vec![after.clone()]);
    assert_eq!((s.focus.as_ref(), s.anchor.as_ref()), (Some(&after), Some(&after)));
    // Shift+Down goes on from there.
    s.run("list.extendNext", &json!({})).unwrap();
    let rows = s.row_keys();
    let from = rows.iter().position(|k| *k == after).unwrap();
    assert_eq!(s.selection, rows[from..=from + 1].to_vec());
    // Answered from the calendar, the message read isn't the invitation: it stays read.
    deliver(&mut s, 3, ZOE, "REQUEST", &[invitation("cal-next@studio.example", 0)], NOW);
    s.run("list.open", &json!({"key": after.to_id()})).unwrap();
    s.run("event.respond", &json!({"uid": "cal-next@studio.example", "answer": "accepted"})).unwrap();
    assert!(has(&mut s, "Trash", "REQUEST 3"));
    assert_eq!(s.reading.as_ref().map(|r| &r.key), Some(&after));
    done(s);
}
