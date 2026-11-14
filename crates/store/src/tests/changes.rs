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
