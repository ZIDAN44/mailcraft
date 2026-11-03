//! An item's attachments with the messages attached to it, and theirs in turn: each attached
//! message comes in as an attached `.eml` with its own files, down to [`MAX_NESTING`] levels
//! (a deeper one is counted as too deep, unread: no real file nests so far).
//! What a damaged file can make of this is bounded for the whole item: at most [`MAX_NODES`]
//! attachments of attached messages are read, the files kept (an attached message's text
//! counted with them) come to at most [`MAX_ITEM_BYTES`], and an attached message that is one of the messages holding it (its
//! search key met again on the way down) isn't read again. A link's file is read here, within
//! the room the item has left, and at most [`MAX_LINK_READS`] for one item; a link's file that
//! isn't read is named in a short text file, which takes room as a file does.

use super::item::{File, Files, Got, Item};
use super::*;

/// Attachments of an item's attached messages (at every level) read for one item; the attached
/// message whose turn it is when they run out comes in without the rest of its files.
pub(super) const MAX_NODES: usize = 1000;

/// Links whose file is looked for, for one item: the others are named.
pub(super) const MAX_LINK_READS: usize = 64;

/// Reads a link's file: `(path, cap)` gives its bytes when it's there and has at most `cap`.
pub(super) type ReadLink<'a> = &'a dyn Fn(&str, usize) -> Option<Vec<u8>>;

/// One of a message's attachments, as read.
pub(super) enum Part<M> {
    /// A file, or nothing, or left out: as [`item::attachment`] has it.
    Got(Got),
    /// An attached message, whose own attachments can be read in turn.
    Message(M),
}

/// A message whose attachments the import reads: one in the file (`pst.rs`'s reader, for each
/// kind of file), or one a test makes up.
pub(super) trait Holder: Sized {
    /// Hands `each` a reader of each attachment in turn, until it says to stop: an attachment is
    /// read only when its reader is called. `top`: the message is the item, not one attached to
    /// it (which hidden files are left out depends on it).
    fn attachments(&self, top: bool, each: &mut dyn FnMut(&dyn Fn() -> Part<Self>) -> bool);
    /// The message as an item, with these files.
    fn item(&self, files: Vec<File>) -> Item;
    /// What tells this message from the ones holding it (its search key, `PidTagSearchKey`).
    fn key(&self) -> Option<Vec<u8>>;
}

/// Where the read of one item's attachments has got to.
pub(super) struct Walk<'a> {
    /// The keys of the messages being read, the item's first.
    chain: Vec<Vec<u8>>,
    /// Attachments of attached messages read so far.
    nodes: usize,
    /// Bytes the item's files, and its attached messages' text, may still come to.
    room: usize,
    /// Links whose file was looked for so far, and what looks.
    links: usize,
    read: ReadLink<'a>,
}

#[cfg(test)]
impl Walk<'static> {
    /// One that reads no link's file (a test's).
    pub(super) fn new() -> Self {
        Self::with_room(MAX_ITEM_BYTES)
    }

    /// With room for `room` bytes of files.
    pub(super) fn with_room(room: usize) -> Self {
        Walk::reading(room, &|_, _| None)
    }
}

impl<'a> Walk<'a> {
    /// With room for `room` bytes of files, links' files read by `read`.
    pub(super) fn reading(room: usize, read: ReadLink<'a>) -> Self {
        Self { chain: Vec::new(), nodes: 0, room, links: 0, read }
    }

    /// A link's file, read within the room left; or the link named, once the item has looked for
    /// [`MAX_LINK_READS`].
    fn link(&mut self, link: item::Link) -> Got {
        if self.links >= MAX_LINK_READS {
            return link.named();
        }
        if !link.web {
            self.links = self.links.saturating_add(1);
        }
        link.resolve(self.room, self.read)
    }
}

/// `m` as an item, with its files (a link's read by `read`) and its attached messages', and those
/// left out counted.
pub(super) fn item_with_files<M: Holder>(m: &M, read: ReadLink<'_>) -> Item {
    item_in(m, &mut Walk::reading(MAX_ITEM_BYTES, read))
}

/// [`item_with_files`], read as `walk` allows.
pub(super) fn item_in<M: Holder>(m: &M, walk: &mut Walk<'_>) -> Item {
    let key = m.key();
    let held = key.is_some();
    if let Some(key) = key {
        walk.chain.push(key);
    }
    let files = files_of(m, 0, walk);
    if held {
        walk.chain.pop();
    }
    Item { left_out: files.left_out, photo: files.photo, ..m.item(files.files) }
}

/// The files of `m`, a message `nesting` attached messages down from the item.
fn files_of<M: Holder>(m: &M, nesting: usize, walk: &mut Walk<'_>) -> Files {
    let mut files = Files::default();
    m.attachments(nesting == 0, &mut |read| {
        if nesting > 0 {
            if walk.nodes >= MAX_NODES {
                log::info!("an attached message's files past the first {MAX_NODES} were left out");
                files.add(Got::Left(SkipReason::AttachedMessageFilesLeftOut));
                return false;
            }
            walk.nodes = walk.nodes.saturating_add(1);
        }
        match read() {
            Part::Got(Got::Linked(link)) => {
                let got = walk.link(link);
                keep(got, walk, &mut files);
            }
            Part::Got(got) => keep(got, walk, &mut files),
            Part::Message(inner) => {
                let got = attached(&inner, nesting, walk, &mut files);
                files.add(got);
            }
        }
        true
    });
    files
}

/// `got` into `files`: a file, picture or link's text taking its bytes from the room left, or
/// counted as too big when they don't fit.
fn keep(got: Got, walk: &mut Walk<'_>, files: &mut Files) {
    let len = match &got {
        Got::File(File::Data { data, .. }) | Got::Link(File::Data { data, .. }) | Got::Photo(_, data) => data.len(),
        _ => 0,
    };
    if len > walk.room {
        log::info!("a file of {len} bytes was left out: the item's files came to their limit");
        files.add(Got::Left(SkipReason::AttachmentTooBig));
    } else {
        walk.room = walk.room.saturating_sub(len);
        files.add(got);
    }
}

/// An attached message, `nesting` down, as a file of the message holding it; what its own files
/// left out is counted in `files`, that message's. Its text takes room as its files do: one whose
/// text no longer fits is counted as too big and left out, its files with it.
fn attached<M: Holder>(m: &M, nesting: usize, walk: &mut Walk<'_>, files: &mut Files) -> Got {
    if nesting >= MAX_NESTING {
        return Got::Left(SkipReason::EmbeddedTooDeep);
    }
    let key = m.key();
    if key.as_ref().is_some_and(|k| walk.chain.contains(k)) {
        log::info!("an attached message holds a message it is attached to: left out");
        return Got::Left(SkipReason::AttachedMessageUnreadable);
    }
    let held = key.is_some();
    if let Some(key) = key {
        walk.chain.push(key);
    }
    let room = walk.room;
    let inner = files_of(m, nesting.saturating_add(1), walk);
    if held {
        walk.chain.pop();
    }
    let it = m.item(inner.files);
    let text = text_bytes(&it);
    if text > walk.room {
        log::info!("an attached message with {text} bytes of text was left out: the item's files came to their limit");
        walk.room = room;
        return Got::Left(SkipReason::AttachmentTooBig);
    }
    walk.room = walk.room.saturating_sub(text);
    for (reason, n) in inner.left_out {
        let count = files.left_out.entry(reason).or_default();
        *count = count.saturating_add(n);
    }
    Got::File(File::Message(Box::new(it)))
}

/// The bytes of an item's own text: the most of what it holds besides its files.
fn text_bytes(it: &Item) -> usize {
    [it.subject.len(), it.body.len(), it.html.as_ref().map_or(0, String::len), it.headers.len()].into_iter().fold(0, usize::saturating_add)
}
