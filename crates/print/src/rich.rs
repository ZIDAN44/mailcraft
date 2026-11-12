//! A formatted message's text on the page: its lines of runs (bold, italic, size, colour, a
//! highlight, underlined and struck-through text) wrapped to the page's width, each run in its
//! own font and size on a shared baseline.

use super::{Face, Placed};

/// A run of text in one format.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Run {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    /// In points; none: the text's usual size.
    pub size: Option<f32>,
    pub color: Option<[u8; 3]>,
    /// A highlight behind the text.
    pub background: Option<[u8; 3]>,
    /// It's on a background of the message's own (a cell's, a button's), so its colour is kept
    /// however light: white words on a blue button stay white.
    pub on_background: bool,
    /// Where a click on it goes.
    pub link: Option<String>,
    /// A picture in the text's place: which of the document's, and how wide and tall it's
    /// shown, in points.
    pub picture: Option<(usize, f32, f32)>,
}

/// The size of text that sets none, and the room a line of it takes.
pub(super) const SIZE: f32 = 11.0;
pub(super) const LEAD: f32 = 14.5;
/// The tallest a picture is shown: more is scaled down, so it's whole on a page.
const TALLEST: f32 = 620.0;
/// The room a picture's line keeps over it.
const OVER_PICTURE: f32 = 4.0;

/// A colour too light to read on white paper (white text from a message with a dark background,
/// which isn't printed): such text is printed black.
fn too_light(c: [u8; 3]) -> bool {
    0.299 * f32::from(c[0]) + 0.587 * f32::from(c[1]) + 0.114 * f32::from(c[2]) > 200.0
}

fn piece(run: &Run, text: String, x: f32, width: f32) -> Placed {
    let size = run.size.map_or(SIZE, |s| s.clamp(6.0, 36.0));
    // A highlight keeps the colour written on it, as a background of the message's does;
    // without one, the paper is white.
    let color = run.color.filter(|c| run.background.is_some() || run.on_background || !too_light(*c));
    Placed {
        font: usize::from(run.bold),
        size,
        x,
        y: 0.0,
        text,
        width,
        slant: run.italic,
        color,
        background: run.background,
        underline: run.underline,
        strike: run.strike,
        link: run.link.clone(),
        picture: None,
    }
}

/// A run's size, and how wide `text` is in it.
fn measured(faces: &[Face<'_>], run: &Run, text: &str) -> f32 {
    let size = run.size.map_or(SIZE, |s| s.clamp(6.0, 36.0));
    faces.get(usize::from(run.bold)).map_or(0.0, |f| f.text_width(text, size))
}

/// What a line needs across: at least its longest word (a picture shrinks to fit), and how
/// wide it is on one line.
pub(super) fn fit(faces: &[Face<'_>], line: &[Run]) -> (f32, f32) {
    let (mut min, mut max) = (0.0f32, 0.0f32);
    for run in line {
        if let Some((_, w, _)) = run.picture {
            min = min.max(w.min(24.0));
            max += w;
            continue;
        }
        let text = run.text.replace(['\n', '\r'], " ").replace('\t', "    ");
        for word in text.split(' ').filter(|w| !w.is_empty()) {
            min = min.max(measured(faces, run, word));
        }
        max += measured(faces, run, &text);
    }
    (min, max)
}

/// Lay `lines` out `width` wide: `put` is given each line of the page in turn, its pieces placed
/// from the left edge at 0 and the room it takes down the page; it says whether there was room.
pub(super) fn lay(faces: &[Face<'_>], lines: &[Vec<Run>], width: f32, mut put: impl FnMut(f32, Vec<Placed>) -> bool) -> bool {
    let measure = |run: &Run, text: &str| measured(faces, run, text);
    // A line is as tall as its biggest text, or its tallest picture.
    let lead_of = |row: &[Placed]| row.iter().map(|p| p.picture.map_or(p.size * LEAD / SIZE, |(_, h)| h + OVER_PICTURE)).fold(LEAD, f32::max);
    for line in lines {
        let mut row: Vec<Placed> = Vec::new();
        let mut x = 0.0f32;
        // A space is owed before the next word (one ended the last word, or began this run).
        let mut space = false;
        for run in line {
            // A picture: one box, never wider than the line nor taller than a page, on the next
            // line when this one has no room left.
            if let Some((index, w, h)) = run.picture {
                let scale = (width / w.max(1.0)).min(TALLEST / h.max(1.0)).min(1.0);
                let (w, h) = (w.max(1.0) * scale, h.max(1.0) * scale);
                let gap = if space && x > 0.0 { measure(run, " ") } else { 0.0 };
                if x > 0.0 && x + gap + w > width {
                    let lead = lead_of(&row);
                    if !put(lead, std::mem::take(&mut row)) {
                        return false;
                    }
                    x = 0.0;
                }
                let at = if x > 0.0 { x + gap } else { 0.0 };
                row.push(Placed { x: at, width: w, size: SIZE, picture: Some((index, h)), link: run.link.clone(), ..Default::default() });
                x = at + w;
                space = false;
                continue;
            }
            let text = run.text.replace(['\n', '\r'], " ").replace('\t', "    ");
            for (i, word) in text.split(' ').enumerate() {
                if i > 0 {
                    space = true;
                }
                if word.is_empty() {
                    continue;
                }
                let gap = if space && x > 0.0 { measure(run, " ") } else { 0.0 };
                let w = measure(run, word);
                if x > 0.0 && x + gap + w > width {
                    let lead = lead_of(&row);
                    if !put(lead, std::mem::take(&mut row)) {
                        return false;
                    }
                    x = 0.0;
                }
                if w > width {
                    // A word wider than the page is cut by characters.
                    let mut part = String::new();
                    for c in word.chars() {
                        part.push(c);
                        if measure(run, &part) > width && part.chars().count() > 1 {
                            let last = part.pop().unwrap_or(' ');
                            let done = std::mem::take(&mut part);
                            let dw = measure(run, &done);
                            if !put(lead_of(&row).max(run.size.map_or(LEAD, |s| s * LEAD / SIZE)), vec![piece(run, done, 0.0, dw)]) {
                                return false;
                            }
                            part.push(last);
                        }
                    }
                    let pw = measure(run, &part);
                    row.push(piece(run, part, 0.0, pw));
                    x = pw;
                } else if space && x > 0.0 && i == 0 {
                    // The space before a run's first word isn't the run's: a link's underline
                    // starts under its first letter.
                    row.push(piece(run, word.to_string(), x + gap, w));
                    x += gap + w;
                } else if space && x > 0.0 {
                    row.push(piece(run, format!(" {word}"), x, gap + w));
                    x += gap + w;
                } else {
                    row.push(piece(run, word.to_string(), x, w));
                    x += w;
                }
                space = false;
            }
        }
        let lead = lead_of(&row);
        if !put(lead, row) {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(lines: &[Vec<Run>], width: f32) -> Vec<Vec<Placed>> {
        let faces = [Face::Helvetica { bold: false }, Face::Helvetica { bold: true }];
        let mut out = Vec::new();
        assert!(lay(&faces, lines, width, |_, row| {
            out.push(row);
            true
        }));
        out
    }

    fn run(text: &str) -> Run {
        Run { text: text.into(), ..Default::default() }
    }

    #[test]
    fn runs_keep_their_format_and_wrap_together() {
        let line = vec![
            run("Hello "),
            Run { bold: true, ..run("Riley") },
            run(", see the "),
            Run { underline: true, color: Some([11, 87, 208]), ..run("link to the report") },
            Run { italic: true, size: Some(16.5), ..run(" now") },
        ];
        let one = rows(std::slice::from_ref(&line), 1000.0);
        assert_eq!(one.len(), 1);
        let row = &one[0];
        let text: String = row.iter().map(|p| p.text.as_str()).collect();
        assert_eq!(text, "HelloRiley, see thelink to the report now", "a space between runs is room, not a letter");
        // Each piece starts where the one before ended, or a space further.
        assert!(row.windows(2).all(|w| w[1].x >= w[0].x + w[0].width - 0.01), "pieces don't overlap");
        let bold = row.iter().find(|p| p.text == "Riley").map(|p| p.font);
        assert_eq!(bold, Some(1));
        // The underline is under the link's words and the spaces between them, not before it.
        let link: Vec<_> = row.iter().filter(|p| p.underline).collect();
        assert_eq!(link.iter().map(|p| p.text.as_str()).collect::<Vec<_>>(), ["link", " to", " the", " report"]);
        assert!(link.iter().all(|p| p.color == Some([11, 87, 208])));
        assert!(row.iter().any(|p| p.slant && (p.size - 16.5).abs() < 0.01));
        // Narrow: more rows, none wider than the page, nothing lost.
        let narrow = rows(std::slice::from_ref(&line), 80.0);
        assert!(narrow.len() > 2);
        assert!(narrow.iter().flatten().all(|p| p.x + p.width <= 80.5), "within the width");
        let words: String = narrow.iter().flatten().map(|p| p.text.trim()).collect();
        assert_eq!(words, "HelloRiley,seethelinktothereportnow");
    }

    #[test]
    fn light_text_prints_black_unless_it_has_a_background() {
        let white = Run { color: Some([255, 255, 255]), ..run("invisible on paper") };
        let on_blue = Run { background: Some([31, 58, 95]), ..white.clone() };
        // In a box the message colours (a button's words): kept, the box being printed too.
        let on_button = Run { on_background: true, ..white.clone() };
        let r = rows(&[vec![white], vec![on_blue], vec![], vec![on_button]], 500.0);
        assert_eq!(r.len(), 4, "an empty line is a line");
        assert!(r[0].iter().all(|p| p.color.is_none()));
        assert!(r[1].iter().all(|p| p.color == Some([255, 255, 255]) && p.background.is_some()));
        assert!(r[3].iter().all(|p| p.color == Some([255, 255, 255]) && p.background.is_none()));
        // A word wider than the page is cut, not run off it.
        let long = rows(&[vec![run(&"x".repeat(400))]], 100.0);
        assert!(long.len() > 5 && long.iter().flatten().all(|p| p.width <= 100.5));
    }
}
