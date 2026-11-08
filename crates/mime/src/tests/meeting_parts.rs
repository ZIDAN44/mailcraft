use super::*;

const ICS: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//MailCraft//EN\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nUID:m1@studio.example\r\nDTSTAMP:20261116T170000Z\r\nDTSTART:20261116T170000Z\r\nDTEND:20261116T180000Z\r\nSUMMARY:Tonmischung – Abnahme für Jürgen\r\nORGANIZER:mailto:riley@studio.example\r\nATTENDEE;PARTSTAT=NEEDS-ACTION;RSVP=TRUE:mailto:ana@soundlab.example\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

fn meeting(method: &str) -> Draft {
    Draft {
        to: vec![EmailAddress::new("Ana", "ana@soundlab.example")],
        subject: "Invitation: Tonmischung".into(),
        body: "When: Mon 16 Nov 18:00".into(),
        calendar: Some(OutCalendar { method: method.into(), data: ICS.into() }),
        ..Default::default()
    }
}

fn built(d: &Draft) -> (Vec<u8>, String) {
    let from = EmailAddress::new("Riley", "riley@studio.example");
    let raw = build(d, &from, 1_700_000_000, "m@studio.example").unwrap();
    let text = String::from_utf8_lossy(&raw).to_string();
    (raw, text)
}

/// The `Content-Type` lines of a message, in order, unfolded and in lower case.
fn content_types(text: &str) -> Vec<String> {
    let unfolded = text.replace("\r\n ", " ").replace("\r\n\t", " ");
    unfolded.lines().filter_map(|l| l.strip_prefix("Content-Type: ")).map(|l| l.to_ascii_lowercase().replace('"', "")).collect()
}

/// A meeting goes out as Outlook and Thunderbird send one, so Exchange shows it as a meeting
/// (RFC 6047 §2.4): the calendar inline, beside the text, with its method and character set;
/// not a file named `invite.ics`.
#[test]
fn a_meeting_is_sent_inline_beside_the_text() {
    for method in ["REQUEST", "REPLY", "CANCEL"] {
        let (raw, text) = built(&meeting(method));
        let types = content_types(&text);
        assert!(types[0].starts_with("multipart/alternative;"), "{text}");
        assert!(types[1].starts_with("text/plain;"), "{text}");
        let cal = &types[2];
        assert!(
            cal.starts_with("text/calendar;") && cal.contains("charset=utf-8") && cal.contains(&format!("method={}", method.to_ascii_lowercase())),
            "{text}"
        );
        assert_eq!(types.len(), 3, "{text}");
        assert!(!text.contains("Content-Disposition") && !text.contains("invite.ics"), "{text}");

        // Read back: the text is the message's text, the calendar is found as it was written.
        let p = parse(&raw).unwrap();
        assert_eq!(p.text.trim(), "When: Mon 16 Nov 18:00");
        let a = p.attachments.iter().find(|a| a.mime.starts_with("text/calendar")).expect("the calendar is found");
        let data = attachment_data(&raw, a.index).unwrap();
        let ics = String::from_utf8(data.to_vec()).unwrap();
        assert!(ics.contains("SUMMARY:Tonmischung – Abnahme für Jürgen\r\n"), "{ics}");
        assert_eq!(ics.replace("\r\n", "\n").trim_end(), ICS.replace("\r\n", "\n").trim_end());
    }
}

/// With files, the meeting stays together and the files go beside it; with a formatted text, the
/// calendar is the last of the three versions.
#[test]
fn a_meeting_with_files_or_formatting_keeps_its_calendar_beside_the_text() {
    let mut d = meeting("REQUEST");
    d.attachments.push(OutAttachment { name: "agenda.txt".into(), mime: "text/plain".into(), data: b"1. Mix".to_vec(), ..Default::default() });
    let (_, text) = built(&d);
    let types = content_types(&text);
    assert!(types[0].starts_with("multipart/mixed;") && types[1].starts_with("multipart/alternative;"), "{text}");
    assert!(types[2].starts_with("text/plain;") && types[3].starts_with("text/calendar;") && types[4].starts_with("text/plain"), "{text}");
    assert_eq!(text.matches("Content-Disposition: attachment").count(), 1, "only the agenda is a file: {text}");

    let mut d = meeting("REQUEST");
    d.html = Some("<p>When: Mon 16 Nov 18:00</p>".into());
    let (_, text) = built(&d);
    let types = content_types(&text);
    assert!(types[0].starts_with("multipart/alternative;"), "{text}");
    assert!(types[1].starts_with("text/plain;") && types[2].starts_with("text/html;") && types[3].starts_with("text/calendar;"), "{text}");
}

/// A method is a word: whatever else is in it stays out of the header.
#[test]
fn a_meetings_method_is_a_word() {
    let (_, text) = built(&meeting("re\"quest;\r\nBcc: x@evil.example"));
    let cal = content_types(&text).into_iter().find(|t| t.starts_with("text/calendar")).unwrap();
    assert!(cal.contains("method=requestbccxevilexample"), "{cal}");
    assert!(!text.contains("Bcc:"), "{text}");
}
