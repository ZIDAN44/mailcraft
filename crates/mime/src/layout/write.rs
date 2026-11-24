//! A laid-out message written out as HTML again: its tables, rows and cells with their widths,
//! colours, pictures behind them and alignment, and the boxes placed over cells, around text
//! the caller writes (the original under a reply, its cells edited in place, as it goes out).

use super::*;
use crate::css::hex;
use crate::html::escaped;

/// `blocks` as HTML, the `n`th block of text ([`text_blocks`]' order) as `text(n)` writes it.
pub fn to_html(blocks: &[Block], text: &mut dyn FnMut(usize) -> String) -> String {
    let mut out = String::new();
    let mut n = 0usize;
    write(blocks, text, &mut n, &mut out, 0);
    out
}

fn width(w: Option<Width>) -> String {
    match w {
        Some(Width::Px(px)) => format!(" width=\"{px}\""),
        Some(Width::Percent(pc)) => format!(" width=\"{pc}%\""),
        None => String::new(),
    }
}

fn align(a: Align) -> &'static str {
    match a {
        Align::Left => "",
        Align::Center => " align=\"center\"",
        Align::Right => " align=\"right\"",
    }
}

fn write(blocks: &[Block], text: &mut dyn FnMut(usize) -> String, n: &mut usize, out: &mut String, depth: usize) {
    if depth > MAX_DEPTH * 2 {
        return;
    }
    for b in blocks {
        match b {
            Block::Text(_) => {
                out.push_str(&text(*n));
                *n += 1;
            }
            Block::Table(t) => {
                let centred = if t.align == Align::Center { ";margin:0 auto" } else { "" };
                let around = t.border.as_ref().map(|l| format!(";border:{}px solid {}", l.width, hex(l.color))).unwrap_or_default();
                out.push_str(&format!(
                    "<table cellpadding=\"0\" cellspacing=\"0\" border=\"0\"{}{}{}{} style=\"border-collapse:collapse{centred}{around}\">",
                    width(t.width),
                    align(t.align),
                    t.background.map(|c| format!(" bgcolor=\"{}\"", hex(c))).unwrap_or_default(),
                    t.background_image.as_deref().map(|s| format!(" background=\"{}\"", escaped(s))).unwrap_or_default(),
                ));
                // Written collapsed: a line two cells share once, as it's read again.
                let shared = super::collapse::shared_lines(&t.rows);
                for (row, row_lines) in t.rows.iter().zip(&shared) {
                    out.push_str("<tr>");
                    for (c, borders) in row.iter().zip(row_lines) {
                        let span = if c.span > 1 { format!(" colspan=\"{}\"", c.span) } else { String::new() };
                        // Its room inside and the lines along its sides, as it has them (a table
                        // pasted from Excel, issue #268, keeps its cells' lines).
                        let room = match c.padding {
                            None => "padding:6px".to_string(),
                            Some([t, r, b, l]) if t == r && r == b && b == l => format!("padding:{t}px"),
                            Some([t, r, b, l]) => format!("padding:{t}px {r}px {b}px {l}px"),
                        };
                        let lines: String = ["top", "right", "bottom", "left"]
                            .iter()
                            .zip(borders)
                            .filter_map(|(side, l)| l.as_ref().map(|l| format!(";border-{side}:{}px solid {}", l.width, hex(l.color))))
                            .collect();
                        out.push_str(&format!(
                            "<td valign=\"top\"{}{span}{}{}{} style=\"{room}{lines}{}\">",
                            width(c.width),
                            c.background.map(|c| format!(" bgcolor=\"{}\"", hex(c))).unwrap_or_default(),
                            align(c.align),
                            c.background_image.as_deref().map(|s| format!(" background=\"{}\"", escaped(s))).unwrap_or_default(),
                            c.radius.map(|r| format!(";border-radius:{r}px")).unwrap_or_default(),
                        ));
                        // Boxes placed over the cell's content: in a box of its own they're placed in.
                        if !c.overlays.is_empty() {
                            out.push_str("<div style=\"position:relative\">");
                        }
                        write(&c.blocks, text, n, out, depth + 1);
                        for o in &c.overlays {
                            let x = if o.from_right { format!("right:{}px", o.x) } else { format!("left:{}px", o.x) };
                            let y = if o.from_bottom { format!("bottom:{}px", o.y) } else { format!("top:{}px", o.y) };
                            let w = match o.width {
                                Some(Width::Px(px)) => format!(";width:{px}px"),
                                Some(Width::Percent(pc)) => format!(";width:{pc}%"),
                                None => String::new(),
                            };
                            let bg = o.background.map(|c| format!(";background-color:{}", hex(c))).unwrap_or_default();
                            out.push_str(&format!("<div style=\"position:absolute;{x};{y}{w}{bg};padding:4px\">"));
                            write(&o.blocks, text, n, out, depth + 1);
                            out.push_str("</div>");
                        }
                        if !c.overlays.is_empty() {
                            out.push_str("</div>");
                        }
                        out.push_str("</td>");
                    }
                    out.push_str("</tr>");
                }
                out.push_str("</table>");
            }
        }
    }
}
