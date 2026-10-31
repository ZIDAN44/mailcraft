//! Where the store's files live: on disk, in data files, or in memory; and which of them each
//! path goes to (an online-only account's are held in memory).

use super::*;

/// Where the files live.
#[derive(Clone, Debug)]
pub(super) enum Files {
    Disk,
    /// Each account in one data file (`packed.rs`), the rest on disk.
    #[cfg(not(target_arch = "wasm32"))]
    Packed(Arc<packed::Packs>),
    /// The files, and the paths changed since they were last taken (when that's tracked).
    Memory(Arc<Mutex<BTreeMap<PathBuf, Vec<u8>>>>, Arc<Mutex<Option<std::collections::BTreeSet<PathBuf>>>>),
    /// Memory whose reads or writes of some files fail, as a disk or a data file can
    /// ([`Store::failing`]): what a store error loses is tested with it.
    Failing(Arc<Faults>),
}

/// The files of [`Files::Failing`], and the file-name endings whose reads and writes fail now.
#[derive(Debug)]
pub(super) struct Faults {
    pub(super) files: Files,
    pub(super) reads: Mutex<Option<String>>,
    pub(super) writes: Mutex<Option<String>>,
}

impl Faults {
    /// An error when `rule` names how `p` ends: never "not found", which the store reads as an
    /// empty file. A rule starting with `!` is a damaged file (invalid data), not a failing disk.
    fn check(rule: &Mutex<Option<String>>, p: &Path) -> io::Result<()> {
        let rule = rule.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(rule) = rule.as_deref() else { return Ok(()) };
        let (damaged, end) = match rule.strip_prefix('!') {
            Some(end) => (true, end),
            None => (false, rule),
        };
        if !p.to_string_lossy().replace('\\', "/").ends_with(end) {
            return Ok(());
        }
        Err(if damaged { io::Error::new(io::ErrorKind::InvalidData, "damaged on purpose") } else { io::Error::other("failed on purpose") })
    }
}

fn not_found() -> io::Error {
    io::Error::from(io::ErrorKind::NotFound)
}

/// The paths at or under `dir` in memory, from where they start in the map (paths sort by
/// component, so everything under a folder is together): an online-only account's listing doesn't
/// pass over every other file.
fn under<'a>(m: &'a BTreeMap<PathBuf, Vec<u8>>, dir: &'a Path) -> impl Iterator<Item = &'a PathBuf> + 'a {
    use std::ops::Bound;
    m.range::<Path, _>((Bound::Included(dir), Bound::Unbounded)).map(|(k, _)| k).take_while(move |k| k.starts_with(dir))
}

impl Files {
    pub(super) fn with<T>(m: &Arc<Mutex<BTreeMap<PathBuf, Vec<u8>>>>, f: impl FnOnce(&mut BTreeMap<PathBuf, Vec<u8>>) -> T) -> T {
        let mut g = m.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        f(&mut g)
    }

    /// Note changed paths, when changes are tracked.
    fn mark<'a>(&self, paths: impl IntoIterator<Item = &'a Path>) {
        if let Files::Memory(_, changed) = self {
            let mut g = changed.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(set) = g.as_mut() {
                set.extend(paths.into_iter().map(Path::to_path_buf));
            }
        }
    }

    pub(super) fn read(&self, p: &Path) -> io::Result<Vec<u8>> {
        match self {
            Files::Disk => fs::read(p),
            #[cfg(not(target_arch = "wasm32"))]
            Files::Packed(x) => x.read(p),
            Files::Memory(m, _) => Files::with(m, |m| m.get(p).cloned().ok_or_else(not_found)),
            Files::Failing(f) => {
                Faults::check(&f.reads, p)?;
                f.files.read(p)
            }
        }
    }

    pub(super) fn write(&self, p: &Path, bytes: &[u8]) -> Result<()> {
        match self {
            Files::Disk => write_atomic(p, bytes),
            #[cfg(not(target_arch = "wasm32"))]
            Files::Packed(x) => x.write(p, bytes),
            Files::Memory(m, _) => {
                Files::with(m, |m| m.insert(p.to_path_buf(), bytes.to_vec()));
                self.mark([p]);
                Ok(())
            }
            Files::Failing(f) => {
                Faults::check(&f.writes, p).map_err(io_err(p))?;
                f.files.write(p, bytes)
            }
        }
    }

    /// Many files at once (importing a data file: desktop only).
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn write_many(&self, items: &[(PathBuf, Vec<u8>)]) -> Result<()> {
        match self {
            Files::Packed(x) => x.write_many(items),
            _ => {
                for (p, bytes) in items {
                    self.write(p, bytes)?;
                }
                Ok(())
            }
        }
    }

    pub(super) fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        match self {
            Files::Disk => atomic::rename(from, to),
            #[cfg(not(target_arch = "wasm32"))]
            Files::Packed(x) => x.rename(from, to),
            Files::Memory(m, _) => {
                let moved = Files::with(m, |m| {
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
                });
                if moved.is_empty() {
                    return Err(not_found());
                }
                self.mark(moved.iter().map(PathBuf::as_path));
                Ok(())
            }
            Files::Failing(f) => {
                Faults::check(&f.writes, from)?;
                Faults::check(&f.writes, to)?;
                f.files.rename(from, to)
            }
        }
    }

    pub(super) fn remove_file(&self, p: &Path) -> io::Result<()> {
        match self {
            Files::Disk => fs::remove_file(p),
            #[cfg(not(target_arch = "wasm32"))]
            Files::Packed(x) => x.remove_file(p),
            Files::Memory(m, _) => {
                Files::with(m, |m| m.remove(p).map(|_| ()).ok_or_else(not_found))?;
                self.mark([p]);
                Ok(())
            }
            Files::Failing(f) => {
                Faults::check(&f.writes, p)?;
                f.files.remove_file(p)
            }
        }
    }

    pub(super) fn remove_dir_all(&self, p: &Path) -> io::Result<()> {
        match self {
            Files::Disk => fs::remove_dir_all(p),
            #[cfg(not(target_arch = "wasm32"))]
            Files::Packed(x) => x.remove_dir_all(p),
            Files::Memory(m, _) => {
                let gone: Vec<PathBuf> = Files::with(m, |m| {
                    let gone: Vec<PathBuf> = m.keys().filter(|k| k.starts_with(p)).cloned().collect();
                    for k in &gone {
                        m.remove(k);
                    }
                    gone
                });
                if gone.is_empty() {
                    return Err(not_found());
                }
                self.mark(gone.iter().map(PathBuf::as_path));
                Ok(())
            }
            Files::Failing(f) => {
                Faults::check(&f.writes, p)?;
                f.files.remove_dir_all(p)
            }
        }
    }

    pub(super) fn is_file(&self, p: &Path) -> bool {
        match self {
            Files::Disk => p.is_file(),
            #[cfg(not(target_arch = "wasm32"))]
            Files::Packed(x) => x.is_file(p),
            Files::Memory(m, _) => Files::with(m, |m| m.contains_key(p)),
            Files::Failing(f) => f.files.is_file(p),
        }
    }

    pub(super) fn exists(&self, p: &Path) -> bool {
        match self {
            Files::Disk => p.exists(),
            #[cfg(not(target_arch = "wasm32"))]
            Files::Packed(x) => x.exists(p),
            Files::Memory(m, _) => Files::with(m, |m| under(m, p).next().is_some()),
            Files::Failing(f) => f.files.exists(p),
        }
    }

    /// The files directly in a folder, by name.
    pub(super) fn list(&self, dir: &Path) -> Vec<(String, PathBuf)> {
        match self {
            #[cfg(not(target_arch = "wasm32"))]
            Files::Packed(x) => x.list(dir),
            Files::Disk => {
                let Ok(rd) = fs::read_dir(dir) else { return Vec::new() };
                rd.filter_map(|e| e.ok())
                    .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
                    .filter_map(|e| Some((e.file_name().to_str()?.to_string(), e.path())))
                    .collect()
            }
            Files::Memory(m, _) => Files::with(m, |m| {
                under(m, dir).filter(|k| k.parent() == Some(dir)).filter_map(|k| Some((k.file_name()?.to_str()?.to_string(), k.clone()))).collect()
            }),
            Files::Failing(f) => f.files.list(dir),
        }
    }

    /// Every file under a folder, at any depth.
    pub(super) fn walk(&self, dir: &Path) -> Vec<PathBuf> {
        fn disk(p: &Path, depth: usize, out: &mut Vec<PathBuf>) {
            let Ok(rd) = fs::read_dir(p) else { return };
            for e in rd.filter_map(|e| e.ok()) {
                match e.file_type() {
                    Ok(t) if t.is_dir() && depth < 8 => disk(&e.path(), depth + 1, out),
                    Ok(t) if t.is_file() => out.push(e.path()),
                    _ => {}
                }
            }
        }
        match self {
            Files::Disk => {
                let mut out = Vec::new();
                disk(dir, 0, &mut out);
                out
            }
            #[cfg(not(target_arch = "wasm32"))]
            Files::Packed(x) => x.walk(dir),
            Files::Memory(m, _) => Files::with(m, |m| under(m, dir).cloned().collect()),
            Files::Failing(f) => f.files.walk(dir),
        }
    }

    /// Bytes under a folder.
    pub(super) fn size(&self, dir: &Path) -> u64 {
        fn walk(p: &Path, depth: usize) -> u64 {
            let Ok(rd) = fs::read_dir(p) else { return 0 };
            rd.filter_map(|e| e.ok())
                .map(|e| match e.file_type() {
                    Ok(t) if t.is_dir() && depth < 8 => walk(&e.path(), depth + 1),
                    Ok(t) if t.is_file() => e.metadata().map(|m| m.len()).unwrap_or(0),
                    _ => 0,
                })
                .fold(0u64, u64::saturating_add)
        }
        match self {
            Files::Disk => walk(dir, 0),
            #[cfg(not(target_arch = "wasm32"))]
            Files::Packed(x) => x.size(dir).unwrap_or_else(|| walk(dir, 0)),
            Files::Memory(m, _) => {
                Files::with(m, |m| m.iter().filter(|(k, _)| k.starts_with(dir)).map(|(_, v)| v.len() as u64).fold(0u64, u64::saturating_add))
            }
            Files::Failing(f) => f.files.size(dir),
        }
    }
}

/// Where each path goes: an account kept online only ([`Store::set_online_only`]) in memory,
/// everything else in the store's own files.
#[derive(Clone, Debug)]
pub(super) struct Routed {
    pub(super) root: PathBuf,
    pub(super) main: Files,
    /// The online-only accounts' files: never written anywhere.
    pub(super) held: Files,
    /// Those accounts (their folder names, `safe_segment`).
    pub(super) online: Arc<Mutex<std::collections::BTreeSet<String>>>,
}

impl Routed {
    pub(super) fn new(root: &Path, main: Files) -> Routed {
        Routed { root: root.to_path_buf(), main, held: Files::Memory(Arc::default(), Arc::default()), online: Arc::default() }
    }

    pub(super) fn online(&self) -> std::sync::MutexGuard<'_, std::collections::BTreeSet<String>> {
        self.online.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The files a path is in: an online-only account's are held in memory.
    fn at(&self, p: &Path) -> &Files {
        let account = p.strip_prefix(&self.root).ok().and_then(|r| r.iter().next()).and_then(|a| a.to_str());
        match account {
            Some(a) if self.online().contains(a) => &self.held,
            _ => &self.main,
        }
    }

    pub(super) fn read(&self, p: &Path) -> io::Result<Vec<u8>> {
        self.at(p).read(p)
    }

    pub(super) fn write(&self, p: &Path, bytes: &[u8]) -> Result<()> {
        self.at(p).write(p, bytes)
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn write_many(&self, items: &[(PathBuf, Vec<u8>)]) -> Result<()> {
        let (held, main): (Vec<_>, Vec<_>) = items.iter().cloned().partition(|(p, _)| std::ptr::eq(self.at(p), &self.held));
        if !held.is_empty() {
            self.held.write_many(&held)?;
        }
        if main.is_empty() { Ok(()) } else { self.main.write_many(&main) }
    }

    pub(super) fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        self.at(from).rename(from, to)
    }

    pub(super) fn remove_file(&self, p: &Path) -> io::Result<()> {
        self.at(p).remove_file(p)
    }

    pub(super) fn remove_dir_all(&self, p: &Path) -> io::Result<()> {
        self.at(p).remove_dir_all(p)
    }

    pub(super) fn is_file(&self, p: &Path) -> bool {
        self.at(p).is_file(p)
    }

    pub(super) fn exists(&self, p: &Path) -> bool {
        self.at(p).exists(p)
    }

    pub(super) fn list(&self, dir: &Path) -> Vec<(String, PathBuf)> {
        self.at(dir).list(dir)
    }

    pub(super) fn size(&self, dir: &Path) -> u64 {
        self.at(dir).size(dir)
    }
}
