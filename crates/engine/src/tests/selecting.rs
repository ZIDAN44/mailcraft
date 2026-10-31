//! Engine tests: moving through the list and selecting a range of it from the keyboard and with
//! Shift+click (issue #26).

use super::*;
use crate::model::MessageKey;

/// The message rows' subjects, in the order the list shows them.
fn rows(s: &Session) -> Vec<String> {
    s.row_keys().iter().filter_map(|k| s.summary(k)).map(|m| m.subject.clone()).collect()
}

/// The selected messages' subjects, in the order the list shows them.
fn selected(s: &Session) -> Vec<String> {
    let rows = s.row_keys();
    let mut sel: Vec<&MessageKey> = s.selection.iter().collect();
    sel.sort_by_key(|k| rows.iter().position(|r| r == *k).unwrap_or(usize::MAX));
    sel.iter().filter_map(|k| s.summary(k)).map(|m| m.subject.clone()).collect()
}

fn focus(s: &Session) -> String {
    s.focus.as_ref().and_then(|k| s.summary(k)).map(|m| m.subject.clone()).unwrap_or_default()
}

/// Shift+Down grows the selection from where it started, Shift+Up past it shrinks it back and
/// then grows it the other way; one message left selected is read again, several aren't.
#[test]
fn shift_arrows_grow_and_shrink_from_the_anchor() {
    let mut s = sample_session("shift-arrows");
    let all = rows(&s);
    assert!(all.len() >= 6, "{all:?}");
    s.run("list.open", &key_of(&s, &all[2])).unwrap();
    s.run("list.extendNext", &json!({})).unwrap();
    s.run("list.extendNext", &json!({})).unwrap();
    assert_eq!(selected(&s), all[2..5].to_vec());
    assert_eq!(focus(&s), all[4]);
    assert!(s.reading.is_none(), "several selected: none is read");
    assert!(s.run("mail.reply", &json!({})).is_err(), "Reply waits for one message");
    // Back up: less is selected, down to the anchor alone, which is read again.
    s.run("list.extendPrev", &json!({})).unwrap();
    assert_eq!(selected(&s), all[2..4].to_vec());
    s.run("list.extendPrev", &json!({})).unwrap();
    assert_eq!(selected(&s), vec![all[2].clone()]);
    assert_eq!(s.reading.as_ref().and_then(|r| s.summary(&r.key)).map(|m| m.subject.clone()), Some(all[2].clone()));
    // On past it: upward from the anchor.
    s.run("list.extendPrev", &json!({})).unwrap();
    s.run("list.extendPrev", &json!({})).unwrap();
    assert_eq!(selected(&s), all[0..3].to_vec());
    // At the top it stays there.
    s.run("list.extendPrev", &json!({})).unwrap();
    assert_eq!(selected(&s), all[0..3].to_vec());
    // Shift+End and Shift+Home: from the anchor to the end, then to the start.
    s.run("list.extendLast", &json!({})).unwrap();
    assert_eq!(selected(&s), all[2..].to_vec());
    s.run("list.extendFirst", &json!({})).unwrap();
    assert_eq!(selected(&s), all[..3].to_vec());
    // A plain arrow lets go of the range: the next one alone, and the anchor moves with it.
    s.run("list.next", &json!({})).unwrap();
    assert_eq!(selected(&s), vec![all[1].clone()]);
    s.run("list.extendNext", &json!({})).unwrap();
    assert_eq!(selected(&s), all[1..3].to_vec());
    done(s);
}

/// Page Down and Page Up go a page of rows (as many as the list shows), clamped at the ends;
/// Home and End go to the first and last.
#[test]
fn pages_home_and_end() {
    let mut s = sample_session("pages");
    let all = rows(&s);
    let n = all.len();
    s.run("list.first", &json!({})).unwrap();
    assert_eq!(selected(&s), vec![all[0].clone()]);
    s.run("list.extendPageDown", &json!({"rows": 3})).unwrap();
    assert_eq!(selected(&s), all[..4].to_vec());
    s.run("list.extendPageDown", &json!({"rows": 10_000_000})).unwrap();
    assert_eq!(selected(&s), all.clone());
    s.run("list.extendPageUp", &json!({"rows": 2})).unwrap();
    assert_eq!(selected(&s), all[..n - 2].to_vec());
    s.run("list.last", &json!({})).unwrap();
    assert_eq!(selected(&s), vec![all[n - 1].clone()]);
    s.run("list.pageUp", &json!({})).unwrap();
    let up = (n - 1).saturating_sub(10);
    assert_eq!(selected(&s), vec![all[up].clone()]);
    s.run("list.pageDown", &json!({"rows": 0})).unwrap();
    assert_eq!(selected(&s), vec![all[(up + 1).min(n - 1)].clone()], "a page is a row at least");
    done(s);
}

/// Nothing chosen yet: Shift+Down starts at the top; an empty list does nothing.
#[test]
fn shift_down_with_nothing_chosen_and_an_empty_list() {
    let mut s = sample_session("shift-empty");
    let all = rows(&s);
    s.run("list.extendNext", &json!({})).unwrap();
    assert_eq!(selected(&s), vec![all[0].clone()]);
    s.run("search.run", &json!({"query": "nothing-at-all-matches-this"})).unwrap();
    assert!(s.list.is_empty());
    for id in ["list.extendNext", "list.extendPrev", "list.extendFirst", "list.extendLast", "list.extendPageDown", "list.first", "list.pageUp"] {
        s.run(id, &json!({})).unwrap();
    }
    assert!(s.selection.is_empty());
    done(s);
}

/// Shift+click goes by the rows as shown, not by where messages sit in the list behind them:
/// pinned mail is shown first, so a range from a pinned message took messages from elsewhere
/// (found testing issue #26). A second Shift+click goes from the same anchor; Ctrl+Shift+click
/// keeps the rest.
#[test]
fn shift_click_goes_by_the_rows_shown() {
    let mut s = sample_session("shift-click");
    let before = rows(&s);
    // Pin the fifth: it goes to the top.
    s.run("list.select", &key_of(&s, &before[4])).unwrap();
    s.run("mail.pin", &json!({"on": true})).unwrap();
    let all = rows(&s);
    assert_eq!(all[0], before[4], "pinned mail first");
    s.run("list.open", &key_of(&s, &all[0])).unwrap();
    s.run("list.selectRange", &key_of(&s, &all[2])).unwrap();
    assert_eq!(selected(&s), all[..3].to_vec());
    // From the same anchor again, shorter.
    s.run("list.selectRange", &key_of(&s, &all[1])).unwrap();
    assert_eq!(selected(&s), all[..2].to_vec());
    // Ctrl+click one further down, then Ctrl+Shift+click: that range joins what was selected.
    s.run("list.select", &json!({"key": s.summary(&s.row_keys()[4]).unwrap().key.to_id(), "add": true})).unwrap();
    let mut add = key_of(&s, &all[5]);
    add["add"] = json!(true);
    s.run("list.selectRange", &add).unwrap();
    let mut want = all[..2].to_vec();
    want.extend(all[4..6].iter().cloned());
    assert_eq!(selected(&s), want);
    assert!(s.run("list.selectRange", &json!({"key": "nobody:1:1:INBOX"})).is_err());
    done(s);
}

/// Ctrl+click: the anchor and the focus go to the message clicked; one left is read; Shift+Down
/// then goes on from there.
#[test]
fn ctrl_click_moves_the_anchor() {
    let mut s = sample_session("ctrl-click");
    let all = rows(&s);
    s.run("list.open", &key_of(&s, &all[0])).unwrap();
    let mut k = key_of(&s, &all[3]);
    k["add"] = json!(true);
    s.run("list.select", &k).unwrap();
    assert_eq!(selected(&s), vec![all[0].clone(), all[3].clone()]);
    assert!(s.reading.is_none());
    s.run("list.extendNext", &json!({})).unwrap();
    assert_eq!(selected(&s), all[3..5].to_vec(), "Shift+Down goes on from the message Ctrl+clicked");
    // Ctrl+click it off again: one left, read.
    let mut k = key_of(&s, &all[4]);
    k["add"] = json!(true);
    s.run("list.select", &k).unwrap();
    assert_eq!(selected(&s), vec![all[3].clone()]);
    assert!(s.reading.as_ref().is_some_and(|r| s.summary(&r.key).is_some_and(|m| m.subject == all[3])));
    done(s);
}

/// With conversations on, a conversation is one row: Shift+Down goes over it as one, and a
/// message read from inside one extends from its conversation's row.
#[test]
fn conversations_are_one_row() {
    let mut s = sample_session("shift-threads");
    s.run("view.conversations", &json!({"on": true})).unwrap();
    let keys = s.row_keys();
    let Some(t) = s.threads.iter().find(|t| t.messages.len() > 1).cloned() else { panic!("the sample has a conversation") };
    let newest = keys.iter().position(|k| t.messages.iter().any(|&i| s.list.get(i).is_some_and(|m| &m.key == k))).unwrap();
    // An older message of it, read from the conversation above the newest.
    let older = t.messages.iter().filter_map(|&i| s.list.get(i)).map(|m| m.key.clone()).find(|k| *k != keys[newest]).unwrap();
    s.open_message(&older);
    s.run("list.extendNext", &json!({})).unwrap();
    let want: Vec<MessageKey> = keys.get(newest..=newest + 1).map(<[MessageKey]>::to_vec).unwrap_or_default();
    assert_eq!(s.selection.len(), want.len(), "{:?}", s.selection);
    assert!(want.iter().all(|k| s.selection.contains(k)));
    // Down from it goes to the row after its conversation's (it went back to the top).
    s.open_message(&older);
    s.run("list.next", &json!({})).unwrap();
    assert_eq!(s.focus.as_ref(), keys.get(newest + 1));
    done(s);
}

/// The anchor goes with its message: one that leaves the list (filtered out) leaves Shift+Down
/// starting afresh, not from a message no longer there.
#[test]
fn the_anchor_leaves_with_its_message() {
    let mut s = sample_session("anchor-gone");
    let keys = s.row_keys();
    let i = keys.iter().position(|k| s.summary(k).is_some_and(|m| !m.flags.flagged)).unwrap();
    s.run("list.open", &json!({"key": keys[i].to_id()})).unwrap();
    s.run("list.extendNext", &json!({})).unwrap();
    let anchor = s.anchor.clone().unwrap();
    assert!(anchor == keys[i] && s.reading.is_none());
    // Only what's flagged: the anchor (not flagged) goes.
    s.run("list.filter", &json!({"value": "flagged"})).unwrap();
    assert!(s.summary(&anchor).is_none(), "it left the list");
    assert!(s.anchor.is_none() && s.focus.as_ref().is_none_or(|k| s.summary(k).is_some()));
    s.run("list.extendNext", &json!({})).unwrap();
    assert!(s.selection.iter().all(|k| s.summary(k).is_some()));
    done(s);
}

/// Ctrl+A is several selected like Shift's: the reading pane lets go of the message it read and
/// Reply waits for one (it answered the message read before; found reviewing issue #26). An empty
/// list's Ctrl+A leaves the pane as it is.
#[test]
fn select_all_is_several_selected() {
    let mut s = sample_session("select-all");
    let keys = s.row_keys();
    s.run("list.open", &json!({"key": keys[1].to_id()})).unwrap();
    s.run("list.selectAll", &json!({})).unwrap();
    assert_eq!(s.selection.len(), s.list.len());
    assert!(s.reading.is_none(), "several selected: none is read");
    assert!(s.run("mail.reply", &json!({})).is_err(), "Reply waits for one message");
    // Shift+Down after it goes on from where the selection started.
    s.run("list.extendNext", &json!({})).unwrap();
    assert_eq!(s.selection, keys[1..3].to_vec());
    // Nothing in the list: nothing to select, and nothing let go of.
    s.run("list.open", &json!({"key": keys[0].to_id()})).unwrap();
    s.run("search.run", &json!({"query": "nothing-at-all-matches-this"})).unwrap();
    let reading = s.reading.as_ref().map(|r| r.key.clone());
    s.run("list.selectAll", &json!({})).unwrap();
    assert_eq!(s.reading.as_ref().map(|r| r.key.clone()), reading);
    done(s);
}

/// Down held faster than a frame: the presses queued in one go as `list.next {"by": n}`, which
/// lands where `n` single steps do but reads only the message landed on; those passed over keep
/// their flags (Outlook's list doesn't mark them read either). `by` that isn't a count of at
/// least one is one step; a huge one stops at the end.
#[test]
fn several_steps_read_only_the_message_landed_on() {
    let mut s = sample_session("steps-by");
    let all = rows(&s);
    assert!(all.len() >= 6, "{all:?}");
    s.run("list.selectAll", &json!({})).unwrap();
    s.run("mail.markUnread", &json!({})).unwrap();
    s.run("list.first", &json!({})).unwrap();
    let unread = |s: &Session, i: usize| s.summary(&s.row_keys()[i]).is_some_and(|m| !m.flags.seen);
    assert!(!unread(&s, 0) && unread(&s, 1) && unread(&s, 3));
    let shown = s.messages_shown();
    s.run("list.next", &json!({"by": 3})).unwrap();
    assert_eq!(focus(&s), all[3]);
    assert_eq!(selected(&s), vec![all[3].clone()]);
    assert_eq!(s.messages_shown(), shown + 1, "one message shown");
    assert!(unread(&s, 1) && unread(&s, 2), "passed over: still unread");
    assert!(!unread(&s, 3), "the one landed on is read");
    // As far as three single steps.
    let mut t = sample_session("steps-one-by-one");
    t.run("list.first", &json!({})).unwrap();
    for _ in 0..3 {
        t.run("list.next", &json!({})).unwrap();
    }
    assert_eq!(focus(&t), focus(&s));
    done(t);
    // Up, and Shift with them.
    s.run("list.prev", &json!({"by": 2})).unwrap();
    assert_eq!(focus(&s), all[1]);
    s.run("list.extendNext", &json!({"by": 2})).unwrap();
    assert_eq!(selected(&s), all[1..4].to_vec());
    s.run("list.extendPrev", &json!({"by": 1})).unwrap();
    assert_eq!(selected(&s), all[1..3].to_vec());
    // Junk: one step; huge: to the end.
    s.run("list.first", &json!({})).unwrap();
    for junk in [json!(0), json!(-4), json!("3"), json!(null), json!(2.5e-3), json!([1])] {
        let at = s.row_keys().iter().position(|k| Some(k) == s.focus.as_ref()).unwrap();
        s.run("list.next", &json!({"by": junk})).unwrap();
        assert_eq!(focus(&s), all[at + 1], "{junk}");
    }
    s.run("list.first", &json!({})).unwrap();
    s.run("list.next", &json!({"by": 1e300})).unwrap();
    assert_eq!(focus(&s), all[all.len() - 1]);
    s.run("list.prev", &json!({"by": u64::MAX})).unwrap();
    assert_eq!(focus(&s), all[0]);
    done(s);
}

/// Nothing focused yet (a folder just opened, a search just run): a step of n lands where n
/// single steps do, the first of them on the top row; Shift with it selects all of them.
#[test]
fn several_steps_from_nothing_focused() {
    let mut one = sample_session("steps-none-singles");
    let all = rows(&one);
    assert!(one.focus.is_none() && all.len() >= 6, "{all:?}");
    for _ in 0..3 {
        one.run("list.next", &json!({})).unwrap();
    }
    assert_eq!(focus(&one), all[2]);
    done(one);
    let mut s = sample_session("steps-none-by");
    s.run("list.next", &json!({"by": 3})).unwrap();
    assert_eq!(focus(&s), all[2], "as three single steps");
    done(s);
    let mut s = sample_session("steps-none-extend");
    s.run("list.extendNext", &json!({"by": 3})).unwrap();
    assert_eq!(selected(&s), all[..3].to_vec());
    done(s);
    // Further than the list goes: the last row; up from nothing: the top, as single Ups.
    let mut s = sample_session("steps-none-ends");
    s.run("list.next", &json!({"by": 1000})).unwrap();
    assert_eq!(focus(&s), all[all.len() - 1]);
    done(s);
    let mut s = sample_session("steps-none-up");
    s.run("list.prev", &json!({"by": 3})).unwrap();
    assert_eq!(focus(&s), all[0]);
    done(s);
}
