//! A laid-out message written out as HTML again: its tables, rows and cells with their widths,
//! colours, pictures behind them and alignment, and the boxes placed over cells, around text
//! the caller writes (the original under a reply, its cells edited in place, as it goes out).

use super::*;

/// `blocks` as HTML, the `n`th block of text ([`text_blocks`]' order) as `text(n)` writes it.
pub fn to_html(blocks: &[Block], text: &mut dyn FnMut(usize) -> String) -> String {
    let mut out = String::new();
    let mut n = 0usize;
    write(blocks, text, &mut n, &mut out, 0);
    out
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;").replace('>', "&gt;")
}

fn hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
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
                out.push_str(&format!(
                    "<table cellpadding=\"0\" cellspacing=\"0\" border=\"0\"{}{}{}{} style=\"border-collapse:collapse{centred}\">",
                    width(t.width),
                    align(t.align),
                    t.background.map(|c| format!(" bgcolor=\"{}\"", hex(c))).unwrap_or_default(),
                    t.background_image.as_deref().map(|s| format!(" background=\"{}\"", esc(s))).unwrap_or_default(),
                ));
                for row in &t.rows {
                    out.push_str("<tr>");
                    for c in row {
                        let span = if c.span > 1 { format!(" colspan=\"{}\"", c.span) } else { String::new() };
                        out.push_str(&format!(
                            "<td valign=\"top\"{}{span}{}{}{} style=\"padding:6px{}\">",
                            width(c.width),
                            c.background.map(|c| format!(" bgcolor=\"{}\"", hex(c))).unwrap_or_default(),
                            align(c.align),
                            c.background_image.as_deref().map(|s| format!(" background=\"{}\"", esc(s))).unwrap_or_default(),
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
