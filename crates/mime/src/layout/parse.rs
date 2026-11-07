//! Reading a message's HTML into the layout's small tree: html5ever's tokenizer, with what a
//! browser closes on its own (a cell ends at the next, a row at the next row); sizes, depth and
//! counts capped.

use std::cell::RefCell;

use html5ever::TokenizerResult;
use html5ever::tendril::StrTendril;
use html5ever::tokenizer::{BufferQueue, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer, TokenizerOpts};

use super::*;

/// Elements that never have content.
pub(super) fn void(tag: &str) -> bool {
    matches!(tag, "area" | "base" | "br" | "col" | "embed" | "hr" | "img" | "input" | "link" | "meta" | "source" | "track" | "wbr")
}

/// Builds the tree from tokens.
struct Sink {
    stack: RefCell<Vec<Node>>,
    nodes: RefCell<usize>,
}

impl Sink {
    fn close_to(stack: &mut Vec<Node>, depth: usize) {
        while stack.len() > depth.max(1) {
            if let Some(n) = stack.pop()
                && let Some(parent) = stack.last_mut()
            {
                parent.kids.push(Kid::El(n));
            }
        }
    }

    /// Close the open elements down to (and including) the nearest `tag`, stopping at `stop`.
    fn close(stack: &mut Vec<Node>, tags: &[&str], stop: &[&str]) {
        let found = stack.iter().rposition(|n| tags.contains(&n.tag.as_str()));
        let barrier = stack.iter().rposition(|n| stop.contains(&n.tag.as_str()));
        if let Some(i) = found
            && barrier.is_none_or(|b| b < i)
        {
            Sink::close_to(stack, i);
        }
    }
}

impl TokenSink for Sink {
    type Handle = ();

    fn process_token(&self, token: Token, _line: u64) -> TokenSinkResult<()> {
        let mut stack = self.stack.borrow_mut();
        match token {
            Token::TagToken(tag) => {
                let name = tag.name.to_string().to_ascii_lowercase();
                match tag.kind {
                    TagKind::StartTag => {
                        let mut count = self.nodes.borrow_mut();
                        *count += 1;
                        if *count > MAX_NODES {
                            return TokenSinkResult::Continue;
                        }
                        // What a new cell, row or table part implies: the open one ends.
                        match name.as_str() {
                            "td" | "th" => Sink::close(&mut stack, &["td", "th"], &["tr", "table"]),
                            "tr" => Sink::close(&mut stack, &["tr"], &["table"]),
                            "tbody" | "thead" | "tfoot" => Sink::close(&mut stack, &["tbody", "thead", "tfoot"], &["table"]),
                            "p" | "li" => Sink::close(&mut stack, &[name.as_str()], &["td", "th", "table", "ul", "ol", "div"]),
                            _ => {}
                        }
                        let node = Node {
                            tag: name.clone(),
                            attrs: tag.attrs.iter().map(|a| (a.name.local.to_string().to_ascii_lowercase(), a.value.to_string())).collect(),
                            kids: Vec::new(),
                        };
                        if void(&name) || tag.self_closing {
                            if let Some(parent) = stack.last_mut() {
                                parent.kids.push(Kid::El(node));
                            }
                        } else if stack.len() < MAX_OPEN {
                            stack.push(node);
                        }
                        // Their content is text, not markup.
                        return match name.as_str() {
                            "style" | "title" | "textarea" | "xmp" | "iframe" | "noembed" | "noframes" => {
                                TokenSinkResult::RawData(html5ever::tokenizer::states::RawKind::Rawtext)
                            }
                            "script" => TokenSinkResult::RawData(html5ever::tokenizer::states::RawKind::ScriptData),
                            _ => TokenSinkResult::Continue,
                        };
                    }
                    TagKind::EndTag => {
                        let stop: &[&str] = match name.as_str() {
                            "td" | "th" | "tr" | "tbody" | "thead" | "tfoot" => &["table"],
                            "table" => &[],
                            _ => &["td", "th", "table"],
                        };
                        Sink::close(&mut stack, &[name.as_str()], stop);
                    }
                }
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

pub(super) fn parse(html: &str) -> Node {
    let sink = Sink { stack: RefCell::new(vec![Node { tag: "#root".into(), ..Default::default() }]), nodes: RefCell::new(0) };
    let tok = Tokenizer::new(sink, TokenizerOpts::default());
    let queue = BufferQueue::default();
    queue.push_back(StrTendril::from(html));
    while let TokenizerResult::Script(()) = tok.feed(&queue) {}
    tok.end();
    let mut stack = tok.sink.stack.take();
    Sink::close_to(&mut stack, 1);
    stack.pop().unwrap_or_default()
}
