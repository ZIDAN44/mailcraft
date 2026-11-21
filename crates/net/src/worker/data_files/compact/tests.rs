//! Compactions as the pool runs them: on a thread of their own, by the plan, told as they go and
//! as they end; and an account's jobs held while its file is had to itself.

use mailcraft_model::job::{CompactMode, CompactPhase, CompactReport, ImportPhase};

use super::*;

fn temp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("mailcraft-compact-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

/// Bytes that don't compress, seeded as the store's tests seed theirs (a small `seed` spread
/// over the word, never 0).
fn noise(n: usize, seed: u64) -> Vec<u8> {
    crate::fakes::random::noise(n, seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
}

/// Each message's size.
const SIZE: usize = 2 << 20;

fn key(uid: u32) -> MessageKey {
    MessageKey { account: "c1".into(), folder: "INBOX".into(), validity: 1, uid }
}

/// A pool over a data store whose account `c1` keeps 10 messages, after 40 more were deleted:
/// enough to give back for compacting to be offered (64 MiB).
fn setup(name: &str) -> (std::path::PathBuf, Store, Pool) {
    let dir = temp(name);
    let store = Store::open_packed(dir.join("app"), dir.join("Files")).unwrap();
    store.save_accounts(&[AccountConfig { id: "c1".into(), email: "zoe@studio.example".into(), ..Default::default() }]).unwrap();
    for uid in 1..=50 {
        store.write_body(&key(uid), &noise(SIZE, u64::from(uid))).unwrap();
    }
    assert_eq!(store.drop_bodies_from("c1", "INBOX", 1, 11).unwrap(), 40);
    let pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    (dir, store, pool)
}

/// Events until one says the compaction ended, that one last, and what it came to. Polled (it
/// releases the jobs a compaction held), so not through the waits on `Pool::wait`.
fn until_compacted(p: &mut Pool) -> (Vec<Event>, CompactReport) {
    let mut seen = Vec::new();
    let mut ended = None;
    crate::waiting::eventually("the compaction to end", || {
        for e in p.poll() {
            if ended.is_some() {
                break;
            }
            if let Event::Compacted { report, .. } = &e {
                ended = Some((**report).clone());
            }
            seen.push(e);
        }
        ended.is_some()
    });
    (seen, ended.unwrap())
}

#[test]
fn asked_for_a_data_file_is_compacted_on_its_thread() {
    let (dir, store, mut p) = setup("manual");
    let before = std::fs::metadata(store.data_file("c1").unwrap()).unwrap().len();
    p.submit(Job::DataFileHealth { account: "c1".into() });
    let mut said = None;
    crate::waiting::eventually("the data file's health", || {
        said = p.poll().into_iter().find_map(|e| match e {
            Event::DataFileHealth { health, error, .. } => Some((health, error)),
            _ => None,
        });
        said.is_some()
    });
    let (health, error) = said.unwrap();
    assert_eq!(error, None);
    let health = health.unwrap();
    assert!(health.can_compact() && !health.needs_compacting(), "offered, not done at idle: {health:?}");

    // At idle it isn't worth it: nothing done, and said so.
    p.submit(Job::CompactDataFile { account: "c1".into(), mode: CompactMode::Idle });
    let (_, report) = until_compacted(&mut p);
    assert!(!report.compacted && report.error.is_none() && report.before == report.after, "{report:?}");

    // Asked for, it is: a file this small is copied into a new file, said as it goes (in place,
    // its database doubled it again at its next write, issue #221).
    p.submit(Job::CompactDataFile { account: "c1".into(), mode: CompactMode::Manual });
    let (seen, report) = until_compacted(&mut p);
    assert!(report.copied && report.error.is_none() && report.mode == CompactMode::Manual, "{report:?}");
    assert!(report.before == before && report.after < before, "{report:?}");
    assert_eq!(std::fs::metadata(store.data_file("c1").unwrap()).unwrap().len(), report.after);
    assert!(seen.iter().any(|e| matches!(e, Event::Compacting { phase: CompactPhase::Copy, .. })), "{seen:?}");
    assert_eq!(store.read_body(&key(3)).unwrap(), noise(SIZE, 3), "every message reads back");
    p.shutdown();
    drop(p);
    let _ = std::fs::remove_dir_all(&dir);
}

/// While one runs: a second is refused, and no import begins beside it (a new one is refused, one
/// going on stays paused), however far it has come.
#[test]
fn a_second_compaction_and_an_import_beside_one_are_refused() {
    let (dir, _store, mut p) = setup("refused");
    // One running (its thread held up by the drive it asks for room), until the test lets it go:
    // however slow the machine, it hasn't ended meanwhile.
    let (go, wait) = std::sync::mpsc::channel::<()>();
    p.compacts.running.insert(
        "c1".into(),
        CompactCtl {
            cancel: Arc::default(),
            exclusive: Arc::default(),
            thread: std::thread::spawn(move || {
                let _ = wait.recv();
            }),
        },
    );
    p.submit(Job::CompactDataFile { account: "c1".into(), mode: CompactMode::Manual });
    let (_, report) = until_compacted(&mut p);
    assert!(report.error.as_deref().is_some_and(|e| e.contains("already")), "{report:?}");
    p.submit(Job::ImportPst { account: "c1".into(), path: "archive.pst".into(), resume: false });
    p.submit(Job::ImportPst { account: "c1".into(), path: "archive.pst".into(), resume: true });
    let seen = p.poll();
    assert!(
        seen.iter().any(|e| matches!(e, Event::Imported { error: Some(e), done: true, .. } if e.contains("being compacted"))),
        "a new one is refused: {seen:?}"
    );
    assert!(seen.iter().any(|e| matches!(e, Event::ImportProgress { progress, .. } if progress.phase == ImportPhase::Paused)), "{seen:?}");
    assert!(!p.importing("c1"), "none began");
    p.submit(Job::CancelCompact { account: "c1".into() });
    assert!(p.compacts.running.get("c1").is_some_and(|c| c.cancel.load(Ordering::Relaxed)), "asked to stop");
    let _ = go.send(());
    p.shutdown();
    drop(p);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_accounts_jobs_wait_while_its_file_is_had_to_itself() {
    let (dir, store, mut p) = setup("held");
    let exclusive = Arc::new(AtomicBool::new(true));
    let (go, wait) = std::sync::mpsc::channel::<()>();
    p.compacts.running.insert(
        "c1".into(),
        CompactCtl {
            cancel: Arc::default(),
            exclusive: exclusive.clone(),
            thread: std::thread::spawn(move || {
                let _ = wait.recv();
            }),
        },
    );
    // Not started: a body asked for is read from the cache, on the pool's own thread.
    p.submit(Job::FetchBody { key: key(2) });
    p.submit(Job::FetchBody { key: MessageKey { account: "other".into(), ..key(2) } });
    p.submit(Job::CancelCompact { account: "c1".into() });
    let seen = p.poll();
    assert!(!seen.iter().any(|e| matches!(e, Event::Body { key, .. } if key.account == "c1")), "held: {seen:?}");
    assert!(seen.iter().any(|e| matches!(e, Event::BodyFailed { key, .. } if key.account == "other")), "another account's goes on");
    assert!(p.compacts.held.get("c1").is_some_and(|h| h.len() == 1), "a stop isn't held");
    // Given back: what waited goes on, in order.
    exclusive.store(false, Ordering::SeqCst);
    let mut seen = Vec::new();
    crate::waiting::eventually("the held body read", || {
        seen.extend(p.poll());
        seen.iter().any(|e| matches!(e, Event::Body { .. }))
    });
    assert!(seen.iter().any(|e| matches!(e, Event::Body { key, raw } if key.uid == 2 && *raw == noise(SIZE, 2))), "{seen:?}");
    assert!(p.compacts.held.is_empty());
    let _ = go.send(());
    drop(store);
    p.shutdown();
    drop(p);
    let _ = std::fs::remove_dir_all(&dir);
}
