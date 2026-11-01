//! An inverted index held in memory: each folded word's documents.

use std::collections::{BTreeMap, BTreeSet};

use super::{Query, index_tokens};

/// Words → documents. Document ids are the caller's (positions in its message table).
#[derive(Clone, Debug, Default)]
pub struct Index {
    /// Each word's documents, sorted (an array: a set per word took several times the memory,
    /// gigabytes for a big mailbox).
    postings: BTreeMap<String, Vec<u32>>,
    docs: BTreeSet<u32>,
}

impl Index {
    pub fn new() -> Index {
        Index::default()
    }

    pub fn len(&self) -> usize {
        self.docs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.docs.is_empty()
    }

    /// Add a document's text (call again with more text, e.g. the body once fetched). Its words
    /// are [folded](super::fold_word), with their [parts](super::parts), and long ones (base64
    /// debris) left out: they only bloat it.
    pub fn add(&mut self, doc: u32, text: &str) {
        self.docs.insert(doc);
        for w in index_tokens(text) {
            let list = self.postings.entry(w.into_owned()).or_default();
            // Documents mostly come in order: appended; one added later goes in its place.
            match list.last() {
                Some(&last) if last == doc => {}
                Some(&last) if last > doc => {
                    if let Err(at) = list.binary_search(&doc) {
                        list.insert(at, doc);
                    }
                }
                _ => list.push(doc),
            }
        }
    }

    /// Documents containing a word starting with `prefix` (folded).
    pub fn prefix(&self, prefix: &str) -> BTreeSet<u32> {
        let mut out = BTreeSet::new();
        for (_, docs) in self.postings.range(prefix.to_string()..).take_while(|(k, _)| k.starts_with(prefix)) {
            out.extend(docs.iter().copied());
        }
        out
    }

    /// Candidates for a query: documents with every index word as a prefix of some word, or all
    /// documents when the query has no words (flag-only queries).
    pub fn candidates(&self, q: &Query) -> BTreeSet<u32> {
        let ws = q.index_words();
        if ws.is_empty() {
            return self.docs.clone();
        }
        let mut acc: Option<BTreeSet<u32>> = None;
        for w in ws {
            let hits = self.prefix(&w);
            acc = Some(match acc {
                None => hits,
                Some(a) => a.intersection(&hits).copied().collect(),
            });
            if acc.as_ref().is_some_and(BTreeSet::is_empty) {
                break;
            }
        }
        acc.unwrap_or_default()
    }
}
