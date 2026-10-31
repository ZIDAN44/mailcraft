//! Engine tests: the list's index (finding a message without walking the list), the
//! conversations worked out with it, and arranging the list.

use proptest::prelude::*;

use super::*;
use crate::model::{EmailAddress, MessageKey, Summary};
use crate::session::{Arrange, arrange_list};

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
    // The last read mark built the list again: its conversations come with the first look.
    let built = s.nav.threadings() + 1;
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
