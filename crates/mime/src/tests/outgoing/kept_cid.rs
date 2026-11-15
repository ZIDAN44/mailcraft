use super::*;

/// A file that keeps the Content-ID it came with goes out as a file with it: listed, not inline,
/// with a paperclip, its bytes whole. Its brackets are the header's; one without goes out with
/// no Content-ID, as before.
#[test]
fn a_file_keeps_its_content_id() {
    let from = EmailAddress::new("Zoe Park", "zoe@studio.example");
    let file = OutAttachment { name: "chart.png".into(), mime: "image/png".into(), data: vec![1, 2, 3], content_id: "chart@studio.example".into() };
    let d =
        Draft { subject: "Chart".into(), body: "Hi".into(), html: Some("<p>Hi</p>".into()), attachments: vec![file.clone()], ..Default::default() };
    let raw = build(&d, &from, 1_700_000_000, "m@x").unwrap();
    assert!(String::from_utf8_lossy(&raw).contains("Content-ID: <chart@studio.example>"));
    let p = parse(&raw).unwrap();
    let got: Vec<_> = p.attachments.iter().map(|a| (a.name.as_str(), a.inline, a.content_id.as_deref())).collect();
    assert_eq!(got, [("chart.png", false, Some("chart@studio.example"))]);
    assert_eq!(attachment_data(&raw, 0).unwrap(), [1, 2, 3]);
    assert!(draft_carries_files(&d));
    assert!(summary_from_full(&raw, key(), Flags::default(), 0, 0).has_attachments);

    let bracketed = Draft { attachments: vec![OutAttachment { content_id: " <chart@studio.example> ".into(), ..file.clone() }], ..d.clone() };
    let p = parse(&build(&bracketed, &from, 1_700_000_000, "m@x").unwrap()).unwrap();
    assert_eq!(p.attachments[0].content_id.as_deref(), Some("chart@studio.example"));
    let plain = Draft { attachments: vec![OutAttachment { content_id: String::new(), ..file }], ..d };
    assert!(!String::from_utf8_lossy(&build(&plain, &from, 1_700_000_000, "m@x").unwrap()).contains("Content-ID"));
}

/// A Content-ID from a data file is hostile: one that would break its header line (a line break,
/// a space, a bracket or a control character inside, or more than the line holds) is left off,
/// and the file still goes out.
#[test]
fn a_content_id_that_would_break_its_line_is_left_off() {
    let from = EmailAddress::new("Zoe Park", "zoe@studio.example");
    let long = "x".repeat(MAX_CID + 1);
    for cid in ["a@b\r\nBcc: x@evil.example", "a b@c", "a<b>@c", "a\u{7f}@c", long.as_str()] {
        let file = OutAttachment { name: "f.bin".into(), mime: "application/octet-stream".into(), data: vec![9], content_id: cid.to_string() };
        let d = Draft { subject: "Hi".into(), body: "Hi".into(), attachments: vec![file], ..Default::default() };
        let raw = build(&d, &from, 1_700_000_000, "m@x").unwrap();
        let text = String::from_utf8_lossy(&raw);
        assert!(!text.contains("Content-ID") && !text.contains("Bcc:"), "{cid:?}");
        let p = parse(&raw).unwrap();
        assert_eq!((p.attachments.len(), p.attachments[0].content_id.as_deref()), (1, None), "{cid:?}");
    }
    let longest = "x".repeat(MAX_CID);
    let d =
        Draft { attachments: vec![OutAttachment { name: "f.bin".into(), content_id: longest.clone(), ..Default::default() }], ..Default::default() };
    let raw = build(&d, &from, 1_700_000_000, "m@x").unwrap();
    assert_eq!(parse(&raw).unwrap().attachments[0].content_id.as_deref(), Some(longest.as_str()));
    assert!(String::from_utf8_lossy(&raw).lines().all(|l| l.len() <= 998));
}
