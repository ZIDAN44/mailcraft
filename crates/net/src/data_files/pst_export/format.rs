//! Which of Outlook's two formats a data file is in, and the writer's calls on a file of it: the
//! Unicode format of Outlook 2003 and later, which a new file is made in, or Outlook 97–2002's
//! ANSI one, which a file added to may be in (its strings 8-bit, a folder of it holding 65,536
//! items at most and the file stopping short of 2 GB; the writer says so when one would pass
//! either).

use super::*;
use outlook_pst::messaging::store::AnsiStore;
use outlook_pst::{AnsiPstFile, UnicodePstSpecialFolder as Special};

/// A data file's format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Format {
    Unicode,
    Ansi,
}

/// `wVer` of Outlook 97–2002's ANSI data files (MS-PST 2.2.2.6: 14 or 15; Unicode files say 23
/// or more).
const ANSI_VERSIONS: [u16; 2] = [14, 15];

impl Format {
    /// The format of the file whose first bytes are `head`.
    pub(super) fn of_head(head: &[u8]) -> Format {
        match head.get(10..12) {
            Some(&[low, high]) if ANSI_VERSIONS.contains(&u16::from_le_bytes([low, high])) => Format::Ansi,
            _ => Format::Unicode,
        }
    }

    /// The format of the file at `path` (Unicode when there's none yet: a new file is made so).
    pub(super) fn of(path: &Path) -> Format {
        let mut head = [0u8; 12];
        match std::fs::File::open(path).and_then(|mut f| io::Read::read_exact(&mut f, &mut head)) {
            Ok(()) => Format::of_head(&head),
            Err(_) => Format::Unicode,
        }
    }

    /// The file at `path`'s store, read within MailCraft's limit for a value.
    pub(super) fn store(self, path: &Path) -> io::Result<Rc<dyn PstStore>> {
        Ok(match self {
            Format::Unicode => {
                let pst = UnicodePstFile::open(path)?;
                pst.set_max_value_size(super::super::pst::MAX_VALUE);
                let store: Rc<dyn PstStore> = UnicodeStore::read(Rc::new(pst))?;
                store
            }
            Format::Ansi => {
                let pst = AnsiPstFile::open(path)?;
                pst.set_max_value_size(super::super::pst::MAX_VALUE);
                let store: Rc<dyn PstStore> = AnsiStore::read(Rc::new(pst))?;
                store
            }
        })
    }

    /// Add `batch` to `folder` of the file at `path`, making the folders that are missing.
    pub(super) fn append(self, path: &Path, folder: &[&str], batch: &[UnicodePstBatchMessage<'_>]) -> io::Result<()> {
        match self {
            Format::Unicode => UnicodePstFile::append_many_in_folder_with_attachments(path, folder, batch).map(drop),
            Format::Ansi => AnsiPstFile::append_many_in_folder_with_attachments(path, folder, batch).map(drop),
        }
    }

    /// Take the messages `ids` out of the file at `path`, in one commit: their rows leave their
    /// folders' tables, their folders count again, and the space they held is free for the next.
    pub(super) fn delete(self, path: &Path, ids: &[NodeId]) -> io::Result<()> {
        match self {
            Format::Unicode => UnicodePstFile::delete_messages(path, ids).map(drop),
            Format::Ansi => AnsiPstFile::delete_messages(path, ids).map(drop),
        }
    }

    /// Make the folders of `names` that are missing, the last with `class`.
    pub(super) fn create_folder(self, path: &Path, names: &[&str], class: &str) -> io::Result<()> {
        match self {
            Format::Unicode => UnicodePstFile::create_folder(path, names, class).map(drop),
            Format::Ansi => AnsiPstFile::create_folder(path, names, class).map(drop),
        }
    }

    /// The folder the file records as `role`, as names from the top of the file.
    pub(super) fn special_folder(self, path: &Path, role: Special) -> io::Result<Option<Vec<String>>> {
        match self {
            Format::Unicode => UnicodePstFile::special_folder(path, role),
            Format::Ansi => AnsiPstFile::special_folder(path, role),
        }
    }

    /// Record the folder at `names` as `role`.
    pub(super) fn set_special_folder(self, path: &Path, role: Special, names: &[&str]) -> io::Result<()> {
        match self {
            Format::Unicode => UnicodePstFile::set_special_folder(path, role, names).map(drop),
            Format::Ansi => AnsiPstFile::set_special_folder(path, role, names).map(drop),
        }
    }
}
