//! Store tests: what a memory cache says changed, for the browser to keep.

use super::*;

/// The browser without IndexedDB asks only whether what it keeps changed (its local storage
/// was written whole every ten seconds).
#[test]
fn a_memory_cache_names_its_changes_without_their_bytes() {
    let s = Store::memory();
    assert!(s.take_changed_paths().is_empty(), "not tracked yet");
    s.track_changes();
    assert!(s.take_changed_paths().is_empty(), "nothing changed");
    s.save_doc("rules.json", &vec![2]).unwrap();
    s.save_account_doc("a1", "windows.json", &vec![1]).unwrap();
    s.remove_account_doc("a1", "windows.json").unwrap();
    assert_eq!(s.take_changed_paths(), ["a1/windows.json", "rules.json"], "a removal is a change too");
    assert!(s.take_changed_paths().is_empty(), "taken");
    assert!(s.take_changes().is_empty(), "taken for both");
}

/// What the browser's autosave without IndexedDB cost every ten seconds with nothing changed:
/// before, the whole cache copied, measured and serialised (`web.rs`, `save`); now, the changed
/// paths asked for.
#[test]
#[ignore = "measurement: cargo test --release -p mailcraft-store --lib autosave -- --ignored --nocapture"]
fn measure_an_unchanged_autosave() {
    let s = Store::memory();
    // About 3 MB of documents, the most local storage is given.
    let text = "x".repeat(100_000);
    for i in 0..30 {
        s.save_account_doc(&format!("a{i}"), "big.json", &text).unwrap();
    }
    s.track_changes();
    let keep = |p: &str| p.ends_with(".json") && !p.ends_with(".bad") && !p.contains("/outbox/");
    let rounds = 50;
    let t = std::time::Instant::now();
    let mut written = 0;
    for _ in 0..rounds {
        let files: Vec<(String, String)> = s.snapshot(keep).into_iter().filter_map(|(p, v)| Some((p, String::from_utf8(v).ok()?))).collect();
        let size = |f: &[(String, String)]| f.iter().map(|(p, v)| p.len() + v.len()).sum::<usize>();
        if size(&files) <= 3 << 20 {
            written += serde_json::to_string(&files).unwrap().len();
        }
    }
    let before = t.elapsed().as_secs_f64() * 1000.0 / f64::from(rounds);
    let t = std::time::Instant::now();
    let mut changed = 0;
    for _ in 0..rounds {
        changed += usize::from(s.take_changed_paths().iter().any(|p| keep(p)));
    }
    let after = t.elapsed().as_secs_f64() * 1000.0 / f64::from(rounds);
    assert!(written > 0 && changed == 0);
    println!("MEASURE unchanged autosave of 3 MB: before {before:.3} ms, after {after:.6} ms");
}

#[test]
fn a_memory_cache_reports_its_changes_once_asked() {
    let s = Store::memory();
    s.save_doc("rules.json", &vec![1]).unwrap();
    assert!(s.take_changes().is_empty(), "not tracked yet");
    s.save_folders("a1", &[Folder { account: "a1".into(), path: "Projects".into(), delimiter: Some('/'), ..Default::default() }]).unwrap();
    s.track_changes();
    s.save_doc("rules.json", &vec![2]).unwrap();
    let key = MessageKey { account: "a1".into(), folder: "Projects".into(), validity: 1, uid: 4 };
    s.write_body(
        &key,
        b"Subject: x

hi",
    )
    .unwrap();
    let changes = s.take_changes();
    assert_eq!(changes.len(), 2, "{changes:?}");
    assert!(changes.contains(&("rules.json".into(), Some(b"[2]".to_vec()))));
    let body = changes.iter().find(|(p, _)| p.ends_with("/4.eml")).unwrap();
    assert!(body.0.starts_with("a1/f"));
    assert!(s.take_changes().is_empty(), "taken");
    // A rename is a removal and a write; removing an account removes each file.
    s.apply_op("a1", &Op::RenameFolder { from: "Projects".into(), to: "Work".into() }).unwrap();
    let changes = s.take_changes();
    assert!(changes.iter().any(|(p, v)| p == &body.0 && v.is_none()), "{changes:?}");
    assert!(changes.iter().any(|(p, v)| p.ends_with("/4.eml") && p != &body.0 && v.is_some()));
    s.remove_account("a1").unwrap();
    assert!(s.take_changes().iter().filter(|(p, _)| p.ends_with(".eml")).all(|(_, v)| v.is_none()));
    // A store kept from them comes back the same.
    let back = Store::memory_from(s.snapshot(|_| true));
    assert_eq!(back.load_doc::<Vec<i32>>("rules.json").unwrap(), vec![2]);
}

/// The browser's cache is in memory, kept in IndexedDB by what it changes: a change waiting for
/// the server goes out with those changes and comes back from them on the next visit, and
/// leaves with them once sent (the page held its queue itself, and lost it to a reload).
#[test]
fn a_memory_cache_keeps_its_queued_changes_between_visits() {
    let s = Store::memory();
    s.track_changes();
    let op = Op::SetFlag { keys: vec![msg(1, false).key], flag: Flag::Flagged, on: true };
    s.add_op("a1", op.clone()).unwrap();
    let kept: Vec<(String, Vec<u8>)> = s.take_changes().into_iter().filter_map(|(p, b)| Some((p, b?))).collect();
    assert_eq!(kept.iter().map(|(p, _)| p.as_str()).collect::<Vec<_>>(), ["a1/ops.json"]);
    let next = Store::memory_from(kept);
    next.track_changes();
    assert_eq!(next.load_ops("a1").unwrap(), std::slice::from_ref(&op), "back on the next visit");
    next.remove_op("a1", &op).unwrap();
    assert_eq!(next.take_changes(), [("a1/ops.json".to_string(), None)], "gone from what's kept once sent");
}
