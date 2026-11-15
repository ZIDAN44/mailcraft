//! Engine tests: read marks, flags and categories on a data file's (a local account's) mail,
//! written on the store's thread for marks rather than in the window's frame.

use mailcraft_model::MessageKey;

use super::*;

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
}

/// While the store's thread for marks is held (its commits slow, as a 38 GB file's were for
/// seconds), the window's thread waits for none of them: Down steps marking messages read,
/// another folder opened, a draft filed and the account's folders counted again (on the marks'
/// thread too, after them) each return at once. Once the marks are written, all of it is on
/// disk. A hold of Down once had frames of 0.5 and 1.8 s while a mark's commit took 3 s.
#[test]
fn the_window_never_waits_for_marks_being_written() {
    let store = Store::memory();
    let mut s = big_projects(&store, 2000);
    let first = s.row_keys()[0].to_id();
    s.run("list.open", &json!({"key": first})).unwrap();
    assert!(store.flush_marks());
    let paused = store.pause_marks();
    // Paused by this thread: a wait for them could only be given up, and is logged.
    let waits = waits_logged();
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
    assert_eq!(waits_logged(), waits, "the window waited for the marks");
    assert!(store.marks_waiting(), "nothing written yet");
    drop(paused);
    assert!(store.flush_marks());
    let projects = store.load_folder("sample", "Projects").unwrap();
    assert_eq!(projects.messages.iter().filter(|m| m.flags.seen).count(), 6, "the marks on disk");
    let listed = store.load_folders("sample").unwrap();
    let count = |p: &str| listed.iter().find(|f| f.path == p).map(|f| (f.unread, f.total));
    assert_eq!(count("Projects"), Some((1994, 2000)));
    assert_eq!(count("Drafts").map(|c| c.1), drafts, "the Drafts counted after them");
}

/// While a read mark's commit holds the lock every change of a list is made under (on a 38 GB
/// data file such a commit took 3 s now and then, the disk flushing), the window's thread waits for
/// nothing: Down steps marking mail read, the account's folders counted again, a rule saved, and
/// the folder whose marks are still to be written opened again after the cache let go of it. That
/// folder isn't read from the disk then (the read waited for its marks' commit): it's read on a
/// tick once they're written, with them. (The rule shows only that it takes no list's lock: on
/// the disk its file is still written, and flushed, on the window's thread; docs/gaps.md.)
#[test]
fn the_window_never_waits_while_a_mark_is_committed() {
    use std::time::Duration;
    let store = Store::memory();
    let mut s = big_projects(&store, 2000);
    s.set_cache_limit(100);
    let first = s.row_keys()[0].to_id();
    s.run("list.open", &json!({"key": first})).unwrap();
    assert!(store.flush_marks());
    // The marks' thread held mid-commit: paused, the lists' lock held, for a while.
    let (held, release) = (std::sync::mpsc::channel(), std::sync::mpsc::channel::<()>());
    let holder = {
        let store = store.clone();
        let (held, release) = (held.0, release.1);
        std::thread::spawn(move || {
            let _paused = store.pause_marks();
            let _lists = store.hold_lists();
            held.send(()).unwrap();
            let _ = release.recv_timeout(Duration::from_secs(20));
        })
    };
    held.1.recv().unwrap();
    // Held until it's let go of below (or 20 s): a wait for it can't end sooner, and is logged.
    let waits = waits_logged();
    for _ in 0..5 {
        s.run("list.next", &json!({})).unwrap();
    }
    assert!(store.marks_waiting());
    s.recount_local("sample", None);
    s.run("rules.add", &json!({"rule": {"name": "Mixes", "conditions": [{"if": "subject", "value": "mix"}], "actions": [{"do": "flag"}]}})).unwrap();
    s.run("folder.open", &json!({"account": "sample", "folder": "Archive"})).unwrap();
    assert!(!s.cache.contains_key(&("sample".to_string(), "Projects".to_string())), "let go of by the cache");
    s.run("folder.open", &json!({"account": "sample", "folder": "Projects"})).unwrap();
    s.tick();
    assert_eq!(waits_logged(), waits, "the window waited for the marks or the lists' lock");
    assert!(s.list.is_empty(), "not read while its marks wait");
    release.0.send(()).unwrap();
    holder.join().unwrap();
    assert!(store.flush_marks());
    s.tick();
    assert_eq!(s.list.len(), 2000, "read once they're written");
    assert_eq!(s.list.iter().filter(|m| m.flags.seen).count(), 6, "with the marks");
    let listed = store.load_folders("sample").unwrap();
    assert_eq!(listed.iter().find(|f| f.path == "Projects").map(|f| (f.unread, f.total)), Some((1994, 2000)));
}

/// A folder said changed on disk (an import going on, a mark that couldn't be kept) while read
/// marks of it are still being written keeps its list in memory until they're written: the list
/// on screen, the message read and the selection stay. Read again then, with the marks. It was
/// let go of and taken for empty meanwhile, the reading pane closed and the selection lost.
#[test]
fn a_folder_changed_while_its_marks_are_written_keeps_the_message_read() {
    let store = Store::memory();
    let mut s = big_projects(&store, 2000);
    let first = s.row_keys()[0].to_id();
    s.run("list.open", &json!({"key": first})).unwrap();
    assert!(store.flush_marks());
    let paused = store.pause_marks();
    for _ in 0..3 {
        s.run("list.next", &json!({})).unwrap();
    }
    assert!(store.marks_waiting());
    let read = s.reading.as_ref().map(|r| r.key.clone());
    let selection = s.selection.clone();
    assert!(read.is_some() && !selection.is_empty());
    s.apply(Event::FolderChanged { account: "sample".into(), folder: "Projects".into() });
    assert_eq!(s.list.len(), 2000, "the list kept");
    assert_eq!(s.reading.as_ref().map(|r| r.key.clone()), read, "the message read kept");
    assert_eq!(s.selection, selection);
    drop(paused);
    assert!(store.flush_marks());
    s.tick();
    assert!(s.marks_deferred.is_empty());
    assert_eq!(s.list.len(), 2000, "read again once they're written");
    assert_eq!(s.list.iter().filter(|m| m.flags.seen).count(), 4, "with the marks");
    assert_eq!(s.reading.as_ref().map(|r| r.key.clone()), read);
    assert_eq!(s.selection, selection);
}

/// Delete All on a folder whose read marks are still being written, let go of by the cache
/// meanwhile, waits for them (asked for, and rare) and moves all of it: the folder isn't taken
/// for empty, which moved nothing and said `{"moved": 0}`.
#[test]
fn delete_all_waits_for_a_folders_marks_being_written() {
    let store = Store::memory();
    let mut s = big_projects(&store, 300);
    s.set_cache_limit(100);
    let first = s.row_keys()[0].to_id();
    s.run("list.open", &json!({"key": first})).unwrap();
    assert!(store.flush_marks());
    let (held, release) = (std::sync::mpsc::channel(), std::sync::mpsc::channel::<()>());
    let holder = {
        let store = store.clone();
        let (held, release) = (held.0, release.1);
        std::thread::spawn(move || {
            let _paused = store.pause_marks();
            held.send(()).unwrap();
            let _ = release.recv_timeout(std::time::Duration::from_secs(20));
        })
    };
    held.1.recv().unwrap();
    s.run("list.next", &json!({})).unwrap();
    s.run("folder.open", &json!({"account": "sample", "folder": "Archive"})).unwrap();
    assert!(!s.cache.contains_key(&("sample".to_string(), "Projects".to_string())), "let go of by the cache");
    assert!(store.marks_waiting_for("sample", Some("Projects")));
    // The marks written a moment after it's asked for.
    let releaser = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(200));
        release.0.send(()).unwrap();
    });
    let out = s.run("folder.empty", &json!({"account": "sample", "folder": "Projects"})).unwrap();
    assert_eq!(out["moved"], 300, "{out}");
    releaser.join().unwrap();
    holder.join().unwrap();
    assert!(store.flush_marks());
    assert!(store.load_folder("sample", "Projects").unwrap().messages.is_empty());
}

/// A search that waits (the CLI's, `--cmd`) over a folder whose read marks are still being
/// written waits for them, as reading the folder does, and then reads it: it went round, a unit
/// at a time and nothing between, for as long as the marks took (forever, were they stuck).
#[test]
fn a_waiting_search_waits_for_a_folders_marks_without_spinning() {
    let store = Store::memory();
    let mut s = big_projects(&store, 300);
    s.set_cache_limit(100);
    let first = s.row_keys()[0].to_id();
    s.run("list.open", &json!({"key": first})).unwrap();
    assert!(store.flush_marks());
    let (held, release) = (std::sync::mpsc::channel(), std::sync::mpsc::channel::<()>());
    let holder = {
        let store = store.clone();
        let (held, release) = (held.0, release.1);
        std::thread::spawn(move || {
            let _paused = store.pause_marks();
            held.send(()).unwrap();
            let _ = release.recv_timeout(std::time::Duration::from_secs(20));
        })
    };
    held.1.recv().unwrap();
    s.run("list.next", &json!({})).unwrap();
    s.run("folder.open", &json!({"account": "sample", "folder": "Archive"})).unwrap();
    assert!(!s.cache.contains_key(&("sample".to_string(), "Projects".to_string())), "let go of by the cache");
    assert!(store.marks_waiting_for("sample", Some("Projects")));
    let releaser = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(300));
        release.0.send(()).unwrap();
    });
    let units = s.search_stats().units;
    s.run_search("subject:report", SearchScope::All);
    releaser.join().unwrap();
    holder.join().unwrap();
    assert!(s.search_progress().unwrap().done);
    let spent = s.search_stats().units - units;
    assert!(spent < 100, "{spent} units");
    let found: Vec<_> = s.list.iter().filter(|m| m.key.folder == "Projects").collect();
    assert_eq!(found.len(), 300);
    assert_eq!(found.iter().filter(|m| m.flags.seen).count(), 2, "with the marks");
}

/// What a Down step costs the window in a data file's folder of 10,941 unread messages, in a real
/// data file (`cargo test -p mailcraft-engine --release down_steps_on_a_big_data_file_folder --
/// --ignored --nocapture`): the step with its read mark handed over, and the mark written on the
/// store's thread (each step waits for it, timed apart).
#[test]
#[ignore = "a measurement: run in release"]
fn down_steps_on_a_big_data_file_folder() {
    use std::time::{Duration, Instant};
    let root = scratch("down-data-file");
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
}

/// What a Down step costs the window in a data file's folder of 10,941 unread messages while an
/// import writes 2 MiB batches into the same data file as fast as it can (`cargo test -p
/// mailcraft-engine --release down_steps_during_an_import -- --ignored --nocapture`): the steps
/// 10 ms apart, as a held key, each read mark handed to the store's thread. The mark's own write
/// waits for the disk the import keeps busy (its p99 45 to 230 ms when the window made it); the
/// window doesn't wait for it.
#[test]
#[ignore = "a measurement: run in release"]
fn down_steps_during_an_import() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};
    let root = scratch("down-import");
    let store = Store::open_packed(&root, root.join("files")).unwrap();
    let mut s = big_projects(&store, 10_941);
    s.defer_bodies(true);
    let first = s.row_keys()[0].to_id();
    s.run("list.open", &json!({"key": first})).unwrap();
    store.flush_marks();
    let stop = std::sync::Arc::new(AtomicBool::new(false));
    let importer = {
        let (store, stop) = (store.clone(), stop.clone());
        std::thread::spawn(move || {
            // Noise that doesn't compress, 100 KiB a message, 2 MiB a batch.
            let one: Vec<u8> = Xorshift(XORSHIFT_SEED).take(100 << 10).map(|x| x.to_le_bytes()[0]).collect();
            let (mut uid, mut batches) = (100_000u32, 0u32);
            while !stop.load(Ordering::Relaxed) {
                let batch = (0..20)
                    .map(|_| {
                        uid += 1;
                        let key = MessageKey { account: "sample".into(), folder: "Imported".into(), validity: 1, uid };
                        (key, Store::prepare(true, one.clone()))
                    })
                    .collect();
                store.write_prepared(batch, false).unwrap();
                batches += 1;
            }
            batches
        })
    };
    std::thread::sleep(Duration::from_millis(100));
    let mut window = Vec::new();
    for _ in 0..100 {
        let t = Instant::now();
        s.run("list.next", &json!({})).unwrap();
        drop(s.take_body_work());
        s.tick();
        window.push(t.elapsed());
        std::thread::sleep(Duration::from_millis(10));
    }
    stop.store(true, Ordering::Relaxed);
    let batches = importer.join().unwrap();
    let t = Instant::now();
    assert!(store.flush_marks());
    let left = t.elapsed();
    window.sort();
    let (median, p99, max) = (window[50], window[98], window[99]);
    println!("{batches} batches of 2 MiB written beside it");
    println!("Down step, the window: median {median:?}, p99 {p99:?}, max {max:?}");
    println!("the marks still to write when the import stopped: {left:?}");
    assert_eq!(s.list.iter().filter(|m| m.flags.seen).count(), 101);
    drop(s);
    drop(store);
}
