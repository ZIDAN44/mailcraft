//! Tables in a message being written (issue #268): pasted from another program (cells copied in
//! Excel, a table on a web page), each kept as a table of MailCraft's writing ([`mime::PastedTable`])
//! while its text, a row a line and its cells apart by tabs, stands where it goes in the message's
//! text, on lines of its own. Sent in its place as HTML ([`Compose::bodies`]); the plain text has
//! its text. A table whose text was changed is that text.

use std::ops::Range;

use mime::rich::{self, Styles};

use super::*;

/// Tables pasted into one message, at most: more are pasted as their text.
pub const TABLES_MAX: usize = 50;

impl Compose {
    /// Where each table stands in the text: its index in [`Compose::tables`] and its characters,
    /// in order. A table stands where its text is, whole lines of it, after the one before it;
    /// none in a message sent as plain text.
    pub fn table_places(&self) -> Vec<(usize, Range<usize>)> {
        let mut out = Vec::new();
        if self.plain {
            return out;
        }
        let bytes = self.body.as_bytes();
        let mut from = 0usize;
        for (i, t) in self.tables.iter().enumerate() {
            if t.text.is_empty() {
                continue;
            }
            let mut search = from;
            let found = loop {
                let Some(at) = self.body.get(search..).and_then(|r| r.find(t.text.as_str())).map(|rel| search + rel) else { break None };
                let end = at + t.text.len();
                let starts_line = at == 0 || bytes.get(at - 1) == Some(&b'\n');
                let ends_line = end == bytes.len() || bytes.get(end) == Some(&b'\n');
                if starts_line && ends_line {
                    break Some((at, end));
                }
                search = at + t.text.chars().next().map_or(1, char::len_utf8);
            };
            if let Some((at, end)) = found {
                let start = self.body.get(..at).map_or(0, |b| b.chars().count());
                out.push((i, start..start + t.text.chars().count()));
                from = end;
            }
        }
        out
    }

    /// Put what was pasted in place of the characters `sel`: its text with its formatting, and
    /// each table on lines of its own, kept as the table it is (a table pasted over goes). Tables
    /// past [`TABLES_MAX`] are their text. Where the cursor goes: after it.
    pub fn paste_parts(&mut self, sel: Range<usize>, parts: Vec<mime::Pasted>) -> usize {
        self.styles.fit(&self.body);
        let n = self.body.chars().count();
        let sel = sel.start.min(n)..sel.end.clamp(sel.start.min(n), n);
        let places = self.table_places();
        let mut room = TABLES_MAX.saturating_sub(places.iter().filter(|(_, r)| r.end <= sel.start || r.start >= sel.end).count());
        let char_at = |i: usize| self.body.chars().nth(i);
        // In the middle of a line before it, a table starts a line of its own.
        let mut open = sel.start > 0 && char_at(sel.start - 1).is_some_and(|c| c != '\n');
        let (mut text, mut styles) = (String::new(), Styles::plain(""));
        let mut added = Vec::new();
        for part in parts {
            match part {
                mime::Pasted::Text(t, st) => {
                    if t.is_empty() {
                        continue;
                    }
                    rich::append(&mut text, &mut styles, &t, &st);
                    open = !t.ends_with('\n');
                }
                mime::Pasted::Table(table) => {
                    if open {
                        rich::append(&mut text, &mut styles, "\n", &Styles::plain("\n"));
                    }
                    let lines = format!("{}\n", table.text);
                    rich::append(&mut text, &mut styles, &lines, &Styles::plain(&lines));
                    open = false;
                    if room > 0 {
                        room -= 1;
                        added.push(table);
                    }
                }
            }
        }
        // A table pasted last needs no line of its own after it where one follows anyway.
        if !added.is_empty() && text.ends_with('\n') && char_at(sel.end).is_none_or(|c| c == '\n') && !open {
            text.pop();
            styles.fit(&text);
        }
        rich::splice(&mut self.body, &mut self.styles, sel.clone(), &text, &styles);
        // The tables in their order in the text: those before it, the ones pasted, those after.
        let (before, after): (Vec<_>, Vec<_>) =
            places.into_iter().filter(|(_, r)| r.end <= sel.start || r.start >= sel.end).partition(|(_, r)| r.end <= sel.start);
        let kept = |list: Vec<(usize, Range<usize>)>, all: &[mime::PastedTable]| {
            list.into_iter().filter_map(|(i, _)| all.get(i).cloned()).collect::<Vec<_>>()
        };
        let mut tables = kept(before, &self.tables);
        tables.extend(added);
        tables.extend(kept(after, &self.tables));
        self.tables = tables;
        sel.start + text.chars().count()
    }
}

/// A saved message's HTML (a draft opened again) as text and styles, each table it went out
/// with ([`mime::TABLE_CLASS`]) its text again on lines of its own, and kept in `tables`, laid out
/// and written again as MailCraft writes a table: what the server keeps isn't sent as it is.
pub(super) fn text_with_tables(html: &str, tables: &mut Vec<mime::PastedTable>) -> (String, Styles) {
    let (mut text, mut styles) = (String::new(), Styles::plain(""));
    let mut rest = html.to_string();
    while tables.len() < TABLES_MAX {
        let Some((before, code, after)) = mime::split_marked(&rest, mime::TABLE_CLASS) else { break };
        let (t, st) = rich::from_rich(&mime::html_to_rich(&before));
        rich::append(&mut text, &mut styles, &t, &st);
        let table = mime::pasted_tables(&code).into_iter().flatten().find_map(|p| match p {
            mime::Pasted::Table(t) => Some(t),
            mime::Pasted::Text(..) => None,
        });
        if !text.is_empty() && !text.ends_with('\n') {
            rich::append(&mut text, &mut styles, "\n", &Styles::plain("\n"));
        }
        match table {
            Some(table) => {
                let lines = format!("{}\n", table.text);
                rich::append(&mut text, &mut styles, &lines, &Styles::plain(&lines));
                tables.push(table);
            }
            // No table any more (a table of one cell is a box of text): its text.
            None => {
                let (t, st) = rich::from_rich(&mime::html_to_rich(&code));
                let lines = format!("{t}\n");
                let mut line_styles = st;
                line_styles.fit(&lines);
                rich::append(&mut text, &mut styles, &lines, &line_styles);
            }
        }
        rest = after;
    }
    let (t, st) = rich::from_rich(&mime::html_to_rich(&rest));
    if t.trim().is_empty() {
        // Nothing after the last table: no empty line after it.
        if text.ends_with('\n') {
            text.pop();
            styles.fit(&text);
        }
    } else {
        rich::append(&mut text, &mut styles, &t, &st);
    }
    (text, styles)
}

#[cfg(test)]
mod tests;
