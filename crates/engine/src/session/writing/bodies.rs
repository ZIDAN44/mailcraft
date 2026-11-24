//! A message being written as it goes: its text, its HTML and the pictures in it, with the
//! signature's own code and the original of a reply where they stand.

use super::*;

impl Compose {
    /// The text, HTML and pictures to send: formatted when the text uses formatting or has
    /// pictures (and the message isn't set to plain text), else plain text alone; the original
    /// of a reply kept as it came after it ([`Quote`]).
    pub fn bodies(&self) -> (String, Option<String>, Vec<mime::InlinePicture>) {
        self.with_quote(self.own_bodies())
    }

    /// [`Compose::bodies`] of the text alone.
    fn own_bodies(&self) -> (String, Option<String>, Vec<mime::InlinePicture>) {
        let mut styles = self.styles.clone();
        styles.fit(&self.body);
        let used = mime::rich::pictures(&self.body, &styles);
        let pictures: Vec<mime::InlinePicture> = self.inline.iter().filter(|p| used.contains(&p.cid)).cloned().collect();
        let text = mime::rich::to_text(&self.body, &styles);
        if self.plain || (styles.is_plain() && pictures.is_empty() && self.font.is_none() && self.size.is_none()) {
            (text, None, Vec::new())
        } else {
            (text, Some(mime::rich::to_html_in(&self.body, &styles, self.font.as_deref(), self.size)), pictures)
        }
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
        let (text, html, pictures) = self.own_bodies();
        let at = self.signature_at().filter(|_| !self.plain && !code.trim().is_empty());
        let Some(b) = at else { return self.with_quote((text, html, pictures)) };
        let mut styles = self.styles.clone();
        styles.fit(&self.body);
        let start = self.body.get(..b).unwrap_or("").chars().count();
        let end = start + self.signature.chars().count();
        // `-- ` stays text, as above every signature; the code is its next line.
        let (before, before_styles) = mime::rich::slice(&self.body, &styles, 0..start + "-- ".len());
        let (after, after_styles) = mime::rich::slice(&self.body, &styles, end..self.body.chars().count());
        let (code_html, code_pictures) = mime::signature_code_parts(code, &self.signature_code_tag());
        let mut out = mime::rich::to_html_in(&before, &before_styles, self.font.as_deref(), self.size);
        // Marked, so a saved draft opened again has the code back as it was.
        out.push_str(&format!("<div class=\"{}\">{code_html}</div>", mime::SIGNATURE_CLASS));
        if !after.trim().is_empty() {
            out.push_str(&mime::rich::to_html_in(&after, &after_styles, self.font.as_deref(), self.size));
        }
        let mut used = mime::rich::pictures(&before, &before_styles);
        used.extend(mime::rich::pictures(&after, &after_styles));
        let mut pictures: Vec<mime::InlinePicture> = pictures.into_iter().filter(|p| used.contains(&p.cid)).collect();
        for p in code_pictures {
            if !pictures.iter().any(|q| q.cid == p.cid) {
                pictures.push(p);
            }
        }
        self.with_quote((text, Some(out), pictures))
    }
}
