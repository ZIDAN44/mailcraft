//! A message being written as it goes: its text, its HTML and the pictures in it, with the
//! signature's own code, the tables pasted into it and the original of a reply where they stand.

use std::ops::Range;

use super::*;

/// What the HTML carries in place of some of the text: its characters, its code and its pictures;
/// whether the line breaks on either side are its own (a table's lines) or the text's.
struct Card {
    range: Range<usize>,
    code: String,
    pictures: Vec<mime::InlinePicture>,
    own_lines: bool,
}

impl Compose {
    /// The text, HTML and pictures to send: formatted when the text uses formatting or has
    /// pictures (and the message isn't set to plain text), else plain text alone; the original
    /// of a reply kept as it came after it ([`Quote`]).
    pub fn bodies(&self) -> (String, Option<String>, Vec<mime::InlinePicture>) {
        self.with_quote(self.own_bodies(None))
    }

    /// [`Compose::bodies`] of the text alone, with the signature's `code` where it stands, if
    /// given ([`Compose::bodies_with`]), and each table pasted in where it stands (issue #268).
    fn own_bodies(&self, code: Option<&str>) -> (String, Option<String>, Vec<mime::InlinePicture>) {
        let mut styles = self.styles.clone();
        styles.fit(&self.body);
        let text = mime::rich::to_text(&self.body, &styles);
        let cards = self.cards(code);
        let used = mime::rich::pictures(&self.body, &styles);
        let pictures: Vec<mime::InlinePicture> = self.inline.iter().filter(|p| used.contains(&p.cid)).cloned().collect();
        if self.plain || (cards.is_empty() && styles.is_plain() && pictures.is_empty() && self.font.is_none() && self.size.is_none()) {
            return (text, None, Vec::new());
        }
        if cards.is_empty() {
            return (text, Some(mime::rich::to_html_in(&self.body, &styles, self.font.as_deref(), self.size)), pictures);
        }
        // The text between the cards as HTML, each card's code in its place.
        let n = self.body.chars().count();
        let (mut out, mut used, mut kept) = (String::new(), Vec::new(), Vec::new());
        let write = |range: Range<usize>, out: &mut String, used: &mut Vec<String>| {
            let (part, part_styles) = mime::rich::slice(&self.body, &styles, range);
            if !part.trim().is_empty() {
                out.push_str(&mime::rich::to_html_in(&part, &part_styles, self.font.as_deref(), self.size));
                used.extend(mime::rich::pictures(&part, &part_styles));
            }
        };
        let mut at = 0;
        for card in cards {
            // A table's lines are its own: the line breaks either side of it aren't empty lines.
            let gap = usize::from(card.own_lines && card.range.start > at);
            write(at..card.range.start - gap, &mut out, &mut used);
            out.push_str(&card.code);
            kept.extend(card.pictures);
            at = (card.range.end + usize::from(card.own_lines && card.range.end < n)).min(n);
        }
        write(at..n, &mut out, &mut used);
        let mut pictures: Vec<mime::InlinePicture> = pictures.into_iter().filter(|p| used.contains(&p.cid)).collect();
        for p in kept {
            if !pictures.iter().any(|q| q.cid == p.cid) {
                pictures.push(p);
            }
        }
        (text, Some(out), pictures)
    }

    /// What the HTML carries in place of some of the text, in order: each table where it stands,
    /// and the signature's `code` where the signature stands as it was put in (after its `-- `
    /// line, which stays text). Marked, so a draft opened again has them back as they were.
    fn cards(&self, code: Option<&str>) -> Vec<Card> {
        let mut cards: Vec<Card> = self
            .table_places()
            .into_iter()
            .filter_map(|(i, range)| {
                let t = self.tables.get(i)?;
                Some(Card { range, code: format!("<div class=\"{}\">{}</div>", mime::TABLE_CLASS, t.code), pictures: Vec::new(), own_lines: true })
            })
            .collect();
        if let Some((code, b)) = code.filter(|c| !self.plain && !c.trim().is_empty()).zip(self.signature_at()) {
            let start = self.body.get(..b).unwrap_or("").chars().count();
            let end = start + self.signature.chars().count();
            let (code_html, pictures) = mime::signature_code_parts(code, &self.signature_code_tag());
            let code = format!("<div class=\"{}\">{code_html}</div>", mime::SIGNATURE_CLASS);
            cards.push(Card { range: start + "-- ".len()..end, code, pictures, own_lines: false });
        }
        cards.sort_by_key(|c| c.range.start);
        // Never one over another.
        let mut end = 0;
        cards.retain(|c| {
            let fits = c.range.start >= end;
            if fits {
                end = c.range.end;
            }
            fits
        });
        cards
    }

    /// The size its text is in where none is set on it, in points (issue #267).
    pub fn base_size(&self) -> u8 {
        mime::rich::font_size(self.size).unwrap_or(mime::rich::DEFAULT_SIZE)
    }

    /// Set the text (and forget its formatting).
    pub fn set_text(&mut self, text: String) {
        self.styles = mime::rich::Styles::plain(&text);
        self.body = text;
    }

    /// [`Compose::bodies`], with the signature's `code` in the HTML where the signature
    /// stands (issue #11: a business card in columns, which the text can't hold), as long as it
    /// stands there as it was put in; the text keeps the signature's text. Its pictures go with
    /// it, under this message's own `cid:`s.
    pub fn bodies_with(&self, code: &str) -> (String, Option<String>, Vec<mime::InlinePicture>) {
        self.with_quote(self.own_bodies(Some(code)))
    }
}
