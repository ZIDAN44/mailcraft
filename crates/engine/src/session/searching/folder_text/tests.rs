//! Tests: the texts made ahead of any search push out none a search made, and those made aside
//! are taken in only where they still fit the folder's list.

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

/// What a thread made of `messages[range]`.
fn made(messages: &[SharedSummary], range: std::ops::Range<usize>) -> aside::Made {
    let summaries = messages[range.clone()].to_vec();
    let texts = summaries.iter().map(|m| SummaryText::of(m)).collect();
    aside::Made { key: key("F"), start: range.start, summaries, texts }
}

/// Texts made aside are taken in where they follow what's kept: those past it wait, those
/// already kept are skipped, and they stop at a summary that isn't the one at its place any more
/// (changed in place, same uid) or a list that lost messages meanwhile; what's kept is each
/// summary's text as it is.
#[test]
fn texts_made_aside_fit_the_list_as_it_is() {
    let mut list: Vec<SharedSummary> = (1..=10)
        .map(|uid| {
            Summary {
                key: MessageKey { account: "a".into(), folder: "F".into(), validity: 1, uid },
                subject: format!("s{uid}"),
                ..Default::default()
            }
            .into()
        })
        .collect();
    let mut t = FolderTexts::default();
    assert!(t.take_in(&key("F"), &list, made(&list, 0..4)).is_none());
    assert_eq!((t.made_of(&key("F")), t.made_aside()), (4, 4));
    // Past what's kept: it waits.
    let later = t.take_in(&key("F"), &list, made(&list, 6..10));
    assert!(later.is_some_and(|m| m.start == 6));
    assert_eq!(t.made_of(&key("F")), 4);
    // Partly kept already (a search made some): the rest taken.
    assert!(t.take_in(&key("F"), &list, made(&list, 2..7)).is_none());
    assert_eq!((t.made_of(&key("F")), t.made_aside()), (7, 7));
    // One changed in place since it was handed out: taken up to it.
    let stale = made(&list, 7..10);
    list[8] = Summary { subject: "changed".into(), ..(*list[8]).clone() }.into();
    assert!(t.take_in(&key("F"), &list, stale).is_none());
    assert_eq!(t.made_of(&key("F")), 8);
    for (at, m) in list.iter().enumerate().take(8) {
        assert_eq!(t.kept(&m.key, at), Some(&SummaryText::of(m)), "{at}");
    }
    // A message gone from the list: what's kept from it on goes, and what was made of the list
    // before isn't taken.
    let before = made(&list, 8..10);
    list.remove(3);
    assert!(t.take_in(&key("F"), &list, before).is_some(), "past what's still kept: it waits");
    assert_eq!(t.made_of(&key("F")), 3);
    assert!(t.take_in(&key("F"), &list, made(&list, 3..9)).is_none());
    assert_eq!(t.made_of(&key("F")), 9);
    for (at, m) in list.iter().enumerate() {
        assert_eq!(t.kept(&m.key, at), Some(&SummaryText::of(m)), "{at}");
    }
}
