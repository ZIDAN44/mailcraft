//! A message rebuilt from its item, and its summary: what the summary takes from the item before
//! the item goes into the message.

use super::super::item::{File, Item};
use super::super::rebuild::{message, summary_of, to_raw};
use super::*;

fn key() -> MessageKey {
    MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid: 1 }
}

fn summary(it: Item) -> mailcraft_model::Summary {
    let (raw, glance) = message(it, "s@x").unwrap();
    summary_of(&raw, glance, key(), u32::try_from(raw.len()).unwrap())
}

/// A reply's own words come after 10 KiB of the message it quotes: the preview finds them, in
/// its text and in the text made of its HTML (which the message's text part is too).
#[test]
fn a_preview_behind_ten_kib_of_quoted_text() {
    let quoted = "> The plan for the week, as it was agreed on Monday.\r\n".repeat(200);
    assert!(quoted.len() > 10 * 1024);
    let plain = Item { body: format!("{quoted}\r\nSounds good, see you then."), ..Default::default() };
    assert_eq!(summary(plain).preview, "Sounds good, see you then.");
    let lines = "<p>&gt; The plan for the week, as it was agreed on Monday.</p>".repeat(200);
    let html = Item { html: Some(format!("<html><body>{lines}<p>Sounds <span>good</span>.</p></body></html>")), ..Default::default() };
    assert_eq!(summary(html).preview, "Sounds good.");
    // Today's limit stays: words past the first 64K characters of the text aren't looked for.
    let far = Item { body: format!("{}Too far in.", "> quoted\r\n".repeat(7000)), ..Default::default() };
    assert_eq!(summary(far).preview, "");
}

/// The summary takes the item's flags, date and files, and the message is the one `to_raw`
/// writes: the item's files are in it, moved rather than copied.
#[test]
fn the_summary_and_the_message_come_from_one_item() {
    let picture = b"\x89PNG\r\n\x1a\npicture".to_vec();
    let item = || Item {
        subject: "Boards".into(),
        from: EmailAddress::new("Ana", "ana@example.com"),
        date: 1_704_067_200,
        body: "The boards are attached.".into(),
        flags: Flags { seen: true, flagged: true, ..Default::default() },
        files: vec![File::Data { name: "board.png".into(), mime: "image/png".into(), cid: String::new(), data: picture.clone() }],
        ..Default::default()
    };
    let (raw, glance) = message(item(), "s@x").unwrap();
    assert_eq!(raw, to_raw(item(), "s@x").unwrap());
    let s = summary_of(&raw, glance, key(), 7);
    assert!(s.flags.seen && s.flags.flagged && s.has_attachments, "{s:?}");
    assert_eq!((s.date, s.size, s.subject.as_str(), s.preview.as_str()), (1_704_067_200, 7, "Boards", "The boards are attached."));
    let p = mime::parse(&raw).unwrap();
    assert_eq!(p.attachments.len(), 1);
    assert!(!summary(Item { body: "x".into(), ..Default::default() }).has_attachments);
}
