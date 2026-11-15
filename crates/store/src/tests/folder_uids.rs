//! Store tests: a folder's message numbers read alone ([`Store::folder_uids`]).

use mailcraft_model::{EmailAddress, Flag, Op};

use super::*;

/// A summary as a real one is, about: a sender, a recipient, a subject and a preview.
fn real(uid: u32) -> SharedSummary {
    SharedSummary::new(Summary {
        key: MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 3, uid },
        message_id: format!("{uid}.abcdef@mail.studio.example"),
        subject: format!("Re: the plans for the next quarter, number {uid}"),
        from: EmailAddress::new("Ana Example", format!("ana{uid}@studio.example")),
        to: vec![EmailAddress::new("Ben", "ben@studio.example")],
        date: 1_790_000_000 + i64::from(uid),
        size: 12_345,
        preview: "Hello Ben, here are the numbers we spoke about on the phone yesterday; the rest follows next week".into(),
        ..Default::default()
    })
}

/// A folder of `uids`, saved.
fn saved(s: &Store, folder: &str, uids: impl Iterator<Item = u32>) -> FolderState {
    let mut st = FolderState { validity: 3, messages: uids.map(real).collect(), ..Default::default() };
    st.uid_next = st.messages.iter().map(|m| m.key.uid).max().unwrap_or(0) + 1;
    sort_newest_first(&mut st.messages);
    s.save_folder("a1", folder, &st).unwrap();
    st
}

fn sorted(mut v: Vec<u32>) -> Vec<u32> {
    v.sort_unstable();
    v
}

/// The numbers read alone are the numbers of the list read whole: a small folder's (its list in
/// one file), a big one's (in pages, with marks beside them), one never kept (none), and one whose
/// page was lost from the middle (what reading it whole would have).
#[test]
fn a_folders_numbers_are_its_lists() {
    let s = Store::memory();
    saved(&s, "Small", [3, 9, 27].into_iter());
    assert_eq!(sorted(s.folder_uids("a1", "Small").unwrap()), [3, 9, 27]);

    let n = PAGE_MESSAGES as u32 * 2 + 749;
    saved(&s, "INBOX", (1..=n).filter(|u| u % 7 != 0));
    let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 3, uid: 50 };
    s.apply_op("a1", &Op::SetFlag { keys: vec![key], flag: Flag::Seen, on: true }).unwrap();
    let whole: Vec<u32> = s.load_folder("a1", "INBOX").unwrap().messages.iter().map(|m| m.key.uid).collect();
    assert_eq!(whole.len(), (1..=n).filter(|u| u % 7 != 0).count());
    assert_eq!(sorted(s.folder_uids("a1", "INBOX").unwrap()), sorted(whole));

    assert!(s.folder_uids("a1", "Never").unwrap().is_empty());

    let dir = s.folder_dir("a1", "INBOX");
    s.files.remove_file(&dir.join("state-1.json")).unwrap();
    let whole: Vec<u32> = s.load_folder("a1", "INBOX").unwrap().messages.iter().map(|m| m.key.uid).collect();
    assert_eq!(sorted(s.folder_uids("a1", "INBOX").unwrap()), sorted(whole));
}

/// Issue #396: a big folder's numbers read alone, against its whole list read for them (what
/// forgetting the numbers no longer used on each full sync did), on disk.
#[test]
#[ignore = "a measurement"]
fn folder_uids_timed() {
    let n: u32 = std::env::var("MC_N").ok().and_then(|v| v.parse().ok()).unwrap_or(100_000);
    let s = temp_store("folder-uids-timed");
    saved(&s, "INBOX", 1..=n);
    for round in 0..3 {
        let t = std::time::Instant::now();
        let whole: Vec<u32> = s.load_folder("a1", "INBOX").unwrap().messages.iter().map(|m| m.key.uid).collect();
        let a = t.elapsed();
        let t = std::time::Instant::now();
        let only = s.folder_uids("a1", "INBOX").unwrap();
        let b = t.elapsed();
        assert_eq!(sorted(whole), sorted(only));
        println!("MEASURE round {round}: {n} messages: whole list {a:?}, numbers only {b:?}");
    }
}
