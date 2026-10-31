//! The session: signatures in messages being written (issue #11). The account's signature for
//! new messages or the one for replies and forwards goes in; another is chosen from the
//! ribbon's Signature menu; and when From changes, the new account's takes the old one's place,
//! as long as it's still there as it was put in.

use mailcraft_model::Signature;

use super::*;

/// A signature as it goes into a message: after the usual `-- ` line (RFC 3676's delimiter).
pub fn signature_block(sig: &str) -> String {
    format!("-- \n{}", sig.trim_end())
}

/// A signature's text, look and pictures: as its HTML has them, else its plain text.
pub fn signature_parts(sig: &Signature) -> (String, mime::rich::Styles, Vec<mime::InlinePicture>) {
    if !sig.html.trim().is_empty() {
        let (text, styles, pictures) = mime::signature_html(&sig.html);
        if !text.trim().is_empty() {
            return (text, styles, pictures);
        }
    }
    (sig.text.clone(), mime::rich::Styles::plain(&sig.text), Vec::new())
}

/// Give the signature in a message (`c.signature`, where it still stands as put there) the look
/// it has: `sig` is its text and styles. Its `pictures` go into the message too, each under a
/// `cid:` of this message's own (`tag`, from [`Session::signature_tag`]): one `cid:` in every
/// message isn't unique, as RFC 2392 asks (#16). Pictures of a signature put in before and no
/// longer in the text go.
pub fn style_signature(c: &mut Compose, sig: &(String, mime::rich::Styles), pictures: &[mime::InlinePicture], tag: &str) {
    let (text, styles) = sig;
    if c.signature.is_empty() || styles.is_plain() {
        return;
    }
    let mut block = "-- \n".to_string();
    let mut block_styles = mime::rich::Styles::plain(&block);
    mime::rich::append(&mut block, &mut block_styles, text.trim_end(), styles);
    if block != c.signature {
        return;
    }
    let Some(b) = c.signature_at() else { return };
    let start = c.body.get(..b).unwrap_or("").chars().count();
    let n = block.chars().count();
    for (i, ch) in block.chars().enumerate() {
        let Some(p) = block_styles.style(i).picture.and_then(|cid| pictures.iter().find(|p| p.cid == cid)).filter(|_| ch == mime::rich::PICTURE)
        else {
            continue;
        };
        let cid = format!("{}.{tag}@mailcraft", p.cid.split('@').next().unwrap_or("signature"));
        block_styles.restyle(&block, i..i + 1, |s| mime::rich::Style { picture: Some(cid.clone()), ..s.clone() });
        // Another signature's picture under that `cid:` (one switched from) gives way.
        match c.inline.iter_mut().find(|q| q.cid == cid) {
            Some(q) => *q = mime::InlinePicture { cid, ..p.clone() },
            None => c.inline.push(mime::InlinePicture { cid, ..p.clone() }),
        }
    }
    c.styles.fit(&c.body);
    mime::rich::splice(&mut c.body, &mut c.styles, start..start + n, &block, &block_styles);
    let shown = mime::rich::pictures(&c.body, &c.styles);
    c.inline.retain(|p| !p.cid.starts_with("signature") || shown.contains(&p.cid));
}

/// The line drawn over the original of a reply or forward: the character its line starts at.
fn quote_line(c: &Compose) -> Option<usize> {
    // A paragraph past the text's last line (styles not yet fitted to it) isn't there.
    let rule = c.styles.paras.iter().position(|p| p.rule).filter(|r| *r <= c.body.matches('\n').count())?;
    Some(if rule == 0 { 0 } else { c.body.chars().enumerate().filter(|(_, ch)| *ch == '\n').nth(rule - 1).map_or(0, |(i, _)| i + 1) })
}

impl Compose {
    /// Where the signature put in stands (a byte offset), as it was put in: the last above the
    /// quote of a reply, never in it (a reply to one's own message quotes the same signature,
    /// and changing that one would change what was said), nor a copy of its text written above
    /// it (a `mailto:` link's words, a message pasted in), which is the writer's.
    pub fn signature_at(&self) -> Option<usize> {
        if self.signature.is_empty() {
            return None;
        }
        let end = quote_line(self).and_then(|i| self.body.char_indices().nth(i).map(|(b, _)| b)).unwrap_or(self.body.len());
        self.body.get(..end)?.rfind(self.signature.as_str())
    }
}

/// Where a signature goes in a message that has none: above the original of a reply or forward
/// (the line drawn over it), else at the end, after a blank line. The character it goes at and
/// what goes there.
fn signature_place(c: &Compose, block: &str) -> (usize, String) {
    // Before the line break that ends the line above the quote's: put at the start of the
    // quote's own line, the signature's first line took that line's rule.
    match quote_line(c) {
        Some(0) => return (0, format!("{block}\n\n")),
        Some(at) => return (at - 1, format!("\n{block}\n")),
        None => {}
    }
    let gap = if c.body.is_empty() || !c.body.ends_with('\n') {
        "\n\n"
    } else if c.body.ends_with("\n\n") {
        ""
    } else {
        "\n"
    };
    (c.body.chars().count(), format!("{gap}{block}"))
}

impl Session {
    /// An account's signature by name.
    pub fn signature_of(&self, account: &str, name: &str) -> Option<Signature> {
        self.account(account).and_then(|a| a.config.signature_named(name)).cloned()
    }

    /// The signature an account puts into new messages, or (`reply`) into replies and forwards.
    pub fn default_signature(&self, account: &str, reply: bool) -> Option<Signature> {
        self.account(account).and_then(|a| a.config.default_signature(reply)).cloned()
    }

    /// A signature's text as it first goes in: plain, its look given after ([`style_signature`]).
    pub(crate) fn signature_text(sig: Option<&Signature>) -> String {
        sig.map(|s| signature_parts(s).0).filter(|t| !t.trim().is_empty()).map(|t| signature_block(&t)).unwrap_or_default()
    }

    /// A tag for the `cid:`s of a signature's pictures in the message being written
    /// ([`style_signature`]): the one it has already, so a signature put in again keeps them.
    /// Its own, never one of the pictures quoted from a message sent from MailCraft, whose
    /// `cid:`s look the same.
    pub(crate) fn signature_tag(&mut self) -> String {
        let had = self.compose.as_ref().map(|c| c.signature_tag.clone()).filter(|t| !t.is_empty());
        had.unwrap_or_else(|| format!("{:012x}", self.random() & 0xFFFF_FFFF_FFFF))
    }

    /// The signature in the message being made, `sig` (whose text went in plain), remembered
    /// with it and given its look and pictures.
    pub(super) fn style_signature(&mut self, sig: Option<&Signature>) {
        let tag = self.signature_tag();
        let Some(c) = self.compose.as_mut() else { return };
        c.signature_name = sig.filter(|_| !c.signature.is_empty()).map(|s| s.name.clone()).unwrap_or_default();
        c.signature_code = sig.filter(|_| !c.signature.is_empty()).map(|s| s.code.clone()).unwrap_or_default();
        c.signature_tag = tag.clone();
        let Some(sig) = sig else { return };
        let (text, styles, pictures) = signature_parts(sig);
        style_signature(c, &(text, styles), &pictures, &tag);
    }

    /// Put signature `sig` into the message being written (`None`: take out the one there).
    /// The one put in before, still there as it was, is replaced (as choosing a signature in
    /// Outlook replaces the one there); otherwise it goes in at character `at`, on a line of its
    /// own, or (no `at`) where a signature goes. The range it now takes.
    pub fn put_signature(&mut self, sig: Option<&Signature>, at: Option<usize>) -> Result<std::ops::Range<usize>, CmdError> {
        let block = Self::signature_text(sig);
        let c = self.compose.as_mut().ok_or_else(|| CmdError::Disabled("no message is being written".into()))?;
        if c.sending {
            return Err(CmdError::Disabled("the message is being sent".into()));
        }
        c.styles.fit(&c.body);
        let n = c.body.chars().count();
        let there = c.signature_at().map(|b| (b, b + c.signature.len()));
        let (start, stop, text) = match there {
            Some((b, e)) => {
                // Taken out, with the blank line it had above it.
                let gap = if block.is_empty() && c.body.get(..b).is_some_and(|s| s.ends_with("\n\n")) { 2 } else { 0 };
                let chars = |i: usize| c.body.get(..i).unwrap_or("").chars().count();
                (chars(b).saturating_sub(gap), chars(e), block.clone())
            }
            None if block.is_empty() => (n, n, String::new()),
            None => match at {
                Some(at) => {
                    let at = at.min(n);
                    let before = if at > 0 && c.body.chars().nth(at - 1) != Some('\n') { "\n" } else { "" };
                    (at, at, format!("{before}{block}\n"))
                }
                None => {
                    let (at, text) = signature_place(c, &block);
                    (at, at, text)
                }
            },
        };
        // Put right above a quote that starts the text, its rule stays on the quote's line.
        let above_quote = start == stop && start == 0 && !text.is_empty() && quote_line(c) == Some(0);
        mime::rich::splice(&mut c.body, &mut c.styles, start..stop, &text, &mime::rich::Styles::plain(&text));
        if above_quote {
            let line = text.matches('\n').count();
            c.styles.paras.iter_mut().take(line).for_each(|p| p.rule = false);
            if let Some(p) = c.styles.paras.get_mut(line) {
                p.rule = true;
            }
        }
        c.signature = block;
        // The pictures of the one taken out go (those of the one put in come back with it).
        let shown = mime::rich::pictures(&c.body, &c.styles);
        let tag = format!(".{}@mailcraft", c.signature_tag);
        c.inline.retain(|p| !(p.cid.starts_with("signature") && p.cid.ends_with(&tag)) || shown.contains(&p.cid));
        self.style_signature(sig);
        Ok(start..start + text.chars().count())
    }

    /// From changed to another account (`was` the one it was from): the new account's
    /// signature takes the place of the one put in, while that's still there as it was, or
    /// goes in where none was because the old account had none; where the new account has
    /// none, the old one goes. A signature changed in the message stays as it is (Outlook's
    /// way too).
    pub fn follow_from(&mut self, was: &str) {
        let Some(c) = self.compose.as_ref() else { return };
        if c.account == was {
            return;
        }
        let reply = c.mode.is_some();
        let untouched = if c.signature.is_empty() { self.default_signature(was, reply).is_none() } else { c.signature_at().is_some() };
        if !untouched {
            return;
        }
        let new = self.default_signature(&c.account, reply);
        if let Err(e) = self.put_signature(new.as_ref(), None) {
            log::warn!("signature: {e}");
        }
    }
}
