//! Adding to data files and writing new ones (MS-PST 2.6, 2.7). Each call below is one change,
//! and a change that doesn't finish leaves the file as it was.

pub(crate) mod table_types {
    pub(crate) use super::table::{Cell, NewRow, kept_elsewhere};
}

mod blocks;
mod btree;
mod create;
mod delete;
mod edit;
mod folders;
mod heap;
mod message;
mod named;
mod space;
mod table;
mod text;
mod types;

use std::io;
use std::path::Path;

pub use edit::ADD_MAX_VALUE_SIZE;
pub use types::*;

use crate::ndb::Format;

/// Makes a new, empty data file at `path`, with an Inbox.
pub fn create_empty(path: impl AsRef<Path>, format: Format) -> io::Result<()> {
    create::create_empty(path.as_ref(), format)
}

/// Makes a new Unicode data file at `path`, its first message in the folder at `folder`.
pub fn create_with(path: impl AsRef<Path>, folder: &[&str], message: &NewMessage<'_>, attachments: &[NewAttachment<'_>]) -> io::Result<()> {
    let path = path.as_ref();
    create::create_empty(path, Format::Unicode)?;
    let made = folders::append(path, folder, &[BatchMessage { message: *message, attachments }]);
    if made.is_err() {
        let _ = std::fs::remove_file(path);
    }
    made
}

/// Adds `batch` to the folder at the path of names `folder` (from the top of the file), making
/// the folders that are missing.
pub fn append(path: impl AsRef<Path>, folder: &[&str], batch: &[BatchMessage<'_>]) -> io::Result<()> {
    folders::append(path.as_ref(), folder, batch)
}

/// Adds one message to the folder at the path `folder`.
pub fn append_message(path: impl AsRef<Path>, folder: &[&str], message: &NewMessage<'_>, attachments: &[NewAttachment<'_>]) -> io::Result<()> {
    folders::append(path.as_ref(), folder, &[BatchMessage { message: *message, attachments }])
}

/// Makes the folders of the path `folder` that are missing: the last with `class`
/// (`PR_CONTAINER_CLASS`), any on the way as mail folders. Folders already there keep theirs.
pub fn create_folder(path: impl AsRef<Path>, folder: &[&str], class: &str) -> io::Result<()> {
    if class.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "container class must not be empty"));
    }
    folders::ensure_path(path.as_ref(), folder, class).map(drop)
}

/// The folder the file records as `role`, as names from the top of the file.
pub fn special_folder(path: impl AsRef<Path>, role: SpecialFolder) -> io::Result<Option<Vec<String>>> {
    folders::special_folder(path.as_ref(), role)
}

/// Records the folder at `folder` (made when missing) as `role`.
pub fn set_special_folder(path: impl AsRef<Path>, role: SpecialFolder, folder: &[&str]) -> io::Result<()> {
    folders::set_special_folder(path.as_ref(), role, folder)
}

/// Sets the file's display name.
pub fn set_store_display_name(path: impl AsRef<Path>, name: &str) -> io::Result<()> {
    folders::set_store_display_name(path.as_ref(), name)
}

/// Takes messages (by their node IDs) out of the file, in one change.
pub fn delete_messages(path: impl AsRef<Path>, messages: &[u32]) -> io::Result<()> {
    delete::delete_messages(path.as_ref(), messages)
}
