//! Reading a message's HTML into the layout's small tree: html5ever's tokenizer, with what a
//! browser closes on its own (a cell ends at the next, a row at the next row); sizes, depth and
//! counts capped.

use std::cell::{Cell, RefCell};

use html5ever::TokenizerResult;
use html5ever::tendril::StrTendril;
use html5ever::tokenizer::{BufferQueue, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer, TokenizerOpts};

use super::*;

/// Elements that never have content.
pub(crate) fn void(tag: &str) -> bool {
    matches!(tag, "area" | "base" | "br" | "col" | "embed" | "hr" | "img" | "input" | "link" | "meta" | "source" | "track" | "wbr")
}

/// Builds the tree from tokens.
struct Sink {
    stack: RefCell<Vec<Node>>,
    nodes: Cell<usize>,
    /// The tables Outlook's own copy leaves open are marked ([`crate::html::OUTLOOK_TABLE`]).
    marks: bool,
    /// Just past [`crate::html::OUTLOOK_MARK`]: the next token is the table it marks.
    marked: Cell<bool>,
    /// The tree reads otherwise than the HTML would ([`Tree::reshaped`]).
    reshaped: Cell<bool>,
    /// More elements than are read ([`MAX_NODES`]), or nested deeper ([`MAX_OPEN`]).
    capped: Cell<bool>,
    /// Once capped, the rest isn't read: the tree isn't used ([`parse_as`]).
    stop: bool,
}

/// A message's HTML read into the tree ([`parse_as`]).
pub(super) struct Tree {
    pub(super) root: Node,
    /// What's written from the tree reads otherwise than the HTML it came from: `<plaintext>`, or
    /// Outlook's marks in text.
    pub(super) reshaped: bool,
}

impl Sink {
    fn close_to(&self, stack: &mut Vec<Node>, depth: usize) {
        while stack.len() > depth.max(1) {
            if let Some(n) = stack.pop()
                && let Some(parent) = stack.last_mut()
            {
                parent.kids.push(Kid::El(n));
            }
        }
    }

    /// Close the open elements down to (and including) the nearest `tag`, stopping at `stop`.
    fn close(&self, stack: &mut Vec<Node>, tags: &[&str], stop: &[&str]) {
        let found = stack.iter().rposition(|n| tags.contains(&n.tag.as_str()));
        let barrier = stack.iter().rposition(|n| stop.contains(&n.tag.as_str()));
        if let Some(i) = found
            && barrier.is_none_or(|b| b < i)
        {
            self.close_to(stack, i);
        }
    }

    /// An attribute's value, without Outlook's marks when they're read ([`unmarked`]).
    fn unmarked<'a>(&self, value: &'a str) -> std::borrow::Cow<'a, str> {
        let mark = crate::html::OUTLOOK_MARK;
        match self.marks && value.contains(mark) {
            true => {
                self.reshaped.set(true);
                std::borrow::Cow::Owned(value.replace(mark, ""))
            }
            false => std::borrow::Cow::Borrowed(value),
        }
    }

    /// A cap is reached: when the tree is read no further, the rest is read as text, left out.
    fn capped(&self) -> TokenSinkResult<()> {
        self.capped.set(true);
        match self.stop {
            true => TokenSinkResult::Plaintext,
            false => TokenSinkResult::Continue,
        }
    }
}

impl TokenSink for Sink {
    type Handle = ();

    fn process_token(&self, token: Token, _line: u64) -> TokenSinkResult<()> {
        if self.stop && self.capped.get() {
            return TokenSinkResult::Continue;
        }
        let mut stack = self.stack.borrow_mut();
        // Whether this token follows Outlook's mark (an error read in the table's tag aside).
        let marked = match token {
            Token::ParseError(_) => false,
            _ => self.marked.replace(false),
        };
        match token {
            Token::TagToken(tag) => {
                let name = tag.name.to_string().to_ascii_lowercase();
                match tag.kind {
                    TagKind::StartTag => {
                        self.nodes.set(self.nodes.get().saturating_add(1));
                        if self.nodes.get() > MAX_NODES {
                            return self.capped();
                        }
                        // What a new cell, row or table part implies: the open one ends.
                        match name.as_str() {
                            "td" | "th" => self.close(&mut stack, &["td", "th"], &["tr", "table"]),
                            "tr" => self.close(&mut stack, &["tr"], &["table"]),
                            "tbody" | "thead" | "tfoot" => self.close(&mut stack, &["tbody", "thead", "tfoot"], &["table"]),
                            "p" | "li" => self.close(&mut stack, &[name.as_str()], &["td", "th", "table", "ul", "ol", "div"]),
                            _ => {}
                        }
                        let outlooks = self.marks && marked && name == "table";
                        let mut attrs: Vec<(String, String)> = tag
                            .attrs
                            .iter()
                            .map(|a| (a.name.local.to_string().to_ascii_lowercase(), self.unmarked(&a.value).into_owned()))
                            // Only MailCraft marks Outlook's tables.
                            .filter(|(k, _)| k != crate::html::OUTLOOK_TABLE)
                            .collect();
                        if outlooks {
                            attrs.push((crate::html::OUTLOOK_TABLE.to_string(), String::new()));
                        }
                        let node = Node { tag: name.clone(), attrs, kids: Vec::new() };
                        if void(&name) || tag.self_closing {
                            if let Some(parent) = stack.last_mut() {
                                parent.kids.push(Kid::El(node));
                            }
                        } else if stack.len() < MAX_OPEN {
                            stack.push(node);
                        } else if self.stop {
                            return self.capped();
                        } else {
                            self.capped.set(true);
                        }
                        // Their content is text, not markup: up to their closing tag, or all the
                        // rest for `<plaintext>`, which nothing closes.
                        return match name.as_str() {
                            "style" | "title" | "textarea" | "xmp" | "iframe" | "noembed" | "noframes" => {
                                TokenSinkResult::RawData(html5ever::tokenizer::states::RawKind::Rawtext)
                            }
                            "script" => TokenSinkResult::RawData(html5ever::tokenizer::states::RawKind::ScriptData),
                            "plaintext" => {
                                self.reshaped.set(true);
                                TokenSinkResult::Plaintext
                            }
                            _ => TokenSinkResult::Continue,
                        };
                    }
                    TagKind::EndTag => {
                        let stop: &[&str] = match name.as_str() {
                            "td" | "th" | "tr" | "tbody" | "thead" | "tfoot" => &["table"],
                            "table" => &[],
                            _ => &["td", "th", "table"],
                        };
                        self.close(&mut stack, &[name.as_str()], stop);
                    }
                }
            }
            Token::CommentToken(c) if self.marks => {
                let mark = crate::html::OUTLOOK_MARK.strip_prefix("<!--").and_then(|m| m.strip_suffix("-->"));
                self.marked.set(mark == Some(&*c));
            }
            Token::CharacterTokens(t) => {
                if let Some(top) = stack.last_mut() {
                    match top.kids.last_mut() {
                        Some(Kid::Text(s)) => s.push_str(&t),
                        _ => top.kids.push(Kid::Text(t.to_string())),
                    }
                }
            }
            _ => {}
        }
        TokenSinkResult::Continue
    }
}

/// `html` read into the tree, as far as it's read: past [`MAX_NODES`] elements, or [`MAX_OPEN`]
/// deep, the elements are left out (their text is still read).
pub(super) fn parse(html: &str) -> Node {
    build(html, false, false).0.root
}

/// [`parse`], the tables in Outlook's own copy marked or not (`marks`); `None` when it reaches a
/// cap, and the rest isn't read: a tree that leaves elements out isn't written out again (found
/// in review: the paragraphs past the elements read ran together, and the tables past those
/// nested ever deeper were read a cell to a line).
pub(super) fn parse_as(html: &str, marks: bool) -> Option<Tree> {
    let (tree, capped) = build(html, marks, true);
    (!capped).then_some(tree)
}

/// The tree, and whether it reached a cap (`stop`: the rest isn't read then).
fn build(html: &str, marks: bool, stop: bool) -> (Tree, bool) {
    let sink = Sink {
        stack: RefCell::new(vec![Node { tag: "#root".into(), ..Default::default() }]),
        nodes: Cell::new(0),
        marks,
        marked: Cell::new(false),
        reshaped: Cell::new(false),
        capped: Cell::new(false),
        stop,
    };
    let tok = Tokenizer::new(sink, TokenizerOpts::default());
    let queue = BufferQueue::default();
    queue.push_back(StrTendril::from(html));
    while let TokenizerResult::Script(()) = tok.feed(&queue) {}
    tok.end();
    let mut stack = tok.sink.stack.take();
    tok.sink.close_to(&mut stack, 1);
    let mut root = stack.pop().unwrap_or_default();
    let unmarked = marks && unmarked(&mut root);
    (Tree { root, reshaped: tok.sink.reshaped.get() || unmarked }, tok.sink.capped.get())
}

/// Outlook's marks ([`crate::html::OUTLOOK_MARK`]) in the tree's text: a conditional comment's
/// copy put where what's written is text (a text box's, `<plaintext>`'s), which would show them
/// as words (found in review: a text box showed `<!--mso-->`). Left out, as they are from
/// attributes ([`Sink::unmarked`]). Whether there were any.
fn unmarked(root: &mut Node) -> bool {
    let mark = crate::html::OUTLOOK_MARK;
    let mut any = false;
    let mut todo: Vec<&mut Node> = vec![root];
    while let Some(n) = todo.pop() {
        for k in n.kids.iter_mut() {
            match k {
                Kid::Text(t) if t.contains(mark) => {
                    *t = t.replace(mark, "");
                    any = true;
                }
                Kid::El(e) => todo.push(e),
                Kid::Text(_) => {}
            }
        }
    }
    any
}
