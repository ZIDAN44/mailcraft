//! Tables pasted into a message being written (issue #268): from another program's HTML (cells
//! copied in Excel, a table copied from a web page or a message), each a table of MailCraft's own
//! writing, from its layout (its rows and cells, their widths, colours, lines and alignment, and
//! each cell's text with its formatting), which goes into the message as it is; and its text,
//! each row a line and its cells apart by tabs, which stands for it in the message's text and is
//! its plain text. Nothing of the pasted HTML goes out as it came: no scripts, styles, classes or
//! pictures.

use serde::{Deserialize, Serialize};

use crate::layout::{self, Block, Table};
use crate::rich::{self, Styles};

/// The class of the `<div>` a table goes out in: a draft opened again has its tables back.
pub const TABLE_CLASS: &str = "mailcraft-table";
/// Longest table code kept: a bigger one is pasted as its text.
pub const TABLE_CODE_MAX: usize = 1 << 20;
/// Most cells in one table: a bigger one is pasted as its text.
pub const TABLE_CELLS_MAX: usize = 20_000;

/// A table in a message being written: its HTML code, of MailCraft's own writing, and its text.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PastedTable {
    pub code: String,
    pub text: String,
}

/// A part of what was pasted, in its order.
#[derive(Clone, Debug, PartialEq)]
pub enum Pasted {
    /// Text, with its formatting.
    Text(String, Styles),
    Table(PastedTable),
}

/// What's pasted as HTML, in parts, when it holds a table (outside any other); `None` when it
/// holds none, and it's pasted as before.
pub fn pasted_tables(html: &str) -> Option<Vec<Pasted>> {
    let mut blocks = layout::html_layout(&layout::inline_css(html));
    if !blocks.iter().any(|b| matches!(b, Block::Table(_))) {
        return None;
    }
    // Pictures stay behind: the pasted HTML's are on the web or nowhere (`file:`).
    layout::each_span(&mut blocks, &mut |sp| sp.image = None);
    let mut out = Vec::new();
    for b in blocks {
        match b {
            Block::Text(t) => {
                let (text, styles) = rich::from_rich(&t);
                if !text.trim().is_empty() {
                    out.push(Pasted::Text(text, styles));
                }
            }
            Block::Table(t) => match table_of(t) {
                Ok(table) => out.push(Pasted::Table(table)),
                // Too big to keep as a table: its text, as it was pasted before.
                Err(text) if !text.trim().is_empty() => {
                    let styles = Styles::plain(&text);
                    out.push(Pasted::Text(text, styles));
                }
                Err(_) => {}
            },
        }
    }
    Some(out)
}

/// A table of MailCraft's writing from one laid out, with its text; its text alone (`Err`) when
/// it's too big to keep as a table.
fn table_of(mut t: Table) -> Result<PastedTable, String> {
    let text = text_of(&t);
    let cells: usize = t.rows.iter().map(Vec::len).sum();
    if cells == 0 || text.trim().is_empty() {
        return Err(String::new());
    }
    if cells > TABLE_CELLS_MAX {
        return Err(text);
    }
    no_pictures_behind(&mut t);
    let block = [Block::Table(t)];
    let texts: Vec<String> = layout::text_blocks(&block)
        .into_iter()
        .map(|(lines, align)| {
            let (text, styles) = layout::cell_text(lines, align);
            rich::to_html(&text, &styles)
        })
        .collect();
    let code = layout::to_html(&block, &mut |n| texts.get(n).cloned().unwrap_or_default());
    if code.len() > TABLE_CODE_MAX {
        return Err(text);
    }
    Ok(PastedTable { code, text })
}

/// No pictures behind the table or its cells: they're on the web, or nowhere.
fn no_pictures_behind(t: &mut Table) {
    t.background_image = None;
    for cell in t.rows.iter_mut().flatten() {
        cell.background_image = None;
        cell.overlays.clear();
        for b in &mut cell.blocks {
            if let Block::Table(inner) = b {
                no_pictures_behind(inner);
            }
        }
    }
}

/// A table's text: each row a line, its cells apart by tabs, a cell's own lines and tabs as
/// spaces.
fn text_of(t: &Table) -> String {
    let cell_text = |cell: &layout::Cell| -> String {
        let lines: Vec<String> = layout::text_blocks(&cell.blocks).into_iter().map(|(lines, _)| rich::from_rich(lines).0).collect();
        lines.join(" ").replace(['\n', '\t', '\r'], " ").trim().to_string()
    };
    t.rows.iter().map(|row| row.iter().map(cell_text).collect::<Vec<_>>().join("\t")).filter(|r| !r.trim().is_empty()).collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests;
