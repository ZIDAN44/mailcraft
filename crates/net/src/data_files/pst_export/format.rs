//! Which of Outlook's two formats a data file is in, and its store: the Unicode format of Outlook
//! 2003 and later, which a new file is made in, or Outlook 97–2002's ANSI one, which a file added
//! to may be in (its strings 8-bit, a folder of it holding 65,536 items at most and the file
//! stopping short of 2 GB; the writer says so when one would pass either).

use super::*;
use mailcraft_pst::{Format, PstFile, PstStore};

/// The format of the file at `path` (Unicode when there's none yet: a new file is made so).
pub(super) fn format_of(path: &Path) -> Format {
    let mut head = [0u8; 12];
    match std::fs::File::open(path).and_then(|mut f| io::Read::read_exact(&mut f, &mut head)) {
        Ok(()) => Format::of_head(&head),
        Err(_) => Format::Unicode,
    }
}

/// The file at `path`'s store, read within MailCraft's limit for a value.
pub(super) fn store_of(path: &Path) -> io::Result<Rc<PstStore>> {
    let pst = PstFile::open(path)?;
    pst.set_max_value_size(super::super::pst::MAX_VALUE);
    PstStore::read(Rc::new(pst))
}
