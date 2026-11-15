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
