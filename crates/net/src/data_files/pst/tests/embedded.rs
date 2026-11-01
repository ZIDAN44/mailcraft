//! An item's attached messages, read through the same seam as the file's (`Holder`) from
//! messages made up here: each comes in with its own files, down to three levels, and a layout a
//! damaged file can make (a message attached to itself, one attached a thousand times over, files
//! of more than an item holds) is read once, within its limits.

use std::cell::Cell;

use super::super::embedded::{Holder, MAX_NODES, Part, Walk, item_in, item_with_files};
use super::super::item::{File, Got, Item, item_of};
use super::*;

/// One of a made-up message's attachments: a file of so many bytes, or the message at that
/// place in the layout.
#[derive(Clone, Copy)]
enum Fake {
    File(usize),
    Message(usize),
}

/// A made-up message: its subject, its search key and its attachments.
struct Node {
    subject: &'static str,
    key: Option<&'static [u8]>,
    parts: Vec<Fake>,
}

/// The message at `at` in a layout, counting the attachments read.
#[derive(Clone, Copy)]
struct At<'a> {
    layout: &'a [Node],
    at: usize,
    reads: &'a Cell<usize>,
}

impl Holder for At<'_> {
    fn attachments(&self, each: &mut dyn FnMut(&dyn Fn() -> Part<Self>) -> bool) {
        for part in &self.layout[self.at].parts {
            let read = || {
                self.reads.set(self.reads.get() + 1);
                match *part {
                    Fake::File(n) => {
                        let file =
                            File::Data { name: format!("f{n}"), mime: "application/octet-stream".into(), cid: String::new(), data: vec![0; n] };
                        Part::Got(Got::File(file))
                    }
                    Fake::Message(at) => Part::Message(At { at, ..*self }),
                }
            };
            if !each(&read) {
                return;
            }
        }
    }

    /// Through the properties' seam, as the file's are read.
    fn item(&self, files: Vec<File>) -> Item {
        let none: [fn(u16) -> Option<Val<'static>>; 0] = [];
        let subject = self.layout[self.at].subject;
        let props = |id: u16| match id {
            0x001A => Some(Val::Text("IPM.Note".into())),
            0x0037 => Some(Val::Text(subject.into())),
            _ => None,
        };
        item_of(&props, &none, files, &crate::pst_pim::Named::default())
    }

    fn key(&self) -> Option<Vec<u8>> {
        self.layout[self.at].key.map(<[u8]>::to_vec)
    }
}

fn node(subject: &'static str, key: Option<&'static [u8]>, parts: Vec<Fake>) -> Node {
    Node { subject, key, parts }
}

/// An item's subject and its files, an attached message's as its own: `A[f3, B[f1]]`.
fn shape(it: &Item) -> String {
    let files: Vec<String> = it
        .files
        .iter()
        .map(|f| match f {
            File::Data { data, .. } => format!("f{}", data.len()),
            File::Message(m) => shape(m),
        })
        .collect();
    format!("{}[{}]", it.subject, files.join(", "))
}

/// The first message of `layout` read as an item in `walk`: its shape, what was left out, and
/// how many attachments were read.
fn read(layout: &[Node], walk: &mut Walk) -> (String, Vec<(SkipReason, usize)>, usize) {
    let reads = Cell::new(0);
    let it = item_in(&At { layout, at: 0, reads: &reads }, walk);
    (shape(&it), it.left_out.into_iter().collect(), reads.get())
}

/// Attached messages come in with their own files, and the ones attached to them with theirs.
#[test]
fn an_attached_message_comes_in_with_its_own_files() {
    use Fake::{File as F, Message as M};
    let layout = [node("A", Some(b"a"), vec![F(3), M(1)]), node("B", Some(b"b"), vec![F(1), M(2)]), node("C", None, vec![F(2)])];
    let reads = Cell::new(0);
    let it = item_with_files(&At { layout: &layout, at: 0, reads: &reads });
    assert_eq!(shape(&it), "A[f3, B[f1, C[f2]]]");
    assert!(it.left_out.is_empty());
}

/// Three levels of attached messages come in; a fourth is counted as too deep, unread.
#[test]
fn attached_messages_stop_three_levels_down() {
    use Fake::{File as F, Message as M};
    let layout = [
        node("A", Some(b"a"), vec![M(1)]),
        node("B", Some(b"b"), vec![M(2)]),
        node("C", Some(b"c"), vec![M(3)]),
        node("D", Some(b"d"), vec![F(1), M(4)]),
        node("E", Some(b"e"), vec![F(1)]),
    ];
    let (shape, left, reads) = read(&layout, &mut Walk::new());
    assert_eq!(shape, "A[B[C[D[f1]]]]");
    assert_eq!(left, [(SkipReason::EmbeddedTooDeep, 1)]);
    assert_eq!(reads, 5, "E's own attachment isn't read");
}

/// A message attached to itself, or to the message it is attached to, is read once: met again on
/// the way down (by its search key), it is counted as unreadable. Without a key it is read only
/// as deep as any attached message.
#[test]
fn a_message_attached_to_itself_is_read_once() {
    use Fake::{File as F, Message as M};
    let itself = [node("A", Some(b"a"), vec![F(1), M(0)])];
    assert_eq!(read(&itself, &mut Walk::new()), ("A[f1]".into(), vec![(SkipReason::AttachedMessageUnreadable, 1)], 2));
    let each_other = [node("A", Some(b"a"), vec![M(1)]), node("B", Some(b"b"), vec![F(2), M(0)])];
    assert_eq!(read(&each_other, &mut Walk::new()), ("A[B[f2]]".into(), vec![(SkipReason::AttachedMessageUnreadable, 1)], 3));
    // The same message attached twice side by side isn't a loop: both come in.
    let twice = [node("A", Some(b"a"), vec![M(1), M(1)]), node("B", Some(b"b"), vec![F(1)])];
    assert_eq!(read(&twice, &mut Walk::new()).0, "A[B[f1], B[f1]]");
    let keyless = [node("A", None, vec![F(1), M(0)])];
    let (shape, left, _) = read(&keyless, &mut Walk::new());
    assert_eq!(shape, "A[f1, A[f1, A[f1, A[f1]]]]");
    assert_eq!(left, [(SkipReason::EmbeddedTooDeep, 1)]);
}

/// Attached messages that attach the same message over and over (ten of ten of ten…) read at
/// most a thousand attachments for the item; the message whose turn it is then comes in without
/// the rest of its files, counted once, and the messages after it with none.
#[test]
fn an_item_reads_at_most_a_thousand_attachments_of_attached_messages() {
    use Fake::{File as F, Message as M};
    let layout = [
        node("A", Some(b"a"), vec![M(1); 10]),
        node("B", Some(b"b"), vec![M(2); 10]),
        node("C", Some(b"c"), vec![M(3); 10]),
        node("D", Some(b"d"), vec![F(1); 10]),
    ];
    let (_, left, reads) = read(&layout, &mut Walk::new());
    // A's own ten, then MAX_NODES of its attached messages'.
    assert_eq!(reads, 10 + MAX_NODES);
    let cut = left.iter().find(|(r, _)| *r == SkipReason::AttachedMessageFilesLeftOut).map(|(_, n)| *n);
    assert!(cut.is_some_and(|n| n >= 1), "{left:?}");
}

/// An item's files, its attached messages' included, come to no more than it may hold: a file
/// past that is counted as too big, and smaller ones after it still come in.
#[test]
fn an_item_s_files_stop_at_its_limit() {
    use Fake::{File as F, Message as M};
    let layout = [node("A", Some(b"a"), vec![F(6), F(6), M(1)]), node("B", Some(b"b"), vec![F(5), F(4)])];
    let (shape, left, _) = read(&layout, &mut Walk::with_room(10));
    assert_eq!(shape, "A[f6, B[f4]]");
    assert_eq!(left, [(SkipReason::AttachmentTooBig, 2)]);
}
