//! What an import loses, and how far it says it has got when it does.

use mailcraft_model::EmailAddress;
use mailcraft_model::job::SkipReason;
use mailcraft_store::Store;

use super::super::item::Item;
use super::super::out::Out;
use super::super::{ImportOptions, Progress};

/// A mail item as read from a data file.
fn mail(n: usize) -> Item {
    Item {
        class: "IPM.Note".into(),
        subject: format!("Message {n}"),
        from: EmailAddress::new("", "me@example.com"),
        date: 1_700_000_000,
        body: "Hi".into(),
        ..Default::default()
    }
}

/// A batch that can't be written is counted as left out, and how far the import has got (items
/// read) doesn't go back for it.
#[test]
fn progress_never_goes_back_when_a_batch_is_not_written() {
    let store = Store::failing();
    let mut seen = Vec::new();
    let mut progress = |t: Progress| seen.push(t);
    let mut out = Out::new(&store, "d1", &ImportOptions::default(), &mut progress);
    let inbox = out.add_folder(None, "Inbox");
    store.fail(None, Some(".eml"));
    for n in 0..200 {
        out.add_message(&inbox, mail(n));
        out.item_read(0);
    }
    out.save_folder(&inbox);
    store.fail(None, None);
    let archive = out.add_folder(None, "Archive");
    for n in 200..400 {
        out.add_message(&archive, mail(n));
        out.item_read(0);
    }
    let done = out.finish();
    assert_eq!((done.messages, done.skipped), (200, 200));
    assert_eq!(done.skips.into_iter().collect::<Vec<_>>(), [(SkipReason::NotWritten, 200)]);
    assert_eq!(
        seen,
        [
            Progress { read: 200, imported: 200, skipped: 0, ..Progress::default() },
            Progress { read: 400, imported: 200, skipped: 200, ..Progress::default() }
        ],
        "the first 200 went from written to left out, and stayed read"
    );
}

/// Items that are read but don't come in (none could be read here) still move the bar.
#[test]
fn progress_goes_on_through_items_left_out() {
    let store = Store::memory();
    let mut seen = Vec::new();
    let mut progress = |t: Progress| seen.push(t);
    let mut out = Out::new(&store, "d1", &ImportOptions::default(), &mut progress);
    for _ in 0..400 {
        out.skip(SkipReason::MessageUnreadable, 1);
        out.item_read(0);
    }
    out.finish();
    assert_eq!(
        seen,
        [
            Progress { read: 200, imported: 0, skipped: 200, ..Progress::default() },
            Progress { read: 400, imported: 0, skipped: 400, ..Progress::default() }
        ]
    );
}

/// A last save that fails says why, and keeps the counts of what came in before it.
#[test]
fn a_failed_last_save_keeps_what_came_in() {
    let store = Store::failing();
    let mut progress = |_| {};
    let mut out = Out::new(&store, "d1", &ImportOptions::default(), &mut progress);
    let inbox = out.add_folder(None, "Inbox");
    out.add_message(&inbox, mail(1));
    store.fail(None, Some("folders.json"));
    let done = out.finish();
    assert_eq!((done.messages, done.folders.len()), (1, 1));
    assert!(done.last_save.is_some(), "{done:?}");
    store.fail(None, None);
    let mut progress = |_| {};
    let mut out = Out::new(&store, "d2", &ImportOptions::default(), &mut progress);
    out.add_folder(None, "Inbox");
    assert_eq!(out.finish().last_save, None);
}

/// A folder of a kind that isn't brought in (a journal) is a loss when it holds items, counted
/// with them; an empty one isn't.
#[test]
fn left_out_folders_are_counted_with_their_items() {
    let store = Store::memory();
    let mut progress = |_| {};
    let mut out = Out::new(&store, "d1", &ImportOptions::default(), &mut progress);
    out.leave_out_folder(0);
    out.leave_out_folder(3);
    out.leave_out_folder(2);
    let done = out.finish();
    assert_eq!(done.skipped, 5, "the items, not the folders");
    assert_eq!(done.skips.into_iter().collect::<Vec<_>>(), [(SkipReason::FolderLeftOut, 2), (SkipReason::ItemInFolderLeftOut, 5)]);
}
