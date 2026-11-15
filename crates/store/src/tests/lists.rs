//! Store tests: a folder's list written a page at a time ([`Store::list_writer`]).

use std::sync::atomic::{AtomicBool, Ordering};

use super::*;

/// A list written a page at a time writes its head under the lists' lock, as every other head
/// written is: a save or a mark under way ends first. It was written without it, so a list read
/// and written again under the lock could have its head changed in between.
#[test]
fn a_list_written_a_page_at_a_time_waits_for_the_lock() {
    let s = Store::memory();
    let n = PAGE_MESSAGES as u32 + 1;
    let msg = |uid| Summary { key: MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 3, uid }, ..Default::default() };
    let mut w = s.list_writer("a1", "INBOX");
    w.push(&s, (1..=n).map(msg).collect()).unwrap();
    let head = FolderState { validity: 3, uid_next: n + 1, ..Default::default() };
    let held = s.hold_changes();
    let done = Arc::new(AtomicBool::new(false));
    let finish = {
        let (s, done) = (s.clone(), done.clone());
        std::thread::spawn(move || {
            let counts = w.finish(&s, &head);
            done.store(true, Ordering::SeqCst);
            counts
        })
    };
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert!(!done.load(Ordering::SeqCst), "the head was written with the lock held elsewhere");
    drop(held);
    assert_eq!(finish.join().unwrap().unwrap(), (n, n));
    assert_eq!(s.load_folder("a1", "INBOX").unwrap().messages.len(), n as usize);
}
