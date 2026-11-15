//! An event's notes read as lines and links in time their length, whatever brackets they hold.

use proptest::prelude::*;

use super::*;
use crate::ical::notes::{bracketed_link, note_line};

/// [`bracketed_link`] as it was: each `<` looked at alone, to the next `>`.
fn each_bracket(line: &str) -> Option<(&str, &str, &str)> {
    let mut from = 0;
    loop {
        let open = from + line.get(from..)?.find('<')?;
        let inner = line.get(open + 1..)?;
        let close = inner.find('>');
        let url = close.and_then(|c| inner.get(..c)).unwrap_or("");
        let lower = url.to_ascii_lowercase();
        let scheme = ["https://", "http://", "mailto:", "tel:"].iter().any(|s| lower.starts_with(s));
        if let Some(c) = close
            && scheme
            && url.len() > 7
            && !url.contains(char::is_whitespace)
        {
            return Some((line.get(..open)?, url, inner.get(c + 1..)?));
        }
        from = open + 1;
    }
}

fn line() -> impl Strategy<Value = String> {
    let piece = proptest::sample::select(vec![
        "<",
        ">",
        "<<",
        " ",
        "\t",
        "\u{a0}",
        "é",
        "x",
        "a.b",
        "https://",
        "HTTP://",
        "mailto:",
        "Tel:",
        "tel:+1",
        "<https://x.example/",
        "<mailto:a@b>",
        "<https:// x>",
    ]);
    proptest::collection::vec(piece, 0..24).prop_map(|v| v.concat())
}

proptest! {
    #[test]
    fn a_bracketed_link_is_the_one_each_bracket_looked_at_alone_finds(l in line()) {
        prop_assert_eq!(bracketed_link(&l), each_bracket(&l));
    }
}

/// Lines a bracket at a time: each `<` looked for its `>` and lower-cased what was before it,
/// over the rest of the line: 100,000 of them without one took seconds, every frame the event
/// was open. The bytes looked through are counted, held in proportion to the line.
#[test]
fn many_brackets_are_read_at_once() {
    for l in many_brackets() {
        let parts = crate::work::linear(l.len(), || note_line(&l));
        let text: usize = parts.iter().map(|p| if let NotePart::Text(s) = p { s.len() } else { 0 }).sum();
        assert_eq!(text, l.len(), "{}", l.get(..20).unwrap_or(""));
    }
}

/// Lines of 100,000 brackets: none closed, all closed by one, or addresses closed past a space.
fn many_brackets() -> [String; 3] {
    ["<".repeat(100_000), format!("{}>", "<".repeat(100_000)), format!("{} >", "<https://".repeat(20_000))]
}

/// How long those lines take: `cargo test --release -p mailcraft-model --lib
/// how_long_many_brackets_take -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement: run in release"]
fn how_long_many_brackets_take() {
    for l in many_brackets() {
        let t = std::time::Instant::now();
        let _ = note_line(&l);
        println!("MEASURE a line of {:?}: {:.2} ms", l.get(..10).unwrap_or(""), t.elapsed().as_secs_f64() * 1000.0);
    }
}

/// An invitation's notes as long as they're read (200,000 characters), read as lines and links.
/// `cargo test --release -p mailcraft-model --lib measure_long_notes -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement: run in release"]
fn measure_long_notes() {
    let invite = "Join with Google Meet<https://meet.google.com/abc-defg-hij>\nOr dial: (US) +1 555-0100<tel:+1-555-0100%3B618%23> PIN: 618\nMore phone numbers<https://tel.meet/abc?pin=618&hs=1>\n\nSee https://example.org/agenda, and a < b > c.\n";
    let text = invite.repeat(200_000 / invite.len() + 1);
    let n = note_lines(&text).len();
    let t = std::time::Instant::now();
    for _ in 0..20 {
        assert_eq!(note_lines(&text).len(), n);
    }
    println!("MEASURE long notes ({n} lines): {:.2} ms", t.elapsed().as_secs_f64() * 1000.0 / 20.0);
}
