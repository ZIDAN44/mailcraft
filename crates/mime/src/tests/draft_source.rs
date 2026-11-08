use super::*;

/// Gap 8: a draft of a forward names the message it forwards in its own copy, and its summary
/// keeps that, for the list to mark the message `[Draft]`: a folder's name in any script,
/// however the header was folded and encoded. A reply's draft, other mail and a field that only
/// looks like one keep nothing.
#[test]
fn a_forwards_draft_names_its_message_in_its_summary() {
    let from = EmailAddress::new("Riley", "riley@studio.example");
    let here = MessageKey { account: "a".into(), folder: "Drafts".into(), validity: 1, uid: 9 };
    let long = format!("Projekte/Entwürfe {}fin", "très long ".repeat(40));
    for folder in ["INBOX", "Entwürfe/Ärger", "Входящие", long.as_str()] {
        let source = MessageKey { account: "acc-1".into(), folder: folder.into(), validity: 7, uid: 42 };
        let d = Draft {
            subject: "Fwd: Hi".into(),
            body: "x".into(),
            keep_bcc: true,
            source: format!("forward {}", source.to_id()),
            ..Default::default()
        };
        let raw = build(&d, &from, 1_700_000_000, "m@x").unwrap();
        let s = summary_from_full(&raw, here.clone(), Flags::default(), 1, 0);
        assert_eq!(s.forward_of, source.to_id(), "{}", String::from_utf8_lossy(&raw));
        assert_eq!(summary_from_header(&raw, here.clone(), Flags::default(), 1, 0).forward_of, source.to_id(), "from its header alone");
        assert_eq!(sending_options(&raw).2, format!("forward {}", source.to_id()), "opened again to carry on writing it");
    }
    let reply = Draft { subject: "Re: Hi".into(), body: "x".into(), keep_bcc: true, source: "reply a:1:2:INBOX".into(), ..Default::default() };
    let raw = build(&reply, &from, 1_700_000_000, "m@x").unwrap();
    assert_eq!(summary_from_full(&raw, here.clone(), Flags::default(), 1, 0).forward_of, "", "a reply is known by its In-Reply-To");
    for field in [
        "X-MailCraft-Source: forward nonsense".to_string(),
        "X-MailCraft-Source: forward a:x:2:INBOX".to_string(),
        "X-MailCraft-Source: forwarded a:1:2:INBOX".to_string(),
        format!("X-MailCraft-Source: forward a:1:2:{}", "F".repeat(9000)),
    ] {
        let raw = format!("From: a@b.example\r\nSubject: s\r\n{field}\r\n\r\nbody\r\n");
        let kept = summary_from_full(raw.as_bytes(), here.clone(), Flags::default(), 1, 0).forward_of;
        assert_eq!(kept, "", "{}", field.get(..60).unwrap_or(&field));
    }
}
