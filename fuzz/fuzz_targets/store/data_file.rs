//! An account's data file as a build before this one may have left it, damaged any way: files
//! whole and in pieces (notes saying any length, pieces missing or too many), compressed or raw,
//! the search index's, and `META` (how the file says it was written, where a recompress stopped).
//! Its health measured, then compressed again (stopped after its first batch, gone on with, then
//! once more), and every file that read back before reads back the same. The input is records, as
//! `mailcraft_store::fuzz_data_file` reads them.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    assert!(mailcraft_store::fuzz_data_file(data), "a file reads back otherwise once compressed again");
});
