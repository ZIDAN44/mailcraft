use proptest::prelude::*;

use super::*;

/// Names close to each other in how they sort: a folder, one its name starts, one with a dot.
const SEGMENTS: [&str; 6] = ["a", "a1", "a1x", "a.json", "a1-", "b"];

fn path(segments: &[usize]) -> PathBuf {
    segments.iter().fold(PathBuf::from("memory"), |p, &i| p.join(SEGMENTS[i % SEGMENTS.len()]))
}

fn memory(keys: &[Vec<usize>]) -> Files {
    let m: BTreeMap<PathBuf, Vec<u8>> = keys.iter().enumerate().map(|(n, k)| (path(k), vec![0; n + 1])).collect();
    Files::Memory(Arc::new(Mutex::new(m)), Arc::new(Mutex::new(Some(Default::default()))))
}

fn contents(f: &Files) -> (BTreeMap<PathBuf, Vec<u8>>, Option<std::collections::BTreeSet<PathBuf>>) {
    let Files::Memory(m, changed) = f else { return Default::default() };
    (Files::with(m, |m| m.clone()), changed.lock().unwrap().clone())
}

/// What the in-memory move, removal and size did when they went over every file.
mod every {
    use super::*;

    pub(super) fn rename(m: &mut BTreeMap<PathBuf, Vec<u8>>, from: &Path, to: &Path) -> Vec<PathBuf> {
        let moving: Vec<PathBuf> = m.keys().filter(|k| k.starts_with(from)).cloned().collect();
        let mut moved = Vec::new();
        for k in moving {
            if let (Some(v), Ok(rest)) = (m.remove(&k), k.strip_prefix(from)) {
                let new = if rest.as_os_str().is_empty() { to.to_path_buf() } else { to.join(rest) };
                m.insert(new.clone(), v);
                moved.extend([k, new]);
            }
        }
        moved
    }

    pub(super) fn remove_dir_all(m: &mut BTreeMap<PathBuf, Vec<u8>>, p: &Path) -> Vec<PathBuf> {
        let gone: Vec<PathBuf> = m.keys().filter(|k| k.starts_with(p)).cloned().collect();
        for k in &gone {
            m.remove(k);
        }
        gone
    }

    pub(super) fn size(m: &BTreeMap<PathBuf, Vec<u8>>, dir: &Path) -> u64 {
        m.iter().filter(|(k, _)| k.starts_with(dir)).map(|(_, v)| v.len() as u64).fold(0u64, u64::saturating_add)
    }
}

proptest! {
    /// Moving, removing and measuring a folder in memory by where it starts in the map does
    /// what going over every file did: the same files moved or gone, the same changes noted,
    /// the same size, whatever the names around it.
    #[test]
    fn a_folder_in_memory_is_found_where_it_starts(
        keys in prop::collection::vec(prop::collection::vec(0usize..6, 1..4), 0..24),
        dir in prop::collection::vec(0usize..6, 1..3),
        to in prop::collection::vec(0usize..6, 1..3),
    ) {
        let (dir, to) = (path(&dir), path(&to));
        let (start, _) = contents(&memory(&keys));

        let files = memory(&keys);
        prop_assert_eq!(files.size(&dir), every::size(&start, &dir));

        let mut want = start.clone();
        let moved = every::rename(&mut want, &dir, &to);
        let got = files.rename(&dir, &to);
        prop_assert_eq!(got.is_err(), moved.is_empty());
        prop_assert_eq!(contents(&files), (want, Some(moved.into_iter().collect())));

        let files = memory(&keys);
        let mut want = start.clone();
        let gone = every::remove_dir_all(&mut want, &dir);
        let got = files.remove_dir_all(&dir);
        prop_assert_eq!(got.is_err(), gone.is_empty());
        prop_assert_eq!(contents(&files), (want, Some(gone.into_iter().collect())));
    }
}

/// A read refused ("access denied") is made again after a wait for as long as it's told to go
/// on, and the answer that ends it is the one given: the file once it's let go of, the refusal
/// once time is up. Any other error is given at once, with no wait.
#[test]
fn a_refused_read_is_made_again_only_while_told_to() {
    let denied = || io::Error::from(io::ErrorKind::PermissionDenied);
    // Refused twice, then read.
    let (mut reads, mut waits) = (0u32, 0u32);
    let read = read_again(
        || {
            reads += 1;
            if reads <= 2 { Err(denied()) } else { Ok(b"ops".to_vec()) }
        },
        || true,
        || waits += 1,
    );
    assert_eq!((read.unwrap(), reads, waits), (b"ops".to_vec(), 3, 2));
    // Refused for good: given up once time is up (after three reads here).
    let (reads, mut waits) = (std::cell::Cell::new(0u32), 0u32);
    let read = read_again(
        || {
            reads.set(reads.get() + 1);
            Err(denied())
        },
        || reads.get() < 3,
        || waits += 1,
    );
    assert_eq!((read.map_err(|e| e.kind()), reads.get(), waits), (Err(io::ErrorKind::PermissionDenied), 3, 2));
    // A disk that fails: not read again.
    let (mut reads, mut waits) = (0u32, 0u32);
    let read = read_again(
        || {
            reads += 1;
            Err(io::Error::other("failed on purpose"))
        },
        || true,
        || waits += 1,
    );
    assert_eq!((read.map_err(|e| e.kind()), reads, waits), (Err(io::ErrorKind::Other), 1, 0));
}

/// Only Windows refuses a read for a moment as the file is replaced; anywhere else "access
/// denied" is the file's permissions, and a read of the disk is made once: it waited half a
/// second there on every file that couldn't be read.
#[cfg(unix)]
#[test]
fn a_file_that_cant_be_read_is_read_once_off_windows() {
    use std::os::unix::fs::PermissionsExt;
    let dir = crate::tests::scratch("read-once");
    fs::create_dir_all(&dir).unwrap();
    let p = dir.join("ops.json");
    fs::write(&p, b"[]").unwrap();
    fs::set_permissions(&p, fs::Permissions::from_mode(0o000)).unwrap();
    // Run as root, every file can be read: nothing to see.
    if fs::read(&p).is_ok() {
        return;
    }
    let began = std::time::Instant::now();
    for _ in 0..20 {
        assert_eq!(Files::Disk.read(&p).map_err(|e| e.kind()), Err(io::ErrorKind::PermissionDenied));
    }
    // 20 reads tried again for half a second each are 10 s.
    assert!(began.elapsed() < std::time::Duration::from_secs(5), "{:?}", began.elapsed());
}

/// The in-memory move, removal and size of one account's folder among many accounts' files,
/// against going over every file as they did.
#[test]
#[ignore = "a measurement: cargo test --release -p mailcraft-store --lib files::tests::measure_one_account_among_many -- --ignored --nocapture"]
fn measure_one_account_among_many() {
    const ACCOUNTS: usize = 200;
    const FILES: usize = 1000;
    const RUNS: u32 = 200;
    let mut all = BTreeMap::new();
    for a in 0..ACCOUNTS {
        for f in 0..FILES {
            all.insert(PathBuf::from("memory").join(format!("acct{a}")).join(format!("f{f}")), vec![0u8; 16]);
        }
    }
    let small = Path::new("memory/acct7/f9");
    let to = Path::new("memory/acct7/g9");
    let files = Files::Memory(Arc::new(Mutex::new(all.clone())), Arc::default());
    let now = std::time::Instant::now();
    for _ in 0..RUNS {
        files.rename(small, to).unwrap();
        files.rename(to, small).unwrap();
        assert_eq!(files.size(small), 16);
        files.remove_dir_all(small).unwrap();
        files.write(small, &[0u8; 16]).unwrap();
    }
    let after = now.elapsed().as_secs_f64() * 1000.0 / f64::from(RUNS);
    let mut m = all;
    let now = std::time::Instant::now();
    for _ in 0..RUNS {
        every::rename(&mut m, small, to);
        every::rename(&mut m, to, small);
        assert_eq!(every::size(&m, small), 16);
        every::remove_dir_all(&mut m, small);
        m.insert(small.to_path_buf(), vec![0u8; 16]);
    }
    let before = now.elapsed().as_secs_f64() * 1000.0 / f64::from(RUNS);
    println!(
        "MEASURE one file's move, size and removal among {} in memory: over every file {before:.3} ms, from where it starts {after:.4} ms",
        ACCOUNTS * FILES
    );
}
