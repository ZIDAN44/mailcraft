//! A `207 Multi-Status` answer read into its [`Item`]s: one pass over the XML, what's been read so
//! far kept in a [`Reading`].

use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event as X};

use super::{Item, MAX_BODY};

/// How deep the answer is read; what's deeper is left out with all it holds.
const MAX_DEPTH: usize = 64;
/// Responses read at most.
const MAX_ITEMS: usize = 100_000;

/// An element's local name, in lower case (`d:Href` → `href`).
fn local(name: &str) -> String {
    crate::xml::local(name).to_ascii_lowercase()
}

/// Read a `207 Multi-Status` body.
pub fn multistatus(xml: &str) -> Vec<Item> {
    let mut r = Reader::from_str(xml);
    let mut reading = Reading::default();
    // Start tags past the depth cap not ended yet: left out with all they hold, their ends
    // closing nothing read.
    let mut skipped = 0usize;
    while let Ok(ev) = r.read_event() {
        match ev {
            X::Start(_) if skipped > 0 || reading.stack.len() >= MAX_DEPTH => skipped += 1,
            X::End(_) if skipped > 0 => skipped -= 1,
            _ if skipped > 0 && !matches!(ev, X::Eof) => {}
            // `<x/>` is a start and an end at once.
            X::Start(e) => reading.start(&e, false),
            X::Empty(e) if reading.stack.len() < MAX_DEPTH => reading.start(&e, true),
            X::Text(t) => reading.push_text(&t),
            X::CData(t) => reading.push_text(&t),
            X::GeneralRef(g) => {
                if let Some(c) = mailcraft_mime::xml_ref_char(&g) {
                    reading.text.push(c);
                }
            }
            X::End(e) => reading.end_tag(&local(e.name().as_ref())),
            X::Eof => break,
            _ => {}
        }
        if reading.out.len() > MAX_ITEMS {
            break;
        }
    }
    reading.out
}

/// What a multistatus answer has been read into so far.
#[derive(Default)]
struct Reading {
    /// The elements open, by local name.
    stack: Vec<String>,
    out: Vec<Item>,
    /// The `<response>` being read.
    cur: Option<Item>,
    /// The properties of the propstat being read, kept only if its status is 200.
    pending: Item,
    status: String,
    /// The text of the element being read.
    text: String,
    privileges: Vec<String>,
}

impl Reading {
    fn push_text(&mut self, t: &str) {
        if self.text.len() < MAX_BODY {
            self.text.push_str(t);
        }
    }

    fn start(&mut self, e: &BytesStart<'_>, empty: bool) {
        let name = local(e.name().as_ref());
        let parent = self.stack.last().map(String::as_str).unwrap_or("");
        if parent == "resourcetype" {
            self.pending.kinds.push(name.clone());
        }
        if name == "comp" && parent == "supported-calendar-component-set" {
            for a in e.attributes().flatten() {
                if local(a.key.as_ref()) == "name" {
                    self.pending.comps.push(a.value.to_ascii_uppercase());
                }
            }
        }
        if self.stack.iter().any(|s| s == "privilege") {
            self.privileges.push(name.clone());
        }
        match name.as_str() {
            "response" => self.cur = Some(Item::default()),
            "propstat" => {
                self.pending = Item::default();
                self.status.clear();
            }
            _ => {}
        }
        self.text.clear();
        if empty {
            self.end(&name);
        } else {
            self.stack.push(name);
        }
    }

    /// An end tag: the element closed, then what it ends.
    fn end_tag(&mut self, name: &str) {
        if self.stack.last().is_some_and(|s| s == name) {
            self.stack.pop();
        }
        self.end(name);
        self.text.clear();
    }

    /// The element `name` ends (in the element on top of the stack).
    fn end(&mut self, name: &str) {
        let parent = self.stack.last().map(String::as_str).unwrap_or("");
        match name {
            "href" if parent == "response" => {
                if let Some(c) = self.cur.as_mut() {
                    c.href = self.text.trim().to_string();
                }
            }
            "href" => {
                // An href inside a property: file it under the property.
                if let Some(prop) = self.stack.iter().rev().find(|s| s.as_str() != "href" && s.as_str() != "prop") {
                    self.pending.hrefs.entry(prop.clone()).or_default().push(self.text.trim().to_string());
                }
            }
            "status" if parent == "propstat" => self.status = self.text.clone(),
            "current-user-privilege-set" => {
                self.pending.writable = Some(self.privileges.iter().any(|p| p == "write" || p == "all" || p == "write-content" || p == "bind"));
                self.privileges.clear();
            }
            "propstat" => {
                let ok = self.status.split_whitespace().nth(1).is_none_or(|c| c.starts_with('2'));
                if ok && let Some(c) = self.cur.as_mut() {
                    let p = std::mem::take(&mut self.pending);
                    c.props.extend(p.props);
                    for (k, v) in p.hrefs {
                        c.hrefs.entry(k).or_default().extend(v);
                    }
                    c.kinds.extend(p.kinds);
                    c.comps.extend(p.comps);
                    c.writable = c.writable.or(p.writable);
                }
            }
            "response" => {
                if let Some(c) = self.cur.take() {
                    self.out.push(c);
                }
            }
            _ => {
                if parent == "prop" && !self.text.trim().is_empty() {
                    self.pending.props.insert(name.to_string(), self.text.trim().to_string());
                }
            }
        }
    }
}
