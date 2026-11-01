//! The messages an import brings in go into the account's search index as they're written:
//! searchable once it's done with nothing read again, and nothing left for the next pass to add.
//! Whatever becomes of the index, the import is the same.

use mailcraft_store::Store;
use mailcraft_store::search::{self, Field, Reader, Writer};

use super::super::ImportOptions;
use super::pipeline::{data_file, run};

/// Messages a folder of [`data_file`] holds…
const PER_FOLDER: usize = 30;
/// …in its three folders.
const MESSAGES: usize = 3 * PER_FOLDER;

/// The messages of [`data_file`] whose text says "agreed" (those with a plain body), and those
/// with a file (a picture the HTML shows isn't one).
fn with_text() -> usize {
    3 * (0..PER_FOLDER).filter(|k| k % 4 != 2).count()
}

fn with_files() -> usize {
    3 * (0..PER_FOLDER).filter(|k| k % 3 == 0).count()
}

fn lanes(n: usize) -> ImportOptions {
    ImportOptions { lanes: n, ..ImportOptions::default() }
}

fn found(store: &Store, account: &str, field: Field, word: &str) -> usize {
    let mut r = Reader::open(store, account).unwrap().expect("an index");
    r.word(store, field, word).unwrap().count() as usize
}

/// What a pass would do now to bring the index up to the mail.
fn plan(store: &Store, account: &str) -> search::Plan {
    let w = Writer::open(store, account).unwrap();
    search::plan(store, &w, account, &[]).unwrap()
}

/// Once imported, on a data file as the app keeps one and in memory, every message is in the
/// index: its text, its files' names, and how many files it has, found with no message read. A
/// pass then finds nothing to add and no folder to look at: each was marked as brought up as it
/// was finished.
#[test]
fn an_imported_file_is_searchable_without_reading_it_again() {
    let (dir, pst) = data_file("indexed", PER_FOLDER, false);
    let packed = Store::open_packed(dir.join("app"), dir.join("Files")).unwrap();
    for store in [Store::memory(), packed] {
        let done = run(&pst, &store, "i1", &lanes(4));
        assert_eq!((done.messages, done.skipped, done.stats.indexed), (MESSAGES, 0, MESSAGES as u64));
        let reads = store.reads();
        let r = Reader::open(&store, "i1").unwrap().expect("an index");
        assert_eq!(r.live() as usize, MESSAGES);
        assert_eq!(found(&store, "i1", Field::Body, "agreed"), with_text());
        assert_eq!(found(&store, "i1", Field::Names, "plan"), with_files());
        assert_eq!(found(&store, "i1", Field::Names, "chart"), 0, "a picture the text shows is no file");
        assert_eq!(store.reads(), reads, "no message read");
        let p = plan(&store, "i1");
        assert!(p.is_empty() && p.add.is_empty(), "{p:?}");
        assert_eq!(p.total, 0, "every folder passed over");
        assert_eq!(store.reads(), reads);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// However many lanes read the file, the index is the same: the same messages behind each word.
#[test]
fn the_index_is_the_same_whatever_the_lanes() {
    let (dir, pst) = data_file("indexed-lanes", PER_FOLDER, false);
    let (one, four) = (Store::memory(), Store::memory());
    run(&pst, &one, "i2", &lanes(1));
    run(&pst, &four, "i2", &lanes(4));
    let keys = |store: &Store, word: &str| {
        let mut r = Reader::open(store, "i2").unwrap().unwrap();
        let mut keys: Vec<_> = r.word(store, Field::Body, word).unwrap().ones().filter_map(|d| r.key(d)).collect();
        keys.sort_by(|a, b| (&a.folder, a.uid).cmp(&(&b.folder, b.uid)));
        keys
    };
    for word in ["agreed", "plan", "here"] {
        assert!(!keys(&one, word).is_empty(), "{word}");
        assert_eq!(keys(&one, word), keys(&four, word), "{word}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// An index that can't be written costs the import nothing: every message comes in, and the
/// next pass finds them all to add.
#[test]
fn an_index_that_cant_be_written_leaves_the_import_whole() {
    let (dir, pst) = data_file("index-fails", PER_FOLDER, false);
    let store = Store::failing();
    store.fail(None, Some("meta.bin"));
    let done = run(&pst, &store, "i3", &lanes(4));
    assert_eq!((done.messages, done.skipped, done.stats.indexed), (MESSAGES, 0, 0));
    assert!(done.last_save.is_none(), "{:?}", done.last_save);
    store.fail(None, None);
    assert!(Reader::open(&store, "i3").unwrap().is_none(), "no index");
    let listed: usize = done.folders.iter().map(|f| store.load_folder("i3", &f.path).unwrap().messages.len()).sum();
    assert_eq!(listed, MESSAGES);
    assert_eq!(plan(&store, "i3").add.len(), MESSAGES, "every message for the next pass");
    let _ = std::fs::remove_dir_all(&dir);
}

/// An index another writer holds (a pass not yet stopped) is waited for a moment: let go within
/// it, the import takes it; held on, the import goes on without it and the next pass adds every
/// message. Asked not to index, it doesn't.
#[test]
fn an_index_held_elsewhere_is_waited_for_then_left() {
    let (dir, pst) = data_file("index-held", PER_FOLDER, false);
    let store = Store::memory();
    let held = Writer::open(&store, "i4").unwrap();
    let done = run(&pst, &store, "i4", &lanes(2));
    assert_eq!((done.messages, done.stats.indexed), (MESSAGES, 0));
    drop(held);
    assert!(Reader::open(&store, "i4").unwrap().is_none());
    assert_eq!(plan(&store, "i4").add.len(), MESSAGES);

    let store = Store::memory();
    let held = Writer::open(&store, "i5").unwrap();
    let letting_go = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(50));
        drop(held);
    });
    let done = run(&pst, &store, "i5", &lanes(2));
    letting_go.join().unwrap();
    assert_eq!((done.messages, done.stats.indexed), (MESSAGES, MESSAGES as u64));
    assert!(plan(&store, "i5").is_empty());

    let store = Store::memory();
    let done = run(&pst, &store, "i6", &ImportOptions { lanes: 2, no_index: true, ..ImportOptions::default() });
    assert_eq!((done.messages, done.stats.indexed), (MESSAGES, 0));
    assert!(Reader::open(&store, "i6").unwrap().is_none());
    let _ = std::fs::remove_dir_all(&dir);
}
