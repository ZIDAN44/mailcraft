//! Search: the end of a run, the list put together as it's to stay a unit at a time.
//!
//! A run whose finds streamed into the list ([`SearchRun::owns_list`]) has them there already,
//! newest first, each change since merged in: what's left to show joins them with what the
//! servers found, and the list is arranged otherwise or put into conversations only when it
//! shows them so. Gathering the finds again from the folders copied every one of them (and let
//! go of as many): tens of milliseconds a unit for a search finding tens of thousands. A run
//! that waited (or one started over a list it doesn't own) gathers them from the folders.

use super::run::SearchRun;
use super::*;

/// The end of a run: what's found, put together a step at a time.
#[derive(Debug, Default)]
pub(crate) enum Finish {
    #[default]
    Due,
    Staged(Vec<SharedSummary>),
    Arranged(Vec<SharedSummary>),
    Done,
}

impl Session {
    /// One step of putting the list together: what's found gathered and filtered, arranged,
    /// then shown with its conversations.
    pub(super) fn finish_unit(&mut self, run: &mut SearchRun) -> u32 {
        run.finish = match std::mem::take(&mut run.finish) {
            Finish::Due if run.owns_list && !run.wait => {
                self.finish_streamed(run);
                Finish::Done
            }
            Finish::Due => {
                let list = self.search_list(run);
                Finish::Staged(self.filtered(list))
            }
            Finish::Staged(mut list) => {
                super::super::mailboxes::arrange_list(&mut list, self.prefs.arrange, self.prefs.reverse_sort);
                Finish::Arranged(list)
            }
            Finish::Arranged(list) => {
                self.keep_senders();
                self.take_list(list);
                run.fresh.clear();
                run.gone.clear();
                run.owns_list = true;
                Finish::Done
            }
            Finish::Done => Finish::Done,
        };
        1
    }

    /// The end of a run that streamed into the list: what's left and what the servers found
    /// merged in, then arranged as the list is arranged (by date it already is) and put into
    /// conversations when they're shown.
    fn finish_streamed(&mut self, run: &mut SearchRun) {
        let hits = self.server_found(run);
        run.fresh.extend(hits.into_iter().map(SharedSummary::new));
        if !run.fresh.is_empty() || !run.gone.is_empty() {
            self.publish_stream(run);
        }
        run.unshown_steps = 0;
        let arrange = self.prefs.arrange;
        if arrange != Arrange::Date || self.prefs.conversations {
            let mut list = std::mem::take(&mut self.list);
            if arrange != Arrange::Date {
                super::super::mailboxes::arrange_list(&mut list, arrange, self.prefs.reverse_sort);
            }
            self.take_list(list);
        }
        self.keep_senders();
    }
}
