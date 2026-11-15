//! BER to DER. The `der` crate reads DER only, but S/MIME is mostly written in BER: mail
//! programs stream it, with lengths left open (ended by two zero bytes) and the encrypted or
//! signed content cut into OCTET STRING pieces. [`to_der`] rewrites such input as DER: every
//! length written out, every constructed OCTET STRING joined into one, and the encrypted content
//! of an `EncryptedContentInfo` (`[0] IMPLICIT OCTET STRING`, which BER may also cut into pieces)
//! put back together under its own tag.
//!
//! It's hostile input: depth, size and counts are bounded, and anything that doesn't parse whole
//! is an error, never a guess.

use std::borrow::Cow;

use crate::Error;

/// Deeper nesting than this isn't a mail program's CMS.
const DEPTH_MAX: usize = 64;
/// The most elements read in one input.
const NODES_MAX: usize = 200_000;

/// One element: its tag byte(s), and either its contents (primitive: as read, unless pieces were
/// joined) or its children. `len` is its contents' length as DER writes them ([`sized`]).
#[derive(Debug)]
struct Node<'a> {
    tag: &'a [u8],
    body: Body<'a>,
    len: usize,
}

#[derive(Debug)]
enum Body<'a> {
    Bytes(Cow<'a, [u8]>),
    Children(Vec<Node<'a>>),
}

impl<'a> Node<'a> {
    fn new(tag: &'a [u8], body: Body<'a>) -> Node<'a> {
        Node { tag, body, len: 0 }
    }
}

const OCTET_STRING: u8 = 0x04;
const OBJECT_ID: u8 = 0x06;
const SEQUENCE: u8 = 0x30;
/// `[0]` constructed and primitive.
const CONTEXT_0_CONSTRUCTED: u8 = 0xA0;
const CONTEXT_0_PRIMITIVE: u8 = 0x80;

/// PKCS #7 `data`, 1.2.840.113549.1.7.1 (the contents of its OBJECT IDENTIFIER).
const DATA: &[u8] = &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x07, 0x01];
/// `hmacWithSHA1` and the SHA-2 ones after it, 1.2.840.113549.2.7 to .11, less the last byte.
const HMAC: &[u8] = &[0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x02];
const NULL: u8 = 0x05;
/// A `.p12` file's data inside data: the file, what it's sure of, and each part's bags.
const INSIDE_MAX: usize = 4;

/// `input` (one BER element, nothing after it) as DER.
pub fn to_der(input: &[u8]) -> Result<Vec<u8>, Error> {
    let mut count = 0usize;
    Ok(written(tidy(whole(input, &mut count)?)))
}

/// A `.p12` file (PKCS #12) as DER, inside as well as out: what it holds is DER or BER again
/// inside an OCTET STRING (`ContentInfo ::= SEQUENCE { data, [0] EXPLICIT OCTET STRING }`), which
/// [`to_der`] leaves as bytes. The file's check of its password (its MAC) is made over those bytes
/// as they were written, so it no longer fits what this gives: make it first.
///
/// A PBKDF2 that names its HMAC with no parameters (as NSS writes it; RFC 8018 has a NULL
/// there, and both are to be read) is given the NULL, which is what `pkcs5` reads.
pub(crate) fn p12_to_der(input: &[u8]) -> Result<Vec<u8>, Error> {
    let mut count = 0usize;
    Ok(written(inside(tidy(whole(input, &mut count)?), 0, &mut count)))
}

/// The one element `input` is, with nothing after it.
fn whole<'a>(input: &'a [u8], count: &mut usize) -> Result<Node<'a>, Error> {
    let (node, used) = read(input, 0, count)?;
    if input.get(used..).is_some_and(|rest| rest.iter().any(|b| *b != 0)) {
        return Err(Error::Malformed("data after the end"));
    }
    Ok(node)
}

/// [`p12_to_der`]'s part: `node` (tidied) with the data inside it tidied too.
fn inside<'a>(node: Node<'a>, depth: usize, count: &mut usize) -> Node<'a> {
    let Node { tag, body, .. } = node;
    let mut children: Vec<Node<'a>> = match body {
        Body::Bytes(b) => return Node::new(tag, Body::Bytes(b)),
        Body::Children(c) => c.into_iter().map(|c| inside(c, depth, count)).collect(),
    };
    if tag != [SEQUENCE] {
        return Node::new(tag, Body::Children(children));
    }
    let oid = |n: Option<&Node>| match n {
        Some(Node { tag, body: Body::Bytes(b), .. }) if *tag == [OBJECT_ID] => Some(b.to_vec()),
        _ => None,
    };
    let first = oid(children.first());
    // `AlgorithmIdentifier ::= SEQUENCE { an HMAC }`, its parameters left out.
    if children.len() == 1 && first.as_deref().and_then(|b| b.strip_prefix(HMAC)).is_some_and(|last| matches!(last, [7..=11])) {
        children.push(Node::new(&[NULL], Body::Bytes(Cow::Borrowed(&[]))));
    }
    // `ContentInfo ::= SEQUENCE { data, [0] EXPLICIT OCTET STRING }`: what the OCTET STRING holds.
    if children.len() == 2
        && first.as_deref() == Some(DATA)
        && depth < INSIDE_MAX
        && let Some(Node { tag, body: Body::Children(wrapped), .. }) = children.get_mut(1)
        && *tag == [CONTEXT_0_CONSTRUCTED]
        && let [Node { tag, body: Body::Bytes(held), .. }] = wrapped.as_mut_slice()
        && *tag == [OCTET_STRING]
        && let Ok(node) = whole(held, count)
    {
        let rewritten = written(inside(tidy(node), depth + 1, count));
        *held = Cow::Owned(rewritten);
    }
    Node::new(tag, Body::Children(children))
}

fn read<'a>(input: &'a [u8], depth: usize, count: &mut usize) -> Result<(Node<'a>, usize), Error> {
    *count += 1;
    if depth > DEPTH_MAX || *count > NODES_MAX {
        return Err(Error::Malformed("nested too deep"));
    }
    let first = *input.first().ok_or(Error::Malformed("cut short"))?;
    let mut at = 1;
    // A tag number over 30 continues in the bytes that follow (high bit set on all but the last).
    if first & 0x1F == 0x1F {
        loop {
            let b = *input.get(at).ok_or(Error::Malformed("cut short"))?;
            at += 1;
            if at > 6 {
                return Err(Error::Malformed("tag too long"));
            }
            if b & 0x80 == 0 {
                break;
            }
        }
    }
    let tag = input.get(..at).ok_or(Error::Malformed("cut short"))?;
    let constructed = first & 0x20 != 0;
    let len_byte = *input.get(at).ok_or(Error::Malformed("cut short"))?;
    at += 1;
    if len_byte == 0x80 {
        // Indefinite: children until two zero bytes.
        if !constructed {
            return Err(Error::Malformed("open length on a primitive"));
        }
        let mut children = Vec::new();
        loop {
            let rest = input.get(at..).ok_or(Error::Malformed("cut short"))?;
            if rest.starts_with(&[0, 0]) {
                at += 2;
                break;
            }
            let (child, used) = read(rest, depth + 1, count)?;
            children.push(child);
            at += used;
        }
        return Ok((Node::new(tag, Body::Children(children)), at));
    }
    let len = if len_byte & 0x80 == 0 {
        usize::from(len_byte)
    } else {
        let n = usize::from(len_byte & 0x7F);
        if n == 0 || n > 4 {
            return Err(Error::Malformed("length too long"));
        }
        let mut len = 0usize;
        for _ in 0..n {
            len = (len << 8) | usize::from(*input.get(at).ok_or(Error::Malformed("cut short"))?);
            at += 1;
        }
        len
    };
    let end = at.checked_add(len).ok_or(Error::Malformed("length too long"))?;
    let contents = input.get(at..end).ok_or(Error::Malformed("cut short"))?;
    if !constructed {
        return Ok((Node::new(tag, Body::Bytes(Cow::Borrowed(contents))), end));
    }
    let mut children = Vec::new();
    let mut pos = 0;
    while pos < contents.len() {
        let (child, used) = read(contents.get(pos..).unwrap_or_default(), depth + 1, count)?;
        children.push(child);
        pos += used;
    }
    Ok((Node::new(tag, Body::Children(children)), end))
}

/// Joins what BER cuts into pieces, the way DER writes it.
fn tidy(node: Node<'_>) -> Node<'_> {
    let Node { tag, body, .. } = node;
    let children = match body {
        Body::Bytes(b) => return Node::new(tag, Body::Bytes(b)),
        Body::Children(c) => c.into_iter().map(tidy).collect::<Vec<_>>(),
    };
    // A constructed OCTET STRING: its pieces, joined.
    if tag == [OCTET_STRING | 0x20] {
        return Node::new(&[OCTET_STRING], Body::Bytes(joined(children)));
    }
    // `EncryptedContentInfo ::= SEQUENCE { OID, AlgorithmIdentifier, [0] IMPLICIT OCTET STRING }`:
    // a constructed `[0]` there holds the encrypted content's pieces. (In `EncapsulatedContentInfo`,
    // `SEQUENCE { OID, [0] EXPLICIT OCTET STRING }`, the `[0]` is a wrapper and stays.)
    if tag == [SEQUENCE] {
        let shape = |i: usize| children.get(i).and_then(|c| c.tag.first().copied());
        if children.len() == 3 && shape(0) == Some(OBJECT_ID) && shape(1) == Some(SEQUENCE) && shape(2) == Some(CONTEXT_0_CONSTRUCTED) {
            let mut children = children;
            if let Some(last) = children.pop() {
                let pieces = match last.body {
                    Body::Children(c) => joined(c),
                    Body::Bytes(b) => b,
                };
                children.push(Node::new(&[CONTEXT_0_PRIMITIVE], Body::Bytes(pieces)));
            }
            return Node::new(tag, Body::Children(children));
        }
    }
    Node::new(tag, Body::Children(children))
}

/// The bytes of OCTET STRING pieces, one after another: a single piece as it is.
fn joined(mut pieces: Vec<Node<'_>>) -> Cow<'_, [u8]> {
    if let [Node { body: Body::Bytes(_), .. }] = pieces.as_slice()
        && let Some(Node { body: Body::Bytes(b), .. }) = pieces.pop()
    {
        return b;
    }
    let mut out = Vec::new();
    gather(&pieces, &mut out);
    Cow::Owned(out)
}

fn gather(pieces: &[Node<'_>], out: &mut Vec<u8>) {
    for p in pieces {
        match &p.body {
            Body::Bytes(b) => out.extend_from_slice(b),
            Body::Children(c) => gather(c, out),
        }
    }
}

/// `node` written as DER, its lengths worked out first so each element is written once,
/// straight into the output.
fn written(mut node: Node<'_>) -> Vec<u8> {
    let total = sized(&mut node);
    let mut out = Vec::with_capacity(total);
    write(&node, &mut out);
    out
}

/// Sets the contents' length of `node` and each element in it; gives how long `node` is
/// written (tag, length and contents).
fn sized(node: &mut Node<'_>) -> usize {
    node.len = match &mut node.body {
        Body::Bytes(b) => b.len(),
        Body::Children(children) => children.iter_mut().map(sized).fold(0usize, usize::saturating_add),
    };
    node.tag.len().saturating_add(length_len(node.len)).saturating_add(node.len)
}

fn write(node: &Node<'_>, out: &mut Vec<u8>) {
    out.extend_from_slice(node.tag);
    length(node.len, out);
    match &node.body {
        Body::Bytes(b) => out.extend_from_slice(b),
        Body::Children(children) => {
            for c in children {
                write(c, out);
            }
        }
    }
}

/// How many bytes [`length`] writes for `n`.
fn length_len(n: usize) -> usize {
    if n < 0x80 {
        return 1;
    }
    1 + n.to_be_bytes().iter().skip_while(|b| **b == 0).count()
}

fn length(n: usize, out: &mut Vec<u8>) {
    if n < 0x80 {
        out.push(n as u8);
        return;
    }
    let bytes = n.to_be_bytes();
    let skip = bytes.iter().take_while(|b| **b == 0).count();
    let used = bytes.get(skip..).unwrap_or_default();
    out.push(0x80 | used.len() as u8);
    out.extend_from_slice(used);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_lengths_and_pieces_become_der() {
        // SEQUENCE (open) { OID 1.2, OCTET STRING (constructed, open) { "ab", "c" } }
        let ber = [0x30, 0x80, 0x06, 0x01, 0x2A, 0x24, 0x80, 0x04, 0x02, b'a', b'b', 0x04, 0x01, b'c', 0, 0, 0, 0];
        assert_eq!(to_der(&ber).ok(), Some(vec![0x30, 0x08, 0x06, 0x01, 0x2A, 0x04, 0x03, b'a', b'b', b'c']));
        // An EncryptedContentInfo's content in pieces comes back under its own [0].
        let eci = [0x30, 0x80, 0x06, 0x01, 0x2A, 0x30, 0x00, 0xA0, 0x80, 0x04, 0x01, 1, 0x04, 0x01, 2, 0, 0, 0, 0];
        assert_eq!(to_der(&eci).ok(), Some(vec![0x30, 0x09, 0x06, 0x01, 0x2A, 0x30, 0x00, 0x80, 0x02, 1, 2]));
        // An explicit [0] wrapper stays as it is.
        let wrapped = [0x30, 0x07, 0x06, 0x01, 0x2A, 0xA0, 0x02, 0x04, 0x00];
        assert_eq!(to_der(&wrapped).ok(), Some(wrapped.to_vec()));
    }

    /// A `.p12` holds DER or BER again inside its OCTET STRINGs, and NSS leaves the NULL out
    /// after PBKDF2's HMAC (issue #89).
    #[test]
    fn a_p12_is_der_inside_too() {
        const DATA_OID: [u8; 11] = [0x06, 0x09, 0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x07, 0x01];
        // SEQUENCE (open) { SEQUENCE { hmacWithSHA256 } }
        let held = [0x30, 0x80, 0x30, 0x0A, 0x06, 0x08, 0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x02, 0x09, 0, 0];
        // SEQUENCE (open) { data, [0] (open) { OCTET STRING (in pieces) { held } } }
        let mut ber = vec![0x30, 0x80];
        ber.extend(DATA_OID);
        ber.extend([0xA0, 0x80, 0x24, 0x80, 0x04, 0x04]);
        ber.extend(&held[..4]);
        ber.extend([0x04, 0x0C]);
        ber.extend(&held[4..]);
        ber.extend([0, 0, 0, 0, 0, 0]);
        let held_der = [0x30, 0x0E, 0x30, 0x0C, 0x06, 0x08, 0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x02, 0x09, 0x05, 0x00];
        let mut der = vec![0x30, 0x1F];
        der.extend(DATA_OID);
        der.extend([0xA0, 0x12, 0x04, 0x10]);
        der.extend(held_der);
        assert_eq!(p12_to_der(&ber).ok(), Some(der.clone()));
        assert_eq!(p12_to_der(&der).ok(), Some(der), "DER stays as it is");
        // Outside a `.p12`, what an OCTET STRING holds is left as it was written.
        let outside = to_der(&ber).unwrap();
        assert!(outside.windows(held.len()).any(|w| w == held));
        // Another algorithm keeps its parameters as they are, and bytes that aren't ASN.1 stay.
        let other = [0x30, 0x0A, 0x06, 0x08, 0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x03, 0x07];
        assert_eq!(p12_to_der(&other).ok(), Some(other.to_vec()));
        let mut odd = vec![0x30, 0x12];
        odd.extend(DATA_OID);
        odd.extend([0xA0, 0x05, 0x04, 0x03, 0x30, 0x80, 0x01]);
        assert_eq!(p12_to_der(&odd).ok(), Some(odd));
    }

    #[test]
    fn junk_is_an_error_not_a_panic() {
        for junk in
            [&[][..], &[0x30], &[0x30, 0x80], &[0x30, 0x85, 1, 1, 1, 1, 1], &[0x04, 0x80, 0, 0], &[0x1F, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0]]
        {
            assert!(to_der(junk).is_err(), "{junk:?}");
            assert!(p12_to_der(junk).is_err(), "{junk:?}");
        }
        let deep: Vec<u8> = std::iter::repeat_n([0x30u8, 0x80], 100).flatten().collect();
        assert!(to_der(&deep).is_err());
    }

    /// How long a large encrypted message in BER takes to become DER: 16 MB of content in 4 KB
    /// pieces, as mail programs stream it, five levels deep (`--release`, by hand).
    #[test]
    #[ignore = "a measurement: run in release"]
    fn measure_to_der() {
        const DATA_OID: [u8; 11] = [0x06, 0x09, 0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x07, 0x01];
        let piece = vec![0x5Au8; 4096];
        // ContentInfo { OID, [0] { EnvelopedData { INTEGER, SET {}, EncryptedContentInfo { OID,
        // AlgorithmIdentifier, [0] { OCTET STRING pieces } } } } }, every length left open.
        let mut ber = vec![0x30, 0x80];
        ber.extend(DATA_OID);
        ber.extend([0xA0, 0x80, 0x30, 0x80, 0x02, 0x01, 0x00, 0x31, 0x00, 0x30, 0x80]);
        ber.extend(DATA_OID);
        ber.extend([0x30, 0x0D]);
        ber.extend(DATA_OID);
        ber.extend([0x05, 0x00]);
        ber.extend([0xA0, 0x80]);
        for _ in 0..4096 {
            ber.extend([0x04, 0x82, 0x10, 0x00]);
            ber.extend(&piece);
        }
        ber.extend([0; 10]);
        let n = 20;
        let t = std::time::Instant::now();
        let mut len = 0;
        for _ in 0..n {
            len = to_der(&ber).unwrap().len();
        }
        assert!(len > 16 << 20);
        println!("MEASURE to_der of {} MB: {:.1} ms", ber.len() >> 20, t.elapsed().as_secs_f64() * 1e3 / f64::from(n));
    }
}
