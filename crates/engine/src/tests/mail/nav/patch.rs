//! Engine tests: a flag or category changed on mail in the list is changed in the list as it
//! stands, which must be the list built again.

use super::*;
use crate::model::PINNED;

/// What a step can do: change a flag or a category on some messages, or open one.
#[derive(Clone, Debug)]
enum Step {
    Flag(Vec<u32>, Flag, bool),
    Keyword(Vec<u32>, &'static str, bool),
    Open(u32),
}

fn step(n: u32) -> impl Strategy<Value = Step> {
    let uids = prop::collection::vec(0..n.max(1), 1..4);
    let flag = prop::sample::select(vec![Flag::Seen, Flag::Seen, Flag::Flagged, Flag::Answered, Flag::Deleted]);
    let keyword = prop::sample::select(vec![PINNED, "work", "$Junk"]);
    prop_oneof![
        3 => (uids.clone(), flag, any::<bool>()).prop_map(|(u, f, on)| Step::Flag(u, f, on)),
        2 => (uids, keyword, any::<bool>()).prop_map(|(u, k, on)| Step::Keyword(u, k, on)),
        3 => (0..n.max(1)).prop_map(Step::Open),
    ]
}

/// A folder of `n` messages: dates that tie, conversations that form, flags and categories
/// at random (from `bits`).
fn folder(bits: &[(u8, u8)]) -> Vec<Summary> {
    bits.iter()
        .enumerate()
        .map(|(i, &(b, parent))| {
            let uid = u32::try_from(i).unwrap();
            let mut m = message(uid);
            m.key.folder = "INBOX".into();
            m.date = NOW - i64::from(b % 5) * 3_600;
            m.message_id = format!("<m{uid}@x>");
            if b & 0x20 != 0 && u32::from(parent) < uid {
                m.in_reply_to = format!("<m{parent}@x>");
            }
            m.from = EmailAddress::new(format!("Person {}", b % 3), format!("p{}@x", b % 3));
            m.size = u32::from(b) * 1_000;
            m.flags.seen = b & 1 != 0;
            m.flags.flagged = b & 2 != 0;
            m.has_attachments = b & 4 != 0;
            m.flags.set_keyword(PINNED, b & 8 != 0);
            m.flags.set_keyword("work", b & 0x10 != 0);
            m
        })
        .collect()
}

/// What the list shows, to compare with it built again.
type Shown = (Vec<crate::model::SharedSummary>, Vec<Row>, Vec<MessageKey>, Option<MessageKey>, Option<MessageKey>);

fn shown(s: &Session) -> Shown {
    (s.list.clone(), s.rows().to_vec(), s.selection.clone(), s.focus.clone(), s.reading.as_ref().map(|r| r.key.clone()))
}

fn key_in_inbox(uid: u32) -> MessageKey {
    MessageKey { account: "a".into(), folder: "INBOX".into(), validity: 1, uid }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Each change, opened message and step taken into the list as it stands leaves it as
    /// building it again would: the messages, their rows, the selection, the focus and the
    /// message read; filtered, arranged and grouped every way.
    #[test]
    fn patched_list_equals_refresh(
        bits in prop::collection::vec(any::<(u8, u8)>(), 1..300),
        conversations in any::<bool>(),
        arrange in prop::sample::select(Arrange::ALL.to_vec()),
        reverse in any::<bool>(),
        filter in prop::sample::select(vec![ListFilter::All, ListFilter::Unread, ListFilter::Flagged, ListFilter::Categorized, ListFilter::Attachments]),
        steps in prop::collection::vec(step(300), 1..40),
    ) {
        let messages = folder(&bits);
        let n = u32::try_from(messages.len()).unwrap();
        let mut s = Session::open(Some(mailcraft_store::Store::memory()), mailcraft_model::dates::Clock::new(NOW, 0));
        s.cache.insert(("a".into(), "INBOX".into()), mailcraft_store::FolderState { validity: 1, messages: messages.into_iter().map(Into::into).collect(), ..Default::default() });
        s.prefs.conversations = conversations;
        s.prefs.arrange = arrange;
        s.prefs.reverse_sort = reverse;
        s.filter = filter;
        s.open_view(View::Folder { account: "a".into(), folder: "INBOX".into() });
        for (at, st) in steps.iter().enumerate() {
            let keys = |u: &[u32]| u.iter().map(|&u| key_in_inbox(u % n)).collect::<Vec<_>>();
            match st {
                Step::Flag(u, flag, on) => s.apply_op("a", Op::SetFlag { keys: keys(u), flag: *flag, on: *on }),
                Step::Keyword(u, k, on) => s.apply_op("a", Op::SetKeyword { keys: keys(u), keyword: (*k).to_string(), on: *on }),
                Step::Open(u) => {
                    let k = key_in_inbox(u % n);
                    if s.index_of(&k).is_some() {
                        s.open_message(&k);
                    }
                }
            }
            let patched = shown(&s);
            s.refresh_list();
            prop_assert_eq!(&patched, &shown(&s), "after step {} ({:?})", at, st);
        }
    }
}

/// `n` unread messages in conversations, as `big_folder` makes them.
fn unread_folder(n: u32) -> Session {
    let mut s = big_folder(n);
    if let Some(st) = s.cache.get_mut(&("a".to_string(), "INBOX".to_string())) {
        st.validity = 1;
        for m in st.messages.iter_mut().map(std::sync::Arc::make_mut) {
            m.flags.seen = false;
        }
    }
    s.refresh_list();
    s
}

/// Opening message after message in a big folder marks each read without building the list
/// again (each read mark built every row of it, worked its conversations out and copied every
/// message out of the folder); what the rows count unread is as built afresh.
#[test]
fn marking_read_does_not_rebuild() {
    let mut s = unread_folder(10_941);
    for conversations in [false, true] {
        s.run("view.conversations", &json!({"on": conversations})).unwrap();
        let when = format!("conversations {conversations}");
        let keys: Vec<MessageKey> = s.row_keys().into_iter().filter(|k| s.summary(k).is_some_and(|m| !m.flags.seen)).take(100).collect();
        assert_eq!(keys.len(), 100, "{when}");
        let (built, threadings, revision) = (s.nav_structure(), s.nav.threadings(), s.revision);
        let started = std::time::Instant::now();
        for k in &keys {
            s.open_message(k);
        }
        eprintln!("{when}: 100 opened and marked read in {:?}", started.elapsed());
        assert!(keys.iter().all(|k| s.summary(k).is_some_and(|m| m.flags.seen)), "{when}: marked read");
        assert_eq!(s.nav_structure(), built, "{when}: no rows built");
        assert_eq!(s.nav.threadings(), threadings, "{when}: no conversations worked out");
        assert_ne!(s.revision, revision, "{when}: the interface draws the change");
        let (rows, _, _) = s.build_rows();
        assert_eq!(s.rows(), &rows[..], "{when}: unread counts as built afresh");
    }
}

/// Shown Unread only, the message read stays while it's read and goes when the next is opened,
/// one row at a time; the list is the one built again.
#[test]
fn unread_filter_drops_the_message_read_before() {
    let mut s = unread_folder(2_000);
    s.run("list.filter", &json!({"value": "unread"})).unwrap();
    let keys: Vec<MessageKey> = s.row_keys().into_iter().take(20).collect();
    let total = s.list.len();
    for (i, k) in keys.iter().enumerate() {
        s.open_message(k);
        assert!(s.index_of(k).is_some(), "the one read stays");
        assert_eq!(s.list.len(), total - i, "the one read before goes");
    }
    let patched = shown(&s);
    s.refresh_list();
    assert_eq!(patched, shown(&s));
}

/// Arranged by flag, a flag moves the message to its group; a pin to the top.
#[test]
fn flag_and_pin_move_their_rows() {
    let mut s = unread_folder(500);
    s.run("view.arrange", &json!({"by": "flag"})).unwrap();
    let k = s.list.iter().rev().find(|m| !m.flags.flagged).map(|m| m.key.clone()).unwrap();
    s.apply_op("a", Op::SetFlag { keys: vec![k.clone()], flag: Flag::Flagged, on: true });
    assert!(s.index_of(&k).unwrap() < s.list.iter().filter(|m| m.flags.flagged).count(), "among the flagged");
    let patched = shown(&s);
    s.refresh_list();
    assert_eq!(patched, shown(&s));
    s.apply_op("a", Op::SetKeyword { keys: vec![k.clone()], keyword: PINNED.to_string(), on: true });
    assert_eq!(s.rows()[0], Row::Header("Pinned"));
    assert_eq!(s.row_index_of(&k), Some(1), "pinned at the top");
    let patched = shown(&s);
    s.refresh_list();
    assert_eq!(patched, shown(&s));
}
