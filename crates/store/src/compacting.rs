//! The store: an account's data file made smaller, giving the room it has free back to the drive
//! (the native data files only: a store in memory has nothing to give back).

use mailcraft_model::job::Work;

use crate::*;

/// What making a data file smaller did ([`Store::compact_data_file`], [`Store::copy_data_file`]).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Shrunk {
    /// The file's size before and after.
    pub before: u64,
    pub after: u64,
    pub elapsed_ms: u64,
    /// It was copied into a new file that took its place.
    pub copied: bool,
    /// The copy was stopped: thrown away, the file as it was.
    pub cancelled: bool,
}

impl Store {
    /// An account's data file compacted in place: everything it keeps moved to the front and the
    /// end given back to the drive. Nothing else of the account is read or written meanwhile
    /// (whoever asks waits for it), and it can't be stopped: 125 s, cold, for a 44 GB file, so
    /// it's asked for knowing how long it may take ([`DataFileHealth::exclusive_ms`]). A kill
    /// meanwhile loses nothing; the file is checked as it next opens. Refused when something holds
    /// the file for seconds (`ResourceBusy`); `None` when the account has no data file.
    pub fn compact_data_file(&self, account: &str) -> Result<Option<Shrunk>> {
        if self.is_online_only(account) {
            return Ok(None);
        }
        match &self.files.main {
            #[cfg(not(target_arch = "wasm32"))]
            Files::Packed(x) => {
                let a = safe_segment(account);
                x.compact_in_place(&a).map_err(|source| StoreError::Io { path: a, source })
            }
            _ => Ok(None),
        }
    }

    /// An account's data file copied into a new one beside it that takes its place, compressing
    /// again on the way what a build before 0.32 kept raw: the account is read and written as
    /// ever meanwhile (what's written is copied again at the end, with the file to itself for
    /// that and the swap). It needs room for the copy on the drive. `go_on` hears how far it has
    /// come before each batch, and stops it by saying no (the copy is thrown away). `None` when
    /// the account has no data file, or is removed meanwhile.
    pub fn copy_data_file(&self, account: &str, go_on: &mut dyn FnMut(Work) -> bool) -> Result<Option<Shrunk>> {
        if self.is_online_only(account) {
            return Ok(None);
        }
        match &self.files.main {
            #[cfg(not(target_arch = "wasm32"))]
            Files::Packed(x) => {
                let a = safe_segment(account);
                x.compact_by_copy(&a, go_on).map_err(|source| StoreError::Io { path: a, source })
            }
            _ => {
                let _ = go_on;
                Ok(None)
            }
        }
    }
}
