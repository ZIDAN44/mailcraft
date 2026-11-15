//! A message's text as the index keeps it (its files' names and its text, each cut to what's
//! kept), and the text pages that hold them.

use super::*;

/// What the index keeps of a message's text: its files' names and its text.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Blob {
    pub names: Vec<String>,
    pub text: String,
}

/// What is kept of a message's text: at most [`MAX_BLOB_TEXT`] bytes, cut where a character
/// starts.
pub(crate) fn kept_text(text: &str) -> &str {
    let mut end = text.len().min(MAX_BLOB_TEXT);
    while !text.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    text.get(..end).unwrap_or_default()
}

/// What is kept of a message's files' names: at most [`MAX_NAMES_BYTES`] of them, each at most
/// [`MAX_NAME_BYTES`], cut where a character starts (an empty one has nothing to keep); and
/// whether some of them was left out.
pub(crate) fn kept_names(all: &[String]) -> (Vec<&str>, bool) {
    let mut names: Vec<&str> = Vec::new();
    let mut room = MAX_NAMES_BYTES;
    let mut cut = false;
    for n in all.iter().filter(|n| !n.is_empty()) {
        let mut end = n.len().min(MAX_NAME_BYTES).min(room);
        while !n.is_char_boundary(end) {
            end = end.saturating_sub(1);
        }
        let kept = n.get(..end).unwrap_or_default();
        if kept.is_empty() || names.len() >= usize::from(u16::MAX) {
            cut = true;
            break;
        }
        cut |= kept.len() < n.len();
        room -= kept.len();
        names.push(kept);
    }
    (names, cut)
}

/// A text kept for a message (at most [`MAX_NAMES_BYTES`] of names and [`MAX_BLOB_TEXT`] of
/// text), and whether some of it was left out.
pub(crate) fn encode_blob(doc: &IndexDoc) -> (Vec<u8>, bool) {
    let (names, names_cut) = kept_names(&doc.names);
    let text = kept_text(&doc.text);
    let cut = text.len() < doc.text.len() || names_cut;
    // Exactly what it takes: a writer holds thousands of these, counted by their length.
    let names_bytes = names.iter().map(|n| n.len().saturating_add(2)).fold(0, usize::saturating_add);
    let mut out = Vec::with_capacity(BLOB.len() + 1 + 2 + names_bytes + 4 + text.len());
    out.extend_from_slice(BLOB);
    out.push(VERSION);
    out.extend_from_slice(&u16::try_from(names.len()).unwrap_or(u16::MAX).to_le_bytes());
    for n in names {
        out.extend_from_slice(&u16::try_from(n.len()).unwrap_or(u16::MAX).to_le_bytes());
        out.extend_from_slice(n.as_bytes());
    }
    out.extend_from_slice(&u32::try_from(text.len()).unwrap_or(u32::MAX).to_le_bytes());
    out.extend_from_slice(text.as_bytes());
    (out, cut)
}

pub(crate) fn decode_blob(b: &[u8]) -> R<Blob> {
    let mut r = Rd::new(b);
    r.header(BLOB)?;
    let n = usize::from(r.u16()?);
    if n.saturating_mul(2) > r.left() {
        return Err("names past what's there");
    }
    let mut blob = Blob { names: Vec::with_capacity(n), text: String::new() };
    for _ in 0..n {
        let len = usize::from(r.u16()?);
        blob.names.push(r.string(len)?);
    }
    let len = r.u32()? as usize;
    blob.text = r.string(len)?;
    r.end()?;
    Ok(blob)
}

/// A text page's bytes for each text before the text: its doc and its length.
pub(crate) const TEXT_REC_HEAD: usize = 8;

/// A text page: each doc's text, by doc (rising).
pub(crate) fn encode_text_page<'a>(recs: impl Iterator<Item = (u32, &'a [u8])>) -> Vec<u8> {
    let recs: Vec<(u32, &[u8])> = recs.collect();
    let mut out = Vec::with_capacity(7 + recs.iter().map(|(_, b)| b.len() + TEXT_REC_HEAD).sum::<usize>());
    out.extend_from_slice(TEXT);
    out.push(VERSION);
    out.extend_from_slice(&u16::try_from(recs.len()).unwrap_or(u16::MAX).to_le_bytes());
    for (doc, blob) in recs {
        out.extend_from_slice(&doc.to_le_bytes());
        out.extend_from_slice(&u32::try_from(blob.len()).unwrap_or(u32::MAX).to_le_bytes());
        out.extend_from_slice(blob);
    }
    out
}

/// A text page's texts (not yet read), their docs rising and below `next_doc`.
pub(crate) fn decode_text_page(b: &[u8], next_doc: u32) -> R<Vec<(u32, &[u8])>> {
    let at = text_page_index(b, next_doc)?;
    Ok(at.into_iter().map(|(doc, at)| (doc, b.get(at).unwrap_or_default())).collect())
}

/// A text page's texts as [`decode_text_page`] reads them: each doc and where its text is in
/// the page (kept by a reader asked for one text after another of the same page).
pub(crate) fn text_page_index(b: &[u8], next_doc: u32) -> R<Vec<(u32, Range<usize>)>> {
    if b.len() > MAX_TEXT_PAGE {
        return Err("too big");
    }
    let mut r = Rd::new(b);
    r.header(TEXT)?;
    let n = usize::from(r.u16()?);
    if n.saturating_mul(TEXT_REC_HEAD) > r.left() {
        return Err("texts past what's there");
    }
    let mut out: Vec<(u32, Range<usize>)> = Vec::with_capacity(n);
    for _ in 0..n {
        let doc = r.u32()?;
        let len = r.u32()? as usize;
        if doc >= next_doc || out.last().is_some_and(|(d, _)| *d >= doc) {
            return Err("text of a doc out of order");
        }
        let start = b.len() - r.left();
        r.take(len)?;
        out.push((doc, start..start + len));
    }
    r.end()?;
    Ok(out)
}
