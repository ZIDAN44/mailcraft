//! Engine tests: the Focused Inbox while mail keeps arriving.

use super::*;

/// Mail arriving in a Focused Inbox doesn't have the address book counted again each time (that
/// read every folder of every account, on each sync of the Inbox on screen): the one counted
/// last sorts it until it's ten minutes old. Suggestions, which want everyone, count it again.
#[test]
fn the_address_book_isnt_counted_again_for_each_sync() {
    let mut s = sample_session("focused-book");
    s.run("view.focused", &json!({"on": true})).unwrap();
    let focused: Vec<String> = s.list.iter().map(|m| m.subject.clone()).collect();
    let built = s.contacts_built;
    for _ in 0..10 {
        s.apply(Event::FolderChanged { account: "sample".into(), folder: "INBOX".into() });
    }
    assert_eq!(s.contacts_built, built, "not counted again");
    assert_eq!(s.list.iter().map(|m| m.subject.clone()).collect::<Vec<_>>(), focused, "sorted as before");
    // Ten minutes on, the next sync counts it again, once.
    s.set_clock(NOW + 601, 0);
    s.apply(Event::FolderChanged { account: "sample".into(), folder: "INBOX".into() });
    s.apply(Event::FolderChanged { account: "sample".into(), folder: "INBOX".into() });
    assert_eq!(s.contacts_built, built + 1);
    // A suggestion counts what's there now.
    s.apply(Event::FolderChanged { account: "sample".into(), folder: "INBOX".into() });
    assert!(!s.suggest_contacts("ana", 5).is_empty());
    assert_eq!(s.contacts_built, built + 2);
}
