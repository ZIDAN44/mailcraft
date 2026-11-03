//! An item's attached messages, read through the same seam as the file's (`Holder`) from
//! messages made up here: each comes in with its own files, down to 32 levels, and a layout a
//! damaged file can make (a message attached to itself, one attached a thousand times over, files
//! of more than an item holds) is read once, within its limits.

use std::cell::Cell;

use super::super::embedded::{Holder, MAX_NODES, Part, Walk, item_in, item_with_files};
use super::super::item::{File, Got, Item, from_row, item_of};
use super::super::rebuild::to_raw;
use super::*;

/// One of a made-up message's attachments: a file of so many bytes, or the message at that
/// place in the layout, shown or hidden (as a repeating appointment's changed occurrence is).
#[derive(Clone, Copy)]
enum Fake {
    File(usize),
    Message(usize),
    Hidden(usize),
}

/// A made-up message: its class, subject, search key, attachments and the bytes of its body.
struct Node {
    class: &'static str,
    subject: &'static str,
    key: Option<&'static [u8]>,
    parts: Vec<Fake>,
    body: usize,
}

/// The message at `at` in a layout, counting the attachments read.
#[derive(Clone, Copy)]
struct At<'a> {
    layout: &'a [Node],
    at: usize,
    reads: &'a Cell<usize>,
}

impl Holder for At<'_> {
    fn attachments(&self, top: bool, each: &mut dyn FnMut(&dyn Fn() -> Part<Self>) -> bool) {
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
                    // Whether a hidden attached message is left out is the file's rule, from its
                    // row (a hidden attachment holding a message), as for one the reader fails on.
                    Fake::Hidden(at) => {
                        let row = |id: u16| match id {
                            0x3705 => Some(Val::Int(5)),
                            0x7FFE => Some(Val::Bool(true)),
                            _ => None,
                        };
                        match from_row(&io::Error::other("unread"), &row, self.layout[self.at].class, top) {
                            Got::Nothing => Part::Got(Got::Nothing),
                            _ => Part::Message(At { at, ..*self }),
                        }
                    }
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
        let Node { class, subject, body, .. } = self.layout[self.at];
        let props = |id: u16| match id {
            0x001A => Some(Val::Text(class.into())),
            0x0037 => Some(Val::Text(subject.into())),
            0x1000 if body > 0 => Some(Val::Text("x".repeat(body).into())),
            _ => None,
        };
        item_of(&props, &none, files, &crate::pst_pim::Named::default())
    }

    fn key(&self) -> Option<Vec<u8>> {
        self.layout[self.at].key.map(<[u8]>::to_vec)
    }
}

fn node(subject: &'static str, key: Option<&'static [u8]>, parts: Vec<Fake>) -> Node {
    Node { class: "IPM.Note", subject, key, parts, body: 0 }
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

/// A chain of `n` messages, each holding a file of one byte and the next: subjects `m0`, `m1`…,
/// each with a search key of its own.
fn chain(n: usize) -> Vec<Node> {
    (0..n)
        .map(|i| {
            let parts = if i + 1 < n { vec![Fake::File(1), Fake::Message(i + 1)] } else { vec![Fake::File(1)] };
            let name: &'static str = Box::leak(format!("m{i}").into_boxed_str());
            node(name, Some(name.as_bytes()), parts)
        })
        .collect()
}

/// The shape a chain read `levels` messages deep has: `m0[f1, m1[f1, …]]`.
fn chain_shape(levels: usize) -> String {
    let mut s = String::new();
    for i in (0..levels).rev() {
        s = if s.is_empty() { format!("m{i}[f1]") } else { format!("m{i}[f1, {s}]") };
    }
    s
}

/// Attached messages six deep come in whole; forty deep, the item and 32 levels of attached
/// messages do, and the 33rd is counted as too deep, unread (its own attachments with it).
#[test]
fn attached_messages_stop_thirty_two_levels_down() {
    let six = chain(6);
    let (shape, left, reads) = read(&six, &mut Walk::new());
    assert_eq!(shape, chain_shape(6));
    assert!(left.is_empty(), "{left:?}");
    assert_eq!(reads, 11);
    let forty = chain(40);
    let (shape, left, reads) = read(&forty, &mut Walk::new());
    assert_eq!(shape, chain_shape(MAX_NESTING + 1));
    assert_eq!(left, [(SkipReason::EmbeddedTooDeep, 1)]);
    assert_eq!(reads, 2 * (MAX_NESTING + 1), "the 34th message's attachments aren't read");
}

/// The message rebuilt from a chain forty deep holds an attached `.eml` in each `.eml`, as deep
/// as the chain came in, each parsed and opened on its own as the reading pane opens one; its
/// files are all there, and the rebuild is the same as from the item.
#[test]
fn a_deep_chain_rebuilds_and_opens_level_by_level() {
    let forty = chain(40);
    let reads = Cell::new(0);
    let it = item_with_files(&At { layout: &forty, at: 0, reads: &reads });
    let raw = to_raw(it, "deep@x.invalid").unwrap();
    // Search reads the text of four of them (`index_text`'s depth, counting each one's multipart).
    let words = mailcraft_mime::index_doc(&raw).joined();
    assert!(words.contains("m4.eml") && !words.contains("m5"), "{words}");
    let mut level = raw;
    let mut depth = 0;
    loop {
        let p = mailcraft_mime::parse(&level).expect("each level parses");
        assert_eq!(p.subject, format!("m{depth}"));
        let names: Vec<&str> = p.attachments.iter().map(|a| a.name.as_str()).collect();
        let Some(inner) = p.attachments.iter().find(|a| a.mime == "message/rfc822") else {
            assert_eq!(names, ["f1"]);
            break;
        };
        assert_eq!(names, ["f1".to_string(), format!("m{}.eml", depth + 1)]);
        level = mailcraft_mime::attachment_data(&level, inner.index).expect("its bytes");
        depth += 1;
    }
    assert_eq!(depth, MAX_NESTING);
}

/// The rebuild stops at the reader's depth whatever item it's handed: an item made deeper than
/// the reader makes them keeps [`MAX_NESTING`] levels, and its recursion stays bounded.
#[test]
fn the_rebuild_stops_at_the_reader_s_depth() {
    let mut it = Item { subject: "bottom".into(), ..Default::default() };
    for i in 0..100 {
        it = Item { subject: format!("l{i}"), files: vec![File::Message(Box::new(it))], ..Default::default() };
    }
    let mut level = to_raw(it, "deeper@x.invalid").unwrap();
    let mut depth = 0;
    while let Some(inner) = mailcraft_mime::parse(&level).unwrap().attachments.into_iter().find(|a| a.mime == "message/rfc822") {
        level = mailcraft_mime::attachment_data(&level, inner.index).unwrap();
        depth += 1;
    }
    assert_eq!(depth, MAX_NESTING);
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
    let (shape, left, reads) = read(&keyless, &mut Walk::new());
    assert_eq!(shape, format!("{}A[f1]{}", "A[f1, ".repeat(MAX_NESTING), "]".repeat(MAX_NESTING)));
    assert_eq!(left, [(SkipReason::EmbeddedTooDeep, 1)]);
    assert_eq!(reads, 2 * (MAX_NESTING + 1));
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
    let layout = [node("A", Some(b"a"), vec![F(6), F(6), M(1)]), node("B", Some(b"b"), vec![F(5), F(3)])];
    let (shape, left, _) = read(&layout, &mut Walk::with_room(10));
    // B's own subject takes the last byte.
    assert_eq!(shape, "A[f6, B[f3]]");
    assert_eq!(left, [(SkipReason::AttachmentTooBig, 2)]);
}

/// An attached message's text takes room as its files do, so attached messages with large
/// bodies can't hold more than the item may: one that no longer fits is counted as too big and
/// left out with its files, whose room goes to the attachments after it.
#[test]
fn attached_messages_text_counts_toward_the_item_s_limit() {
    use Fake::{File as F, Message as M};
    let mut parts = vec![M(1); 10];
    parts.push(M(2));
    let big = Node { body: 100, ..node("B", Some(b"b"), vec![F(10)]) };
    let layout = [node("A", Some(b"a"), parts), big, node("C", Some(b"c"), vec![F(50)])];
    // Each B takes 111 bytes: two fit in 300, and the 78 left still take C's 51.
    let (shape, left, _) = read(&layout, &mut Walk::with_room(300));
    assert_eq!(shape, "A[B[f10], B[f10], C[f50]]");
    assert_eq!(left, [(SkipReason::AttachmentTooBig, 8)]);
}

/// A repeating appointment's changed occurrences (hidden attached messages) come in with it when
/// it is the item, which goes to the calendar; an appointment attached to a mail keeps them
/// hidden, so the mail doesn't gain them as files. A mail's own hidden attached message comes in.
#[test]
fn an_attached_appointment_keeps_its_changed_occurrences_hidden() {
    use Fake::{File as F, Hidden as H, Message as M};
    let series = Node { class: "IPM.Appointment", ..node("Series", Some(b"s"), vec![F(2), H(1)]) };
    let changed = Node { class: "IPM.OLE.CLASS.{00061055-0000-0000-C000-000000000046}", ..node("Changed", Some(b"c"), vec![F(1)]) };
    let appointment = [series, changed];
    assert_eq!(read(&appointment, &mut Walk::new()).0, "Series[f2, Changed[f1]]", "the item's own");
    let series = Node { class: "IPM.Appointment", ..node("Series", Some(b"s"), vec![F(2), H(2)]) };
    let changed = Node { class: "IPM.OLE.CLASS.{00061055-0000-0000-C000-000000000046}", ..node("Changed", Some(b"c"), vec![F(1)]) };
    let mail = [node("Mail", Some(b"m"), vec![F(3), M(1), H(3)]), series, changed, node("Kept", Some(b"k"), vec![])];
    let (shape, left, _) = read(&mail, &mut Walk::new());
    assert_eq!(shape, "Mail[f3, Series[f2], Kept[]]");
    assert!(left.is_empty(), "{left:?}");
}
