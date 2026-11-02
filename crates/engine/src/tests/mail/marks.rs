//! Engine tests: read marks, flags and categories on a data file's (a local account's) mail,
//! written on the store's thread for marks rather than in the window's frame.

use mailcraft_model::{MessageKey, Summary};

use super::*;

/// The sample's Projects made a folder of `n` unread messages in `store`, and a session on it.
fn big_projects(store: &Store, n: u32) -> Session {
    sample::seed_if_empty(store, NOW).unwrap();
    let mut st = store.load_folder("sample", "Projects").unwrap();
    st.validity = 7;
    st.messages = (1..=n)
        .map(|uid| Summary {
            key: MessageKey { account: "sample".into(), folder: "Projects".into(), validity: 7, uid },
            subject: format!("Report {uid}"),
            date: NOW - i64::from(uid) * 60,
            ..Default::default()
        })
        .collect();
    store.save_folder("sample", "Projects", &st).unwrap();
    let mut s = Session::open(Some(store.clone()), mailcraft_model::dates::Clock::new(NOW, 0));
    s.run("folder.open", &json!({"account": "sample", "folder": "Projects"})).unwrap();
    s
}

/// A message marked read in a folder of 10,941 of a data file's account writes nothing on the
/// window's thread: it's written on the store's thread for marks, a few bytes with the folder's
/// count, in one write. It saved the whole page of 4,096 summaries it's in (about 2 MB) and the
/// counts in a second durable commit, 24 ms by median of the window's frame on the real file and
/// up to 1.4 s when the disk was slow. Read again, the folder has the mark.
#[test]
fn a_mark_read_in_a_big_folder_writes_under_64_kib_off_the_window() {
    let store = Store::memory();
    let n = 10_941u32;
    let mut s = big_projects(&store, n);
    store.track_changes();
    let k = s.list.get(5000).map(|m| m.key.clone()).unwrap();
    let paused = store.pause_marks();
    s.apply_op("sample", Op::SetFlag { keys: vec![k.clone()], flag: Flag::Seen, on: true });
    assert!(s.list.iter().find(|m| m.key == k).is_some_and(|m| m.flags.seen), "the list has it at once");
    assert_eq!(store.take_changes(), Vec::new(), "nothing written on the window's thread");
    assert!(store.marks_waiting());
    drop(paused);
    assert!(store.flush_marks());
    let written = store.take_changes();
    let bytes: usize = written.iter().map(|(_, b)| b.as_ref().map_or(0, Vec::len)).sum();
    assert!(bytes < 64 << 10, "{bytes} bytes in {} files", written.len());
    let mut names: Vec<&str> = written.iter().map(|(p, _)| p.rsplit('/').next().unwrap_or_default()).collect();
    names.sort_unstable();
    assert_eq!(names, ["folders.json", "state-marks.json"]);
    assert!(store.load_folder("sample", "Projects").unwrap().find(k.uid).unwrap().flags.seen);
    let unread = store.load_folders("sample").unwrap().into_iter().find(|f| f.path == "Projects").map(|f| f.unread);
    assert_eq!(unread, Some(n - 1), "the count kept with it");
    let mut again = Session::open(Some(store), mailcraft_model::dates::Clock::new(NOW, 0));
    again.run("folder.open", &json!({"account": "sample", "folder": "Projects"})).unwrap();
    assert!(again.summary(&k).is_some_and(|m| m.flags.seen), "opened again: read");
}

/// A data file's message marked read and moved at once (deleted just after it opened) keeps its
/// mark where it went, and the folder's count is the window's: the move reads the lists only
/// once the mark handed to the store's thread is written.
#[test]
fn a_message_marked_read_and_moved_at_once_keeps_its_mark() {
    let mut s = sample_session("mark-then-move");
    let m = s.list.iter().find(|m| !m.flags.seen).cloned().unwrap();
    s.apply_op("sample", Op::SetFlag { keys: vec![m.key.clone()], flag: Flag::Seen, on: true });
    s.apply_op("sample", Op::Move { keys: vec![m.key.clone()], to: "Archive".into() });
    let store = s.store.clone().unwrap();
    let moved = store.load_folder("sample", "Archive").unwrap();
    assert!(moved.messages.iter().any(|x| x.message_id == m.message_id && x.flags.seen), "read where it went");
    let held = s.folder("sample", &m.key.folder).map(|f| f.unread);
    let saved = store.load_folders("sample").unwrap().into_iter().find(|f| f.path == m.key.folder).map(|f| f.unread);
    assert_eq!(saved, held);
    done(s);
}

/// While the store's thread for marks is held (its commits slow, as a 38 GB file's were for
/// seconds), the window's thread waits for none of them: Down steps marking messages read,
/// another folder opened, a draft filed and the account's folders counted again (on the marks'
/// thread too, after them) each return at once. Once the marks are written, all of it is on
/// disk. A hold of Down once had frames of 0.5 and 1.8 s while a mark's commit took 3 s.
#[test]
fn the_window_never_waits_for_marks_being_written() {
    use std::time::{Duration, Instant};
    let store = Store::memory();
    let mut s = big_projects(&store, 2000);
    let first = s.row_keys()[0].to_id();
    s.run("list.open", &json!({"key": first})).unwrap();
    assert!(store.flush_marks());
    let paused = store.pause_marks();
    let began = Instant::now();
    for _ in 0..5 {
        s.run("list.next", &json!({})).unwrap();
    }
    assert!(store.marks_waiting());
    s.run("folder.open", &json!({"account": "sample", "folder": "Archive"})).unwrap();
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "ana@soundlab.example", "subject": "Mix notes", "body": "First take"})).unwrap();
    s.run("compose.saveDraft", &json!({})).unwrap();
    let drafts = s.folder("sample", "Drafts").map(|f| f.total);
    s.run("folder.open", &json!({"account": "sample", "folder": "Projects"})).unwrap();
    assert!(began.elapsed() < Duration::from_secs(5), "the window waited {:?}", began.elapsed());
    assert!(store.marks_waiting(), "nothing written yet");
    drop(paused);
    assert!(store.flush_marks());
    let projects = store.load_folder("sample", "Projects").unwrap();
    assert_eq!(projects.messages.iter().filter(|m| m.flags.seen).count(), 6, "the marks on disk");
    let listed = store.load_folders("sample").unwrap();
    let count = |p: &str| listed.iter().find(|f| f.path == p).map(|f| (f.unread, f.total));
    assert_eq!(count("Projects"), Some((1994, 2000)));
    assert_eq!(count("Drafts").map(|c| c.1), drafts, "the Drafts counted after them");
    done(s);
}

/// What a Down step costs the window in a data file's folder of 10,941 unread messages, in a real
/// data file (`cargo test -p mailcraft-engine --release down_steps_on_a_big_data_file_folder --
/// --ignored --nocapture`): the step with its read mark handed over, and the mark written on the
/// store's thread (each step waits for it, timed apart).
#[test]
#[ignore]
fn down_steps_on_a_big_data_file_folder() {
    use std::time::{Duration, Instant};
    let root = std::env::temp_dir().join(format!("mailcraft-engine-down-data-file-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let store = Store::open_packed(&root, root.join("files")).unwrap();
    let mut s = big_projects(&store, 10_941);
    s.defer_bodies(true);
    let first = s.row_keys()[0].to_id();
    s.run("list.open", &json!({"key": first})).unwrap();
    store.flush_marks();
    let (mut window, mut marks) = (Vec::new(), Vec::new());
    for _ in 0..100 {
        let t = Instant::now();
        s.run("list.next", &json!({})).unwrap();
        drop(s.take_body_work());
        window.push(t.elapsed());
        let t = Instant::now();
        assert!(store.flush_marks());
        marks.push(t.elapsed());
    }
    let median = |mut v: Vec<Duration>| {
        v.sort();
        (v[v.len() / 2], v[v.len() - 1])
    };
    let ((w, wmax), (m, mmax)) = (median(window), median(marks));
    println!("Down step, the window: median {w:?}, max {wmax:?}");
    println!("the read mark on disk (the store's thread): median {m:?}, max {mmax:?}");
    drop(s);
    drop(store);
    let _ = std::fs::remove_dir_all(&root);
}
