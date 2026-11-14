//! The shared `%XX` encoder writes what each copy it replaced wrote: the copies as they were (the
//! JMAP client's `pct`, the HTTP client's `pct`, the editor's mailto subject), kept here as the
//! reference, for any text; and what it writes reads back as it was.

use mailcraft_model::mailto::{percent_decode, percent_encode};
use proptest::prelude::*;

/// `mailcraft_jmap::pct` and `mailcraft_net::http::pct`, as they were (the same code).
fn pct(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The editor's mailto subject (`ui-egui/src/rich_edit/links.rs`), as it was.
fn subject(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    #[test]
    fn writes_what_each_copy_wrote(s in any::<String>()) {
        let one = percent_encode(&s);
        prop_assert_eq!(&one, &pct(&s));
        prop_assert_eq!(&one, &subject(&s));
        prop_assert_eq!(percent_decode(&one), s);
    }
}

#[test]
fn every_byte() {
    let all: String = (0u32..0x800).filter_map(char::from_u32).collect();
    assert_eq!(percent_encode(&all), pct(&all));
    assert_eq!(percent_encode("a b/é~"), "a%20b%2F%C3%A9~");
}
