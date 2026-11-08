//! A message's files as they go into the data file. A `message/rfc822` part goes in as the
//! message it is (an attached message, `afEmbeddedMessage`, MS-OXCMSG 2.2.2.9), with its own
//! files and attached messages in turn, down to the writer's [`MAX_EMBEDDED_DEPTH`] and while the
//! attached messages above it hold no more than [`MAX_ATTACHED`] between them; one deeper or
//! bigger, or one that doesn't parse, stays a `.eml` file. A file bigger than [`HELD_UP_TO`]
//! isn't held with its batch: it is put in a file of its own as the batch is collected
//! ([`Spill`], beside the data file being written, gone when its batch is in), and the writer
//! reads it from there as it writes it ([`StoredFile`]), so a batch holds one big file at a time,
//! whatever its files come to, and a message that leaves the cache while its batch is written (a
//! sync's expunge, a move) can't take the export with it.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::PathBuf;

use mailcraft_pst::write::{AttachmentSource, MAX_EMBEDDED_DEPTH};

use super::item::{Owned, State};
use super::*;

/// The biggest file held with its batch; a bigger one waits in a file of its own ([`Spill`]).
pub(super) const HELD_UP_TO: usize = 4 << 20;

/// The biggest file a data file takes (`PidTagAttachSize` is a signed 32-bit integer, the
/// attachment's other properties counted in); a bigger one is left out.
const MAX_FILE: usize = (i32::MAX as usize).saturating_sub(1 << 20);

/// The most the attached messages along one chain hold decoded at once: each level's message is
/// decoded from its holder's and held while its own are read, so 32 levels of a 50 MB message
/// sent as it is (not shrinking level by level) came to 1.6 GB. One past it stays a `.eml` file.
pub(super) const MAX_ATTACHED: usize = 256 << 20;

/// The longest name and type an attachment is given, in characters: an attachment's properties
/// share one block, which an attached message's 5,000-character subject outgrew: the writer
/// refused the batch, and the export with it.
const MAX_NAME: usize = 255;
/// The longest extension a name cut short keeps.
const MAX_EXTENSION: usize = 16;
/// The longest `Content-ID` written: a longer one is cut, and the HTML's references with it.
const MAX_CID: usize = 1000;

/// `s`, at most `n` characters.
fn cut(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// A file's name, at most [`MAX_NAME`] characters: a longer one keeps its extension (Outlook
/// opens a file by it), its stem cut short.
fn short_name(name: &str) -> String {
    if name.chars().count() <= MAX_NAME {
        return name.to_string();
    }
    let extension = name.rfind('.').and_then(|at| name.get(at..)).filter(|e| e.chars().count() <= MAX_EXTENSION && e.len() > 1).unwrap_or("");
    let stem = name.get(..name.len().saturating_sub(extension.len())).unwrap_or(name);
    format!("{}{extension}", cut(stem, MAX_NAME.saturating_sub(extension.chars().count())))
}

/// One of a message's attachments.
pub(super) enum File {
    /// A file, held: its name, type, `Content-ID` and bytes.
    Data { name: String, mime: String, cid: Option<String>, data: Vec<u8> },
    /// A big file, waiting in a file of its own ([`Spill`]): its size, and that file.
    Stored { name: String, mime: String, cid: Option<String>, size: u64, file: PathBuf },
    /// An attached message, `name` its display name.
    Message { name: String, message: Box<Owned> },
}

/// Where an export's big files wait for their batch to be written: a folder beside the data file
/// being written, emptied as each batch is in and taken away with the export.
pub(super) struct Spill {
    dir: PathBuf,
    made: Cell<u64>,
    bytes: Cell<u64>,
}

impl Spill {
    /// The folder `dir`, emptied of anything an export that stopped short left there.
    pub(super) fn new(dir: PathBuf) -> Spill {
        let _ = std::fs::remove_dir_all(&dir);
        Spill { dir, made: Cell::new(0), bytes: Cell::new(0) }
    }

    /// `data` in a file of its own: its path, or `None` when it can't be written there.
    fn put(&self, data: &[u8]) -> Option<PathBuf> {
        let n = self.made.get();
        self.made.set(n.saturating_add(1));
        let file = self.dir.join(format!("{n}.bin"));
        if let Err(e) = std::fs::create_dir_all(&self.dir).and_then(|()| std::fs::write(&file, data)) {
            log::info!("a big file couldn't be kept beside the data file ({e}): it is held with its batch");
            let _ = std::fs::remove_file(&file);
            return None;
        }
        self.bytes.set(self.bytes.get().saturating_add(u64::try_from(data.len()).unwrap_or(u64::MAX)));
        Some(file)
    }

    /// The bytes waiting.
    pub(super) fn bytes(&self) -> u64 {
        self.bytes.get()
    }

    /// Let go of the files waiting: their batch is written.
    pub(super) fn clear(&self) {
        if self.bytes.get() > 0 || self.dir.exists() {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
        self.bytes.set(0);
    }
}

impl Drop for Spill {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Whether an attachment is a message, which goes in as an attached message.
fn is_message(a: &mime::AttachmentInfo) -> bool {
    a.mime.eq_ignore_ascii_case("message/rfc822")
}

/// The files of the message in `raw` (parsed as `p`), `path` its place under the message the
/// export reads (empty for that one); `owner`: whose mail it is; `spill`: where its big files
/// wait (none: they are held); `left`: what attached messages below it may hold
/// ([`MAX_ATTACHED`]).
pub(super) fn of(raw: &[u8], p: &mime::Parsed, path: &[usize], owner: &super::calendar::Owner, spill: Option<&Spill>, left: usize) -> Vec<File> {
    let at = |index: usize| [path, &[index]].concat();
    let mut slots: Vec<Option<File>> = p.attachments.iter().map(|_| None).collect();
    // Its attached messages first, each let go of before the next and before its files are read.
    let mut files = Vec::new();
    for (slot, a) in slots.iter_mut().zip(&p.attachments) {
        if is_message(a)
            && path.len() < MAX_EMBEDDED_DEPTH
            && a.size <= left
            && let Some(inner) = mime::attachment_data(raw, a.index)
            && let Some(below) = left.checked_sub(inner.len())
            && let Some(message) = item::owned_in(&inner, 0, State::default(), owner, &at(a.index), spill, below)
        {
            let name = cut(Some(message.subject.trim()).filter(|s| !s.is_empty()).unwrap_or(a.name.trim_end_matches(".eml")), MAX_NAME);
            *slot = Some(File::Message { name, message: Box::new(message) });
        } else {
            files.push(a);
        }
    }
    let file = |a: &mime::AttachmentInfo, data: Vec<u8>| -> Option<File> {
        let (name, mime, cid) = (short_name(&a.name), cut(&a.mime, MAX_NAME), a.content_id.clone());
        if data.len() <= HELD_UP_TO {
            return Some(File::Data { name, mime, cid, data });
        }
        if data.len() > MAX_FILE {
            log::info!("a file of {} bytes was left out: a data file can't hold it", data.len());
            return None;
        }
        Some(match spill.and_then(|s| s.put(&data)) {
            Some(file) => File::Stored { name, mime, cid, size: u64::try_from(data.len()).unwrap_or(u64::MAX), file },
            None => File::Data { name, mime, cid, data },
        })
    };
    let place: HashMap<usize, usize> = p.attachments.iter().enumerate().map(|(i, a)| (a.index, i)).collect();
    let mut put = |index: usize, data: Vec<u8>| {
        if let Some(&i) = place.get(&index)
            && let (Some(a), Some(slot)) = (p.attachments.get(i), slots.get_mut(i))
            && slot.is_none()
        {
            *slot = file(a, data);
        }
    };
    // The small files from one parse each (a message with many pictures was parsed per picture)…
    let small: Vec<usize> = files.iter().filter(|a| a.size <= HELD_UP_TO).map(|a| a.index).collect();
    let mut found = HashSet::new();
    for chunk in small.chunks(mime::MAX_WANTED) {
        for (index, data) in mime::attachments_data(raw, chunk, usize::MAX, usize::MAX) {
            found.insert(index);
            put(index, data);
        }
    }
    // …and the rest from one more, each moved out of the parse and put by before the next.
    let rest: Vec<usize> = files.iter().map(|a| a.index).filter(|i| !found.contains(i)).collect();
    mime::attachments_moved(raw, &rest, &mut put);
    slots.into_iter().flatten().collect()
}

/// Cut a `Content-ID` longer than [`MAX_CID`] of `files`, and the `cid:` references to it in
/// `html` the same way, so the picture still shows where the message shows it.
pub(super) fn fit_cids(files: &mut [File], html: &mut Option<String>) {
    for f in files {
        let (File::Data { cid: Some(cid), .. } | File::Stored { cid: Some(cid), .. }) = f else { continue };
        if cid.chars().count() <= MAX_CID {
            continue;
        }
        let short = cut(cid, MAX_CID);
        if let Some(h) = html.as_mut() {
            *h = h.replace(&format!("cid:{cid}"), &format!("cid:{short}"));
        }
        *cid = short;
    }
}

/// A big file waiting in a file of its own, as the writer reads it when it writes it.
pub(super) struct StoredFile<'a> {
    pub(super) file: &'a Path,
    pub(super) size: u64,
}

impl AttachmentSource for StoredFile<'_> {
    fn size(&self) -> u64 {
        self.size
    }

    fn open(&self) -> io::Result<Box<dyn Read + '_>> {
        let f = std::fs::File::open(self.file)?;
        if f.metadata()?.len() != self.size {
            return Err(io::Error::other("a file kept for the export changed before it was written"));
        }
        Ok(Box::new(io::BufReader::with_capacity(1 << 20, f)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A name cut short keeps its extension; a short one, or one whose "extension" is no such
    /// thing, is cut as it is.
    #[test]
    fn names_cut_short_keep_their_extension() {
        let long = format!("{}.pdf", "é".repeat(300));
        let short = short_name(&long);
        assert_eq!(short.chars().count(), MAX_NAME);
        assert!(short.ends_with("é.pdf"));
        assert_eq!(short_name("a.pdf"), "a.pdf");
        let odd = format!("{}.{}", "a".repeat(300), "b".repeat(40));
        assert_eq!(short_name(&odd), "a".repeat(MAX_NAME));
        assert_eq!(short_name(&"x".repeat(300)), "x".repeat(MAX_NAME));
    }
}
