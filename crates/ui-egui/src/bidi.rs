//! Right-to-left text (Arabic, Hebrew, Persian…) for egui.
//!
//! egui shapes each run of text (a stretch drawn from one font) in the direction its first
//! letter's script has, so a word of Arabic or Hebrew comes out joined and right to left; but it
//! lays the runs out left to right in the order they are written, and a run that mixes scripts
//! is shaped in one direction. This module gives egui text it draws the right way round:
//!
//! 1. The Unicode Bidirectional Algorithm (UAX #9, the `unicode-bidi` crate) puts each line in
//!    the order it is seen (visual order).
//! 2. The line is cut where egui will cut it into runs: where the font changes. The main font's
//!    coverage of Arabic and Hebrew decides that ([`set_coverage`]).
//! 3. A run egui will turn around (its first letter is right-to-left) is handed over turned
//!    around already, so it comes out right; in a run egui keeps left to right, Arabic letters
//!    are given in their joined forms (Arabic Presentation Forms), as the shaper won't join them.
//!
//! The result is for drawing only, never for storing, searching or copying. Lines are laid out
//! one at a time: wrapped text goes through [`job`], which breaks the lines first.

use std::sync::atomic::{AtomicU8, Ordering};

use egui::epaint::text::ByteIndex;
use egui::text::{LayoutJob, LayoutSection};
use unicode_bidi::{BidiInfo, Level};

const ARABIC: u8 = 1;
const HEBREW: u8 = 2;
const FORMS: u8 = 4;
/// What the main font draws: Arabic letters, Hebrew letters, Arabic presentation forms.
static COVERAGE: AtomicU8 = AtomicU8::new(ARABIC | HEBREW | FORMS);

/// Note which right-to-left scripts the main interface font has (from its character map).
pub fn set_coverage(font: Option<&[u8]>) {
    let has = |c: char| font.is_some_and(|f| covers(f, c));
    let mut bits = 0;
    if has('\u{0627}') && has('\u{0644}') {
        bits |= ARABIC;
    }
    if has('\u{05D0}') {
        bits |= HEBREW;
    }
    if has('\u{FE8E}') && has('\u{FEFB}') {
        bits |= FORMS;
    }
    COVERAGE.store(bits, Ordering::Relaxed);
}

/// Whether the main font has both Arabic and Hebrew.
pub fn covers_rtl() -> bool {
    COVERAGE.load(Ordering::Relaxed) & (ARABIC | HEBREW) == ARABIC | HEBREW
}

fn covers(font: &[u8], c: char) -> bool {
    use skrifa::MetadataProvider as _;
    skrifa::FontRef::new(font).is_ok_and(|f| f.charmap().map(c).is_some())
}

/// A right-to-left letter (or mark, or digit of a right-to-left script).
pub fn is_rtl(c: char) -> bool {
    matches!(u32::from(c), 0x0590..=0x08FF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFF | 0x10800..=0x10FFF | 0x1E800..=0x1EFFF)
}

pub fn has_rtl(s: &str) -> bool {
    s.chars().any(is_rtl)
}

/// Whether egui draws `c` from the main font (so it shares a run with Latin text).
fn in_main_font(c: char) -> bool {
    let bits = COVERAGE.load(Ordering::Relaxed);
    match u32::from(c) {
        0x0600..=0x08FF => bits & ARABIC != 0,
        0xFB50..=0xFDFF | 0xFE70..=0xFEFF => bits & FORMS != 0,
        0x0590..=0x05FF | 0xFB1D..=0xFB4F => bits & HEBREW != 0,
        _ => true,
    }
}

/// A letter that decides a run's direction for the shaper (marks, digits, spaces and
/// punctuation don't).
fn is_letter(c: char) -> bool {
    c.is_alphabetic() && !matches!(u32::from(c), 0x064B..=0x065F | 0x0670 | 0x0591..=0x05C7)
}

/// The paired brackets' mirror images, for right-to-left runs.
fn mirror(c: char) -> char {
    match c {
        '(' => ')',
        ')' => '(',
        '[' => ']',
        ']' => '[',
        '{' => '}',
        '}' => '{',
        '<' => '>',
        '>' => '<',
        '«' => '»',
        '»' => '«',
        '‹' => '›',
        '›' => '‹',
        _ => c,
    }
}

/// How an Arabic letter joins: to both sides (`D`), only to the one before (`R`), or makes its
/// neighbours join without changing (`C`, the tatweel and the zero-width joiner).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Join {
    D,
    R,
    C,
    /// Doesn't join (and breaks joining).
    U,
    /// Combining marks: skipped over.
    T,
}

/// A letter's presentation forms: (isolated, final, initial, medial); `R` letters have the
/// first two.
fn forms(c: char) -> Option<(Join, u32)> {
    let d = |base| Some((Join::D, base));
    let r = |base| Some((Join::R, base));
    match u32::from(c) {
        0x0622 => r(0xFE81),
        0x0623 => r(0xFE83),
        0x0624 => r(0xFE85),
        0x0625 => r(0xFE87),
        0x0626 => d(0xFE89),
        0x0627 => r(0xFE8D),
        0x0628 => d(0xFE8F),
        0x0629 => r(0xFE93),
        0x062A => d(0xFE95),
        0x062B => d(0xFE99),
        0x062C => d(0xFE9D),
        0x062D => d(0xFEA1),
        0x062E => d(0xFEA5),
        0x062F => r(0xFEA9),
        0x0630 => r(0xFEAB),
        0x0631 => r(0xFEAD),
        0x0632 => r(0xFEAF),
        0x0633 => d(0xFEB1),
        0x0634 => d(0xFEB5),
        0x0635 => d(0xFEB9),
        0x0636 => d(0xFEBD),
        0x0637 => d(0xFEC1),
        0x0638 => d(0xFEC5),
        0x0639 => d(0xFEC9),
        0x063A => d(0xFECD),
        0x0641 => d(0xFED1),
        0x0642 => d(0xFED5),
        0x0643 => d(0xFED9),
        0x0644 => d(0xFEDD),
        0x0645 => d(0xFEE1),
        0x0646 => d(0xFEE5),
        0x0647 => d(0xFEE9),
        0x0648 => r(0xFEED),
        0x0649 => r(0xFEEF),
        0x064A => d(0xFEF1),
        // Persian and Urdu letters (Presentation Forms-A).
        0x0671 => r(0xFB50),
        0x067E => d(0xFB56),
        0x0686 => d(0xFB7A),
        0x0698 => r(0xFB8A),
        0x06A9 => d(0xFB8E),
        0x06AF => d(0xFB92),
        0x06CC => d(0xFBFC),
        _ => None,
    }
}

fn join_type(c: char) -> Join {
    if let Some((j, _)) = forms(c) {
        return j;
    }
    match u32::from(c) {
        0x0640 | 0x200D => Join::C,
        0x064B..=0x065F | 0x0670 | 0x06D6..=0x06ED => Join::T,
        _ => Join::U,
    }
}

/// Arabic letters in their joined forms, in logical order. Each output character keeps the
/// index of the character it came from (a lam-alef ligature, the lam's).
fn arabic_forms(chars: &[(char, usize)]) -> Vec<(char, usize)> {
    let types: Vec<Join> = chars.iter().map(|(c, _)| join_type(*c)).collect();
    // The nearest non-mark neighbour on each side.
    let neighbour = |i: usize, back: bool| -> Join {
        let mut j = i;
        loop {
            if back {
                if j == 0 {
                    return Join::U;
                }
                j -= 1;
            } else {
                j += 1;
            }
            match types.get(j) {
                Some(Join::T) => continue,
                Some(t) => return *t,
                None => return Join::U,
            }
        }
    };
    let alef_ligature = |c: char| match c {
        '\u{0622}' => Some(0xFEF5),
        '\u{0623}' => Some(0xFEF7),
        '\u{0625}' => Some(0xFEF9),
        '\u{0627}' => Some(0xFEFB),
        _ => None,
    };
    let mut out = Vec::with_capacity(chars.len());
    let mut i = 0;
    while let Some(&(c, at)) = chars.get(i) {
        let Some((join, base)) = forms(c) else {
            out.push((c, at));
            i += 1;
            continue;
        };
        let before = matches!(neighbour(i, true), Join::D | Join::C);
        // Lam then alef: one ligature.
        if c == '\u{0644}'
            && let Some(lig) = chars.get(i + 1).and_then(|(n, _)| alef_ligature(*n))
        {
            out.push((char::from_u32(lig + u32::from(before)).unwrap_or(c), at));
            i += 2;
            continue;
        }
        let after = join == Join::D && matches!(neighbour(i, false), Join::D | Join::R | Join::C);
        let offset = match (before, after, join) {
            (true, true, Join::D) => 3,
            (false, true, Join::D) => 2,
            (true, _, _) => 1,
            _ => 0,
        };
        out.push((char::from_u32(base + offset).unwrap_or(c), at));
        i += 1;
    }
    out
}

/// One line (no line breaks) in the order egui needs to draw it right. `rtl`: the paragraph's
/// direction, `None` to take it from the first strong letter. Each output character keeps the
/// index of the input character it shows.
fn line_indexed(text: &str, rtl: Option<bool>) -> Vec<(char, usize)> {
    let level = rtl.map(|r| if r { Level::rtl() } else { Level::ltr() });
    let info = BidiInfo::new(text, level);
    let Some(para) = info.paragraphs.first() else { return text.chars().enumerate().map(|(i, c)| (c, i)).collect() };
    // Character index of each byte offset.
    let mut index_of = vec![0usize; text.len() + 1];
    for (i, (b, _)) in text.char_indices().enumerate() {
        if let Some(slot) = index_of.get_mut(b) {
            *slot = i;
        }
    }
    let (levels, runs) = info.visual_runs(para, para.range.clone());
    let mut visual: Vec<(char, usize)> = Vec::with_capacity(text.len());
    for run in runs {
        let Some(part) = text.get(run.clone()) else { continue };
        let rtl_run = levels.get(run.start).is_some_and(|l| l.is_rtl());
        let chars = part.char_indices().map(|(b, c)| (c, index_of.get(run.start + b).copied().unwrap_or(0)));
        if rtl_run {
            visual.extend(chars.rev().map(|(c, i)| (mirror(c), i)));
        } else {
            visual.extend(chars);
        }
    }
    // Joined forms where the shaper won't join: computed in reading order, then placed.
    let pre_join = COVERAGE.load(Ordering::Relaxed) & (ARABIC | FORMS) == ARABIC | FORMS;
    let joined: Option<std::collections::HashMap<usize, Option<char>>> = pre_join.then(|| {
        let logical: Vec<(char, usize)> = text.chars().enumerate().map(|(i, c)| (c, i)).collect();
        let shaped = arabic_forms(&logical);
        let mut map: std::collections::HashMap<usize, Option<char>> = logical.iter().map(|(_, i)| (*i, None)).collect();
        for (c, i) in shaped {
            map.insert(i, Some(c));
        }
        map
    });
    // Cut where egui's runs are cut, and turn around what it will turn around.
    let mut out = Vec::with_capacity(visual.len());
    let mut start = 0;
    while start < visual.len() {
        let main = visual.get(start).is_some_and(|(c, _)| in_main_font(*c));
        let len = visual.get(start..).map_or(0, |rest| rest.iter().take_while(|(c, _)| in_main_font(*c) == main).count()).max(1);
        let run = visual.get(start..start + len).unwrap_or_default();
        start += len;
        if !run.iter().any(|(c, _)| is_rtl(*c)) {
            out.extend_from_slice(run);
            continue;
        }
        // Handed over turned around, the run starts with its last letter: egui turns it around
        // (and mirrors brackets) when that letter is right-to-left.
        if run.iter().rev().find(|(c, _)| is_letter(*c)).is_some_and(|(c, _)| is_rtl(*c)) {
            out.extend(run.iter().rev().map(|(c, i)| (mirror(*c), *i)));
        } else {
            // Kept left to right, so not joined by the shaper: joined forms (a lam-alef
            // ligature stands for both letters; the alef is dropped).
            for &(c, i) in run {
                match joined.as_ref().map(|m| m.get(&i).copied().flatten()) {
                    Some(Some(j)) => out.push((j, i)),
                    Some(None) => {}
                    None => out.push((c, i)),
                }
            }
        }
    }
    out
}

/// One line of text as egui should be given it to show it right (unchanged without
/// right-to-left letters).
pub fn line(text: &str, rtl: Option<bool>) -> String {
    if !has_rtl(text) {
        return text.to_string();
    }
    text.split('\n').map(|l| line_indexed(l, rtl).into_iter().map(|(c, _)| c).collect::<String>()).collect::<Vec<_>>().join("\n")
}

/// One line of mail (a subject, a name) ready to draw, and whether it reads right to left (then
/// it belongs against the right edge, where it starts).
pub fn shown(text: &str) -> (std::borrow::Cow<'_, str>, bool) {
    if !has_rtl(text) {
        return (std::borrow::Cow::Borrowed(text), false);
    }
    (std::borrow::Cow::Owned(line(text, None)), paragraph_rtl(text))
}

/// A label for text that may be right to left: wrapped at the width there is, each line in
/// order, right-to-left paragraphs against the right edge.
pub fn label(ui: &mut egui::Ui, text: &str, font: egui::FontId, color: egui::Color32) -> egui::Response {
    if !has_rtl(text) {
        return ui.label(egui::RichText::new(text).font(font).color(color));
    }
    let mut j = LayoutJob::simple(text.to_string(), font, color, ui.available_width());
    j.wrap.max_width = ui.available_width();
    let j = ui.fonts_mut(|f| job(f, j));
    ui.add(egui::Label::new(j).selectable(false))
}

/// Whether a paragraph reads right to left (its first strong letter is).
pub fn paragraph_rtl(text: &str) -> bool {
    BidiInfo::new(text, None).paragraphs.first().is_some_and(|p| p.level.is_rtl())
}

/// A layout job with right-to-left text made to show right: its lines broken at
/// `job.wrap.max_width` (between words), each line put in visual order, right-to-left paragraphs
/// against the right edge. Unchanged without right-to-left letters.
pub fn job(fonts: &mut egui::epaint::text::FontsView<'_>, job: LayoutJob) -> LayoutJob {
    if !has_rtl(&job.text) {
        return job;
    }
    let width = job.wrap.max_width;
    // Each character with its section.
    let chars: Vec<(char, usize)> = job
        .text
        .char_indices()
        .map(|(b, c)| (c, job.sections.iter().position(|s| (s.byte_range.start.0..s.byte_range.end.0).contains(&b)).unwrap_or(0)))
        .collect();
    let measure = |fonts: &mut egui::epaint::text::FontsView<'_>, part: &[(char, usize)]| -> f32 {
        // Pieces of one section at a time (fonts differ).
        let mut w = 0.0;
        let mut start = 0;
        while let Some(&(_, s)) = part.get(start) {
            let n = part.get(start..).map_or(0, |r| r.iter().take_while(|(_, x)| *x == s).count()).max(1);
            let text: String = part.get(start..start + n).unwrap_or_default().iter().map(|(c, _)| *c).collect();
            if let Some(sec) = job.sections.get(s) {
                w += fonts.layout_no_wrap(text, sec.format.font_id.clone(), sec.format.color).rect.width();
            }
            start += n;
        }
        w
    };
    let mut out = LayoutJob { text: String::new(), sections: Vec::new(), ..job.clone() };
    out.wrap.max_width = f32::INFINITY;
    let mut first_line = true;
    for para in chars.split(|(c, _)| *c == '\n') {
        let para_text: String = para.iter().map(|(c, _)| *c).collect();
        let rtl = paragraph_rtl(&para_text);
        // Words, each with the spaces after it.
        let mut words: Vec<&[(char, usize)]> = Vec::new();
        let mut start = 0;
        for i in 0..para.len() {
            let space = para.get(i).is_some_and(|(c, _)| c.is_whitespace());
            let next_space = para.get(i + 1).is_some_and(|(c, _)| c.is_whitespace());
            if space && !next_space {
                words.push(para.get(start..=i).unwrap_or_default());
                start = i + 1;
            }
        }
        if start < para.len() || para.is_empty() {
            words.push(para.get(start..).unwrap_or_default());
        }
        // Fill lines.
        let mut lines: Vec<(Vec<(char, usize)>, f32)> = Vec::new();
        let mut cur: Vec<(char, usize)> = Vec::new();
        let mut cur_w = 0.0;
        for w in words {
            let ww = measure(fonts, w);
            if !cur.is_empty() && cur_w + ww > width {
                lines.push((std::mem::take(&mut cur), cur_w));
                cur_w = 0.0;
            }
            cur.extend_from_slice(w);
            cur_w += ww;
        }
        lines.push((cur, cur_w));
        for (line, line_w) in lines {
            if !first_line {
                let s = line.first().map_or(0, |(_, s)| *s);
                push(&mut out, &job, "\n", s, 0.0);
            }
            first_line = false;
            let text: String = line.iter().map(|(c, _)| *c).collect();
            let shown = line_indexed(text.trim_end(), Some(rtl));
            let lead = if rtl && width.is_finite() { (width - line_w).max(0.0) } else { 0.0 };
            let mut first = true;
            for (c, i) in shown {
                let s = line.get(i).map_or(0, |(_, s)| *s);
                let mut b = [0u8; 4];
                push(&mut out, &job, c.encode_utf8(&mut b), s, if first { lead } else { 0.0 });
                first = false;
            }
        }
    }
    out
}

/// Append text in the format of `job`'s section `s` (a new section when it changes, or when the
/// row starts with space before it).
fn push(out: &mut LayoutJob, job: &LayoutJob, text: &str, s: usize, lead: f32) {
    let Some(format) = job.sections.get(s).map(|x| x.format.clone()) else { return };
    let start = out.text.len();
    out.text.push_str(text);
    let end = out.text.len();
    if lead == 0.0
        && let Some(last) = out.sections.last_mut()
        && last.byte_range.end.0 == start
        && last.format == format
    {
        last.byte_range.end = ByteIndex(end);
        return;
    }
    out.sections.push(LayoutSection { leading_space: lead, byte_range: ByteIndex(start)..ByteIndex(end), format });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(bits: u8, f: impl FnOnce()) {
        let old = COVERAGE.swap(bits, Ordering::Relaxed);
        f();
        COVERAGE.store(old, Ordering::Relaxed);
    }

    #[test]
    fn arabic_letters_join() {
        let idx = |s: &str| s.chars().enumerate().map(|(i, c)| (c, i)).collect::<Vec<_>>();
        let shaped: String = arabic_forms(&idx("سلام")).into_iter().map(|(c, _)| c).collect();
        // seen initial, lam-alef final ligature, meem alone (alef never joins what follows).
        assert_eq!(shaped, "\u{FEB3}\u{FEFC}\u{FEE1}");
        let shaped: String = arabic_forms(&idx("دار")).into_iter().map(|(c, _)| c).collect();
        // dal and alef join only to what comes before them: each stands alone.
        assert_eq!(shaped, "\u{FEA9}\u{FE8D}\u{FEAD}");
        // A mark between letters doesn't break the join; Persian peh joins.
        let shaped: Vec<char> = arabic_forms(&idx("بَب پ")).into_iter().map(|(c, _)| c).collect();
        assert_eq!(shaped, ['\u{FE91}', '\u{064E}', '\u{FE90}', ' ', '\u{FB56}']);
    }

    #[test]
    fn lines_come_in_the_order_egui_needs() {
        // One font for everything (Windows' Segoe UI): runs are whole lines.
        with(ARABIC | HEBREW | FORMS, || {
            // Pure Hebrew: egui turns the line around itself, so it's handed over as written.
            assert_eq!(line("שלום עולם", None), "שלום עולם");
            // Numbers and Latin inside: turned around twice, so they read right.
            assert_eq!(line("العدد 123", None), "العدد 321");
            // Ending in Arabic: turned around whole, so the shaper joins the Arabic.
            assert_eq!(line("English عربي", None), "عربي hsilgnE");
            // Latin at both ends: egui keeps the line left to right; the Arabic comes visual and
            // joined.
            assert_eq!(line("a عربي b", None), "a \u{FEF2}\u{FE91}\u{FEAE}\u{FECB} b");
            assert_eq!(line("plain", None), "plain");
        });
        // Arabic from a second font: runs split at the script; each Arabic run is handed over
        // as written (egui turns it), the runs in visual order.
        with(0, || {
            assert_eq!(line("مرحبا بالعالم", None), "بالعالم مرحبا");
            assert_eq!(line("Hi مرحبا بالعالم!", None), "Hi بالعالم مرحبا!");
        });
        // Brackets mirror in right-to-left text.
        with(ARABIC | HEBREW | FORMS, || {
            // (The shaper mirrors them back in the run it turns around.)
            assert_eq!(line("(שלום)", None), "(שלום)");
            assert_eq!(line("a (שלום) b", None), "a (םולש) b");
        });
    }
}

#[cfg(test)]
mod layout_tests {
    #[test]
    fn wrapped_paragraphs_keep_every_word() {
        let ctx = egui::Context::default();
        let text = "مرحبًا Riley،

أرفقنا الترجمة العربية للفيلم القصير (النسخة 2). راجعنا التوقيت مع ملف shot_list_v2 وعدد الأسطر 128.

שלום! גם הכתוביות בעברית מוכנות.

English stays as it is.";
        let mut out = None;
        let mut frame = ctx.run_ui(egui::RawInput::default(), |ui| {
            let mut j = egui::text::LayoutJob::simple(text.to_string(), egui::FontId::proportional(14.5), egui::Color32::WHITE, 300.0);
            j.wrap.max_width = 300.0;
            out = Some(ui.fonts_mut(|f| super::job(f, j)));
        });
        frame.textures_delta.clear();
        let out = out.unwrap();
        // Every letter is there, lines and all.
        let letters = |s: &str| {
            let mut v: Vec<char> = s.chars().filter(|c| !c.is_whitespace()).collect();
            v.sort_unstable();
            v
        };
        assert_eq!(letters(&out.text), letters(text));
        assert!(out.text.lines().count() > text.lines().count(), "the long paragraph wraps: {:?}", out.text);
        assert!(out.text.ends_with("English stays as it is."));
        // Right-to-left lines start with space before them (against the right edge).
        assert!(out.sections.iter().filter(|s| s.leading_space > 0.0).count() >= 3, "{:?}", out.sections);
    }
}
