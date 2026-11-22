//! Search for AI agents (issue #255, their `search_mail`): the query as the search box reads it, over
//! the folders' lists and the text of the messages downloaded, apart from the search box's own
//! search: the window's list, selection, reading pane and search stay as they are. What a
//! message's summary settles is settled from it; what needs its words is read from its text, the
//! newest first, as many as [`TEXTS_READ`].

use std::cmp::Reverse;

use mailcraft_model::search::{Doc, SummaryText, Tri, WholeText};
use mailcraft_model::summary::SharedSummary;

use super::*;

/// The most messages whose text one search reads: in the open window, it reads them on the
/// window's own thread.
pub(crate) const TEXTS_READ: usize = 1_000;

/// What a search for agents found.
pub(crate) struct AgentFound {
    /// The newest first.
    pub hits: Vec<SharedSummary>,
    /// Messages whose text settled it (and those it would have, past [`TEXTS_READ`]).
    pub texts_read: usize,
    pub texts_left: usize,
}

impl Session {
    /// Search `folders` (account, path) for `query`, as the search box does.
    pub(crate) fn agent_search_in(&mut self, query: &str, folders: &[(String, String)]) -> AgentFound {
        let q = self.query_of(query);
        let mut hits: Vec<SharedSummary> = Vec::new();
        let mut unsure: Vec<SharedSummary> = Vec::new();
        for (a, f) in folders {
            for m in self.folder_state(a, f).messages.iter().filter(|m| !m.flags.deleted) {
                let st = SummaryText::for_query(m, &q);
                match q.eval(&Doc { summary: m, summary_text: &st, awaiting: false, files: None, text: &matching::Unindexed }) {
                    Tri::Yes => hits.push(m.clone()),
                    Tri::Unknown => unsure.push(m.clone()),
                    Tri::No => {}
                }
            }
        }
        unsure.sort_by_key(|m| Reverse(m.date));
        let texts_left = unsure.len().saturating_sub(TEXTS_READ);
        let mut texts_read = 0;
        for m in unsure.into_iter().take(TEXTS_READ) {
            let Some(text) = self.body_raw(&m.key).and_then(|raw| mailcraft_mime::parse(&raw)).map(|p| p.text) else { continue };
            texts_read += 1;
            let st = SummaryText::for_query(&m, &q);
            if q.matches(&Doc { summary: &m, summary_text: &st, awaiting: false, files: None, text: &WholeText::new(&text) }) {
                hits.push(m);
            }
        }
        hits.sort_by_key(|m| Reverse(m.date));
        AgentFound { hits, texts_read, texts_left }
    }
}
