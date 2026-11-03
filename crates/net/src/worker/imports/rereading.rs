//! Data files read again: an Outlook file whose copy MailCraft keeps beside it, changed since it
//! was read, is read afresh into a store of its own (a folder beside MailCraft's data files,
//! gone once it's done), and what it has that the copy hasn't is added to the copy
//! (`Store::merge_from`). The file itself is only read, as any import reads it. Stopped, paused
//! or failed part-way, nothing of it is added and the account is as it was: it's read again from
//! the start when asked. The account's mail is never removed by it, whatever becomes of it.

use std::path::Path;

use mailcraft_model::job::{ImportPhase, ImportReport, SkipReason, StopHow};
use mailcraft_store::Merged;

use super::*;

/// The account the file is read into, in its own store.
const READ: &str = "reread";
/// The import's own note in the account (`pst::resume`): its source is the file as last read.
const NOTE_DOC: &str = "import.json";

/// The window told how the read goes, as an import's: its progress only. The folders it reads
/// into are its own store's, not the account's (those come in once they're added).
struct Relay<'t, 'a>(&'t mut Told<'a>);

impl ImportSink for Relay<'_, '_> {
    fn planned(&mut self, _: &[mailcraft_model::job::PlannedFolder]) {}

    fn progress(&mut self, p: Progress) {
        self.0.last = p;
        self.0.say(ImportPhase::Importing);
    }

    fn folder(&mut self, _: &Folder) {}
}

/// Read the data file at `path` again into `account`'s copy of it, on the account's import
/// thread. Nothing is left over to remove later: the copy is never removed.
pub(super) fn run(store: &Store, sink: &Sink, account: &str, path: &str, stop: &Stop) -> Vec<PathBuf> {
    let began = Instant::now();
    if let Some(how) = stop.how() {
        never_began(sink, account, path, how);
        return Vec::new();
    }
    let file = store.data_file(account);
    let mut told = Told { sink, store, account, stop, began, last: Progress::default(), folders_done: 0, folders_said: None, file, own: false };
    told.say(ImportPhase::Planning);
    let dir = store.data_files_dir().unwrap_or_else(|| store.root().to_path_buf()).join(format!(".reread-{}", safe(account)));
    // One left by a read cut off (MailCraft closed, the computer turned off) goes first.
    let _ = std::fs::remove_dir_all(&dir);
    let outcome = read_and_add(store, &mut told, &dir, path);
    if let Err(e) = std::fs::remove_dir_all(&dir)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        log::warn!("{}: the file read again wasn't removed: {e}", dir.display());
    }
    let error = match outcome {
        Ok(Some((merged, skips))) => return added(&told, path, &merged, &skips),
        Ok(None) => return Vec::new(),
        Err(e) => e,
    };
    sink.emit(Event::Imported { account: account.to_string(), messages: 0, skipped: 0, done: true, error: Some(error), report: None });
    Vec::new()
}

/// An account id as a file name's part.
fn safe(account: &str) -> String {
    account.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect()
}

/// The file read into a store in `dir`, then what it has new added to the account. `None` when
/// it stopped before that (said already), with what was left out of it otherwise.
fn read_and_add(
    store: &Store,
    told: &mut Told<'_>,
    dir: &Path,
    path: &str,
) -> std::result::Result<Option<(Merged, Vec<(SkipReason, usize)>)>, String> {
    let (account, sink, stop) = (told.account, told.sink, told.stop);
    let fresh = Store::open_packed(dir.join("root"), dir.join("files")).map_err(|e| e.to_string())?;
    let opts = ImportOptions { stop: stop.clone(), no_index: true, ..ImportOptions::default() };
    #[cfg(test)]
    let opts = ImportOptions { fault: tests::fault_for(account), space: tests::space_for(account), ..opts };
    let done = crate::pst::import_into(Path::new(path), &fresh, READ, &opts, &mut Relay(told))?;
    match (stop.how(), done.blocked, done.stopped) {
        // The account removed meanwhile: nothing more is said but the end.
        (Some(StopHow::Discard), ..) => {
            let report = ImportReport { file: path.to_string(), stopped: Some(StopHow::Discard), runs: 1, ..ImportReport::default() };
            sink.emit(Event::Imported {
                account: account.to_string(),
                messages: 0,
                skipped: 0,
                done: true,
                error: None,
                report: Some(Box::new(report)),
            });
            return Ok(None);
        }
        (_, Some(blocked), _) => {
            let progress = ImportProgress { blocked: Some(blocked), ..Told::progress(told, ImportPhase::Blocked) };
            sink.emit(Event::ImportProgress { account: account.to_string(), progress });
            return Ok(None);
        }
        (_, _, Some(StopHow::Pause)) => {
            told.say(ImportPhase::Paused);
            return Ok(None);
        }
        (_, _, Some(how)) => {
            let report = ImportReport { file: path.to_string(), stopped: Some(how), runs: 1, elapsed_ms: ms(told.began), ..ImportReport::default() };
            sink.emit(Event::Imported {
                account: account.to_string(),
                messages: 0,
                skipped: 0,
                done: true,
                error: None,
                report: Some(Box::new(report)),
            });
            return Ok(None);
        }
        _ => {}
    }
    if let Some(e) = done.last_save {
        return Err(format!("the file read again couldn't all be saved: {e}"));
    }
    // Added while nothing else writes the account's folders; never after it was removed.
    let seen = store.removals(account);
    let merged =
        store.while_kept(account, seen, || store.merge_from(account, &fresh, READ)).ok_or("the account was removed")?.map_err(|e| e.to_string())?;
    // The file as read now is the one its copy has: the next opening finds it unchanged.
    if let Ok(note) = fresh.load_account_doc::<serde_json::Value>(READ, NOTE_DOC)
        && let Some(source) = note.get("source")
        && let Ok(mut mine) = store.load_account_doc::<serde_json::Value>(account, NOTE_DOC)
        && let Some(o) = mine.as_object_mut()
    {
        o.insert("source".into(), source.clone());
        if let Err(e) = store.save_account_doc(account, NOTE_DOC, &mine) {
            log::warn!("{account}: the file read again wasn't noted as read: {e}");
        }
    }
    Ok(Some((merged, done.skips.into_iter().collect())))
}

/// What was added said: the folders it went into, and the end with its report.
fn added(told: &Told<'_>, path: &str, merged: &Merged, skips: &[(SkipReason, usize)]) -> Vec<PathBuf> {
    let (store, sink, account) = (told.store, told.sink, told.account);
    match store.load_folders(account) {
        Ok(folders) => sink.emit(Event::Folders { account: account.to_string(), folders }),
        Err(e) => log::warn!("{account}: {e}"),
    }
    for folder in &merged.folders {
        sink.emit(Event::FolderChanged { account: account.to_string(), folder: folder.clone() });
    }
    let mut report = ImportReport {
        file: path.to_string(),
        data_file: data_file(store, account),
        messages: count(merged.messages),
        folders: u32::try_from(merged.folders.len()).unwrap_or(u32::MAX),
        contacts: count(merged.cards),
        events: count(merged.events),
        notes: count(merged.notes),
        tasks: count(merged.tasks),
        runs: 1,
        elapsed_ms: ms(told.began),
        ..ImportReport::default()
    };
    for (reason, n) in skips {
        report.skip(*reason, count(*n));
    }
    sink.emit(Event::Imported {
        account: account.to_string(),
        messages: merged.messages,
        skipped: 0,
        done: true,
        error: None,
        report: Some(Box::new(report)),
    });
    Vec::new()
}
