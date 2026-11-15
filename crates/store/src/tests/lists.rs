//! Store tests: a folder's list written a page at a time ([`Store::list_writer`]).

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use super::*;

/// A list written a page at a time writes its head under the lists' lock, as every other head
/// written is: a save or a mark under way ends first. It was written without it, so a list read
/// and written again under the lock could have its head changed in between.
#[test]
fn a_list_written_a_page_at_a_time_waits_for_the_lock() {
    let s = Store::failing();
    let n = PAGE_MESSAGES as u32 + 1;
    let msg = |uid| Summary { key: MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 3, uid }, ..Default::default() };
    let mut w = s.list_writer("a1", "INBOX");
    w.push(&s, (1..=n).map(msg).collect()).unwrap();
    let head = FolderState { validity: 3, uid_next: n + 1, ..Default::default() };
    // Each time the head is read or written: whether the lock is held (it can't be taken).
    let (free, held) = (Arc::new(AtomicBool::new(false)), Arc::new(AtomicU32::new(0)));
    let (lock, was_free, was_held) = (s.changes.clone(), free.clone(), held.clone());
    s.watch_files(Some(Arc::new(move |p: &Path| {
        if p.ends_with(folders::HEAD) {
            match lock.try_lock() {
                Err(std::sync::TryLockError::WouldBlock) => drop(was_held.fetch_add(1, Ordering::SeqCst)),
                _ => was_free.store(true, Ordering::SeqCst),
            }
        }
    })));
    let counts = w.finish(&s, &head);
    s.watch_files(None);
    assert_eq!(counts.unwrap(), (n, n));
    assert!(!free.load(Ordering::SeqCst), "the head was read or written without the lock");
    assert_eq!(held.load(Ordering::SeqCst), 2, "the head read, then written, under the lock");
    assert_eq!(s.load_folder("a1", "INBOX").unwrap().messages.len(), n as usize);
}

/// A list of `n` messages in `folder`, of validity 3, newest first.
fn list(folder: &str, n: u32) -> FolderState {
    let msg = |uid| Summary {
        key: MessageKey { account: "a1".into(), folder: folder.into(), validity: 3, uid },
        subject: format!("Report {uid}"),
        ..Default::default()
    };
    let mut st = FolderState { validity: 3, uid_next: n + 1, messages: (1..=n).map(msg).map(Into::into).collect(), ..Default::default() };
    sort_newest_first(&mut st.messages);
    st
}

/// A read of a list's head that fails (not missing, not damaged: the disk refused it) fails the
/// save by [`Store::save_folder`]: taken for a list with no pages,
/// every page was written again and the pages the list no longer has were left behind, with
/// nothing said. A save leaves the list as it was. A [`ListWriter`] has written its pages over
/// the old list's by then, so its head is written all the same: failing there left the old head
/// naming the new list's pages and, after them, the old one's (a list of both, read without an
/// error). The pages it no longer has are found by their files, and go.
#[test]
fn a_head_that_cant_be_read_fails_the_save() {
    let s = Store::failing();
    let page = u32::try_from(PAGE_MESSAGES).unwrap();
    let (three, two) = (list("INBOX", page * 2 + 9), list("INBOX", page + 5));
    s.save_folder("a1", "INBOX", &three).unwrap();
    s.save_folder("a1", "Archive", &list("Archive", page * 2 + 9)).unwrap();
    for folder in ["INBOX", "Archive"] {
        assert!(s.files.is_file(&folders::page_path(&s.folder_dir("a1", folder), 2)));
    }
    // Two pages now, the head unreadable.
    s.fail(Some(folders::HEAD), None);
    assert!(s.save_folder("a1", "INBOX", &two).is_err(), "saved over a head it couldn't read");
    // A writer has written its pages as they filled, over the old list's: its end names them
    // all the same (the old head would name two of the new pages and one of the old), and the
    // page the list no longer has goes.
    let filed = list("Archive", page + 5);
    let mut w = s.list_writer("a1", "Archive");
    w.push(&s, filed.messages.iter().rev().map(|m| (**m).clone()).collect()).unwrap();
    w.finish(&s, &filed).unwrap();
    s.fail(None, None);
    assert_eq!(s.load_folder("a1", "Archive").unwrap(), filed, "one list, the writer's");
    assert!(!s.files.is_file(&folders::page_path(&s.folder_dir("a1", "Archive"), 2)));
    assert_eq!(s.load_folder("a1", "INBOX").unwrap(), three, "as it was");
    // Readable again: saved, and the page it no longer has goes.
    s.save_folder("a1", "INBOX", &two).unwrap();
    assert!(!s.files.is_file(&folders::page_path(&s.folder_dir("a1", "INBOX"), 2)));
    assert_eq!(s.load_folder("a1", "INBOX").unwrap(), two);
}

/// A folder's list written part-way, noted as its tail, and gone on with from that tail (an
/// import that stopped and goes on) comes out as the list written in one go: the same pages,
/// counts and order. A tail that doesn't add up is refused, with nothing written.
#[test]
fn a_list_goes_on_from_its_tail_as_if_never_stopped() {
    let n = PAGE_MESSAGES as u32 * 2 + 700;
    let unread = |uid: u32| msg(uid, !uid.is_multiple_of(3));
    let head = FolderState { validity: 3, uid_next: n + 1, ..Default::default() };
    for stop in [10, PAGE_MESSAGES as u32 - 1, PAGE_MESSAGES as u32 + 5, 2 * PAGE_MESSAGES as u32] {
        let (whole, parts) = (Store::memory(), Store::memory());
        let mut w = whole.list_writer("a1", "INBOX");
        w.push(&whole, (1..=n).map(unread).collect()).unwrap();
        let counts = w.finish(&whole, &head).unwrap();

        let mut w = parts.list_writer("a1", "INBOX");
        w.push(&parts, (1..=stop).map(unread).collect()).unwrap();
        // Messages written since the last page, not yet handed over, go in the tail too.
        let tail = w.tail(&[unread(stop + 1), unread(stop + 2)]);
        drop(w);
        let back: ListTail = serde_json::from_slice(&serde_json::to_vec(&tail).unwrap()).unwrap();
        let mut w = parts.resume_list_writer("a1", "INBOX", back).unwrap();
        w.push(&parts, (stop + 3..=n).map(unread).collect()).unwrap();
        assert_eq!(w.finish(&parts, &head).unwrap(), counts, "stopped at {stop}");
        assert_eq!(parts.load_folder("a1", "INBOX").unwrap(), whole.load_folder("a1", "INBOX").unwrap(), "stopped at {stop}");
        assert_eq!(parts.snapshot(|_| true), whole.snapshot(|_| true), "stopped at {stop}: the same files");
    }
    let s = Store::memory();
    s.track_changes();
    let refused = |tail: ListTail| s.resume_list_writer("a1", "INBOX", tail).is_err();
    assert!(refused(ListTail { pages: vec![1], in_pages: 3, ..Default::default() }), "a page that isn't a full one");
    assert!(refused(ListTail { pages: vec![1], in_pages: PAGE_MESSAGES as u64, unread_in_pages: u32::MAX, ..Default::default() }));
    assert!(refused(ListTail { page: (1..=TAIL_PAGE_MAX as u32 + 1).map(unread).collect(), ..Default::default() }), "too long");
    assert!(refused(ListTail { page: vec![unread(5), unread(4)], ..Default::default() }), "out of order");
    assert!(refused(ListTail {
        pages: vec![0; TAIL_PAGES_MAX + 1],
        in_pages: (TAIL_PAGES_MAX as u64 + 1) * PAGE_MESSAGES as u64,
        ..Default::default()
    }));
    assert!(s.take_changes().is_empty(), "nothing written for a tail refused");
}
