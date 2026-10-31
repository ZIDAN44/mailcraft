//! Issue #32: an event's classification and Show as, read from what other calendars write,
//! set, and written back without losing what was there.

use super::*;

fn one(lines: &str) -> CalEvent {
    let text = format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:x\r\nDTSTART:20261022T090000Z\r\nDTEND:20261022T100000Z\r\n{lines}END:VEVENT\r\nEND:VCALENDAR\r\n"
    );
    parse(&text, 0).events.into_iter().next().unwrap()
}

fn written(e: &CalEvent) -> String {
    write(std::slice::from_ref(e), None, 0)
}

#[test]
fn a_classification_set_elsewhere_is_read_and_kept() {
    assert_eq!(privacy(&one("")), Privacy::Public);
    assert_eq!(privacy(&one("CLASS:PUBLIC\r\n")), Privacy::Public);
    assert_eq!(privacy(&one("CLASS:PRIVATE\r\n")), Privacy::Private);
    assert_eq!(privacy(&one("class:confidential\r\n")), Privacy::Confidential, "any case");
    // One a calendar doesn't know counts as private (RFC 5545), and goes back as it came.
    let odd = one("CLASS:X-TEAM-ONLY\r\nSUMMARY:Board\r\n");
    assert_eq!(privacy(&odd), Privacy::Private);
    let mut kept = odd.clone();
    set_privacy(&mut kept, Privacy::Private);
    assert_eq!(kept, odd, "already private: nothing changes");
    assert!(written(&kept).contains("CLASS:X-TEAM-ONLY\r\n"));
    // A property that only starts the same isn't it.
    assert_eq!(privacy(&one("CLASSIFICATION:secret\r\n")), Privacy::Public);
    // Edited for something else, Thunderbird's or SOGo's classification stays.
    let mut e = one("CLASS:CONFIDENTIAL\r\nX-MOZ-GENERATION:3\r\n");
    e.summary = "Renamed".into();
    let text = written(&e);
    assert!(text.contains("CLASS:CONFIDENTIAL\r\n") && text.contains("X-MOZ-GENERATION:3\r\n"), "{text}");
}

#[test]
fn private_and_show_time_only_are_set_once_and_never_inside_an_alarm() {
    let mut e = one("CLASS:PUBLIC\r\nBEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER:-PT15M\r\nEND:VALARM\r\n");
    set_privacy(&mut e, Privacy::Private);
    let text = written(&e);
    assert_eq!(text.matches("CLASS:").count(), 1, "{text}");
    assert!(text.find("CLASS:PRIVATE").unwrap() < text.find("BEGIN:VALARM").unwrap(), "{text}");
    assert_eq!(reminder_minutes(&e), Some(15), "the reminder stays");
    set_privacy(&mut e, Privacy::Confidential);
    assert_eq!(privacy(&parse(&written(&e), 0).events[0]), Privacy::Confidential, "read back");
    set_privacy(&mut e, Privacy::Public);
    assert!(!written(&e).contains("CLASS"), "public is the default: left out");
    assert_eq!(privacy(&e), Privacy::Public);
    // A CLASS inside an alarm isn't the event's.
    let nested = one("BEGIN:VALARM\r\nCLASS:PRIVATE\r\nEND:VALARM\r\n");
    assert_eq!(privacy(&nested), Privacy::Public);
    // An alarm left open in a damaged cache: the line still goes among the event's own.
    let mut broken = CalEvent { extra: vec!["X-A:1".into(), "BEGIN:VALARM".into(), "TRIGGER:-PT5M".into()], ..Default::default() };
    set_privacy(&mut broken, Privacy::Private);
    assert_eq!(broken.extra.get(1).map(String::as_str), Some("CLASS:PRIVATE"));
}

#[test]
fn show_as_reads_transp_and_outlooks_busy_status() {
    assert_eq!(show_as(&one("")), ShowAs::Busy);
    assert_eq!(show_as(&one("TRANSP:OPAQUE\r\n")), ShowAs::Busy);
    assert_eq!(show_as(&one("TRANSP:TRANSPARENT\r\n")), ShowAs::Free);
    // As Outlook writes them.
    for (status, transp, want) in [
        ("FREE", "TRANSPARENT", ShowAs::Free),
        ("TENTATIVE", "OPAQUE", ShowAs::Tentative),
        ("BUSY", "OPAQUE", ShowAs::Busy),
        ("OOF", "OPAQUE", ShowAs::OutOfOffice),
        ("WORKINGELSEWHERE", "OPAQUE", ShowAs::WorkingElsewhere),
    ] {
        let e = one(&format!("TRANSP:{transp}\r\nX-MICROSOFT-CDO-BUSYSTATUS:{status}\r\n"));
        assert_eq!(show_as(&e), want, "{status}");
    }
    assert_eq!(show_as(&one("X-MICROSOFT-CDO-BUSYSTATUS:oof\r\n")), ShowAs::OutOfOffice, "alone, any case");
    // Changed to free by a calendar that writes TRANSP alone: free, whatever Outlook had said.
    assert_eq!(show_as(&one("TRANSP:TRANSPARENT\r\nX-MICROSOFT-CDO-BUSYSTATUS:BUSY\r\n")), ShowAs::Free);
    assert_eq!(show_as(&one("TRANSP:OPAQUE\r\nX-MICROSOFT-CDO-BUSYSTATUS:FREE\r\n")), ShowAs::Busy);
    assert_eq!(show_as(&one("X-MICROSOFT-CDO-BUSYSTATUS:SOMETHING\r\n")), ShowAs::Busy, "unknown: busy");
}

#[test]
fn show_as_is_written_as_outlook_writes_it_and_round_trips() {
    for s in ShowAs::ALL {
        let mut e = one("TRANSP:OPAQUE\r\nX-MICROSOFT-CDO-INTENDEDSTATUS:BUSY\r\n");
        set_show_as(&mut e, s);
        let text = written(&e);
        let back = parse(&text, 0).events.into_iter().next().unwrap();
        assert_eq!(show_as(&back), s, "{text}");
        assert_eq!(text.matches("TRANSP:").count(), 1, "one, the old one replaced: {text}");
        assert!(text.contains("X-MICROSOFT-CDO-INTENDEDSTATUS:BUSY"), "the rest stays");
        if s == ShowAs::Free {
            assert!(text.contains("TRANSP:TRANSPARENT\r\nX-MICROSOFT-CDO-BUSYSTATUS:FREE\r\n"), "{text}");
        }
        if s == ShowAs::OutOfOffice {
            assert!(text.contains("TRANSP:OPAQUE\r\nX-MICROSOFT-CDO-BUSYSTATUS:OOF\r\n"), "{text}");
        }
        // Back to busy: one TRANSP and one status, never two.
        let mut b = back;
        set_show_as(&mut b, ShowAs::Busy);
        let text = written(&b);
        assert!(text.matches("TRANSP:").count() <= 1 && text.matches("BUSYSTATUS").count() <= 1, "{text}");
        assert_eq!(show_as(&b), ShowAs::Busy);
    }
    for s in ShowAs::ALL {
        assert_eq!(ShowAs::from_code(s.code()), Some(s));
    }
    for p in Privacy::ALL {
        assert_eq!(Privacy::from_code(p.code()), Some(p));
    }
    assert_eq!(ShowAs::from_code("WorkingElsewhere"), Some(ShowAs::WorkingElsewhere), "EWS's name");
    assert_eq!(ShowAs::from_code("OOF"), Some(ShowAs::OutOfOffice));
    assert_eq!(ShowAs::from_code(""), None);
    assert_eq!(Privacy::from_code("personal"), Some(Privacy::Private), "Exchange's Personal");
}
