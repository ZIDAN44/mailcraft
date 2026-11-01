//! An account's search index as its files read back: the meta, the doc table, text pages and
//! segments, each damaged any way, opened, looked up, followed and written to as the app does.
//! The input is the files one after another, each starting with its four letters (`MCSM` the
//! meta first), as `mailcraft_store::search::hostile_index` splits them.
#![no_main]

use libfuzzer_sys::fuzz_target;
use mailcraft_store::search;

fuzz_target!(|data: &[u8]| {
    let store = search::hostile_index(data);
    search::exercise_index(&store);
});
