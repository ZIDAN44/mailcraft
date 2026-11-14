use std::hint::black_box;
use std::time::Instant;

use mailcraft_pst::{Bytes, PropertyValue, Utf16};
use proptest::prelude::*;

use super::*;

/// What a text came to before NULs were cut in place: trimmed, then copied.
fn copied(s: &str) -> String {
    s.trim_end_matches('\0').to_string()
}

/// Cutting the NULs in place leaves what trimming and copying left: the trailing NULs go, and
/// nothing else (NULs in the middle stay, a text of NULs alone is empty).
#[test]
fn trailing_nuls_go_and_nothing_else() {
    for (s, kept) in [
        ("", ""),
        ("\0", ""),
        ("\0\0\0\0", ""),
        ("plain", "plain"),
        ("plain\0", "plain"),
        ("plain\0\0\0", "plain"),
        ("a\0b", "a\0b"),
        ("a\0b\0\0", "a\0b"),
        ("\0a", "\0a"),
        ("caf\u{e9}\0", "caf\u{e9}"),
        ("\u{1F600}\0\0", "\u{1F600}"),
        ("\u{FFFD}\0", "\u{FFFD}"),
    ] {
        assert_eq!(without_nuls(s.to_string()), kept, "{s:?}");
        assert_eq!(without_nuls(s.to_string()), copied(s), "{s:?}");
    }
}

/// Texts with NULs anywhere, multi-byte characters around them.
fn texts() -> impl Strategy<Value = String> {
    let c = prop_oneof![3 => Just('\0'), 1 => Just('a'), 1 => Just('\u{e9}'), 1 => Just('\u{20AC}'), 1 => Just('\u{1F600}'), 1 => any::<char>()];
    proptest::collection::vec(c, 0..40).prop_map(|cs| cs.into_iter().collect())
}

/// 8-bit texts, NULs common.
fn bytes() -> impl Strategy<Value = Vec<u8>> {
    proptest::collection::vec(prop_oneof![3 => Just(0u8), 2 => any::<u8>()], 0..40)
}

proptest! {
    #[test]
    fn cutting_in_place_is_trimming(s in texts()) {
        prop_assert_eq!(without_nuls(s.clone()), copied(&s));
    }

    /// A property's value is what it was when it was trimmed and copied: UTF-16, unpaired
    /// surrogates and all, and 8-bit in a code page, alone and in a list.
    #[test]
    fn a_text_value_is_unchanged(s in texts(), lone in any::<bool>(), b in bytes(), page in prop_oneof![Just(0u32), Just(1252), Just(65001), Just(932)]) {
        let mut units: Vec<u16> = s.encode_utf16().collect();
        if lone {
            units.insert(units.len() / 2, 0xD800);
        }
        let wide = copied(&String::from_utf16_lossy(&units));
        let narrow = copied(&decode(&b, page, false));
        let one_wide = PropertyValue::Unicode(Utf16(units.clone()));
        let one_narrow = PropertyValue::String8(Bytes(b.clone()));
        let some_wide = PropertyValue::MultipleUnicode(vec![Utf16(units.clone()), Utf16(units)]);
        let some_narrow = PropertyValue::MultipleString8(vec![Bytes(b.clone()), Bytes(b)]);
        prop_assert_eq!(val(&one_wide, page), Val::Text(Cow::Borrowed(wide.as_str())));
        prop_assert_eq!(val(&one_narrow, page), Val::Text(Cow::Borrowed(narrow.as_str())));
        prop_assert_eq!(val(&some_wide, page), Val::Texts(vec![wide.clone(), wide]));
        prop_assert_eq!(val(&some_narrow, page), Val::Texts(vec![narrow.clone(), narrow]));
    }
}

/// How long turning a multi-megabyte UTF-16 and 8-bit text property into its value takes.
/// `cargo test --release -p mailcraft-net --lib text_property_speed -- --ignored --nocapture`
#[test]
#[ignore = "a measurement"]
fn text_property_speed() {
    const ROUNDS: u32 = 20;
    let line = "Dear Ana, the r\u{e9}sum\u{e9} is attached; see you on Thursday at the caf\u{e9}.\r\n";
    let text = line.repeat(8 * 1024 * 1024 / line.len());
    let wide = PropertyValue::Unicode(Utf16(text.encode_utf16().chain([0, 0]).collect()));
    // Windows-1252, not valid UTF-8, so it is read in its page.
    let narrow = PropertyValue::String8(Bytes(text.chars().map(|c| if c == '\u{e9}' { 0xE9 } else { c as u8 }).chain([0]).collect()));
    for (name, v) in [("UTF-16", &wide), ("8-bit", &narrow)] {
        assert_eq!(val(v, 1252), Val::Text(Cow::Borrowed(text.as_str())), "{name}");
        let started = Instant::now();
        for _ in 0..ROUNDS {
            black_box(val(black_box(v), 1252));
        }
        let per = started.elapsed() / ROUNDS;
        println!("MEASURE {name} text property of {} bytes into its value: {per:?} each ({ROUNDS} rounds)", text.len());
    }
}
