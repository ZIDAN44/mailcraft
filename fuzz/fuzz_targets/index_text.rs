//! A message source read for search: its text parts found by their boundaries and decoded, its
//! files counted and named but never decoded, pictures its HTML shows told from files.
#![no_main]

use libfuzzer_sys::fuzz_target;
use mailcraft_mime as mime;
use mailcraft_model::search::MAX_INDEX_CHARS;

fuzz_target!(|raw: &[u8]| {
    let (doc, _) = mime::index_doc_stats(raw);
    assert!(doc.text.chars().count() <= MAX_INDEX_CHARS);
    let _ = mime::search_text(raw);
    // The paperclip's count, from the same rule.
    let key = mailcraft_model::MessageKey { account: "a".into(), folder: "INBOX".into(), validity: 1, uid: 1 };
    let _ = mime::summary_from_header(raw, key, mailcraft_model::Flags::default(), 0, 0);
});
