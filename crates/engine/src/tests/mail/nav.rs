//! Engine tests: the list's index (finding a message without walking the list), the
//! conversations worked out with it, and arranging the list.

use proptest::prelude::*;

use super::*;
use crate::model::{EmailAddress, MessageKey, Summary};
use crate::session::{Arrange, arrange_list};

mod patch;

/// A session over nothing, its list `list` indexed as `refresh_list` indexes it.
fn indexed(list: Vec<Summary>) -> Session {
    let mut s = Session::open(None, mailcraft_model::dates::Clock::new(NOW, 0));
    s.nav.build(&list, &[], false, false);
    s.list = list;
    s
}

fn key(uid: u32) -> MessageKey {
    MessageKey { account: "a".into(), folder: if uid.is_multiple_of(2) { "INBOX".into() } else { "Archive".into() }, validity: 1, uid }
}

fn message(uid: u32) -> Summary {
    Summary { key: key(uid), date: NOW - i64::from(uid), ..Default::default() }
}

/// Each key, found or not, wherever the list has it: as the first place a walk finds it.
fn agrees(s: &Session, keys: impl IntoIterator<Item = MessageKey>) {
    for k in keys {
        let walked = s.list.iter().position(|m| m.key == k);
        assert_eq!(s.index_of(&k), walked, "{k:?}");
        assert_eq!(s.summary(&k).map(|m| &m.key), walked.map(|_| &k));
    }
}

proptest! {
    /// A list with the same message in it more than once (a search over two folders can't, but
    /// the index mustn't count on it).
    #[test]
    fn index_of_matches_position(uids in prop::collection::vec(0u32..40, 0..120), absent in 40u32..80) {
        let s = indexed(uids.iter().map(|&u| message(u)).collect());
        agrees(&s, (0..40).map(key).chain([key(absent)]));
    }
}

/// The list is public: changed by hand without `refresh_list` (tests and the CLI do), it's
/// looked through instead of trusting an index built for another list.
#[test]
fn index_survives_direct_list_edits() {
    let mut s = indexed((0..50).map(message).collect());
    let all = || (0..60).map(key);
    agrees(&s, all());
    s.list.reverse();
    agrees(&s, all());
    s.list.push(message(55));
    agrees(&s, all());
    s.list.swap(0, 50);
    agrees(&s, all());
    s.list.truncate(10);
    agrees(&s, all());
    s.list.clear();
    agrees(&s, all());
}

/// A list changed by hand that keeps its length (reversed, two swapped) isn't given the
/// conversations worked out for the old order: each message's is the list's own.
#[test]
fn conversations_survive_direct_list_edits() {
    let mut s = sample_session("nav-threads-edited");
    s.run("view.conversations", &json!({"on": true})).unwrap();
    s.run("view.arrange", &json!({"by": "from"})).unwrap();
    assert!(s.list.len() > 3);
    let check = |s: &Session, when: &str| {
        let oracle = mailcraft_model::thread::threads(&s.list);
        assert!(oracle.iter().any(|t| t.messages.len() > 1), "{when}: a conversation of more than one");
        for (i, m) in s.list.iter().enumerate() {
            let want: Vec<MessageKey> = oracle
                .iter()
                .find(|t| t.messages.contains(&i))
                .map(|t| t.messages.iter().filter_map(|&j| s.list.get(j)).map(|x| x.key.clone()).collect())
                .unwrap_or_default();
            assert_eq!(s.thread_of(&m.key), want, "{when}");
        }
    };
    check(&s, "as built");
    s.list.reverse();
    check(&s, "reversed");
    let last = s.list.len() - 1;
    s.list.swap(0, last);
    check(&s, "swapped");
    done(s);
}

/// A list changed by hand since its rows were built still finds an older message of a
/// conversation (no row of its own) on its conversation's row, so a step goes on from there.
#[test]
fn conversation_rows_survive_direct_list_edits() {
    let mut s = sample_session("nav-rows-edited");
    s.run("view.conversations", &json!({"on": true})).unwrap();
    let own: Vec<MessageKey> = (0..s.row_count()).filter_map(|p| s.row_key(p).cloned()).collect();
    let older = s.list.iter().map(|m| m.key.clone()).find(|k| !own.contains(k)).expect("a message without a row of its own");
    let (row, at) = (s.msg_row_of(&older), s.row_index_of(&older));
    assert!(row.is_some() && at.is_some(), "its conversation's row");
    s.list.push(message(1));
    assert_eq!(s.msg_row_of(&older), row, "after an edit by hand");
    assert_eq!(s.row_index_of(&older), at, "after an edit by hand");
    done(s);
}

/// What's drawn each frame finds the selection without copying it, in list order where it says
/// so; `selected` copies, and counts what it copied.
#[test]
fn the_selection_is_read_without_copies() {
    let mut s = indexed((0..20_000).map(message).collect());
    s.selection = s.list.iter().rev().map(|m| m.key.clone()).collect();
    assert_eq!(s.first_selected().map(|m| &m.key), s.list.first().map(|m| &m.key), "highest in the list, not first selected");
    assert_eq!(s.selected_iter().count(), 20_000);
    assert_eq!(s.summary_clones(), 0);
    s.selection = vec![key(9), key(3), key(7)];
    let picked: Vec<u32> = s.selected().iter().map(|m| m.key.uid).collect();
    assert_eq!(picked, [3, 7, 9], "in list order");
    assert_eq!(s.selected_iter().map(|m| m.key.uid).collect::<Vec<_>>(), [9, 3, 7], "in the order selected");
    assert_eq!(s.first_selected().map(|m| m.key.uid), Some(3));
    assert_eq!(s.summary_clones(), 3);
    s.selection = vec![key(30_000)];
    assert!(s.first_selected().is_none() && s.selected_iter().next().is_none() && s.selected().is_empty());
}

/// With conversations on, the open message's conversation is worked out with the list, once,
/// whatever the list is arranged by (arranged by sender, it was worked out again for every
/// message opened: each Down went over the whole list).
#[test]
fn conversations_are_worked_out_with_the_list() {
    let mut s = sample_session("nav-threads");
    s.run("view.conversations", &json!({"on": true})).unwrap();
    for by in ["from", "date", "subject"] {
        s.run("view.arrange", &json!({"by": by})).unwrap();
        let built = s.nav.threadings();
        for m in s.list.clone() {
            s.open_message(&m.key);
            let oracle = mailcraft_model::thread::threads(&s.list);
            let Some(i) = s.list.iter().position(|x| x.key == m.key) else { continue };
            let want: Vec<MessageKey> = oracle
                .iter()
                .find(|t| t.messages.contains(&i))
                .map(|t| t.messages.iter().filter_map(|&j| s.list.get(j)).map(|x| x.key.clone()).collect())
                .unwrap_or_default();
            assert_eq!(s.reading.as_ref().map(|r| r.thread.clone()), Some(want), "arranged by {by}");
        }
        assert!(s.list.len() > 3);
        // Opening marks read, which builds the list again (each once); nothing else does.
        assert!(s.nav.threadings() - built <= s.list.len() as u64, "arranged by {by}");
    }
    // Conversations off: none, and none worked out.
    s.run("view.conversations", &json!({"on": false})).unwrap();
    let built = s.nav.threadings();
    let k = s.list[0].key.clone();
    s.open_message(&k);
    assert!(s.reading.as_ref().is_some_and(|r| r.thread.is_empty()));
    assert_eq!(s.nav.threadings(), built);
    done(s);
}

/// Opening a message already read doesn't build the list again, and finds its conversation
/// without working the conversations out again.
#[test]
fn opening_read_mail_works_nothing_out() {
    let mut s = sample_session("nav-threads-once");
    s.run("view.conversations", &json!({"on": true})).unwrap();
    s.run("view.arrange", &json!({"by": "from"})).unwrap();
    let keys: Vec<MessageKey> = s.list.iter().map(|m| m.key.clone()).collect();
    for k in &keys {
        s.open_message(k);
    }
    // A read mark changes the list as it stands: the conversations worked out are kept.
    let built = s.nav.threadings();
    for k in keys.iter().chain(&keys) {
        s.open_message(k);
        assert!(s.reading.as_ref().is_some_and(|r| r.thread.contains(k)), "a message is in its own conversation");
    }
    assert_eq!(s.nav.threadings(), built);
    done(s);
}

/// Arranged by sender, the list shows no conversations: building it again (a sync, a flag
/// changed) works none out until a message is opened, and then once for the list.
#[test]
fn rebuilding_a_list_without_conversations_shown_works_none_out() {
    let mut s = sample_session("nav-threads-lazy");
    s.run("view.conversations", &json!({"on": true})).unwrap();
    s.run("view.arrange", &json!({"by": "from"})).unwrap();
    s.reading = None;
    let (account, folder) = (s.list[0].key.account.clone(), s.list[0].key.folder.clone());
    let built = s.nav.threadings();
    for _ in 0..5 {
        s.apply(Event::FolderChanged { account: account.clone(), folder: folder.clone() });
    }
    assert!(s.threads.is_empty());
    assert_eq!(s.nav.threadings(), built, "rebuilt with nothing read");
    let keys: Vec<MessageKey> = s.list.iter().map(|m| m.key.clone()).collect();
    for k in &keys {
        assert!(s.thread_of(k).contains(k));
    }
    assert_eq!(s.nav.threadings(), built + 1, "once for every message looked at");
    // Arranged by date the list shows them: worked out with each rebuild.
    s.run("view.arrange", &json!({"by": "date"})).unwrap();
    let built = s.nav.threadings();
    s.apply(Event::FolderChanged { account, folder });
    assert_eq!(s.nav.threadings(), built + 1);
    assert!(!s.threads.is_empty());
    done(s);
}

/// A folder of `n` read messages over a day and more, in conversations of three, some pinned,
/// from a handful of senders and sizes, listed by a session over a store in memory.
fn big_folder(n: u32) -> Session {
    let mut s = Session::open(Some(mailcraft_store::Store::memory()), mailcraft_model::dates::Clock::new(NOW, 0));
    let messages: Vec<Summary> = (0..n)
        .map(|uid| {
            let mut m = message(uid);
            m.key.folder = "INBOX".into();
            m.date = NOW - i64::from(uid) * 37;
            m.message_id = format!("<m{uid}@x>");
            if uid % 3 != 0 {
                m.in_reply_to = format!("<m{}@x>", uid - uid % 3);
                m.subject = format!("Re: Plan {}", uid / 3);
            } else {
                m.subject = format!("Plan {}", uid / 3);
            }
            m.from = EmailAddress::new(format!("Person {}", uid % 7), format!("p{}@x", uid % 7));
            m.size = (uid % 5) * 40_000;
            m.flags.seen = true;
            m.flags.flagged = uid % 11 == 0;
            m.has_attachments = uid % 4 == 0;
            m.flags.set_keyword(mailcraft_model::PINNED, uid % 997 == 5);
            m
        })
        .collect();
    let st = mailcraft_store::FolderState { messages: messages.into_iter().map(Into::into).collect(), ..Default::default() };
    s.cache.insert(("a".into(), "INBOX".into()), st);
    s.open_view(View::Folder { account: "a".into(), folder: "INBOX".into() });
    s
}

/// Each message's row is its own, or its conversation's when the list shows conversations.
fn rows_hold_their_messages(s: &Session, when: &str) {
    let rows = s.rows();
    let mut thread_of = vec![usize::MAX; s.list.len()];
    for (t, thread) in s.threads.iter().enumerate() {
        for &i in &thread.messages {
            thread_of[i] = t;
        }
    }
    for (i, m) in s.list.iter().enumerate() {
        let r = s.row_index_of(&m.key).unwrap_or_else(|| panic!("{when}: no row for {i}"));
        let Some(Row::Message { index, .. }) = rows.get(r) else { panic!("{when}: row {r} is not a message") };
        assert!(*index == i || (thread_of[i] != usize::MAX && thread_of[i] == thread_of[*index]), "{when}: {i} on {index}'s row");
    }
}

/// Down through a big folder looks its rows up: they're built with the list, not again for
/// each step (each Down built every row and copied every key), however the list is arranged,
/// with conversations on or off; and it visits the rows in the order the list shows them.
#[test]
fn move_focus_never_rebuilds_rows() {
    let mut s = big_folder(20_000);
    for conversations in [false, true] {
        s.run("view.conversations", &json!({"on": conversations})).unwrap();
        for arrange in Arrange::ALL {
            s.run("view.arrange", &json!({"by": arrange.code()})).unwrap();
            let when = format!("{arrange:?}, conversations {conversations}");
            s.focus = None;
            let order = s.row_keys();
            assert!(order.len() > 500, "{when}");
            rows_hold_their_messages(&s, &when);
            let (built, revision) = (s.nav_structure(), s.revision);
            let mut visited = Vec::new();
            let started = std::time::Instant::now();
            for _ in 0..500 {
                s.run("list.next", &json!({})).unwrap();
                visited.extend(s.focus.clone());
            }
            let stepped = started.elapsed();
            let started = std::time::Instant::now();
            drop(s.build_rows());
            eprintln!("{when}: 500 Downs in {stepped:?}; building the rows (each Down's share before) {:?}", started.elapsed());
            assert_eq!(s.nav_structure(), built, "{when}: no rows built");
            assert_eq!(s.revision, revision, "{when}");
            assert_eq!(visited, order[..500].to_vec(), "{when}");
            // Shift+Up back over them selects as many rows, in a range that's looked up too.
            for _ in 0..99 {
                s.run("list.extendPrev", &json!({})).unwrap();
            }
            assert_eq!(s.selection.len(), 100, "{when}");
            assert_eq!(s.focus.as_ref(), order.get(400), "{when}");
            assert_eq!(s.nav_structure(), built, "{when}: no rows built");
        }
    }
}

/// The date headings follow the day: past midnight the list's Today is Yesterday, though
/// nothing else changed and no snoozed mail is due (`tick` returned early for those).
#[test]
fn rows_follow_the_day() {
    let mut s = sample_session("nav-day");
    assert_eq!(s.rows()[0], Row::Header("Today"));
    let (built, revision) = (s.nav_structure(), s.revision);
    s.tick();
    assert_eq!(s.nav_structure(), built, "the same day builds nothing");
    assert_eq!(s.revision, revision, "the same day builds nothing");
    let today = mailcraft_model::dates::today(&s.clock);
    let midnight = (today + 1) * 86_400 - i64::from(s.clock.offset);
    s.set_clock(midnight + 60, s.clock.offset);
    assert_eq!(s.rows()[0], Row::Header("Today"), "until the next tick");
    s.tick();
    assert_eq!(s.rows()[0], Row::Header("Yesterday"));
    assert!(!s.rows().contains(&Row::Header("Today")));
    assert_ne!(s.nav_structure(), built);
    assert_ne!(s.revision, revision, "the interface lays the rows out again");
    let built = s.nav_structure();
    s.tick();
    assert_eq!(s.nav_structure(), built, "once a day");
    done(s);
}

/// The comparator `arrange_list` used before names were lower-cased once each: the oracle.
fn arranged_before(list: &mut [Summary], arrange: Arrange, reverse: bool) {
    let person = |a: Option<&EmailAddress>| {
        a.map(|a| if a.name.trim().is_empty() { a.email.trim().to_string() } else { a.name.trim().to_string() }).unwrap_or_default()
    };
    let text = |s: String| s.to_lowercase();
    list.sort_by(|a, b| {
        let primary = match arrange {
            Arrange::Date => b.date.cmp(&a.date),
            Arrange::From => text(person(Some(&a.from))).cmp(&text(person(Some(&b.from)))),
            Arrange::To => text(person(a.to.first())).cmp(&text(person(b.to.first()))),
            Arrange::Subject => text(a.base_subject()).cmp(&text(b.base_subject())),
            Arrange::Size => b.size.cmp(&a.size),
            Arrange::Flag => b.flags.flagged.cmp(&a.flags.flagged),
            Arrange::Attachments => b.has_attachments.cmp(&a.has_attachments),
        };
        let primary = if reverse { primary.reverse() } else { primary };
        primary.then_with(|| b.date.cmp(&a.date)).then_with(|| b.key.cmp(&a.key))
    });
}

fn address() -> impl Strategy<Value = EmailAddress> {
    (prop::sample::select(vec!["", " ", "Ann", "ann", "ANN ", "Bob", "Zoë", "Émile"]), prop::sample::select(vec!["", "a@x", "B@x", " c@x"]))
        .prop_map(|(n, e)| EmailAddress::new(n.to_string(), e.to_string()))
}

fn arrangeable() -> impl Strategy<Value = Vec<Summary>> {
    let one = (
        address(),
        prop::collection::vec(address(), 0..2),
        prop::sample::select(vec!["", "Plan", "plan", "Re: Plan", "RE: plan", "Fwd: budget", "Budget", "Ünder"]),
        0i64..4,
        prop::sample::select(vec![0u32, 9_000, 30_000, 2_000_000]),
        any::<(bool, bool)>(),
    );
    prop::collection::vec(one, 0..40).prop_map(|v| {
        v.into_iter()
            .enumerate()
            .map(|(i, (from, to, subject, date, size, (flagged, files)))| {
                let mut m = message(u32::try_from(i).unwrap_or(0));
                m.from = from;
                m.to = to;
                m.subject = subject.to_string();
                m.date = date;
                m.size = size;
                m.flags.flagged = flagged;
                m.has_attachments = files;
                m
            })
            .collect()
    })
}

proptest! {
    /// Lower-casing each name once arranges the list exactly as before, every way round.
    #[test]
    fn arrange_cached_matches_old_comparator(list in arrangeable()) {
        for arrange in Arrange::ALL {
            for reverse in [false, true] {
                let (mut now, mut before) = (list.clone(), list.clone());
                arrange_list(&mut now, arrange, reverse);
                arranged_before(&mut before, arrange, reverse);
                let keys = |l: &[Summary]| l.iter().map(|m| m.key.clone()).collect::<Vec<_>>();
                prop_assert_eq!(keys(&now), keys(&before), "{:?} reversed {}", arrange, reverse);
            }
        }
    }
}
