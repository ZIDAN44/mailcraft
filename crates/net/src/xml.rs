//! XML read into a small tree of elements, and text escaped for XML: one reader for every
//! answer and file that comes as XML (Exchange's SOAP answers, Autodiscover, a mail provider's
//! configuration file), so comments, CDATA and character references are read one way.
//!
//! What's read is hostile input: elements are capped in number and depth, nothing here panics.

use quick_xml::Reader;
use quick_xml::events::Event as X;

/// Elements read from one answer at most, and how deep.
const MAX_ELEMENTS: usize = 500_000;
const MAX_DEPTH: usize = 64;

/// An element of an answer: its local name, attributes, text and children.
#[derive(Clone, Debug, Default)]
pub struct El {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    pub text: String,
    pub kids: Vec<El>,
}

impl El {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
    /// The first child called `name`.
    pub fn child(&self, name: &str) -> Option<&El> {
        self.kids.iter().find(|k| k.name == name)
    }
    pub fn children<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a El> + 'a {
        self.kids.iter().filter(move |k| k.name == name)
    }
    /// The first element called `name` below this one (depth first).
    pub fn find(&self, name: &str) -> Option<&El> {
        self.kids.iter().find_map(|k| if k.name == name { Some(k) } else { k.find(name) })
    }
    /// Every element called `name` below this one.
    pub fn all<'a>(&'a self, name: &str, out: &mut Vec<&'a El>) {
        for k in &self.kids {
            if k.name == name {
                out.push(k);
            } else {
                k.all(name, out);
            }
        }
    }
    pub fn text_of(&self, name: &str) -> Option<&str> {
        self.child(name).map(|c| c.text.as_str())
    }
}

/// An element's or attribute's name without its namespace prefix (`t:Folder` → `Folder`).
pub(crate) fn local(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or("")
}

/// Read XML into elements (namespaces dropped: the local names are enough here). Past the caps
/// on depth and count, elements are left out with all they hold, the rest kept where it was.
pub fn parse(xml: &str) -> El {
    let mut r = Reader::from_str(xml);
    let mut stack: Vec<El> = vec![El { name: "#doc".into(), ..Default::default() }];
    let mut count = 0usize;
    // Start tags left out and not ended yet: their ends close nothing kept.
    let mut skipped = 0usize;
    let close = |stack: &mut Vec<El>| {
        if stack.len() > 1
            && let Some(done) = stack.pop()
            && let Some(parent) = stack.last_mut()
        {
            parent.kids.push(done);
        }
    };
    while let Ok(ev) = r.read_event() {
        let empty = matches!(ev, X::Empty(_));
        match ev {
            X::Start(_) if skipped > 0 || stack.len() >= MAX_DEPTH || count >= MAX_ELEMENTS => skipped += 1,
            X::End(_) if skipped > 0 => skipped -= 1,
            // What a left-out element holds is left out with it.
            _ if skipped > 0 && !matches!(ev, X::Eof) => {}
            X::Start(e) | X::Empty(e) if stack.len() < MAX_DEPTH && count < MAX_ELEMENTS => {
                count += 1;
                let el = El {
                    name: local(e.name().as_ref()).to_string(),
                    attrs: e
                        .attributes()
                        .flatten()
                        .map(|a| (local(a.key.as_ref()).to_string(), mailcraft_mime::decode_xml_refs(&a.value).into_owned()))
                        .collect(),
                    ..Default::default()
                };
                stack.push(el);
                if empty {
                    close(&mut stack);
                }
            }
            X::Text(t) => {
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(&t);
                }
            }
            X::CData(t) => {
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(&t);
                }
            }
            X::GeneralRef(g) => {
                if let (Some(c), Some(top)) = (mailcraft_mime::xml_ref_char(&g), stack.last_mut()) {
                    top.text.push(c);
                }
            }
            X::End(_) => close(&mut stack),
            X::Eof => break,
            _ => {}
        }
    }
    while stack.len() > 1 {
        close(&mut stack);
    }
    stack.pop().unwrap_or_default()
}

/// Text for XML.
pub fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            // Not allowed in XML 1.0.
            c if (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') => {}
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An element past the depth cap is left out with what it holds; its end doesn't close the
    /// element it's in, so what comes after stays where it was.
    #[test]
    fn past_the_depth_cap_the_tree_keeps_its_shape() {
        // `#doc` and `r` and 62 `a`s fill the 64 levels: `deep` is one too many.
        let opens = "<a>".repeat(MAX_DEPTH - 2);
        let closes = "</a>".repeat(MAX_DEPTH - 2);
        let doc = parse(&format!("<r>{opens}<deep>lost<x/><y>gone</y></deep>{closes}<sibling/></r>"));
        let r = doc.child("r").unwrap();
        assert!(r.child("sibling").is_some(), "the sibling stays in r");
        assert_eq!(doc.kids.len(), 1, "nothing climbs to the top");
        let mut innermost = r;
        while let Some(a) = innermost.child("a") {
            innermost = a;
        }
        assert!(innermost.kids.is_empty() && innermost.text.is_empty(), "what's past the cap is left out: {innermost:?}");
        assert!(doc.find("deep").is_none() && doc.find("x").is_none());
    }

    /// The same past the cap on how many elements are read: one too many is left out with what
    /// it holds, and its end closes nothing, so the text after it is still its parent's.
    #[test]
    fn past_the_element_cap_the_tree_keeps_its_shape() {
        // `r` and its items are all the elements read: `over` is one too many.
        let items = "<i/>".repeat(MAX_ELEMENTS - 1);
        let doc = parse(&format!("<r>{items}<over>lost<x/><y>gone</y></over>tail<late/></r>after"));
        let r = doc.child("r").unwrap();
        assert_eq!((doc.kids.len(), doc.text.as_str()), (1, "after"), "nothing climbs to the top");
        assert_eq!(r.kids.len(), MAX_ELEMENTS - 1);
        assert!(r.kids.iter().all(|k| k.name == "i"), "what's past the cap is left out");
        assert_eq!(r.text, "tail", "r is still open after the element left out ends");
    }
}
