//! Tests: the texts made ahead of any search push out none a search made.

use super::*;

/// A folder of `n` summaries, each with a subject of about 60 KB.
fn folder(n: u32) -> Vec<SharedSummary> {
    let subject = "abc ".repeat(15_000);
    (1..=n)
        .map(|uid| {
            Summary { key: MessageKey { account: "a".into(), folder: "F".into(), validity: 1, uid }, subject: subject.clone(), ..Default::default() }
                .into()
        })
        .collect()
}

fn key(f: &str) -> (String, String) {
    ("a".to_string(), f.to_string())
}

/// A folder searched keeps its texts while others are made ahead until the room they leave: none
/// is trimmed for those, and a search after that needs room lets those made ahead go first,
/// never the folder it searched (made ahead, they were counted as used later and went past the
/// budget a batch at a time, so the searched folder went).
#[test]
fn texts_made_ahead_push_out_none_a_search_made() {
    let each = SummaryText::of(&folder(1)[0]).bytes();
    let n = |bytes: usize| u32::try_from(bytes / each).unwrap();
    let mut t = FolderTexts::default();
    let searched = folder(n(20 << 20));
    t.texts("a", "S", &searched, 0..searched.len(), usize::MAX);
    let ahead = folder(n(TEXT_BYTES));
    let mut calls = 0;
    while t.room_ahead() && t.made_of(&key("A")) < ahead.len() {
        let before = t.made_of(&key("A"));
        t.ahead(&key("A"), &ahead, AHEAD);
        assert!(t.made_of(&key("A")) > before, "each call makes some");
        assert!(t.bytes <= TEXT_BYTES, "{} bytes", t.bytes);
        calls += 1;
        assert!(calls < 10_000);
    }
    assert!(t.made_of(&key("A")) < ahead.len(), "stopped short of the room");
    assert_eq!(t.made_of(&key("S")), searched.len(), "the searched folder kept");
    // A search needing room lets the folder made ahead go, not the one searched before.
    let next = folder(n(8 << 20));
    t.texts("a", "T", &next, 0..next.len(), usize::MAX);
    assert!(t.bytes <= TEXT_BYTES);
    assert_eq!((t.made_of(&key("S")), t.made_of(&key("T")), t.made_of(&key("A"))), (searched.len(), next.len(), 0));
}
