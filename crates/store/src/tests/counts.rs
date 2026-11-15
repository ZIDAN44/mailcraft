//! Store tests: an account's folder counts set, and folders deleted, in a big folder list.

use proptest::prelude::*;

use super::*;

/// [`folders::set_counts`] as it was, a search of the list for each count: what it does is the
/// reference.
fn set_counts_by_search(list: &mut [Folder], counts: &[(String, u32, u32)]) -> (usize, bool) {
    let (mut found, mut changed) = (0, false);
    for (path, unread, total) in counts {
        if let Some(f) = list.iter_mut().find(|f| f.path == *path) {
            found += 1;
            changed |= f.unread != *unread || f.total != *total;
            f.unread = *unread;
            f.total = *total;
        }
    }
    (found, changed)
}

fn folder(path: String, unread: u32, total: u32) -> Folder {
    Folder { account: "a1".into(), path, unread, total, ..Default::default() }
}

proptest! {
    /// The counts set as a search of the list set them: the first folder at a path (a damaged
    /// list may name one twice), a path counted twice set twice, paths the list hasn't left out.
    #[test]
    fn counts_are_set_as_a_search_sets_them(
        list in proptest::collection::vec((0u8..12, 0u32..3, 0u32..3), 0..20),
        counts in proptest::collection::vec((0u8..14, 0u32..3, 0u32..3), 0..20),
    ) {
        let mut a: Vec<Folder> = list.iter().map(|(p, u, t)| folder(format!("F{p}"), *u, *t)).collect();
        let mut b = a.clone();
        let counts: Vec<(String, u32, u32)> = counts.iter().map(|(p, u, t)| (format!("F{p}"), *u, *t)).collect();
        let by_search = set_counts_by_search(&mut a, &counts);
        let now = folders::set_counts(&mut b, &counts);
        prop_assert_eq!(by_search, now);
        prop_assert_eq!(a, b);
    }
}

/// A folder deleted with its subfolders in a big list: they all go, and nothing else.
#[test]
fn a_folder_deleted_takes_its_subfolders_and_nothing_else() {
    let s = Store::memory();
    let mut list: Vec<Folder> = (0..300).map(|i| folder(format!("Projects/P{i}"), 0, 0)).collect();
    list.extend((0..300).map(|i| folder(format!("Archive/A{i}"), 0, 0)));
    list.push(folder("Projects".into(), 0, 0));
    list.push(folder("ProjectsOld".into(), 0, 0));
    for f in &mut list {
        f.delimiter = Some('/');
    }
    s.save_folders("a1", &list).unwrap();
    s.apply_op("a1", &Op::DeleteFolder { path: "Projects".into() }).unwrap();
    let left: Vec<String> = s.load_folders("a1").unwrap().into_iter().map(|f| f.path).collect();
    assert_eq!(left.len(), 301, "{left:?}");
    assert!(left.iter().all(|p| p.starts_with("Archive/") || p == "ProjectsOld"));
}

/// How long the counts of every folder of a big list (an imported data file's) take to set, and a
/// folder with as many subfolders to delete:
/// `cargo test --release -p mailcraft-store --lib measure_counts_of_many_folders -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement: run alone with --ignored --nocapture"]
fn measure_counts_of_many_folders() {
    const N: usize = 5_000;
    const RUNS: usize = 11;
    let list: Vec<Folder> = (0..N).map(|i| folder(format!("Imported/Folder number {i:05}"), 0, 0)).collect();
    let counts: Vec<(String, u32, u32)> = (0..N).rev().map(|i| (format!("Imported/Folder number {i:05}"), 1, 2)).collect();
    let mut times: Vec<f64> = (0..RUNS)
        .map(|_| {
            let mut l = list.clone();
            let t = std::time::Instant::now();
            let (found, _) = folders::set_counts(&mut l, &counts);
            let took = t.elapsed().as_secs_f64() * 1000.0;
            assert_eq!(found, N);
            took
        })
        .collect();
    times.sort_by(f64::total_cmp);
    let counts_ms = times.get(RUNS / 2).copied().unwrap_or_default();
    let mut deleted: Vec<f64> = (0..RUNS)
        .map(|_| {
            let s = Store::memory();
            let mut l: Vec<Folder> = list.iter().map(|f| Folder { delimiter: Some('/'), ..f.clone() }).collect();
            l.push(Folder { delimiter: Some('/'), ..folder("Imported".into(), 0, 0) });
            s.save_folders("a1", &l).unwrap();
            let t = std::time::Instant::now();
            s.apply_op("a1", &Op::DeleteFolder { path: "Imported".into() }).unwrap();
            t.elapsed().as_secs_f64() * 1000.0
        })
        .collect();
    deleted.sort_by(f64::total_cmp);
    let delete_ms = deleted.get(RUNS / 2).copied().unwrap_or_default();
    println!(
        "MEASURE counts of {N} folders set: median {counts_ms:.2} ms; a folder of {N} subfolders deleted (memory cache): median {delete_ms:.2} ms"
    );
}
