//! What the user types in the search box (and saved searches read back): the query language,
//! the Advanced search form, the server-side forms of a search, and IMAP's modified UTF-7.
#![no_main]

use libfuzzer_sys::fuzz_target;
use mailcraft_model::search::{Advanced, Query};
use mailcraft_model::utf7;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let q = Query::parse(&text);
    let _ = q.index_words();
    let _ = q.needs_body();
    let a = Advanced::from_query(&text);
    let _ = Advanced::from_query(&a.to_query());
    let _ = mailcraft_jmap::search_filter(&text);
    let _ = utf7::encode(&utf7::decode(&text));
});
