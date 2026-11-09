use super::*;
use crate::html::mark_headings;

/// A message of `html`, as received.
fn message(html: &str) -> crate::Parsed {
    let raw = format!("From: a@shop.example\r\nSubject: s\r\nContent-Type: text/html\r\n\r\n{html}");
    crate::parse(raw.as_bytes()).unwrap_or_default()
}

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

/// Hostile: these scans stay linear (found in review of the same kind of scan: a million `<h`
/// each looked through to the end of the message took minutes).
#[test]
fn the_scans_stay_quick() {
    for unit in ["<h ", "<h1 ", "<h1>", "<sup ", "<sup>", "</sup", "<sub>x"] {
        let html = format!("{}>", unit.repeat(1_000_000 / unit.len()));
        let started = std::time::Instant::now();
        let _ = mark_headings(&html);
        let _ = mark_scripts(&html);
        let _ = plain_scripts(&html);
        assert!(started.elapsed() < std::time::Duration::from_secs(2), "{unit}: {:?}", started.elapsed());
    }
}
