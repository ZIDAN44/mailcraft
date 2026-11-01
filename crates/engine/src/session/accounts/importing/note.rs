//! The session: what an import's own note inside the account's data file (its checkpoint,
//! written by the network side as it imports) says of where it got. It decides over
//! `imports.json`, which is written apart from it: an import the user cancelled isn't gone on
//! with by itself because MailCraft closed before it heard so, and a data file's account whose
//! record was lost is found from its note.

use mailcraft_model::job::{DataSource, ImportStage};

use super::*;

/// A note is read only when it's at most this big.
const NOTE_MAX: usize = 64 << 20;

/// What the session reads of an import's note: how far it got, and the file it reads.
#[derive(Default, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Note {
    state: String,
    source: NoteSource,
}

#[derive(Default, serde::Deserialize)]
#[serde(default)]
struct NoteSource {
    path: String,
}

/// The stage `account`'s import's note says it's at (`Running`, `Stopped` for one cancelled,
/// `Done`), and the file it reads; none when there's no note, or one that can't be read.
pub(super) fn noted(store: &Store, account: &str) -> Option<(ImportStage, String)> {
    let bytes = store.load_account_doc_bytes(account, CHECKPOINT_DOC).ok().flatten().filter(|b| b.len() <= NOTE_MAX)?;
    let note: Note = serde_json::from_slice(&bytes).ok()?;
    let stage = match note.state.as_str() {
        "running" => ImportStage::Running,
        "cancelled" => ImportStage::Stopped,
        "done" => ImportStage::Done,
        _ => return None,
    };
    Some((stage, note.source.path))
}

impl Session {
    /// The notes inside the data files put `imports.json` right: an import it has running that
    /// its note says the user cancelled waits on them, and a data file's account it has no row
    /// for gets one from its note when that import isn't done (going on by itself, or waiting).
    pub(super) fn imports_from_notes(&mut self, store: &Store) {
        let mut changed = false;
        let running: Vec<String> = self
            .importing
            .sources
            .iter()
            .filter(|(a, s)| s.stage == ImportStage::Running && self.account(a).is_some())
            .map(|(a, _)| a.clone())
            .collect();
        for a in running {
            if let Some((ImportStage::Stopped, _)) = noted(store, &a)
                && let Some(row) = self.importing.sources.get_mut(&a)
            {
                row.stage = ImportStage::Stopped;
                changed = true;
            }
        }
        let unrecorded: Vec<String> = self
            .accounts
            .iter()
            .filter(|a| a.is_data_file() && !self.importing.sources.contains_key(&a.config.id))
            .map(|a| a.config.id.clone())
            .collect();
        for a in unrecorded {
            if let Some((stage @ (ImportStage::Running | ImportStage::Stopped), path)) = noted(store, &a)
                && !path.is_empty()
                && path.len() <= PATH_MAX
            {
                self.importing.sources.insert(a, DataSource { path, stage, ..DataSource::default() });
                changed = true;
            }
        }
        if changed {
            self.save_imports();
        }
    }
}
