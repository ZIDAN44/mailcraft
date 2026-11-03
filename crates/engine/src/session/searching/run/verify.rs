//! Search: a run's candidates checked against their messages' texts as the index keeps them, a
//! slice a unit, or (waiting) against their sources.

use super::super::matching::NoText;
use super::*;

impl Session {
    /// One unit of checking a message against the text the index keeps for it.
    pub(super) fn verify_unit(&mut self, run: &mut SearchRun) -> u32 {
        self.search_state.stats.texts = self.search_state.stats.texts.saturating_add(1);
        let mut v = match run.verifying.take() {
            Some(v) => v,
            None => {
                if !run.verify_sorted {
                    // Checked in the folders' order, a message after another read a page of texts
                    // afresh (a common phrase over 50,000 messages: 1,015 pages read, each 256 KiB
                    // unpacked); in the docs' order each page is read once (103). The folders on
                    // view still come first, as they're scanned first: what's on screen is found
                    // first. (What's sorted already stays a run the sort takes as it is.)
                    let view = run.folders.get(..run.on_view).unwrap_or_default().to_vec();
                    let off = |c: &Candidate| !view.iter().any(|(a, f)| *a == c.m.key.account && *f == c.m.key.folder);
                    run.verify
                        .make_contiguous()
                        .sort_by(|a, b| off(a).cmp(&off(b)).then(a.m.key.account.cmp(&b.m.key.account)).then(a.doc.cmp(&b.doc)));
                    run.verify_sorted = true;
                }
                let Some(c) = run.verify.pop_front() else { return 0 };
                let reader = self.search_state.readers.get(&c.m.key.account).and_then(|s| s.reader.as_ref());
                match (c.doc, reader) {
                    (Some(doc), Some(r)) if r.info(doc).is_some() => {
                        let info = r.info(doc).unwrap_or_default();
                        // Its summary's text as the scan made it (folding it again, twice, for
                        // each of a common phrase's candidates was most of checking them).
                        let st = match c.at.and_then(|at| self.search_state.texts.kept(&c.m.key, at)) {
                            Some(st) => st.clone(),
                            None => SummaryText::for_query(&c.m, &run.query),
                        };
                        let of = TextOf { epoch: r.epoch(), doc };
                        let kept = self.search_state.folded.get(&c.m.key, of);
                        Verifying::new(&run.query, c, st, of, info, kept)
                    }
                    // Its index went meanwhile: it may be missing.
                    _ => {
                        run.record(c.fi, c.m, Verdict::Missing, c.awaiting, true);
                        return 1;
                    }
                }
            }
        };
        let account = v.c.m.key.account.clone();
        let (Some(store), Some(reader)) = (self.store.clone(), self.search_state.readers.get_mut(&account).and_then(|s| s.reader.as_mut())) else {
            run.record(v.c.fi, v.c.m, Verdict::Missing, v.c.awaiting, true);
            return 1;
        };
        let (before, texts) = (reader.pages_read(), reader.text_pages_read());
        let said = v.step(reader, &store);
        let read = reader.pages_read().saturating_sub(before);
        let stats = &mut self.search_state.stats;
        stats.reader_pages = stats.reader_pages.saturating_add(read);
        stats.text_pages = stats.text_pages.saturating_add(reader.text_pages_read().saturating_sub(texts));
        match said {
            None => run.verifying = Some(v),
            Some(answers) => {
                if let Some(folded) = v.take_folded() {
                    self.search_state.folded.keep(&v.c.m.key, v.text_of(), folded);
                }
                let (c, st) = (v.c, v.st);
                // A long message's stays cut short whenever it's weighed again from these.
                let verdict = classify(&run.query, &c.m, &st, c.awaiting, None, Some(&answers));
                if answers.said() {
                    run.known.insert(RunKey::of(c.fi, &c.m.key), answers);
                }
                run.record(c.fi, c.m, verdict, c.awaiting, true);
            }
        }
        1
    }

    /// Waiting: one message the index doesn't have read from its source.
    pub(super) fn fallback_unit(&mut self, run: &mut SearchRun) -> u32 {
        let Some(c) = run.fallback.pop_front() else { return 0 };
        let st = SummaryText::for_query(&c.m, &run.query);
        let atoms = Recorder::atoms(&run.query, &c.m, &st, c.awaiting);
        let raw = self.store.as_ref().and_then(|s| s.read_body(&c.m.key));
        self.search_state.stats.sources = self.search_state.stats.sources.saturating_add(1);
        let verdict = match raw {
            Some(raw) => {
                let answers = from_source(&raw, &atoms);
                let verdict = classify(&run.query, &c.m, &st, c.awaiting, None, Some(&answers));
                run.known.insert(RunKey::of(c.fi, &c.m.key), answers);
                verdict
            }
            // Not kept here (only its headers came): its summary alone is searched, for now
            // (what it said isn't kept: its text may come).
            None => {
                let doc = Doc { summary: &c.m, summary_text: &st, awaiting: c.awaiting, files: None, text: &NoText };
                match run.query.eval(&doc) {
                    Tri::Yes => Verdict::Yes,
                    Tri::No => Verdict::No,
                    Tri::Unknown => Verdict::Missing,
                }
            }
        };
        run.record(c.fi, c.m, verdict, c.awaiting, true);
        1
    }
}
