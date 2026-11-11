use super::*;

/// High and low importance, and a read receipt, go out as headers other mail programs read,
/// and come back as the message's importance.
#[test]
fn importance_and_read_receipts_round_trip() {
    let from = EmailAddress::new("Riley", "riley@studio.example");
    let k = MessageKey { account: "a".into(), folder: "INBOX".into(), validity: 1, uid: 1 };
    for (importance, header, priority) in [(1, "Importance: high", "X-Priority: 1"), (-1, "Importance: low", "X-Priority: 5")] {
        let d = Draft { subject: "Hi".into(), body: "x".into(), importance, read_receipt: true, ..Default::default() };
        let raw = build(&d, &from, 1_700_000_000, "m@x").unwrap();
        let text = String::from_utf8_lossy(&raw);
        assert!(text.contains(header) && text.contains(priority), "{text}");
        assert!(text.contains("Disposition-Notification-To: <riley@studio.example>"), "{text}");
        assert_eq!(summary_from_full(&raw, k.clone(), Flags::default(), 1, 0).importance, importance);
    }
    let plain = build(&Draft { subject: "Hi".into(), body: "x".into(), ..Default::default() }, &from, 1_700_000_000, "m@x").unwrap();
    let text = String::from_utf8_lossy(&plain);
    assert!(!text.contains("Importance") && !text.contains("Disposition-Notification-To"));
    // Other mail programs' ways of saying it.
    for (h, want) in [("X-Priority: 2 (High)", 1), ("X-Priority: 3", 0), ("Priority: non-urgent", -1), ("Importance: HIGH", 1)] {
        let raw = format!("From: a@b.example\r\nSubject: s\r\n{h}\r\n\r\nbody\r\n");
        assert_eq!(summary_from_full(raw.as_bytes(), k.clone(), Flags::default(), 1, 0).importance, want, "{h}");
    }
}

/// A reply keeps the `[tag]` a help desk matches answers on, and names what its parent answers
/// when the parent has no References.
#[test]
fn a_reply_keeps_tags_and_the_thread() {
    let raw = b"From: Desk <desk@studio.example>\r\nTo: ann@studio.example\r\nSubject: AW: [Ticket#123] Printer broken\r\nMessage-ID: <p1@x>\r\nIn-Reply-To: <p0@x>\r\n\r\nbody\r\n";
    let p = parse(raw).unwrap();
    let clock = mailcraft_model::dates::Clock::new(1_792_000_000, 0);
    let r = reply(&p, ReplyMode::Reply, &["ann@studio.example".to_string()], &clock, "");
    assert_eq!(r.subject, "Re: [Ticket#123] Printer broken");
    assert_eq!(r.references, ["p0@x", "p1@x"]);
    assert_eq!(reply(&p, ReplyMode::Forward, &[], &clock, "").subject, "Fwd: [Ticket#123] Printer broken");
}
