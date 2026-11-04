//! The original under a reply or forward written in HTML, kept as it came: shown under the text
//! as the reading pane shows it (its columns, colours and pictures) and edited where it stands,
//! each block of its text (a table's cell, a box over a picture) a text of its own, as Outlook
//! lets the original be changed in place. Sent after the text: as its own HTML while nothing in
//! it was changed, else its layout written out again around the changed text.

use std::collections::BTreeMap;

use mime::layout::{Block, cell_text, text_blocks};
use mime::rich::{self, Styles};

use super::*;

/// The original under a reply or forward, as it came.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Quote {
    /// The message quoted: its pictures, those behind its boxes too, are shown from it.
    pub key: MessageKey,
    /// That message as the reading pane shows it.
    pub parsed: mime::Parsed,
    /// Its parts as laid out (its tables and the text in them, or its text as one block), the
    /// pictures in its text under this message's `cid:`s ([`Compose::inline`]).
    pub blocks: Vec<Block>,
    /// Its HTML as it came ([`mime::quote_html`]), sent while nothing in it was changed.
    pub html: String,
    /// Each picture's address in the message, and the `cid:` it goes under.
    pub given: Vec<(String, String)>,
    /// The blocks of text changed here, by their place ([`text_blocks`]): text and styles.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub edits: BTreeMap<usize, (String, Styles)>,
}

impl Quote {
    /// The quote of message `key` (`original` as it's shown, `parsed` its copy whose pictures
    /// were given this message's `cid:`s, `given` the addresses they were given for), from its
    /// HTML as written; `None` for mail with no HTML.
    pub fn of(key: MessageKey, original: mime::Parsed, parsed: &mime::Parsed, given: &[(String, String)], html: &str) -> Option<Quote> {
        if !parsed.had_html || parsed.rich.is_empty() || html.trim().is_empty() {
            return None;
        }
        let blocks = if parsed.layout.is_empty() { vec![Block::Text(parsed.rich.clone())] } else { parsed.layout.clone() };
        Some(Quote {
            key,
            parsed: original,
            blocks,
            html: mime::quote_html(html, given, &parsed.trackers.pictures),
            given: given.to_vec(),
            edits: BTreeMap::new(),
        })
    }

    /// How many blocks of text it has.
    pub fn cells(&self) -> usize {
        text_blocks(&self.blocks).len()
    }

    /// Block of text `n` as text and styles to edit: as changed here, else as it came.
    pub fn cell(&self, n: usize) -> Option<(String, Styles)> {
        if let Some(e) = self.edits.get(&n) {
            return Some(e.clone());
        }
        text_blocks(&self.blocks).get(n).map(|(lines, align)| cell_text(lines, *align))
    }

    /// Its text, for the plain-text twin: each block of text on lines of its own.
    pub fn text(&self) -> String {
        (0..self.cells())
            .filter_map(|n| self.cell(n))
            .map(|(t, s)| rich::to_text(&t, &s))
            .filter(|t| !t.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Its HTML as it goes out: as it came, or with its changed text written into its layout.
    pub fn html(&self) -> String {
        if self.edits.is_empty() {
            return self.html.clone();
        }
        let blocks = text_blocks(&self.blocks);
        let out = mime::layout::to_html(&self.blocks, &mut |n| {
            let (t, s) = match self.edits.get(&n) {
                Some(e) => e.clone(),
                None => blocks.get(n).map(|(lines, _)| rich::from_rich(lines)).unwrap_or_default(),
            };
            rich::to_html(&t, &s)
        });
        // Pictures behind its boxes under this message's `cid:`s, as the HTML that came.
        mime::quote_html(&out, &self.given, &[])
    }

    /// The `cid:`s of the pictures it shows (their pictures go with the message).
    pub fn picture_cids(&self) -> Vec<String> {
        let mut v: Vec<String> = Vec::new();
        let edited = self.edits.values().flat_map(|(t, s)| rich::pictures(t, s));
        for cid in self.given.iter().map(|(_, c)| c.clone()).chain(edited) {
            if !v.contains(&cid) {
                v.push(cid);
            }
        }
        v
    }
}

/// Whether two texts with their styles read and look the same (their styles' palettes may be
/// laid out differently).
fn same((a, sa): &(String, Styles), (b, sb): &(String, Styles)) -> bool {
    let n = a.chars().count();
    let lines = a.split('\n').count();
    a == b && (0..n).all(|i| sa.style(i) == sb.style(i)) && (0..lines).all(|l| sa.para(l) == sb.para(l))
}

impl Compose {
    /// What [`Compose::bodies`] gives (`text`, `html`, `pictures`), with the quote after it: its
    /// HTML and pictures, and its text in the plain-text twin.
    pub(super) fn with_quote(
        &self,
        (text, html, mut pictures): (String, Option<String>, Vec<mime::InlinePicture>),
    ) -> (String, Option<String>, Vec<mime::InlinePicture>) {
        let Some(q) = &self.quote else { return (text, html, pictures) };
        let text = format!("{text}\n\n{}", q.text());
        if self.plain {
            return (text, None, Vec::new());
        }
        let mut out = html.unwrap_or_else(|| {
            let mut styles = self.styles.clone();
            styles.fit(&self.body);
            rich::to_html(&self.body, &styles)
        });
        let quoted = q.html();
        // Marked, so a message taken back to change (from the Outbox) has it as it was.
        out.push_str(&format!("<div class=\"{}\">{quoted}</div>\n", mime::QUOTE_CLASS));
        for p in self.inline.iter().filter(|p| quoted.contains(&format!("cid:{}", p.cid))) {
            if !pictures.iter().any(|x| x.cid == p.cid) {
                pictures.push(p.clone());
            }
        }
        (text, Some(out), pictures)
    }

    /// Do `f` with block of text `n` of the quote standing in for the text (as the editor and
    /// commands change the text), then keep what it changed in it. `None` when there's no such
    /// block.
    pub fn in_quote_cell<R>(&mut self, n: usize, f: impl FnOnce(&mut Compose) -> R) -> Option<R> {
        let q = self.quote.take()?;
        let Some((text, styles)) = q.cell(n) else {
            self.quote = Some(q);
            return None;
        };
        let body = std::mem::replace(&mut self.body, text);
        let own = std::mem::replace(&mut self.styles, styles);
        let r = f(self);
        let text = std::mem::replace(&mut self.body, body);
        let styles = std::mem::replace(&mut self.styles, own);
        self.quote = Some(q);
        self.set_quote_cell(n, text, styles);
        Some(r)
    }

    /// Block of text `n` of the quote is now `text` with `styles` (kept only when it changed).
    pub fn set_quote_cell(&mut self, n: usize, text: String, mut styles: Styles) {
        let Some(q) = self.quote.as_mut() else { return };
        styles.fit(&text);
        let now = (text, styles);
        if q.cell(n).is_some_and(|was| !same(&was, &now)) {
            Arc::make_mut(q).edits.insert(n, now);
        }
    }
}
