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
    // A plain body is read through, as before: words past 64K characters of quoted text are found.
    let far = Item { body: format!("{}Too far in.", "> quoted\r\n".repeat(7000)), ..Default::default() };
    assert_eq!(summary(far).preview, "Too far in.");
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

/// A received item keeps when it was sent as its date (its header's `Date:`) and is placed by
/// when it was delivered; one delivered as it was dated keeps no second time. Its conversation
/// index, its header's or the item's own, comes in as threading's keys.
#[test]
fn a_summary_is_placed_by_delivery_and_keeps_the_conversation() {
    let mut index = vec![1, 0xD9, 1, 2, 3, 4];
    index.extend([7; 16]);
    let original = mailcraft_model::thread::index_keys(&index);
    index.extend([1; 5]);
    let header = Item {
        headers: "From: Ana <ana@example.com>\r\nDate: Mon, 1 Jan 2024 00:00:00 +0000\r\nSubject: Plan\r\n".into(),
        date: 1_704_070_800,
        received: 1_704_070_800,
        body: "b".into(),
        thread_index: index.clone(),
        ..Default::default()
    };
    let s = summary(header);
    assert_eq!((s.date, s.received, s.when()), (1_704_067_200, 1_704_070_800, 1_704_070_800));
    assert_eq!((s.thread_parent, s.thread_conversation), (original.own, original.own));
    let own = Item { date: 1_704_067_200, received: 1_704_067_200, body: "b".into(), thread_index: index, ..Default::default() };
    let s = summary(own);
    assert_eq!((s.date, s.received, s.thread_parent), (1_704_067_200, 0, original.own));
    assert_eq!(summary(Item { date: 5, body: "b".into(), ..Default::default() }).thread_own, 0);
}

/// A received message's header is put back in the built source's own buffer without doubling
/// it: the source waits in its import batch in that buffer, and a file that won't compress is
/// stored as it is.
#[test]
fn the_original_header_goes_back_without_doubling_the_source() {
    let mut x = 0x9e37_79b9_7f4a_7c15u64;
    let data: Vec<u8> = (0..2 << 20)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect();
    let received = "Received: from relay.example.com by mx.example.org; Mon, 1 Jan 2024 00:00:00 +0000\r\n".repeat(80);
    let item = Item {
        headers: format!("{received}From: Ana <ana@example.com>\r\nSubject: Scans\r\nContent-Type: multipart/mixed\r\n"),
        body: "The scans.".into(),
        files: vec![File::Data { name: "scans.bin".into(), mime: "application/octet-stream".into(), cid: String::new(), data }],
        ..Default::default()
    };
    let raw = to_raw(item, "s@x").unwrap();
    assert!(raw.starts_with(b"Received: "), "the original header is back");
    assert!(raw.capacity() - raw.len() <= raw.len() / 100, "{} held for {} bytes", raw.capacity(), raw.len());
}
