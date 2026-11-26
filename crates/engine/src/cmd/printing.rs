//! The open message as a PDF (Print, Save as PDF): as it's laid out on screen, its tables in
//! columns with their backgrounds and lines, its pictures where they've been shown, its links
//! to click.

use std::collections::HashMap;

use mailcraft_mime::layout;
use mailcraft_model::MessageKey;
use mailcraft_print as print;

use super::*;

/// CSS pixels to points.
const PT: f32 = 0.75;
/// Pictures embedded in one PDF, at most, and their pixels in all.
const MAX_PICTURES: usize = 80;
const MAX_PIXELS: u64 = 40_000_000;
/// The message's own pictures decoded at once, each at most and all together at most this
/// (one past either is read on its own when it's wanted).
const PICTURE_BYTES: usize = 16 << 20;
const PICTURES_BYTES: usize = 64 << 20;
/// Tables nested deeper than this are printed without their boxes.
const MAX_DEPTH: usize = 24;
const LINK: [u8; 3] = [0x0B, 0x57, 0xD0];

impl Session {
    /// The open message as a PDF: subject, header and the names of its attachments, then the
    /// message as it's read on screen. Its pictures from the web are in it when they've been
    /// shown (nothing is fetched for a PDF); a picture not shown is its description.
    pub fn message_pdf(&self) -> Result<Vec<u8>, CmdError> {
        let r = self.reading.as_ref().ok_or_else(|| CmdError::Disabled("open a message first".into()))?;
        let m = r.parsed.as_ref().ok_or_else(|| CmdError::Disabled("the message is still downloading".into()))?;
        // In the window's language, the date as MailCraft writes it there (issue #291).
        let w = &self.mail_words.print;
        let mut header = vec![(w.from.clone(), m.from.to_string())];
        if !m.to.is_empty() {
            header.push((w.to.clone(), mailcraft_model::addr::format_list(&m.to)));
        }
        if !m.cc.is_empty() {
            header.push((w.cc.clone(), mailcraft_model::addr::format_list(&m.cc)));
        }
        if m.date > 0 {
            header.push((w.date.clone(), self.mail_words.time(m.date, &self.clock)));
        }
        let files: Vec<String> = m.attachments.iter().filter(|a| !a.inline).map(|a| a.name.clone()).collect();
        if !files.is_empty() {
            header.push((w.attachments.clone(), files.join(", ")));
        }
        let mut printing =
            Printing { s: self, key: &r.key, p: m, raw: None, own: None, pictures: Vec::new(), by_src: HashMap::new(), pixels: 0, not_loaded: false };
        // A formatted message: its layout, or (one with no columns) its lines.
        let blocks = if !m.layout.is_empty() {
            printing.blocks(&m.layout, 0, false)
        } else if !m.rich.is_empty() {
            vec![print::Block::Text(printing.runs(&m.rich, false))]
        } else {
            Vec::new()
        };
        let note = if printing.not_loaded { w.not_loaded.clone() } else { String::new() };
        let doc = print::Doc {
            title: m.subject.clone(),
            header,
            note,
            cut: w.cut.clone(),
            body: m.text.clone(),
            rich: Vec::new(),
            blocks,
            pictures: printing.pictures,
        };
        let fallbacks: Vec<&[u8]> = self.print_fallbacks.iter().map(|f| (**f).as_ref()).collect();
        let fonts = self.print_fonts.as_ref().map(|(r, b)| print::Fonts { regular: r, bold: b.as_deref(), fallbacks: &fallbacks });
        Ok(print::pdf(&doc, fonts))
    }
}

/// A message being made into the PDF's blocks, and the pictures found for it so far.
struct Printing<'a> {
    s: &'a Session,
    key: &'a MessageKey,
    p: &'a mailcraft_mime::Parsed,
    /// The message's source, read once a picture of its own is wanted (`Some(None)`: it isn't
    /// to be had).
    raw: Option<Option<Vec<u8>>>,
    /// The message's pictures, by attachment, decoded from one parse when the first is wanted
    /// (it was a parse of the whole message per picture).
    own: Option<HashMap<usize, Vec<u8>>>,
    pictures: Vec<print::Picture>,
    /// Each picture address looked up: its place in `pictures`, or that there's none.
    by_src: HashMap<String, Option<usize>>,
    pixels: u64,
    /// A picture from the web is left out: not loaded (a tracking one aside, never loaded).
    not_loaded: bool,
}

impl Printing<'_> {
    /// The picture at `src`, when it can be shown: one the message carries, or one from the web
    /// that was loaded for it (the reader asked, or it loads for this sender).
    fn picture(&mut self, src: &str) -> Option<usize> {
        if let Some(known) = self.by_src.get(src) {
            return *known;
        }
        let found = self.find(src);
        self.by_src.insert(src.to_string(), found);
        found
    }

    fn find(&mut self, src: &str) -> Option<usize> {
        if self.pictures.len() >= MAX_PICTURES || self.pixels >= MAX_PIXELS {
            return None;
        }
        let lower = src.trim().to_ascii_lowercase();
        let bytes: Vec<u8> = if lower.starts_with("https://") || lower.starts_with("http://") {
            if self.p.trackers.pictures.iter().any(|t| t == src) {
                return None;
            }
            match self.s.pictures.get(src).filter(|_| self.s.pictures_allowed()) {
                Some(crate::Picture::Ready(b)) => b.to_vec(),
                _ => {
                    self.not_loaded = true;
                    return None;
                }
            }
        } else {
            let index = mailcraft_mime::picture_attachment(self.p, src)?;
            let (s, key, p) = (self.s, self.key, self.p);
            let raw = self.raw.get_or_insert_with(|| s.readable_raw(key)).as_ref()?;
            let own = self.own.get_or_insert_with(|| {
                let pictures: Vec<usize> = p.attachments.iter().filter(|a| a.mime.starts_with("image/")).map(|a| a.index).collect();
                mailcraft_mime::attachments_data(raw, &pictures, PICTURE_BYTES, PICTURES_BYTES).into_iter().collect()
            });
            match own.get(&index) {
                Some(b) => b.clone(),
                None => mailcraft_mime::attachment_data(raw, index)?,
            }
        };
        let picture = print::Picture::decode(&bytes)?;
        self.pixels = self.pixels.saturating_add(u64::from(picture.width) * u64::from(picture.height));
        self.pictures.push(picture);
        Some(self.pictures.len() - 1)
    }

    /// How big a picture is shown, in points: as the message says, else as it is.
    fn shown(&self, index: usize, size: Option<mailcraft_mime::ImageSize>) -> (f32, f32) {
        let (pw, ph) = self.pictures.get(index).map_or((1.0, 1.0), |p| (p.width.max(1) as f32, p.height.max(1) as f32));
        let size = size.unwrap_or_default();
        let width = match size.width {
            Some(layout::Width::Px(w)) => Some(w as f32),
            _ => None,
        };
        let (mut w, mut h) = match (width, size.height.map(|h| h as f32)) {
            (Some(w), Some(h)) => (w, h),
            (Some(w), None) => (w, w * ph / pw),
            (None, Some(h)) => (h * pw / ph, h),
            (None, None) => (pw, ph),
        };
        if let Some(most) = size.max_width.map(|m| m as f32).filter(|m| *m > 0.0 && *m < w) {
            (w, h) = (most, h * most / w);
        }
        ((w * PT).clamp(1.0, 4000.0), (h * PT).clamp(1.0, 4000.0))
    }

    /// A formatted message's lines as the PDF prints them (issue #72: it printed the plain
    /// text, its formatting gone). A picture that can't be shown is its description.
    /// `on_background`: they're in a box with a background of its own, which is printed, so a
    /// light colour written on it is kept.
    fn runs(&mut self, rich: &mailcraft_mime::RichText, on_background: bool) -> Vec<Vec<print::Run>> {
        let mut out = Vec::with_capacity(rich.len());
        for line in rich {
            let mut runs = Vec::with_capacity(line.len());
            for s in line {
                let heading = match s.heading {
                    0 => None,
                    1 => Some(20.0),
                    2 => Some(17.0),
                    3 => Some(14.5),
                    _ => Some(12.5),
                };
                let mut run = print::Run {
                    // Struck-through text comes with a stroke over each letter for where no line
                    // can be drawn: here one is.
                    text: s.text.replace('\u{336}', ""),
                    bold: s.bold || heading.is_some(),
                    italic: s.italic,
                    underline: s.link.is_some(),
                    strike: s.strike,
                    size: heading.or(s.size.map(|px| f32::from(px) * PT)),
                    color: s.color.or(s.link.as_ref().map(|_| LINK)),
                    background: s.background,
                    on_background,
                    link: s.link.clone(),
                    picture: None,
                };
                if let Some(src) = &s.image {
                    match self.picture(src) {
                        Some(index) => {
                            let (w, h) = self.shown(index, s.image_size);
                            run.picture = Some((index, w, h));
                            run.text.clear();
                        }
                        // Not shown: the room it says it takes, as on screen (a small one's
                        // description came out a letter to a line), else what it's of.
                        None => match held_back(s.image_size) {
                            Some((w, h)) => {
                                run.picture = Some((print::HELD_BACK, w, h));
                                run.text.clear();
                            }
                            None if s.text.trim().is_empty() => continue,
                            None => {
                                let alt = s.text.trim();
                                run.text = match alt.char_indices().nth(48) {
                                    Some((i, _)) => format!("[{}…]", alt.get(..i).unwrap_or(alt).trim_end()),
                                    None => format!("[{alt}]"),
                                };
                                run.underline = false;
                            }
                        },
                    }
                }
                runs.push(run);
            }
            out.push(runs);
        }
        out
    }

    fn blocks(&mut self, blocks: &[layout::Block], depth: usize, on_background: bool) -> Vec<print::Block> {
        let mut out = Vec::with_capacity(blocks.len());
        for b in blocks {
            match b {
                layout::Block::Text(lines) => out.push(print::Block::Text(self.runs(lines, on_background))),
                layout::Block::Table(t) if depth < MAX_DEPTH => out.push(print::Block::Table(self.table(t, depth, on_background))),
                // Too deep to box: what it says, one cell after another, on the paper.
                layout::Block::Table(t) => {
                    for cell in t.rows.iter().flatten() {
                        out.extend(self.blocks(&cell.blocks, depth, false));
                    }
                }
            }
        }
        out
    }

    fn table(&mut self, t: &layout::Table, depth: usize, on_background: bool) -> print::Table {
        let pt = |px: u16| f32::from(px) * PT;
        let on = on_background || t.background.is_some();
        print::Table {
            rows: t.rows.iter().map(|row| row.iter().map(|c| self.cell(c, depth, on)).collect()).collect(),
            width: t.width.map(width),
            fit: t.fit,
            align: align(t.align),
            background: t.background,
            spacing: t.spacing.map_or(0.0, pt),
            padding: t.padding.map_or(0.0, pt),
            radius: t.radius.map_or(0.0, pt),
            border: t.border.map(line),
        }
    }

    fn cell(&mut self, c: &layout::Cell, depth: usize, on_background: bool) -> print::Cell {
        let pt = |px: u16| f32::from(px) * PT;
        let on = on_background || c.background.is_some();
        let mut blocks = self.blocks(&c.blocks, depth + 1, on);
        // What's placed over it goes under it: the page has one layer.
        for over in &c.overlays {
            blocks.extend(self.blocks(&over.blocks, depth + 1, on));
        }
        print::Cell {
            blocks,
            width: c.width.map(width),
            background: c.background,
            align: align(c.align),
            span: c.span,
            padding: c.padding.map(|p| [pt(p[0]), pt(p[1]), pt(p[2]), pt(p[3])]),
            valign: match c.valign {
                layout::VAlign::Top => print::VAlign::Top,
                layout::VAlign::Middle => print::VAlign::Middle,
                layout::VAlign::Bottom => print::VAlign::Bottom,
            },
            height: c.height.map(pt),
            radius: c.radius.map_or(0.0, pt),
            borders: c.borders.map(|b| b.map(line)),
        }
    }
}

/// The room a picture that isn't shown takes, in points, when it says its width in pixels (and
/// its height, or it's small enough to be an icon: square).
fn held_back(size: Option<mailcraft_mime::ImageSize>) -> Option<(f32, f32)> {
    let z = size?;
    let Some(layout::Width::Px(w)) = z.width else { return None };
    let w = w.min(4000) as f32;
    let h = match z.height {
        Some(h) => h.min(4000) as f32,
        None if w <= 64.0 => w,
        None => return None,
    };
    (w >= 0.5 && h >= 0.5).then_some((w * PT, h * PT))
}

fn width(w: layout::Width) -> print::Width {
    match w {
        layout::Width::Px(px) => print::Width::Pt(px.min(100_000) as f32 * PT),
        layout::Width::Percent(p) => print::Width::Percent(f32::from(p)),
    }
}

fn align(a: layout::Align) -> print::Align {
    match a {
        layout::Align::Left => print::Align::Left,
        layout::Align::Center => print::Align::Center,
        layout::Align::Right => print::Align::Right,
    }
}

fn line(l: layout::Line) -> print::Line {
    print::Line { width: f32::from(l.width) * PT, color: l.color }
}
