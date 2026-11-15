//! A cache kept in memory, for the browser (which has no files) and for tests: made empty or
//! from an earlier snapshot, its changed files taken for the browser to keep, and made to
//! fail for tests of what a store error loses.

use super::*;

impl Store {
    /// A cache kept in memory only (clones share it).
    pub fn memory() -> Store {
        Store::with(PathBuf::from("memory"), Files::Memory(Arc::default(), Arc::default()), None)
    }

    /// A memory cache whose files can be made to fail ([`Store::fail`]), for tests of what a
    /// store error loses.
    #[doc(hidden)]
    pub fn failing() -> Store {
        let faults = files::Faults {
            files: Files::Memory(Arc::default(), Arc::default()),
            reads: Mutex::default(),
            writes: Mutex::default(),
            watch: Mutex::default(),
        };
        Store::with(PathBuf::from("memory"), Files::Failing(Arc::new(faults)), None)
    }

    /// From now on, a [`Store::failing`] cache fails to read the files whose path ends with
    /// `reads` (`state.json`, `.eml`), and to write, rename or remove those ending with `writes`;
    /// `None` lets them work again; a leading `!` makes them damaged rather than failing, and a
    /// leading `#` fails them as a full drive would; `|` separates several endings
    /// (`7.json|7.eml`). Other caches ignore it.
    #[doc(hidden)]
    pub fn fail(&self, reads: Option<&str>, writes: Option<&str>) {
        if let Files::Failing(f) = &self.files.main {
            *f.reads.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = reads.map(str::to_string);
            *f.writes.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = writes.map(str::to_string);
        }
    }

    /// A memory cache holding files from an earlier [`Store::snapshot`].
    pub fn memory_from(files: Vec<(String, Vec<u8>)>) -> Store {
        let s = Store::memory();
        if let Files::Memory(m, _) = &s.files.main {
            // Paths stay inside the cache.
            let inside = files.into_iter().filter(|(p, _)| !p.split('/').any(|seg| seg.is_empty() || seg == "." || seg == ".."));
            let entries: BTreeMap<PathBuf, Vec<u8>> = inside.map(|(p, v)| (p.split('/').fold(s.root.clone(), |acc, seg| acc.join(seg)), v)).collect();
            Files::with(m, |m| m.extend(entries));
        }
        s
    }

    /// A memory cache's files whose relative path (`a1/folders.json`) `keep` accepts; empty for
    /// a cache on disk.
    pub fn snapshot(&self, keep: impl Fn(&str) -> bool) -> Vec<(String, Vec<u8>)> {
        let Files::Memory(m, _) = &self.files.main else { return Vec::new() };
        Files::with(m, |m| m.iter().filter_map(|(k, v)| Some((self.relative(k)?, v))).filter(|(p, _)| keep(p)).map(|(p, v)| (p, v.clone())).collect())
    }

    /// Whether the cache lives in memory.
    pub fn is_memory(&self) -> bool {
        matches!(self.files.main, Files::Memory(..))
    }

    /// Start noting which files of a memory cache change, for [`Store::take_changes`].
    pub fn track_changes(&self) {
        if let Files::Memory(_, changed) = &self.files.main {
            let mut g = changed.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            g.get_or_insert_with(Default::default);
        }
    }

    /// The files of a memory cache changed since the last call (once [`Store::track_changes`]
    /// started): relative path and the bytes now, `None` for a file removed. Lets the browser
    /// keep the cache between visits.
    pub fn take_changes(&self) -> Vec<(String, Option<Vec<u8>>)> {
        let Files::Memory(m, _) = &self.files.main else { return Vec::new() };
        let paths = self.take_marked();
        Files::with(m, |m| paths.into_iter().filter_map(|k| Some((self.relative(&k)?, m.get(&k).cloned()))).collect())
    }

    /// The relative paths of [`Store::take_changes`], without the files' bytes: for one that only
    /// asks whether something it keeps changed.
    pub fn take_changed_paths(&self) -> Vec<String> {
        self.take_marked().into_iter().filter_map(|k| self.relative(&k)).collect()
    }

    /// The paths noted as changed since the last call, taken.
    fn take_marked(&self) -> std::collections::BTreeSet<PathBuf> {
        let Files::Memory(_, changed) = &self.files.main else { return Default::default() };
        let mut g = changed.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        g.as_mut().map(std::mem::take).unwrap_or_default()
    }

    /// A memory cache's file as a path relative to its root (`a1/folders.json`).
    fn relative(&self, k: &Path) -> Option<String> {
        let rel = k.strip_prefix(&self.root).ok()?;
        let rel: Vec<&str> = rel.iter().map(|s| s.to_str()).collect::<Option<_>>()?;
        Some(rel.join("/"))
    }
}
