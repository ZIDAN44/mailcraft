//! `%XX` decoded by one decoder ([`mailcraft_model::mailto::percent_decode`]) where there were
//! five: the paperclip's, the reading pane's, the trackers', and the editor's two. Each old one is
//! kept here, as it was, to show the one gives what each gave.

use mailcraft_model::mailto::{percent_decode, percent_decode_bytes};
use proptest::prelude::*;

/// The paperclip's (`attachments.rs`) and, byte for byte, the reading pane's
/// (`picture_attachment`).
fn old_mime(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while let Some(&c) = b.get(i) {
        match (c, b.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok())) {
            (b'%', Some(v)) => {
                out.push(v);
                i += 3;
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The trackers' (`trackers.rs`), `+` a space in a query.
fn old_trackers(s: &str, plus: bool) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while let Some(&c) = b.get(i) {
        match c {
            b'%' => {
                let hex = s.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok());
                match hex {
                    Some(v) => {
                        out.push(v);
                        i += 3;
                        continue;
                    }
                    None => out.push(c),
                }
            }
            b'+' if plus => out.push(b' '),
            _ => out.push(c),
        }
        i += 1;
    }
    String::from_utf8(out).ok()
}

/// The editor's for a mailto link (`ui-egui/src/rich_edit/links.rs`).
fn old_links(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while let Some(&c) = b.get(i) {
        let hex = |k: usize| b.get(k).and_then(|h| (*h as char).to_digit(16));
        match (c, hex(i + 1), hex(i + 2)) {
            (b'%', Some(h), Some(l)) => {
                out.push(u8::try_from(h * 16 + l).unwrap_or(b'?'));
                i += 3;
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The editor's for a picture's file in a saved page (`ui-egui/src/app/actions/inserting.rs`),
/// its bytes (read as UTF-8 or Windows' Western characters after).
fn old_inserting(s: &str) -> Vec<u8> {
    let raw = s.as_bytes();
    let mut out = Vec::with_capacity(raw.len());
    let mut i = 0;
    while let Some(&b) = raw.get(i) {
        let hex = |k: usize| raw.get(k).and_then(|h| char::from(*h).to_digit(16));
        match (b, hex(i + 1), hex(i + 2)) {
            (b'%', Some(a), Some(c)) => {
                out.push(u8::try_from(a * 16 + c).unwrap_or(b'%'));
                i += 3;
            }
            _ => {
                out.push(b);
                i += 1;
            }
        }
    }
    out
}

/// Text made of what escapes are made of.
fn escapes() -> impl Strategy<Value = String> {
    let piece = prop_oneof![Just("%".to_string()), "[0-9a-fA-FgG]{1,3}", "[+ /\\\\?#]", "[\u{e9}\u{20ac}\u{1f600}]", "%[89a-fA-F][0-9a-fA-F]",];
    proptest::collection::vec(piece, 0..24).prop_map(|p| p.concat())
}

/// What the `from_str_radix` decoders took for two hex digits that aren't: a `+` before one
/// (`%+f`, a sign to Rust, not a digit to RFC 3986).
fn signed_escape(s: &str) -> bool {
    s.as_bytes().windows(3).any(|w| w.first() == Some(&b'%') && w.get(1) == Some(&b'+') && w.get(2).is_some_and(u8::is_ascii_hexdigit))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(4000))]

    #[test]
    fn one_decoder_gives_what_each_gave(s in escapes()) {
        prop_assert_eq!(percent_decode(&s), old_links(&s));
        prop_assert_eq!(percent_decode_bytes(&s), old_inserting(&s));
        if !signed_escape(&s) {
            prop_assert_eq!(crate::attachments::percent_decoded(&s), old_mime(&s));
            prop_assert_eq!(crate::trackers::percent_decode(&s, false), old_trackers(&s, false));
            prop_assert_eq!(crate::trackers::percent_decode(&s, true), old_trackers(&s, true));
        }
    }
}

/// The one change: `%+f` isn't an escape (RFC 3986's are `%` and two hex digits), and stays as
/// written; the old decoders read it as byte 15.
#[test]
fn a_sign_is_no_hex_digit() {
    assert_eq!(old_mime("a%+fb"), "a\u{f}b");
    assert_eq!(crate::attachments::percent_decoded("a%+fb"), "a%+fb");
    assert_eq!(crate::trackers::percent_decode("a%+fb", true).as_deref(), Some("a% fb"));
    assert_eq!(crate::attachments::percent_decoded("logo%20big.png"), "logo big.png");
}
