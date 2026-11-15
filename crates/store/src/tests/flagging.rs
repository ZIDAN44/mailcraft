//! Store tests: a flag or category changed in a big folder's list ([`Store::apply_op`]), and
//! marks kept on the store's thread for them ([`Store::mark_later`]).

use mailcraft_model::{EmailAddress, Flag, Op};

use super::*;

/// A summary as a real one is, about: a sender, two recipients, a subject and a preview.
fn real(uid: u32) -> Summary {
    let who = |n: u32| EmailAddress { name: format!("Person Number {n}"), email: format!("person.number.{n}@example.com") };
    Summary {
        key: MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 3, uid },
        message_id: format!("{uid}.message-id-of-a-real-length@mail.example.com"),
        subject: format!("Re: the quarterly figures and what they mean for us, part {uid}"),
        from: who(uid % 97),
        to: vec![who(1), who(uid % 13)],
        date: 1_700_000_000 + i64::from(uid) * 60,
        size: 20_000 + uid % 5_000,
        preview: "Thanks for sending these over. I had a look at the numbers and I think we should talk about the second half before Friday".into(),
        ..Default::default()
    }
}

/// A big folder with these messages, saved.
fn big(s: &Store, n: u32) -> FolderState {
    let mut st = FolderState { validity: 3, uid_next: n + 1, messages: (1..=n).map(real).map(Into::into).collect(), ..Default::default() };
    sort_newest_first(&mut st.messages);
    s.save_folder("a1", "INBOX", &st).unwrap();
    st
}

fn key(uid: u32) -> MessageKey {
    MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 3, uid }
}

/// A message flagged or given a category in a big folder writes the marks beside its pages, no
/// page; read back (and reloaded from disk by another store), the list is the one changed in
/// memory, the counts of what was done the same as a full save gives.
#[test]
fn a_flag_in_a_big_folder_writes_no_page() {
    let s = Store::memory();
    let n = u32::try_from(PAGE_MESSAGES).unwrap() * 2 + 9;
    let mut st = big(&s, n);
    s.track_changes();
    s.apply_op("a1", &Op::SetFlag { keys: vec![key(7), key(n)], flag: Flag::Flagged, on: true }).unwrap();
    s.apply_op("a1", &Op::SetKeyword { keys: vec![key(5000)], keyword: "Blue".into(), on: true }).unwrap();
    let mut names: Vec<String> = s.take_changes().into_iter().map(|(p, _)| p.rsplit('/').next().unwrap_or_default().to_string()).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names, ["state-marks.json"], "no page, no head");
    for m in st.messages.iter_mut().filter(|m| [7, n].contains(&m.key.uid)) {
        SharedSummary::make_mut(m).flags.flagged = true;
    }
    if let Some(m) = st.messages.iter_mut().find(|m| m.key.uid == 5000) {
        SharedSummary::make_mut(m).flags.set_keyword("Blue", true);
    }
    assert_eq!(s.load_folder("a1", "INBOX").unwrap(), st);
    let again = Store::memory_from(s.snapshot(|_| true));
    assert_eq!(again.load_folder("a1", "INBOX").unwrap(), st, "the files written read back the same");
    // A move or a delete still writes the list whole: the marks go into the pages.
    s.apply_op("a1", &Op::Expunge { keys: vec![key(1)] }).unwrap();
    st.messages.retain(|m| m.key.uid != 1);
    assert!(!s.has_marks(&s.folder_dir("a1", "INBOX").join(folders::MARKS)), "emptied");
    assert_eq!(Store::memory_from(s.snapshot(|_| true)).load_folder("a1", "INBOX").unwrap(), st);
}

/// When the thread for marks can't be started (the system has no more threads), marks are written
/// as they're made, and starting it isn't tried again for each one: once a while has passed,
/// when it starts.
#[test]
fn a_thread_for_marks_that_cant_start_isnt_tried_on_every_mark() {
    let s = Store::memory();
    let mut st = FolderState { validity: 3, uid_next: 11, messages: (1..=10).map(real).map(Into::into).collect(), ..Default::default() };
    sort_newest_first(&mut st.messages);
    s.save_folder("a1", "INBOX", &st).unwrap();
    let seen = Flags { seen: true, ..Default::default() };
    let marks = |uid| vec![FolderMarks { folder: "INBOX".into(), validity: 3, flags: vec![(uid, seen.clone())] }];
    marking::NO_THREAD.set(true);
    marking::STARTS_TRIED.set(0);
    for uid in 1..=5 {
        s.mark_later("a1", marks(uid), Vec::new());
    }
    assert_eq!(marking::STARTS_TRIED.get(), 1, "tried once, not for every mark");
    let back = s.load_folder("a1", "INBOX").unwrap();
    assert!((1..=5).all(|uid| back.find(uid).unwrap().flags.seen), "written as they were made");
    // A while later, it's tried again, and starts.
    marking::NO_THREAD.set(false);
    s.mark_later("a1", marks(6), Vec::new());
    assert_eq!(marking::STARTS_TRIED.get(), 1, "not yet");
    s.marks_thread_failed(std::time::Duration::from_secs(61));
    s.mark_later("a1", marks(7), Vec::new());
    assert_eq!(marking::STARTS_TRIED.get(), 2);
    assert!(s.marks_thread_running());
    assert!(s.flush_marks());
    let back = s.load_folder("a1", "INBOX").unwrap();
    assert!((1..=7).all(|uid| back.find(uid).unwrap().flags.seen));
}

/// How long one message flagged in a folder of 50,000 takes, on disk, and what it writes:
/// `cargo test --release -p mailcraft-store --lib a_flag_in_a_folder_of_50000_costs -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement: run alone with --ignored --nocapture"]
fn a_flag_in_a_folder_of_50000_costs() {
    const N: u32 = 50_000;
    const RUNS: u32 = 21;
    // The median, least and most of `RUNS` runs of `f` (given the run's number), in ms.
    fn timed(mut f: impl FnMut(u32)) -> (f64, f64, f64) {
        let mut times: Vec<f64> = (0..RUNS)
            .map(|i| {
                let t = std::time::Instant::now();
                f(i);
                t.elapsed().as_secs_f64() * 1000.0
            })
            .collect();
        times.sort_by(f64::total_cmp);
        let at = |i: usize| times.get(i).copied().unwrap_or_default();
        (at(times.len() / 2), at(0), at(times.len().saturating_sub(1)))
    }
    let s = temp_store("flag-50000");
    big(&s, N);
    let flag = timed(|i| {
        s.apply_op("a1", &Op::SetFlag { keys: vec![key(25_000)], flag: Flag::Flagged, on: i % 2 == 0 }).unwrap();
    });
    let load = timed(|_| {
        s.load_folder("a1", "INBOX").unwrap();
    });
    let mem = Store::memory();
    big(&mem, N);
    mem.track_changes();
    mem.apply_op("a1", &Op::SetFlag { keys: vec![key(25_000)], flag: Flag::Flagged, on: true }).unwrap();
    let written = mem.take_changes();
    let bytes: usize = written.iter().map(|(_, b)| b.as_ref().map_or(0, Vec::len)).sum();
    println!(
        "MEASURE flag one of {N}: median {:.1} ms (min {:.1}, max {:.1}) over {RUNS}; reading the list alone median {:.1} ms (min {:.1}); {} files, {bytes} bytes written",
        flag.0,
        flag.1,
        flag.2,
        load.0,
        load.1,
        written.len()
    );
    drop(s);
}

/// A message marked read in a big folder writes a few bytes beside its pages (with the folder's
/// counts, in one write), not a page of thousands; read back, the list has the mark, and what
/// puts the summaries right about their files keeps it. Too many marks go into their pages; a
/// list written afresh has none; a damaged marks file isn't fatal.
#[test]
fn a_mark_in_a_big_folder_writes_a_few_bytes() {
    let s = Store::memory();
    let n = PAGE_MESSAGES as u32 * 2 + 749;
    let mut st =
        FolderState { validity: 3, uid_next: n + 1, messages: (1..=n).map(|uid| msg(uid, false)).map(Into::into).collect(), ..Default::default() };
    sort_newest_first(&mut st.messages);
    s.save_folder("a1", "INBOX", &st).unwrap();
    s.save_folders("a1", &[Folder { account: "a1".into(), path: "INBOX".into(), unread: n, total: n, ..Default::default() }]).unwrap();
    s.track_changes();
    let mark = |st: &mut FolderState, uids: &[u32]| {
        for m in st.messages.iter_mut().filter(|m| uids.contains(&m.key.uid)).map(std::sync::Arc::make_mut) {
            m.flags.seen = true;
        }
        let flags = st.messages.iter().filter(|m| uids.contains(&m.key.uid)).map(|m| (m.key.uid, m.flags.clone())).collect();
        let unread = u32::try_from(st.messages.iter().filter(|m| !m.flags.seen).count()).unwrap();
        s.save_flags("a1", &[FolderMarks { folder: "INBOX".into(), validity: 3, flags }], &[("INBOX".into(), unread, n)]).unwrap()
    };
    assert_eq!(mark(&mut st, &[5000]), 1, "the folder is in the account's list");
    let written = s.take_changes();
    let bytes: usize = written.iter().map(|(_, b)| b.as_ref().map_or(0, Vec::len)).sum();
    let mut names: Vec<&str> = written.iter().map(|(p, _)| p.rsplit('/').next().unwrap_or_default()).collect();
    names.sort_unstable();
    assert_eq!(names, ["folders.json", "state-marks.json"], "no page");
    assert!(bytes < 64 << 10, "{bytes} bytes");
    let back = s.load_folder("a1", "INBOX").unwrap();
    assert_eq!(back, st, "read back with the mark");
    assert_eq!(s.load_folders("a1").unwrap()[0].unread, n - 1);
    // Put right about its files, the page is written again from disk: the mark stays.
    let found = search::FilesFound { validity: 3, with: vec![5001], without: Vec::new() };
    assert_eq!(s.set_has_files("a1", "INBOX", &found).unwrap(), 1);
    if let Some(m) = st.messages.iter_mut().find(|m| m.key.uid == 5001).map(std::sync::Arc::make_mut) {
        m.has_attachments = true;
    }
    assert_eq!(s.load_folder("a1", "INBOX").unwrap(), st);
    // More than the marks hold: into their pages, the marks emptied.
    let many: Vec<u32> = (1..=u32::try_from(folders::MARKS_MAX).unwrap()).map(|i| i * 17).collect();
    s.take_changes();
    mark(&mut st, &many);
    let mut names: Vec<String> = s.take_changes().into_iter().map(|(p, _)| p.rsplit('/').next().unwrap_or_default().to_string()).collect();
    names.sort_unstable();
    assert_eq!(names, ["folders.json", "state-0.json", "state-1.json", "state-2.json", "state-marks.json", "state.json"]);
    let marks = s.folder_dir("a1", "INBOX").join(folders::MARKS);
    assert!(!s.has_marks(&marks), "emptied");
    assert_eq!(s.load_folder("a1", "INBOX").unwrap(), st);
    // A damaged marks file: set aside, the list read as its pages have it.
    mark(&mut st, &[7]);
    s.files.write(&marks, b"[[7, {\"seen\": tr").unwrap();
    let back = s.load_folder("a1", "INBOX").unwrap();
    assert!(!back.find(7).unwrap().flags.seen && back.find(17).unwrap().flags.seen);
    // A list written afresh (an import's) has no marks from before.
    mark(&mut st, &[9]);
    let mut w = s.list_writer("a1", "INBOX");
    let mut fresh: Vec<Summary> = (1..=n).map(|uid| msg(uid, false)).collect();
    fresh.sort_by_key(|m| m.key.uid);
    w.push(&s, fresh).unwrap();
    w.finish(&s, &st).unwrap();
    assert!(!s.load_folder("a1", "INBOX").unwrap().find(9).unwrap().flags.seen);
}

/// Marks handed to the store's thread ([`Store::mark_later`]) are in the list whoever reads it
/// next (a reader waits for them), the counts with them; a small list is written whole, a list
/// made anew since (another validity) isn't marked, and a mark that can't be written is told.
#[test]
fn marks_kept_later_are_there_for_the_next_reader() {
    let s = Store::failing();
    let mut st =
        FolderState { validity: 3, uid_next: 11, messages: (1..=10).map(|uid| msg(uid, false)).map(Into::into).collect(), ..Default::default() };
    sort_newest_first(&mut st.messages);
    s.save_folder("a1", "INBOX", &st).unwrap();
    s.save_folders("a1", &[Folder { account: "a1".into(), path: "INBOX".into(), unread: 10, total: 10, ..Default::default() }]).unwrap();
    let seen = Flags { seen: true, ..Default::default() };
    let marks = |validity, uid| vec![FolderMarks { folder: "INBOX".into(), validity, flags: vec![(uid, seen.clone())] }];
    {
        let _paused = s.pause_marks();
        s.mark_later("a1", marks(3, 4), vec![("INBOX".into(), 9, 10)]);
        assert!(s.marks_waiting(), "not written while the thread waits");
    }
    assert!(s.load_folder("a1", "INBOX").unwrap().find(4).unwrap().flags.seen, "the reader waited for it");
    assert_eq!(s.load_folders("a1").unwrap()[0].unread, 9);
    s.mark_later("a1", marks(2, 5), vec![("INBOX".into(), 8, 10)]);
    assert!(!s.load_folder("a1", "INBOX").unwrap().find(5).unwrap().flags.seen, "another list's");
    s.fail(None, Some("state.json"));
    s.mark_later("a1", marks(3, 6), Vec::new());
    assert!(s.flush_marks());
    s.fail(None, None);
    let after = s.take_marks_failed();
    assert!(matches!(after.as_slice(), [MarksAfter::Failed { account, folders, .. }] if account == "a1" && folders == &["INBOX"]), "{after:?}");
    assert!(!s.load_folder("a1", "INBOX").unwrap().find(6).unwrap().flags.seen);
}

/// A big list saved after only flags changed, with a newer modseq: the few marks beside its
/// pages, and its head with the modseq (it was dropped, the marks alone written).
#[test]
fn a_flag_save_keeps_a_newer_head() {
    let s = Store::memory();
    let n = PAGE_MESSAGES as u32 * 2 + 9;
    let mut st = FolderState {
        validity: 3,
        uid_next: n + 1,
        highest_modseq: 40,
        messages: (1..=n).map(|uid| msg(uid, false)).map(Into::into).collect(),
        ..Default::default()
    };
    sort_newest_first(&mut st.messages);
    s.save_folder("a1", "INBOX", &st).unwrap();
    if let Some(m) = st.messages.iter_mut().find(|m| m.key.uid == 7).map(std::sync::Arc::make_mut) {
        m.flags.seen = true;
    }
    st.highest_modseq = 41;
    s.track_changes();
    s.save_folder_changed("a1", "INBOX", &st, &[7].into_iter().collect()).unwrap();
    let mut names: Vec<String> = s.take_changes().into_iter().map(|(p, _)| p.rsplit('/').next().unwrap_or_default().to_string()).collect();
    names.sort_unstable();
    assert_eq!(names, ["state-marks.json", "state.json"], "no page");
    assert_eq!(s.load_folder("a1", "INBOX").unwrap(), st);
}

/// The marks' thread lets the store go once its last handle has: the data file it holds is
/// closed, and a store opened again on the folder reads it (it was "open in another copy").
#[test]
fn the_marks_thread_lets_the_store_go() {
    let s = Store::memory();
    let st = FolderState { validity: 3, uid_next: 2, messages: vec![msg(1, false).into()], ..Default::default() };
    s.save_folder("a1", "INBOX", &st).unwrap();
    let seen = Flags { seen: true, ..Default::default() };
    s.mark_later("a1", vec![FolderMarks { folder: "INBOX".into(), validity: 3, flags: vec![(1, seen)] }], Vec::new());
    assert!(s.flush_marks());
    let (marking, changes) = (Arc::downgrade(&s.marking), Arc::downgrade(&s.changes));
    drop(s);
    let gone = (0..200).any(|_| {
        std::thread::sleep(std::time::Duration::from_millis(10));
        marking.upgrade().is_none() && changes.upgrade().is_none()
    });
    assert!(gone, "the thread still holds the store");
}
