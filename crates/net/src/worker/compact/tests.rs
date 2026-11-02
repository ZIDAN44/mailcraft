//! Compactions as the pool runs them: on a thread of their own, by the plan, told as they go and
//! as they end; and an account's jobs held while its file is had to itself.

use mailcraft_model::job::{CompactMode, CompactPhase, CompactReport};

use super::*;

fn temp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("mailcraft-compact-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

/// Bytes that don't compress (xorshift).
fn noise(n: usize, seed: u64) -> Vec<u8> {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect()
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

/// Events until one says the compaction ended (or a minute passes), and what it came to.
fn until_compacted(p: &mut Pool) -> (Vec<Event>, Option<CompactReport>) {
    let end = Instant::now() + Duration::from_secs(60);
    let mut seen = Vec::new();
    while Instant::now() < end {
        for e in p.poll() {
            if let Event::Compacted { report, .. } = &e {
                let report = (**report).clone();
                seen.push(e);
                return (seen, Some(report));
            }
            seen.push(e);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    (seen, None)
}

#[test]
fn asked_for_a_data_file_is_compacted_on_its_thread() {
    let (dir, store, mut p) = setup("manual");
    let before = std::fs::metadata(store.data_file("c1").unwrap()).unwrap().len();
    p.submit(Job::DataFileHealth { account: "c1".into() });
    let end = Instant::now() + Duration::from_secs(30);
    let health = loop {
        assert!(Instant::now() < end, "no health");
        if let Some(Event::DataFileHealth { health, error, .. }) = p.poll().into_iter().find(|e| matches!(e, Event::DataFileHealth { .. })) {
            assert_eq!(error, None);
            break health.unwrap();
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    assert!(health.can_compact() && !health.needs_compacting(), "offered, not done at idle: {health:?}");

    // At idle it isn't worth it: nothing done, and said so.
    p.submit(Job::CompactDataFile { account: "c1".into(), mode: CompactMode::Idle });
    let (_, report) = until_compacted(&mut p);
    let report = report.unwrap();
    assert!(!report.compacted && report.error.is_none() && report.before == report.after, "{report:?}");

    // Asked for, it is: in place, the file had to itself for a moment, said as it begins.
    p.submit(Job::CompactDataFile { account: "c1".into(), mode: CompactMode::Manual });
    let (seen, report) = until_compacted(&mut p);
    let report = report.unwrap();
    assert!(report.compacted && report.error.is_none() && report.mode == CompactMode::Manual, "{report:?}");
    assert!(report.before == before && report.after < before, "{report:?}");
    assert!(seen.iter().any(|e| matches!(e, Event::Compacting { phase: CompactPhase::Exclusive, exclusive_s: Some(_), .. })), "{seen:?}");
    assert_eq!(store.read_body(&key(3)).unwrap(), noise(SIZE, 3), "every message reads back");
    p.shutdown();
    drop(p);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_second_compaction_and_one_beside_an_import_are_refused() {
    let (dir, _store, mut p) = setup("refused");
    // One running (its thread held up by the drive it asks for room).
    p.compacts.running.insert(
        "c1".into(),
        CompactCtl {
            cancel: Arc::default(),
            exclusive: Arc::default(),
            thread: std::thread::spawn(|| std::thread::sleep(Duration::from_millis(500))),
        },
    );
    p.submit(Job::CompactDataFile { account: "c1".into(), mode: CompactMode::Manual });
    let (_, report) = until_compacted(&mut p);
    assert!(report.unwrap().error.is_some_and(|e| e.contains("already")));
    p.submit(Job::CancelCompact { account: "c1".into() });
    assert!(p.compacts.running.get("c1").is_some_and(|c| c.cancel.load(Ordering::Relaxed)), "asked to stop");
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
    let end = Instant::now() + Duration::from_secs(10);
    let mut seen = Vec::new();
    while !seen.iter().any(|e| matches!(e, Event::Body { .. })) && Instant::now() < end {
        seen.extend(p.poll());
    }
    assert!(seen.iter().any(|e| matches!(e, Event::Body { key, raw } if key.uid == 2 && *raw == noise(SIZE, 2))), "{seen:?}");
    assert!(p.compacts.held.is_empty());
    let _ = go.send(());
    drop(store);
    p.shutdown();
    drop(p);
    let _ = std::fs::remove_dir_all(&dir);
}
