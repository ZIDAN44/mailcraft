//! A text all ASCII split into words as UAX #29 splits it ([`unicode_words`]), by what its rules
//! say of ASCII alone, a byte at a time: segmenting was most of what folding a summary's text
//! cost, and most summaries' subjects, addresses and previews are ASCII. A proptest holds it to
//! unicode-segmentation's words.
//!
//! In ASCII, a word is letters, digits and `_` together (WB5, WB8–WB10, WB13a–b), with a `:`,
//! `.` or `'` between two letters (WB6–7) and a `,`, `;`, `.` or `'` between two digits
//! (WB11–12) kept inside it. Nothing else joins anything to a word, and a piece with no letter
//! or digit (spaces, punctuation, `_` alone) isn't one.
//!
//! [`unicode_words`]: unicode_segmentation::UnicodeSegmentation::unicode_words

/// The words of an ASCII text.
pub(crate) struct AsciiWords<'a> {
    text: &'a str,
    at: usize,
}

impl<'a> AsciiWords<'a> {
    /// `text` should be ASCII: a word that would end inside a character that isn't ends the
    /// words there.
    pub(crate) fn new(text: &'a str) -> AsciiWords<'a> {
        AsciiWords { text, at: 0 }
    }
}

/// Which bytes are a letter, a digit or `_`: what a word is made of (a look-up, the test made
/// for each byte of each text).
const INSIDE: [bool; 256] = {
    let mut t = [false; 256];
    let mut b = 0;
    while b < 256 {
        let c = b as u8;
        t[b] = c.is_ascii_alphanumeric() || c == b'_';
        b += 1;
    }
    t
};

fn inside(b: u8) -> bool {
    INSIDE.get(usize::from(b)).copied().unwrap_or(false)
}

impl<'a> Iterator for AsciiWords<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<&'a str> {
        let bytes = self.text.as_bytes();
        loop {
            // The next word's start: past what can't start one.
            let start = self.at.checked_add(bytes.get(self.at..)?.iter().position(|b| inside(*b))?)?;
            let mut end = start;
            loop {
                let run = bytes.get(end..).unwrap_or_default();
                end = end.saturating_add(run.iter().position(|b| !inside(*b)).unwrap_or(run.len()));
                let Some(&b) = bytes.get(end) else { break };
                if !matches!(b, b':' | b'.' | b'\'' | b',' | b';') {
                    break;
                }
                // A mark between two letters or two digits keeps them one word.
                let (prev, next) = (end.checked_sub(1).and_then(|i| bytes.get(i)).copied(), end.checked_add(1).and_then(|i| bytes.get(i)).copied());
                let between = |what: fn(&u8) -> bool| prev.as_ref().is_some_and(what) && next.as_ref().is_some_and(what);
                let joined = match b {
                    b':' => between(u8::is_ascii_alphabetic),
                    b'.' | b'\'' => between(u8::is_ascii_alphabetic) || between(u8::is_ascii_digit),
                    b',' | b';' => between(u8::is_ascii_digit),
                    _ => false,
                };
                if !joined {
                    break;
                }
                end = end.saturating_add(2);
            }
            self.at = end;
            // Not `_` alone: a letter or a digit is in it (a mark only ever is between them).
            let piece = self.text.get(start..end)?;
            if piece.bytes().any(|b| b != b'_') {
                return Some(piece);
            }
        }
    }
}

/// ASCII spaces, tabs and line ends: no UAX #29 rule joins them to what's before them, nor to an
/// ASCII character after them.
fn apart(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

/// A text cut where UAX #29 always breaks, so each piece splits into the words it has in the
/// whole: before a run of ASCII spaces and line ends that comes after an ASCII character (a space
/// past ASCII keeps an ASCII one after it, WB3d), and after one when an ASCII character follows
/// (a combining mark, a format character or a joiner after it would join it, and no ASCII
/// character does). No rule looks across such a run. A text with a few characters past ASCII (a
/// curly quote, a no-break space, an accent) is then mostly pieces all ASCII, split a byte at a
/// time ([`AsciiWords`]): of a real file's 34,691 summaries, 1,247 had a subject and preview
/// ASCII all through, and splitting the rest whole was most of what making their texts took. A
/// proptest holds it to unicode-segmentation's words.
pub(crate) fn pieces(text: &str) -> impl Iterator<Item = &str> {
    let bytes = text.as_bytes();
    let mut at = 0;
    std::iter::from_fn(move || {
        let start = at;
        let mut prev = *bytes.get(start)?;
        let mut end = start.saturating_add(1);
        while let Some(&b) = bytes.get(end) {
            if (apart(b) && prev.is_ascii() && !apart(prev)) || (apart(prev) && !apart(b) && b.is_ascii()) {
                break;
            }
            prev = b;
            end = end.saturating_add(1);
        }
        at = end;
        // Each cut is at an ASCII byte, so at a character's start.
        text.get(start..end)
    })
}
