//! Search: the slices of a long text checked a unit at a time.

use super::*;

/// The slices `Verifying::step` checks, in order.
fn slices(text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < text.len() {
        let (start, end) = slice_at(text, at);
        out.push((start, end));
        at = end.max(at.saturating_add(1));
    }
    out
}

const MOST: usize = VERIFY_BYTES + 2 * OVERLAP + VERIFY_BYTES / 4;

/// A text with no spaces (CJK, a long run of base64) is still checked a bounded slice at a
/// time, each slice once over, and a phrase across a cut is in one of them.
#[test]
fn a_text_without_spaces_is_sliced_bounded() {
    // "甲乙" across the first slice's end: 87,381 characters of three bytes are 262,143.
    let mut text = "字".repeat(87_381);
    text.push_str("甲乙");
    text.push_str(&"字".repeat(400_000));
    let all = slices(&text);
    assert!(all.iter().all(|&(s, e)| e - s <= MOST && text.is_char_boundary(s) && text.is_char_boundary(e)), "{all:?}");
    let total: usize = all.iter().map(|&(s, e)| e - s).sum();
    assert!(total <= text.len() + all.len() * 2 * OVERLAP, "{total} for {}", text.len());
    assert!(all.iter().any(|&(s, e)| text.get(s..e).is_some_and(|t| t.contains("甲乙"))));
    let base64 = "QUJD".repeat(500_000);
    assert!(slices(&base64).iter().all(|&(s, e)| e - s <= MOST));
}

/// With words near, a slice starts at a word's start and ends at a word's end: never a word's
/// tail taken for a word starting so.
#[test]
fn a_slice_starts_and_ends_between_words() {
    let text = "alpha-beta, gamma.delta ".repeat(40_000);
    for (s, e) in slices(&text).into_iter().skip(1) {
        let before = text.get(..s).and_then(|t| t.chars().next_back());
        assert!(before.is_none_or(|c| !c.is_alphanumeric()), "starts inside a word at {s}");
        let last = text.get(..e).and_then(|t| t.chars().next_back());
        assert!(e == text.len() || last.is_none_or(|c| !c.is_alphanumeric()), "ends inside a word at {e}");
    }
}

/// Folded texts are kept up to their room: what the search running uses stays, room is made of
/// what only searches before it used, and past that a text isn't kept (a search through more
/// than fits keeps the first it checked, rather than letting each go before it comes round).
#[test]
fn folded_texts_keep_what_the_search_running_uses() {
    use super::super::folded::{FOLDED_BYTES, Folded, FoldedTexts};
    let key = |uid: u32| MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid };
    let text = |n: usize| Folded { names: Vec::new(), slices: vec!["x".repeat(n)] };
    let quarter = FOLDED_BYTES / 4;
    let mut kept = FoldedTexts::default();
    kept.begin();
    for uid in 0..3 {
        kept.keep(&key(uid), text(quarter));
    }
    // The next search uses one of them: the others make room for what it checks.
    kept.begin();
    assert!(kept.get(&key(1)).is_some());
    kept.keep(&key(10), text(quarter));
    kept.keep(&key(11), text(quarter));
    assert!(kept.get(&key(0)).is_none() && kept.get(&key(2)).is_none(), "made room");
    assert!(kept.get(&key(1)).is_some() && kept.get(&key(10)).is_some() && kept.get(&key(11)).is_some());
    // No room left but what this search uses: not kept, the others stay.
    kept.keep(&key(12), text(quarter));
    assert!(kept.get(&key(12)).is_none() && kept.get(&key(10)).is_some());
    assert!(kept.bytes() <= FOLDED_BYTES);
    kept.retain(|a| a != "a1");
    assert_eq!(kept.bytes(), 0);
}
