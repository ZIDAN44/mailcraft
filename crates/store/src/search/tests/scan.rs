//! Search index: a word looked for a little at a time.

use super::super::reader::{DOCS_A_PAGE, DOCS_A_WORD};
use super::*;

/// A one-letter prefix of thousands of words, each in a doc or two (their docs kept in the
/// lexicon page itself, no page read for them): each call sets the docs of a bounded number of
/// words, not all of a page's, and the calls together find them all. Each lexicon page is decoded
/// once, however many calls look through it (decoding it again for each was most of what `k`
/// took over 50,000 messages: 177 ms of 681 units).
#[test]
fn a_short_prefix_of_thousands_of_words_is_scanned_a_little_a_call() {
    let s = Store::memory();
    let mut w = Writer::open(&s, "a1").unwrap();
    let n = 6000u32;
    for i in 0..n {
        w.add(&mk("INBOX", i + 1), u64::from(i), 100, &text(&format!("kw{i:05} other"), &[], 0)).unwrap();
    }
    assert_eq!(w.flush(&s).unwrap(), Some(1));
    let mut r = Reader::open(&s, "a1").unwrap().unwrap();
    let meta = load_meta(&s, "a1").unwrap().unwrap();
    let lex_pages = meta.segments.iter().map(|seg| seg.lex_pages).sum::<u32>();
    let (mut cur, mut out, mut calls) = (ScanCursor::default(), Bits::new(r.len()), 1u32);
    while !r.scan_prefix(&s, Field::Body, "k", &mut cur, &mut out, 1).unwrap() {
        calls += 1;
    }
    assert_eq!(out.ones().count(), n as usize);
    let words_a_call = DOCS_A_PAGE / DOCS_A_WORD;
    assert!(calls as usize >= n as usize / words_a_call, "{calls} calls over {lex_pages} lexicon pages");
    assert!(calls > lex_pages, "{calls} calls over {lex_pages} lexicon pages");
    assert!(r.lex_decoded() <= u64::from(lex_pages), "{} decoded for {lex_pages} lexicon pages", r.lex_decoded());
}
