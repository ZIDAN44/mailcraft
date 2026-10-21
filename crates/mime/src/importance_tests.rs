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
