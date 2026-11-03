//! A MailCraft data folder's own files as another data folder reads them, damaged any way: its id
//! file (`datafolder.json`), a data file's record of the folder it's an account of (`owner` in
//! its `meta` table) and its list of data files (`datafiles.json`). Whatever is taken from them
//! is an id as MailCraft makes them, or nothing.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    assert!(mailcraft_store::fuzz_data_folder(data), "an id taken from a data folder's files isn't one");
});
