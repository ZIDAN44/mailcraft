//! What a writer holds in memory: counted as the heap holds it, so the writer flushes at what
//! it says it holds (an import's peak memory goes by it).

use std::collections::BTreeMap;

use super::*;

/// A message's kept text takes the room its bytes need: a writer holds thousands of them
/// waiting for the next flush, counted by their length.
#[test]
fn a_kept_text_takes_the_room_its_bytes_need() {
    for names in [vec![], vec!["plan.pdf".to_string(), "notes.txt".to_string()]] {
        let doc = IndexDoc { text: "The week's plan, as agreed.".repeat(40), names, files: 2, has_text: true, ..IndexDoc::default() };
        let (blob, cut) = encode_blob(&doc);
        assert!(!cut);
        assert_eq!(blob.capacity(), blob.len(), "no room kept spare");
        assert!(blob.len() < 2048);
    }
}

/// The words to write are counted for at least the room their keys, lists and places take, and
/// their keys' and postings' places in the map, and not far over it.
#[test]
fn words_are_counted_as_the_heap_holds_them() {
    let mut words = BTreeMap::new();
    let mut counted = 0usize;
    for doc in 0..3000u32 {
        // Some words in every message, some in a few, most in one.
        let text = format!("plan week{} item{} note{doc} draft{}", doc % 7, doc % 300, doc / 2);
        counted += writer::add_words(&mut words, doc, &text, &[]);
    }
    let held: usize = words.iter().map(|(k, v)| k.capacity() + v.docs.capacity() * 4 + v.places.capacity() + 72).sum();
    assert!(counted >= held, "counted {counted}, held {held}");
    assert!(counted <= held * 2, "counted {counted}, held {held}");
}
