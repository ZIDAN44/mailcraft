//! The text of someone's message read for the details in their signature (issue #178): their
//! phone numbers and web addresses, never more than a few, each found in the text.
#![no_main]

use libfuzzer_sys::fuzz_target;
use mailcraft_model::pim::signature::signature_details;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    // The name the sender's mail gives: the first line's start.
    let name: String = text.lines().next().unwrap_or("").chars().take(40).collect();
    let found = signature_details(&text, &name);
    assert!(found.phones.len() <= 5 && found.websites.len() <= 5);
    for p in &found.phones {
        let digits = p.chars().filter(char::is_ascii_digit).count();
        assert!((7..=15).contains(&digits), "{p}");
    }
});
