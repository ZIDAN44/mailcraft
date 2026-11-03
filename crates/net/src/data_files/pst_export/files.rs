//! A message's files as they go into the data file. A `message/rfc822` part goes in as the
//! message it is (an attached message, `afEmbeddedMessage`, MS-OXCMSG 2.2.2.9), with its own
//! files and attached messages in turn, down to the writer's [`MAX_EMBEDDED_DEPTH`]; one deeper,
//! or one that doesn't parse, stays a `.eml` file. A file bigger than [`HELD_UP_TO`] isn't held
//! with its batch: it is left in the cache, and read again from its message's source when the
//! writer comes to it ([`StoredFile`]), which writes it as it reads it, so a batch holds one big
//! file at a time, whatever its files come to.

use std::collections::HashMap;
use std::io::Read;

use outlook_pst::{MAX_EMBEDDED_DEPTH, UnicodePstAttachmentSource};

use super::item::{Owned, State};
use super::*;

/// The biggest file held with its batch; a bigger one is read from the cache when it's written.
pub(super) const HELD_UP_TO: usize = 4 << 20;

/// The biggest file a data file takes (`PidTagAttachSize` is a signed 32-bit integer, the
/// attachment's other properties counted in); a bigger one is left out.
const MAX_FILE: usize = (i32::MAX as usize).saturating_sub(1 << 20);

/// The longest name, type and `Content-ID` an attachment is given, in characters: an
/// attachment's properties share one block, which an attached message's 5,000-character subject
/// outgrew: the writer refused the batch, and the export with it.
const MAX_NAME: usize = 255;

/// `s`, at most [`MAX_NAME`] characters.
fn short(s: &str) -> String {
    s.chars().take(MAX_NAME).collect()
}

/// One of a message's attachments.
pub(super) enum File {
    /// A file, held: its name, type, `Content-ID` and bytes.
    Data { name: String, mime: String, cid: Option<String>, data: Vec<u8> },
    /// A file left in the cache: its size, and where it is in the message's source (the index of
    /// each attached message down to it, then its own).
    Stored { name: String, mime: String, cid: Option<String>, size: u64, path: Vec<usize> },
    /// An attached message, `name` its display name.
    Message { name: String, message: Box<Owned> },
}

/// Whether an attachment is a message, which goes in as an attached message.
fn is_message(a: &mime::AttachmentInfo) -> bool {
    a.mime.eq_ignore_ascii_case("message/rfc822")
}

/// The files of the message in `raw` (parsed as `p`), `path` its place under the message the
/// export reads (empty for that one); `owner`: whose mail it is.
pub(super) fn of(raw: &[u8], p: &mime::Parsed, path: &[usize], owner: &super::calendar::Owner) -> Vec<File> {
    let at = |index: usize| [path, &[index]].concat();
    // The small files from one parse each (a message with many pictures was parsed per picture).
    let small: Vec<usize> = p.attachments.iter().filter(|a| a.size <= HELD_UP_TO && !is_message(a)).map(|a| a.index).collect();
    let mut held: HashMap<usize, Vec<u8>> = HashMap::new();
    for chunk in small.chunks(mime::MAX_WANTED) {
        held.extend(mime::attachments_data(raw, chunk, usize::MAX, usize::MAX));
    }
    let mut out = Vec::new();
    for a in &p.attachments {
        if is_message(a)
            && path.len() < MAX_EMBEDDED_DEPTH
            && let Some(message) =
                mime::attachment_data(raw, a.index).and_then(|inner| item::owned_in(&inner, 0, State::default(), owner, &at(a.index)))
        {
            let name = short(Some(message.subject.trim()).filter(|s| !s.is_empty()).unwrap_or(a.name.trim_end_matches(".eml")));
            out.push(File::Message { name, message: Box::new(message) });
            continue;
        }
        let Some(data) = held.remove(&a.index).or_else(|| mime::attachment_data_moved(raw, a.index)) else { continue };
        let (name, mime, cid) = (short(&a.name), short(&a.mime), a.content_id.as_deref().map(short));
        if data.len() <= HELD_UP_TO {
            out.push(File::Data { name, mime, cid, data });
        } else if data.len() <= MAX_FILE {
            // Its size known, it's let go of until it is written.
            let size = u64::try_from(data.len()).unwrap_or(u64::MAX);
            out.push(File::Stored { name, mime, cid, size, path: at(a.index) });
        } else {
            log::info!("a file of {} bytes was left out: a data file can't hold it", data.len());
        }
    }
    out
}

/// A file left in the cache, as the writer reads it when it writes it.
pub(super) struct StoredFile<'a> {
    pub(super) store: &'a Store,
    pub(super) key: &'a MessageKey,
    pub(super) path: &'a [usize],
    pub(super) size: u64,
}

impl UnicodePstAttachmentSource for StoredFile<'_> {
    fn size(&self) -> u64 {
        self.size
    }

    fn open(&self) -> io::Result<Box<dyn Read + '_>> {
        let data = stored(self.store, self.key, self.path).ok_or_else(|| io::Error::other("a file's message is no longer on this computer"))?;
        if u64::try_from(data.len()).ok() != Some(self.size) {
            return Err(io::Error::other("a file's message changed while it was exported"));
        }
        Ok(Box::new(io::Cursor::new(data)))
    }
}

/// The file at `path` in the source of the message at `key`: each index but the last an attached
/// message's, down to the file's own. Its bytes are moved out of their parse, not copied.
pub(super) fn stored(store: &Store, key: &MessageKey, path: &[usize]) -> Option<Vec<u8>> {
    let (last, through) = path.split_last()?;
    let mut raw = store.read_body(key)?;
    for index in through {
        raw = mime::attachment_data(&raw, *index)?;
    }
    mime::attachment_data_moved(&raw, *last)
}
