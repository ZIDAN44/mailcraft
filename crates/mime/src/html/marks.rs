//! The characters MailCraft marks a message's HTML with while it becomes text: where a heading,
//! a picture, raised or lowered text, a size, a typeface or a signature's picture is. All in one
//! place, so that no two are the same character: private-use ones, which mail doesn't use. A
//! pass that marks takes its marks out of the HTML it's given first ([`without`]), so that a
//! message (or text pasted) can't write one.

use std::borrow::Cow;

/// Where a heading starts ([`super::mark_headings`]).
pub(crate) const HEADING: char = '\u{E000}';
/// The description given to a picture that has none ([`super::mark_pictures`]).
pub(crate) const PICTURE: char = '\u{E001}';
/// Where raised and lowered text start, and where they end ([`super::mark_scripts`]).
pub(crate) const SUP: char = '\u{E002}';
pub(crate) const SUB: char = '\u{E003}';
pub(crate) const SCRIPT_END: char = '\u{E004}';
/// Where a signature's picture stands while its HTML becomes text ([`super::signature_html`]).
pub(crate) const SIGNATURE: char = '\u{E005}';
/// Where an element with a size or typeface starts (the next character, from [`FONT_INDEX`],
/// says which) and ends ([`super::fonts`]).
pub(crate) const FONT_START: char = '\u{E020}';
pub(crate) const FONT_END: char = '\u{E021}';
pub(crate) const FONT_INDEX: u32 = 0xE100;
/// Where text of a size starts (the first plus its size in pixels, at most 255) and ends, in a
/// plane of private characters of its own ([`super::size_mark`]).
pub(crate) const SIZES: u32 = 0xF_0000;
pub(crate) const SIZE_END: char = '\u{F_0100}';

/// Whether `c` is a size's mark or its end.
pub(crate) fn is_size(c: char) -> bool {
    (SIZES..=u32::from(SIZE_END)).contains(&u32::from(c))
}

/// `s` without the marks `is` says are marks; `s` itself when it has none.
pub(crate) fn without(s: &str, is: impl Fn(char) -> bool) -> Cow<'_, str> {
    if s.contains(&is) { Cow::Owned(s.chars().filter(|c| !is(*c)).collect()) } else { Cow::Borrowed(s) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// No two marks are one character (a signature's picture and the start of raised text were
    /// both U+E002), nor in another's range.
    #[test]
    fn every_mark_is_its_own() {
        let one = [HEADING, PICTURE, SUP, SUB, SCRIPT_END, SIGNATURE, FONT_START, FONT_END];
        for (i, a) in one.iter().enumerate() {
            assert!(one.iter().skip(i + 1).all(|b| a != b), "{a:?}");
            assert!(!is_size(*a));
            let index = u32::from(*a).wrapping_sub(FONT_INDEX);
            assert!(index > u32::try_from(super::super::fonts::MAX_MARKS).unwrap_or(0), "{a:?} is a typeface's index");
        }
        assert!(is_size(SIZE_END) && is_size(char::from_u32(SIZES).unwrap()));
        assert_eq!(without("a\u{E000}b\u{F000A}c", is_size), "a\u{E000}bc");
        assert!(matches!(without("abc", is_size), Cow::Borrowed(_)));
    }
}
