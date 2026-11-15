//! The drafts kept on this computer as they're kept: a short list, each source a file of its own;
//! and those an earlier version kept, each source in the list itself, moved into it with none
//! lost or doubled, whenever MailCraft stops.

use super::drafts::{kept_texts, on_the_server, with_drafts};
use super::*;

fn text(n: usize) -> String {
    format!("From: me@studio.example\r\nSubject: Plans {n}\r\nMessage-ID: <old{n}@studio.example>\r\n\r\nwritten before\r\n")
}

/// The kept drafts' list as an earlier version wrote it: each source in it, a byte a number.
fn kept_by_an_earlier_version(store: &Store, texts: &[String]) {
    let old: Vec<PendingDraft> =
        texts.iter().map(|t| PendingDraft { folder: "Drafts".into(), replaces: None, raw: t.clone().into_bytes() }).collect();
    store.save_account_doc("a1", PENDING_DRAFTS, &old).unwrap();
    let written = store.load_account_doc_bytes("a1", PENDING_DRAFTS).unwrap().unwrap();
    assert!(written.starts_with(br#"[{"folder":"Drafts","replaces":null,"raw":[70,114,111,109,"#), "{}", String::from_utf8_lossy(&written));
}

fn old_list_there(store: &Store) -> bool {
    store.load_account_doc_bytes("a1", PENDING_DRAFTS).unwrap().is_some()
}

/// Drafts an earlier version kept are moved into the list as it's read, ahead of one kept after
/// them, and all filed at the next connection, in order; the old list goes, and each source with
/// its draft. The list itself stays small whatever the drafts' size.
#[test]
fn drafts_an_earlier_version_kept_are_moved_and_filed() {
    let (st, store, account) = with_drafts("kept-moved");
    let mut texts: Vec<String> = (1..=3).map(text).collect();
    kept_by_an_earlier_version(&store, &texts);
    let big = format!("From: me@studio.example\r\nSubject: Big\r\nMessage-ID: <big@studio.example>\r\n\r\n{}\r\n", "x".repeat(256 << 10));
    keep_draft(&store, "a1", "Drafts", None, big.clone().into_bytes(), false, &Order::default()).unwrap();
    texts.push(big);
    assert_eq!(kept_texts(&store), texts);
    assert!(!old_list_there(&store), "the old list removed once moved");
    assert_eq!(store.kept_draft_ids("a1").len(), 4, "a source each");
    let list = store.load_account_doc_bytes("a1", KEPT_DRAFTS).unwrap().unwrap();
    assert!(list.len() < 1500, "the list is {} bytes", list.len());
    let mut pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    pool.submit(Job::Start { account, password: Some(Secret("pw".into())) });
    eventually("every draft filed", || on_the_server(&st).len() == 4 && kept_texts(&store).is_empty());
    assert_eq!(on_the_server(&st), texts);
    assert!(store.kept_draft_ids("a1").is_empty(), "no source left");
    pool.submit(Job::Remove { account: "a1".into() });
    drop(pool);
}

/// Stopped as the drafts were moved, after the new list was written and before the old one was
/// removed: both are there, and read again, the drafts aren't doubled; each is filed once.
#[test]
fn stopped_after_the_move_the_drafts_are_not_doubled() {
    let (st, store, account) = with_drafts("kept-moved-twice");
    let texts: Vec<String> = (1..=3).map(text).collect();
    kept_by_an_earlier_version(&store, &texts);
    assert_eq!(kept_texts(&store), texts);
    // The old list as it was before it was removed.
    kept_by_an_earlier_version(&store, &texts);
    assert_eq!(kept_texts(&store), texts, "not doubled");
    assert!(!old_list_there(&store));
    assert_eq!(store.kept_draft_ids("a1").len(), 3, "no source written twice");
    let mut pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    pool.submit(Job::Start { account, password: Some(Secret("pw".into())) });
    eventually("every draft filed", || on_the_server(&st).len() == 3 && kept_texts(&store).is_empty());
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(on_the_server(&st), texts, "each filed once");
    pool.submit(Job::Remove { account: "a1".into() });
    drop(pool);
}

/// The move failing part way (the new list, or a source, can't be written): nothing is lost,
/// nothing left over, and the next read moves them, once.
#[test]
fn a_move_that_fails_loses_nothing() {
    let store = Store::failing();
    let texts: Vec<String> = (1..=2).map(text).collect();
    kept_by_an_earlier_version(&store, &texts);
    let newer = text(3);
    for fails in [KEPT_DRAFTS, ".eml"] {
        store.fail(None, Some(fails));
        assert!(keep_draft(&store, "a1", "Drafts", None, newer.clone().into_bytes(), false, &Order::default()).is_err(), "{fails}");
        store.fail(None, None);
        assert!(old_list_there(&store), "{fails}: the old list stays");
        assert!(store.kept_draft_ids("a1").is_empty(), "{fails}: no source left over");
    }
}

/// The old list can't be removed once its drafts were moved (another program has it open): the
/// drafts are kept and read all the same (keeping one failed, and none was filed, for as long as
/// it stayed), and it isn't moved a second time: a draft filed meanwhile doesn't come back.
/// It goes at the first read that can remove it; another old list after it is moved as any.
#[test]
fn an_old_list_that_cant_be_removed_is_moved_once() {
    let store = Store::failing();
    let texts: Vec<String> = (1..=2).map(text).collect();
    kept_by_an_earlier_version(&store, &texts);
    let newer = text(3);
    store.fail(None, Some(PENDING_DRAFTS));
    keep_draft(&store, "a1", "Drafts", None, newer.clone().into_bytes(), false, &Order::default()).unwrap();
    assert_eq!(kept_texts(&store), [texts[0].clone(), texts[1].clone(), newer.clone()]);
    assert!(old_list_there(&store));
    // The first is filed: off the list, as its filing takes it.
    let mut list: Vec<serde_json::Value> = store.load_account_doc("a1", KEPT_DRAFTS).unwrap();
    list.remove(0);
    store.save_account_doc("a1", KEPT_DRAFTS, &list).unwrap();
    assert_eq!(kept_texts(&store), [texts[1].clone(), newer.clone()], "not moved again");
    store.fail(None, None);
    assert_eq!(kept_texts(&store), [texts[1].clone(), newer.clone()]);
    assert!(!old_list_there(&store), "removed once it can be");
    assert!(store.load_account_doc_bytes("a1", "drafts-pending-moved.json").unwrap().is_none(), "and the note of it");
    let later = text(4);
    kept_by_an_earlier_version(&store, std::slice::from_ref(&later));
    assert_eq!(kept_texts(&store), [texts[1].clone(), newer, later]);
    assert!(!old_list_there(&store));
}

/// A source no draft in the list names (MailCraft stopped between writing it and the list) is
/// removed; but not while a damaged list is set aside, whose draft it may be.
#[test]
fn a_source_no_draft_names_is_removed_unless_a_damaged_list_may() {
    let store = crate::fake::store("kept-sweep");
    for n in 1..=2 {
        keep_draft(&store, "a1", "Drafts", None, text(n).into_bytes(), false, &Order::default()).unwrap();
    }
    std::fs::write(store.root().join("a1").join(KEPT_DRAFTS), b"[{\"id\":").unwrap();
    assert!(kept_texts(&store).is_empty(), "a damaged list is set aside");
    sweep_kept(&store, "a1", &[]);
    assert_eq!(store.kept_draft_ids("a1").len(), 2, "its drafts' sources stay");
    std::fs::remove_file(store.root().join("a1").join(format!("{KEPT_DRAFTS}.bad"))).unwrap();
    sweep_kept(&store, "a1", &[]);
    assert!(store.kept_draft_ids("a1").is_empty(), "removed");
}
