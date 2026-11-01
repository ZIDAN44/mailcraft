//! An import's checkpoint as it's read back from an account's data file (`import.json`): parsed
//! and checked, and, when it's one an import would go on from, written again and read back the
//! same.
#![no_main]

use libfuzzer_sys::fuzz_target;
use mailcraft_net::pst;

fuzz_target!(|input: &[u8]| {
    if let Some(same) = pst::checkpoint_from_bytes(input) {
        assert!(same, "a checkpoint gone on from read back otherwise once written again");
    }
});
