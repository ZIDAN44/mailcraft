use super::*;
use crate::html::mark_headings;
use crate::tests::util::message;

/// Found on the same notice: a heading holding a box (`<h2><div>`) kept the `##` the text
/// converter starts each of its lines with, and wasn't centred; so with a line break in it.
#[test]
fn a_heading_in_lines_of_its_own_has_no_marks() {
    for html in ["<h2><div>Your package</div></h2>", "<h2 class=x>\n<div>Your package</div>\n</h2>", "<h2>Your <b>package</b></h2>"] {
        let r = html_to_rich(html);
        let line: String = r[0].iter().map(|s| s.text.as_str()).collect();
        assert_eq!(line.trim(), "Your package", "{html}: {r:?}");
        assert!(r[0].iter().all(|s| s.heading == 2), "{html}: {r:?}");
    }
    let r = html_to_rich("<h1>One<br>Two</h1>");
    let seen: Vec<String> = r.iter().map(|l| l.iter().map(|s| s.text.as_str()).collect()).collect();
    assert_eq!(seen, ["One", "Two"], "{r:?}");
    assert!(r.iter().flatten().all(|s| s.heading == 1));
    // A line that only begins with one stays as it was written.
    assert_eq!(html_to_rich("<p># not a heading</p>")[0][0].text, "# not a heading");
    let b = crate::layout::html_layout(
        r#"<table width="600"><tr><td><div style="text-align:center"><h2><div>Your package</div></h2></div></td></tr></table>"#,
    );
    let crate::layout::Block::Table(outer) = &b[0] else { panic!("{b:?}") };
    assert!(matches!(&outer.rows[0][0].blocks[0], crate::layout::Block::Table(t) if t.rows[0][0].align == crate::layout::Align::Center), "{b:?}");
}

/// Found on issue #122: a title in a quote, its words in a colour of their own
/// (`<blockquote><h2><span style="color:…">`), kept the marks the text converter writes before
/// it as words: `> ## How the week went`. It reads as a title, as one in no colour does.
#[test]
fn a_coloured_title_in_a_quote_has_no_marks() {
    for html in [
        r#"<blockquote><h2><span style="color: #15245e;">How the week went</span></h2></blockquote>"#,
        r#"<blockquote style="line-height:175%"><div dir="ltr"><h2 style="text-align:center" align="center"><span style="color:#15245e">How the week went</span></h2><ul></ul></div></blockquote>"#,
        "<blockquote><h2>How the week went</h2></blockquote>",
        r#"<h2><span style="color:#15245e">How the week went</span></h2>"#,
    ] {
        let r = html_to_rich(html);
        let line = r.iter().find(|l| l.iter().any(|s| s.text.contains("week"))).unwrap_or_else(|| panic!("{html}: {r:?}"));
        assert_eq!(line.iter().map(|s| s.text.as_str()).collect::<String>().trim(), "How the week went", "{html}: {r:?}");
        assert!(line.iter().filter(|s| !s.text.is_empty()).all(|s| s.heading == 2), "{html}: {r:?}");
        let p = message(&format!(r#"<table width="600"><tr><td>{html}<p>Then the rest.</p></td></tr></table>"#));
        // Laid out too (the plain text keeps its `## `, as it does for every title).
        let laid_out = format!("{:?}", p.layout);
        assert!(laid_out.contains("How the week went") && !laid_out.contains("##") && !laid_out.contains("\"> "), "{html}: {laid_out}");
    }
}

/// Found on the same notice: `^{$}15⁹⁸` for a price written `<sup>$</sup>15<sup>98</sup>`.
#[test]
fn raised_text_is_raised_and_not_written_out() {
    let r = html_to_rich("<p>From <sup style=\"color:red\">$</sup>15<sup>98</sup> or H<sub>2</sub>O</p>");
    let parts: Vec<(&str, Rise)> = r[0].iter().map(|s| (s.text.as_str(), s.rise)).collect();
    assert_eq!(
        parts,
        [
            ("From ", Rise::Normal),
            ("$", Rise::Sup),
            ("15", Rise::Normal),
            ("98", Rise::Sup),
            (" or H", Rise::Normal),
            ("2", Rise::Sub),
            ("O", Rise::Normal)
        ],
        "{r:?}"
    );
    assert_eq!(r[0][1].color, Some([255, 0, 0]));
    let p = message("<p>From <sup>$</sup>15<sup>98</sup></p>");
    assert!(p.text.contains("From $15⁹⁸"), "{:?}", p.text);
    // Left open, it ends with its line; the sender's own marks are no marks.
    let r = html_to_rich("<p>A<sup>B</p><p>C\u{E002}D\u{E004}E</p>");
    assert_eq!(r[0][1].rise, Rise::Sup);
    let last: String = r.last().unwrap().iter().map(|s| s.text.as_str()).collect();
    assert_eq!(last, "CDE");
    assert!(r.last().unwrap().iter().all(|s| s.rise == Rise::Normal));
}

/// A megabyte of each of the marks the scans look for, none of them ended.
fn hostile_scans() -> impl Iterator<Item = String> {
    ["<h ", "<h1 ", "<h1>", "<sup ", "<sup>", "</sup", "<sub>x"].into_iter().map(|unit| format!("{}>", unit.repeat(1_000_000 / unit.len())))
}

/// Hostile: these scans stay linear (found in review of the same kind of scan: a million `<h`
/// each looked through to the end of the message took minutes): the bytes they look through
/// are counted, held in proportion to the message whatever the machine's load.
#[test]
fn the_scans_stay_quick() {
    for html in hostile_scans() {
        work::linear(html.len(), || {
            let _ = mark_headings(&html);
            let _ = mark_scripts(&html);
            let _ = plain_scripts(&html);
        });
    }
}

/// How long those scans take: `cargo test --release -p mailcraft-mime --lib how_long_the_scans_take -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement: run in release"]
fn how_long_the_scans_take() {
    for html in hostile_scans() {
        let started = std::time::Instant::now();
        let _ = mark_headings(&html);
        let _ = mark_scripts(&html);
        let _ = plain_scripts(&html);
        println!("MEASURE scans of a megabyte of {:?}: {} ms", &html[..5], started.elapsed().as_millis());
    }
}

/// Lowered text in plain text: the converter writes what's in a `<sub>` as it is, so
/// [`plain_scripts`] has only `<sup>` to look at.
#[test]
fn lowered_text_is_written_plainly() {
    assert_eq!(html_to_text("<p>H<sub>2</sub>O and x<sub>ab</sub> and 1<sup>st</sup></p>"), "H2O and xab and 1st");
}

/// A mark stands for a size [`size_mark`] writes, whatever character of the marks' range it is:
/// one from outside what's written (U+F00FF, 255 pixels) set its text that large.
#[test]
fn a_mark_is_no_size_past_what_is_written() {
    assert_eq!(marked_size(size_mark(24)), Some(24));
    assert_eq!((marked_size('\u{F00FF}'), marked_size('\u{F0001}')), (Some(96), Some(9)));
    assert_eq!((marked_size(SIZE_END), marked_size('a')), (None, None));
}
