//! Issue #254: a guest's place handed to someone else (RFC 5545 §3.2.4, §3.2.5): who it went to
//! and who it came from, read and written back as they came.

use super::*;

/// A reply delegating (RFC 5546 §4.2.5's shape): the guest's answer and whom to, and the delegate
/// with whom from; a list of them reads as its first, and a value can't end its quotes.
#[test]
fn a_delegation_reads_and_writes_back() {
    let reply = "BEGIN:VCALENDAR\r\nMETHOD:REPLY\r\nBEGIN:VEVENT\r\nUID:d1\r\nDTSTART:20261016T140000Z\r\nDTEND:20261016T150000Z\r\n\
ORGANIZER:mailto:zoe@studio.example\r\n\
ATTENDEE;PARTSTAT=DELEGATED;DELEGATED-TO=\"mailto:sam@studio.example\",\"mailto:kim@studio.example\":mailto:riley@studio.example\r\n\
ATTENDEE;RSVP=TRUE;DELEGATED-FROM=\"MAILTO:riley@studio.example\":mailto:sam@studio.example\r\n\
END:VEVENT\r\nEND:VCALENDAR\r\n";
    let e = &parse(reply, 0).events[0];
    let [riley, sam] = e.attendees.as_slice() else { panic!("{:?}", e.attendees) };
    assert_eq!((riley.partstat.as_str(), riley.delegated_to.as_str(), riley.delegated_from.as_str()), ("DELEGATED", "sam@studio.example", ""));
    assert_eq!((sam.delegated_from.as_str(), sam.delegated_to.as_str(), sam.rsvp), ("riley@studio.example", "", true));
    assert!(e.extra.iter().all(|l| !l.contains("DELEGATED")), "read, not kept as written: {:?}", e.extra);
    let out = write(std::slice::from_ref(e), Some("REPLY"), 0).replace("\r\n ", "");
    assert!(out.contains("ATTENDEE;PARTSTAT=DELEGATED;DELEGATED-TO=\"mailto:sam@studio.example\":mailto:riley@studio.example"), "{out}");
    assert!(out.contains("ATTENDEE;RSVP=TRUE;DELEGATED-FROM=\"mailto:riley@studio.example\":mailto:sam@studio.example"), "{out}");
    assert_eq!(parse(&out, 0).events[0].attendees, e.attendees);
    // What would end the quotes or the line isn't written.
    let mut odd = e.clone();
    odd.attendees[0].delegated_to = "x\"@y\r\nATTENDEE:mailto:evil@z".into();
    let out = write(std::slice::from_ref(&odd), None, 0).replace("\r\n ", "");
    assert_eq!(out.lines().filter(|l| l.starts_with("ATTENDEE")).count(), 2, "{out}");
    assert!(out.contains("DELEGATED-TO=\"mailto:x@yATTENDEEmailtoevil@z\""), "{out}");
}

/// A meeting the user handed on shows their time as free, as Outlook shows a declined one gone.
#[test]
fn a_delegated_meeting_shows_as_free() {
    let mut e = parse(INVITE, 0).events.remove(0);
    assert_eq!(show_as(&e), ShowAs::Busy);
    show_as_answered(&mut e, "DELEGATED");
    assert_eq!(show_as(&e), ShowAs::Free);
}
