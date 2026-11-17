//! What a laid-out message's parts take across, as CSS 2.1's automatic table layout (§17.5.2.2)
//! measures them: the least a part can be without its words or pictures spilling out
//! (min-content, its widest word or picture) and what it takes with nothing wrapped
//! (max-content, its widest line). Text is measured with the fonts it's drawn in. A table's
//! columns, and a table that sets no width, are sized from these ([`super::cell_widths`]).

use std::collections::{HashMap, VecDeque};

use mime::layout::{Block, Cell, Table, Width};

use super::*;

/// A part's min-content and max-content widths, in points.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Fit {
    pub(crate) min: f32,
    pub(crate) max: f32,
}

/// A table measured: each of its columns (from the cells that span one alone), and the whole of
/// it with its room. `None` where something has no size to measure (a picture that sets none, a
/// width that's a share of the room): that part's room is shared out as before.
#[derive(Clone, Debug, Default)]
pub(crate) struct Measured {
    pub(crate) columns: Vec<Option<Fit>>,
    pub(crate) whole: Option<Fit>,
}

/// Words measured at most for one table; past them it's left unmeasured (and shared out as
/// before), so a huge message costs no more than this to draw the first time.
const MAX_WORDS: usize = 20_000;
/// Tables whose measures are kept, the oldest let go past it.
const KEPT: usize = 2_048;

/// The measures kept between frames (measured once, the first time a table is drawn, or with
/// the table it's in).
#[derive(Clone, Default)]
struct Kept {
    each: HashMap<egui::Id, Entry>,
    order: VecDeque<egui::Id>,
}

/// A table's measures as kept, and the pictures that were still coming when it was measured
/// (the first of each block of text that has one, whose words after it weren't measured): kept
/// until one of those comes.
#[derive(Clone)]
struct Entry {
    measured: Measured,
    coming: Vec<mime::Span>,
}

impl Kept {
    fn keep(&mut self, id: egui::Id, entry: Entry) {
        if let Some(e) = self.each.get_mut(&id) {
            *e = entry;
            return;
        }
        while self.order.len() >= KEPT {
            if let Some(old) = self.order.pop_front() {
                self.each.remove(&old);
            }
        }
        self.order.push_back(id);
        self.each.insert(id, entry);
    }
}

/// How wide a picture that sets no width of its own shows.
pub(crate) enum PictureWidth {
    /// This wide (it's here).
    Is(f32),
    /// Its description in brackets (it can't be shown).
    Alt,
    /// Not known yet (it's coming).
    Coming,
}

/// [`measured_with`], no picture's own size known, the table in no other.
#[cfg(test)]
pub(crate) fn measured(ui: &Ui, key: &MessageKey, t: &Table) -> Measured {
    measured_with(ui, key, None, t, false, &mut |_| PictureWidth::Coming)
}

/// What tells the measures of a table that's in no other, and of the tables inside it, apart from
/// every other table's: all the outer one holds and the fonts it's measured in, hashed once, and
/// each table's number in it. (Every table hashed all it holds on every frame, so what's in a
/// table ten tables down was hashed eleven times a frame.) Made as the outer table is drawn and
/// used only while it is: the tables inside are found by where they are, which is theirs alone
/// for as long as the outer one is held.
#[derive(Default)]
pub(crate) struct Held {
    all: u64,
    /// Where each table is, and its number: the outer one's 0, the others' as they come in it.
    numbers: Vec<(usize, u32)>,
}

impl Held {
    pub(crate) fn of(ui: &Ui, t: &Table) -> Held {
        use std::hash::{Hash, Hasher};
        let mut all = Quick(fonts_stamp(ui));
        t.hash(&mut all);
        let mut numbers = Vec::new();
        number(t, 0, &mut numbers);
        numbers.sort_unstable();
        Held { all: all.finish(), numbers }
    }

    /// `t`'s number, when it's the outer table or one inside it.
    fn number(&self, t: &Table) -> Option<u32> {
        let at = std::ptr::from_ref(t).addr();
        self.numbers.binary_search_by_key(&at, |(a, _)| *a).ok().and_then(|i| self.numbers.get(i)).map(|(_, n)| *n)
    }

    /// Whether `t` is the table this was made of, or one inside it.
    #[cfg(test)]
    pub(crate) fn has(&self, t: &Table) -> bool {
        self.number(t).is_some()
    }

    /// What `t`'s measures are kept under, with `key` and `allowed`, whether its web pictures
    /// may show. (A table's place in memory and its shape told it apart at first: one laid out
    /// again where another of the same shape had been took that one's measures.)
    fn id(&self, key: &MessageKey, t: &Table, allowed: bool) -> Option<egui::Id> {
        self.number(t).map(|n| egui::Id::unique(("table-measures", key, self.all, n, allowed)))
    }
}

/// `t` and the tables inside it numbered in the order they're in, as deep as they're drawn.
fn number(t: &Table, depth: usize, out: &mut Vec<(usize, u32)>) {
    out.push((std::ptr::from_ref(t).addr(), u32::try_from(out.len()).unwrap_or(u32::MAX)));
    if depth >= MAX_NESTED {
        return;
    }
    for c in t.rows.iter().flatten() {
        for b in c.blocks.iter().chain(c.overlays.iter().flat_map(|o| &o.blocks)) {
            if let Block::Table(inside) = b {
                number(inside, depth + 1, out);
            }
        }
    }
}

/// What tells the fonts text is laid out in apart from those before them: each font's name and
/// face, where its file's bytes are and how many, and each family's fonts in their order. What's
/// measured in them and kept between frames is kept by it too: fonts installed again (another
/// language's, one that came for Chinese) measure it again.
pub(crate) fn fonts_stamp(ui: &Ui) -> u64 {
    use std::hash::{Hash, Hasher};
    ui.fonts(|f| {
        let fonts = f.definitions();
        let mut h = Quick(0);
        for (name, data) in &fonts.font_data {
            let bytes: &[u8] = (*data.font).as_ref();
            (name, data.index, bytes.as_ptr().addr(), bytes.len()).hash(&mut h);
        }
        fonts.families.hash(&mut h);
        h.finish()
    })
}

/// A quick hash of what a table holds, eight bytes at a time (as rustc's FxHasher): SipHash made
/// it a tenth of a newsletter's frame.
struct Quick(u64);

#[cfg(test)]
thread_local! {
    /// Words hashed on this thread, eight bytes each.
    pub(crate) static HASHED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

impl Quick {
    fn add(&mut self, word: u64) {
        #[cfg(test)]
        HASHED.with(|n| n.set(n.get().saturating_add(1)));
        self.0 = (self.0.rotate_left(5) ^ word).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
}

impl std::hash::Hasher for Quick {
    fn write(&mut self, bytes: &[u8]) {
        let (words, rest) = bytes.as_chunks::<8>();
        for word in words {
            self.add(u64::from_le_bytes(*word));
        }
        if !rest.is_empty() {
            let mut word = [0u8; 8];
            if let Some(w) = word.get_mut(..rest.len()) {
                w.copy_from_slice(rest);
            }
            self.add(u64::from_le_bytes(word));
        }
    }

    fn write_u8(&mut self, n: u8) {
        self.add(u64::from(n));
    }

    fn write_u16(&mut self, n: u16) {
        self.add(u64::from(n));
    }

    fn write_u32(&mut self, n: u32) {
        self.add(u64::from(n));
    }

    fn write_u64(&mut self, n: u64) {
        self.add(n);
    }

    fn write_usize(&mut self, n: usize) {
        self.add(n as u64);
    }

    fn finish(&self) -> u64 {
        self.0
    }
}

/// `t`'s measures, from those kept or measured now; `held` what the table it's inside holds
/// ([`Held`]; `None` for one that's in no other), `picture` how wide one that sets no width is.
/// Measured again once a picture that was coming has come (or can't be shown); the tables inside
/// it that are measured with it are kept too, not measured again when drawn.
pub(crate) fn measured_with(
    ui: &Ui,
    key: &MessageKey,
    held: Option<&Held>,
    t: &Table,
    allowed: bool,
    picture: &mut dyn FnMut(&mime::Span) -> PictureWidth,
) -> Measured {
    // One that isn't of the table it's said to be in is held alone.
    let alone;
    let (held, id) = match held.and_then(|h| h.id(key, t, allowed).map(|id| (h, id))) {
        Some(found) => found,
        None => {
            alone = Held::of(ui, t);
            (&alone, egui::Id::unique(("table-measures", key, alone.all, 0u32, allowed)))
        }
    };
    let store = egui::Id::unique("mailcraft-table-measures");
    if let Some(e) = ui.ctx().data_mut(|d| d.get_temp_mut_or_default::<Kept>(store).each.get(&id).cloned())
        && e.coming.iter().all(|sp| matches!(picture(sp), PictureWidth::Coming))
    {
        return e.measured;
    }
    let mut measure = Measure { ui, key, held, allowed, words: 0, picture, coming: Vec::new(), cut: false, inside: Vec::new() };
    let measured = measure.table(t, 0);
    let (coming, inside) = (measure.coming, measure.inside);
    #[cfg(test)]
    MEASURED.with(|n| n.set(n.get().saturating_add(1)));
    ui.ctx().data_mut(|d| {
        let kept = d.get_temp_mut_or_default::<Kept>(store);
        for (id, e) in inside {
            kept.keep(id, e);
        }
        kept.keep(id, Entry { measured: measured.clone(), coming });
    });
    measured
}

#[cfg(test)]
thread_local! {
    /// Tables measured on this thread (as drawn: those inside one are measured with it).
    pub(crate) static MEASURED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

struct Measure<'a> {
    ui: &'a Ui,
    key: &'a MessageKey,
    /// What the table that's in no other holds: those inside it are kept by it.
    held: &'a Held,
    allowed: bool,
    /// Words measured so far.
    words: usize,
    picture: &'a mut dyn FnMut(&mime::Span) -> PictureWidth,
    /// The pictures still to come, the first of each block of text that has one.
    coming: Vec<mime::Span>,
    /// Something was left unmeasured for a limit (words, tables nested) that a table inside,
    /// measured alone, might not have reached.
    cut: bool,
    /// The tables inside, measured whole on the way: kept under their own ids.
    inside: Vec<(egui::Id, Entry)>,
}

impl Measure<'_> {
    /// How wide `text` is in `font`, unwrapped.
    fn width(&mut self, text: &str, font: egui::FontId) -> Option<f32> {
        self.words = self.words.saturating_add(1);
        if self.words > MAX_WORDS {
            self.cut = true;
            return None;
        }
        Some(self.ui.fonts_mut(|f| f.layout_no_wrap(text.to_string(), font, Color32::WHITE).size().x))
    }

    /// A table inside, measured as it would be alone unless a limit cut it short (it then
    /// would be measured again, alone, when drawn).
    fn inside(&mut self, t: &Table, depth: usize) -> Measured {
        let (from, cut) = (self.coming.len(), std::mem::take(&mut self.cut));
        let measured = self.table(t, depth);
        if !self.cut
            && let Some(id) = self.held.id(self.key, t, self.allowed)
        {
            let coming = self.coming.get(from..).unwrap_or_default().to_vec();
            self.inside.push((id, Entry { measured: measured.clone(), coming }));
        }
        self.cut |= cut;
        measured
    }

    /// Formatted lines: the widest word or picture, and the widest line.
    fn text(&mut self, lines: &[Vec<mime::Span>]) -> Option<Fit> {
        let mut fit = Fit::default();
        for line in lines {
            let mut across = 0.0f32;
            let mut words = false;
            for sp in line {
                if sp.image.is_some() {
                    let w = match sp.image_size.and_then(|z| z.width) {
                        Some(Width::Px(px)) => px as f32,
                        // A share of its room.
                        Some(Width::Percent(_)) => return None,
                        None => match (self.picture)(sp) {
                            PictureWidth::Is(w) => w,
                            PictureWidth::Alt => self.width(&held_back_words(&sp.text), span_font(sp))?,
                            PictureWidth::Coming => {
                                self.coming.push(sp.clone());
                                return None;
                            }
                        },
                    };
                    fit.min = fit.min.max(w);
                    across += w;
                    continue;
                }
                let font = span_font(sp);
                for word in sp.text.split_whitespace() {
                    fit.min = fit.min.max(self.width(word, font.clone())?);
                }
                across += self.width(&sp.text, font)?;
                words = words || !sp.text.trim().is_empty();
            }
            // A point more than its words, so that a line measured to the edge doesn't wrap.
            fit.max = fit.max.max(across + if words { 1.0 } else { 0.0 });
        }
        Some(fit)
    }

    /// Blocks one under another: the widest of each measure.
    fn blocks(&mut self, blocks: &[Block], depth: usize) -> Option<Fit> {
        let mut fit = Fit::default();
        for b in blocks {
            let f = match b {
                Block::Text(lines) => self.text(lines)?,
                Block::Table(t) => match t.width {
                    Some(Width::Px(px)) => Fit { min: px as f32, max: px as f32 },
                    Some(Width::Percent(_)) => return None,
                    None if depth < MAX_NESTED => self.inside(t, depth + 1).whole?,
                    None => {
                        self.cut = true;
                        return None;
                    }
                },
            };
            fit.min = fit.min.max(f.min);
            fit.max = fit.max.max(f.max);
        }
        Some(fit)
    }

    /// A cell with its room inside (its own, or its table's `padding`); a width of pixels it sets
    /// is what it takes where there's room, unless its content can't be that narrow, and it can be
    /// as narrow as its content where there isn't, as a browser shrinks it (#152).
    fn cell(&mut self, c: &Cell, padding: f32, depth: usize) -> Option<Fit> {
        let pad = c.padding.map_or([padding; 4], |p| p.map(f32::from));
        let inner = self.blocks(&c.blocks, depth)?;
        let fit = Fit { min: inner.min + pad[1] + pad[3], max: inner.max + pad[1] + pad[3] };
        match c.width {
            Some(Width::Px(px)) => Some(Fit { min: fit.min, max: fit.min.max(px as f32) }),
            Some(Width::Percent(_)) => None,
            None => Some(fit),
        }
    }

    /// A table's columns (each the widest of its cells that span it alone) and the whole of it,
    /// its columns side by side with the room between and around them.
    fn table(&mut self, t: &Table, depth: usize) -> Measured {
        let padding = f32::from(t.padding.unwrap_or(0));
        let spacing = f32::from(t.spacing.unwrap_or(0));
        let mut columns: Vec<Option<Fit>> = Vec::new();
        let mut seen: Vec<bool> = Vec::new();
        let mut rows_fit = Some(Fit::default());
        for row in &t.rows {
            let mut col = 0usize;
            let mut row_fit = Some(Fit { min: spacing * (row.len() + 1) as f32, max: spacing * (row.len() + 1) as f32 });
            for c in row {
                let span = usize::from(c.span.max(1));
                let fit = self.cell(c, padding, depth);
                row_fit = row_fit.zip(fit).map(|(r, f)| Fit { min: r.min + f.min, max: r.max + f.max });
                if span == 1 && col < MAX_COLUMNS {
                    if columns.len() <= col {
                        columns.resize(col + 1, Some(Fit::default()));
                        seen.resize(col + 1, false);
                    }
                    if let Some(slot) = columns.get_mut(col) {
                        *slot = slot.zip(fit).map(|(a, f)| Fit { min: a.min.max(f.min), max: a.max.max(f.max) });
                    }
                    if let Some(s) = seen.get_mut(col) {
                        *s = true;
                    }
                }
                col = col.saturating_add(span);
            }
            rows_fit = rows_fit.zip(row_fit).map(|(a, r)| Fit { min: a.min.max(r.min), max: a.max.max(r.max) });
        }
        // A column only spanned has nothing of its own to measure.
        for (c, s) in columns.iter_mut().zip(&seen) {
            if !s {
                *c = None;
            }
        }
        // The columns side by side, each as wide as its widest cell, when they're all measured.
        let by_columns =
            columns.iter().try_fold(Fit { min: spacing * (columns.len() + 1) as f32, max: spacing * (columns.len() + 1) as f32 }, |a, c| {
                c.map(|c| Fit { min: a.min + c.min, max: a.max + c.max })
            });
        let whole = match (by_columns, rows_fit) {
            (Some(c), Some(r)) => Some(Fit { min: c.min.max(r.min), max: c.max.max(r.max) }),
            _ => None,
        };
        Measured { columns, whole }
    }
}
