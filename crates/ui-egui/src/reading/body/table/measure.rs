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
/// Tables nested deeper than this inside the one measured are left unmeasured.
const MAX_NESTED: usize = 16;
/// Columns measured at most.
const MAX_COLUMNS: usize = 64;
/// Tables whose measures are kept, the oldest let go past it.
const KEPT: usize = 2_048;

/// The measures kept between frames (measured once, the first time a table is drawn).
#[derive(Clone, Default)]
struct Kept {
    each: HashMap<egui::Id, Measured>,
    order: VecDeque<egui::Id>,
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

/// [`measured_with`], no picture's own size known.
#[cfg(test)]
pub(crate) fn measured(ui: &Ui, key: &MessageKey, t: &Table) -> Measured {
    measured_with(ui, key, t, false, &mut |_| PictureWidth::Coming)
}

/// `t`'s measures, from those kept or measured now. `key` with the table's place in memory and
/// its shape tells it apart from every other table drawn; `allowed` whether its web pictures may
/// show; `picture` how wide one that sets no width is. Measured again while a picture's coming.
pub(crate) fn measured_with(ui: &Ui, key: &MessageKey, t: &Table, allowed: bool, picture: &mut dyn FnMut(&mime::Span) -> PictureWidth) -> Measured {
    let shape: Vec<usize> = t.rows.iter().map(Vec::len).take(64).collect();
    let width = t.width.map(|w| match w {
        Width::Px(px) => (false, px),
        Width::Percent(pc) => (true, u32::from(pc)),
    });
    let id = egui::Id::unique(("table-measures", key, std::ptr::from_ref(t).addr(), t.rows.len(), shape, width, t.spacing, t.padding, allowed));
    let store = egui::Id::unique("mailcraft-table-measures");
    if let Some(m) = ui.ctx().data_mut(|d| d.get_temp_mut_or_default::<Kept>(store).each.get(&id).cloned()) {
        return m;
    }
    let mut measure = Measure { ui, words: 0, picture, coming: false };
    let m = measure.table(t, 0);
    if measure.coming {
        return m;
    }
    ui.ctx().data_mut(|d| {
        let kept = d.get_temp_mut_or_default::<Kept>(store);
        while kept.order.len() >= KEPT {
            if let Some(old) = kept.order.pop_front() {
                kept.each.remove(&old);
            }
        }
        kept.order.push_back(id);
        kept.each.insert(id, m.clone());
    });
    m
}

struct Measure<'a> {
    ui: &'a Ui,
    /// Words measured so far.
    words: usize,
    picture: &'a mut dyn FnMut(&mime::Span) -> PictureWidth,
    /// A picture's still to come: these measures aren't kept.
    coming: bool,
}

impl Measure<'_> {
    /// How wide `text` is in `font`, unwrapped.
    fn width(&mut self, text: &str, font: egui::FontId) -> Option<f32> {
        self.words = self.words.saturating_add(1);
        (self.words <= MAX_WORDS).then(|| self.ui.fonts_mut(|f| f.layout_no_wrap(text.to_string(), font, Color32::WHITE).size().x))
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
                                self.coming = true;
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
                    None if depth < MAX_NESTED => self.table(t, depth + 1).whole?,
                    None => return None,
                },
            };
            fit.min = fit.min.max(f.min);
            fit.max = fit.max.max(f.max);
        }
        Some(fit)
    }

    /// A cell with its room inside (its own, or its table's `padding`); a width of pixels it sets
    /// is its own, unless its content can't be that narrow.
    fn cell(&mut self, c: &Cell, padding: f32, depth: usize) -> Option<Fit> {
        let pad = c.padding.map_or([padding; 4], |p| p.map(f32::from));
        let inner = self.blocks(&c.blocks, depth)?;
        let fit = Fit { min: inner.min + pad[1] + pad[3], max: inner.max + pad[1] + pad[3] };
        match c.width {
            Some(Width::Px(px)) => {
                let w = fit.min.max(px as f32);
                Some(Fit { min: w, max: w })
            }
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
