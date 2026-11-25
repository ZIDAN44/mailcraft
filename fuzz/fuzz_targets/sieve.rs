//! Another program's Sieve script, as a ManageSieve server hands it over (issue #243): read for
//! the automatic reply it sends.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    let _ = mailcraft_net::sieve::vacation_in(&String::from_utf8_lossy(input));
});
