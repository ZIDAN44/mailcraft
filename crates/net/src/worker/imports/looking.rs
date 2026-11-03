//! Data files looked at before they're imported: what kind each is, how big, what it holds as
//! its import would plan it, and where MailCraft's copy goes, with the room there.

use super::*;

/// A data file as it was when it was looked at ([`Job::InspectDataFile`]), and its plan.
pub(super) struct Looked {
    pub(super) path: String,
    pub(super) size: u64,
    pub(super) modified: i64,
    pub(super) planned: Arc<Planned>,
}

impl Pool {
    /// Look at a data file before it's imported, on a thread of its own (it may be on a slow
    /// drive): what kind it is, how big, when it last changed, and for an Outlook one what it
    /// holds, planned as its import would bring it in; and how big MailCraft's copy will be, and
    /// the room for it where it goes: an Outlook file's beside it (or the copy made there
    /// earlier, and whether the file changed since), any other with the data files. That plan is
    /// kept, and the file's import goes by it when the file hasn't changed since. A file that
    /// makes the reader panic is said to be one that couldn't be looked at.
    pub(crate) fn inspect_data_file(&mut self, path: String) {
        let (sink, asked, looked, store) = (self.sink.clone(), path.clone(), self.imports.looked.clone(), self.store.clone());
        let dir = self.store.data_files_dir().unwrap_or_else(|| self.store.root().to_path_buf());
        let space = drive_for(&path);
        let spawned = std::thread::Builder::new().name("mail-inspect".into()).spawn(move || {
            let looked_at = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| look_at(&path, &looked)));
            let mut facts = looked_at.unwrap_or_else(|_| DataFileFacts {
                path: path.clone(),
                error: Some("the data file couldn't be looked at: it may be damaged".into()),
                ..DataFileFacts::default()
            });
            let beside = if facts.error.is_none() && facts.kind.starts_with("pst") { store.copy_beside(Path::new(&path)) } else { None };
            let found = beside.as_ref().and_then(|b| b.found.clone());
            let destination = match &beside {
                // Read again afresh, with the data files, before what's new is added to the copy.
                _ if found.as_ref().is_some_and(|f| f.changed) => dir,
                Some(b) if !b.elsewhere => b.path.parent().map_or(dir, Path::to_path_buf),
                _ => dir,
            };
            room_for(&mut facts, &destination, space.as_ref());
            if let Some(b) = beside {
                facts.elsewhere = b.elsewhere;
                if let Some(f) = found {
                    facts.copy = b.path.to_string_lossy().into_owned();
                    facts.copy_changed = f.changed;
                    facts.copy_rereadable = f.seen;
                    if f.held {
                        facts.copy_held_by.clone_from(&f.owner_path);
                    }
                    // A copy whole and up to date is opened as it is: nothing to write.
                    if !f.changed && f.state == "done" {
                        (facts.estimate, facts.need) = (0, 0);
                    }
                }
            }
            sink.emit(Event::DataFileInspected { path, facts: Box::new(facts) });
        });
        if let Err(e) = spawned {
            let facts = DataFileFacts { path: asked.clone(), error: Some(format!("inspect: {e}")), ..DataFileFacts::default() };
            self.sink.emit(Event::DataFileInspected { path: asked, facts: Box::new(facts) });
        }
    }
}

/// Where the room is asked for a data file looked at, or for an account's copy (by the account):
/// the computer's own drives (a test's, in a test that gave one).
pub(super) fn drive_for(path: &str) -> Arc<dyn crate::space::Space> {
    #[cfg(test)]
    if let Some(drive) = tests::space_for(path) {
        return drive;
    }
    let _ = path;
    Arc::new(crate::space::Drives)
}

/// What the data file at `path` holds, planned (an Outlook one's plan kept in `looked`).
pub(super) fn look_at(path: &str, looked: &Mutex<Option<Looked>>) -> DataFileFacts {
    let mut facts = facts_of(path);
    if facts.error.is_none() && facts.kind == "pst" {
        match crate::pst::inspect(Path::new(path)) {
            Ok(planned) => {
                (facts.messages, facts.folders, facts.pim_items, facts.bytes) = (planned.messages, planned.folders, planned.pim_items, planned.bytes);
                facts.record_key.clone_from(&planned.record_key);
                let planned = Arc::new(planned);
                *lock(looked) = Some(Looked { path: path.to_string(), size: facts.size, modified: facts.modified, planned });
            }
            Err(e) => facts.error = Some(e),
        }
    }
    facts
}

/// The size MailCraft's copy of a data file is expected to come to, the room asked for to write
/// it, and the room there is in `destination` (on the drive of the nearest folder of it that's
/// there; none when that can't be told). An Outlook file's copy is about [`COPY_PCT`] of its
/// messages' own sizes; one of MailCraft's is copied as it is, and read from a copy of it made on
/// that drive first ([`copy_need`]).
pub(super) fn room_for(facts: &mut DataFileFacts, destination: &Path, space: &dyn crate::space::Space) {
    facts.estimate = if facts.kind == "mailcraft" { facts.size } else { facts.bytes / 100 * COPY_PCT };
    facts.need = if facts.kind == "mailcraft" { copy_need(facts.size) } else { need(facts.estimate) };
    facts.destination = destination.to_string_lossy().into_owned();
    facts.free = space.free(destination);
}

/// The room asked for a copy of `estimate` bytes.
pub(super) fn need(estimate: u64) -> u64 {
    (estimate / 100).saturating_mul(NEED_PCT).saturating_add(NEED_EXTRA)
}

/// The room asked for a MailCraft data file of `size` bytes copied in: it's read from a copy of
/// it made beside the data files (the whole file, until the end), as the account's own is
/// written there, so about twice its size at the peak.
pub(super) fn copy_need(size: u64) -> u64 {
    need(size).saturating_add(size)
}

/// The plan made of the data file at `path` when it was looked at, while the file is as it was.
pub(super) fn planned_for(looked: &Mutex<Option<Looked>>, path: &str) -> Option<Arc<Planned>> {
    let now = facts_of(path);
    let looked = lock(looked);
    looked.as_ref().filter(|l| l.path == path && now.error.is_none() && (l.size, l.modified) == (now.size, now.modified)).map(|l| l.planned.clone())
}

/// What the file system says of a data file (nothing read from inside it).
pub(super) fn facts_of(path: &str) -> DataFileFacts {
    let lower = path.to_ascii_lowercase();
    let kind = [".pst", ".ost", ".mailcraft"].iter().find(|k| lower.ends_with(*k)).map_or("", |k| k.trim_start_matches('.'));
    let mut facts = DataFileFacts { path: path.to_string(), kind: kind.to_string(), ..DataFileFacts::default() };
    match std::fs::metadata(path) {
        Ok(m) if m.is_file() => {
            facts.size = m.len();
            facts.modified =
                m.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX));
        }
        Ok(_) => facts.error = Some("not a file".into()),
        Err(e) => facts.error = Some(e.to_string()),
    }
    facts
}
