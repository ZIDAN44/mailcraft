//! The order of the browser's index passes (`backend/indexing.rs`): each account asked for, one
//! at a time. One whose index was held elsewhere is put off a while, without holding up the
//! others, and a stop takes an account out whether it waits its turn or is put off.
#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

use std::collections::{BTreeMap, BTreeSet, HashMap};

/// What's asked of the indexer.
#[derive(Default)]
pub struct Queue {
    /// Each account asked for and its folders (`None`: every folder).
    pending: BTreeMap<String, Option<BTreeSet<String>>>,
    /// An account put off (its index held elsewhere): not run before this time, in ms.
    not_before: HashMap<String, f64>,
    /// Bumped by a stop: a pass that sees its account's change stops.
    stops: HashMap<String, u64>,
    /// Accounts whose index was found damaged and is being made again ([`Queue::remake`]).
    remade: BTreeSet<String>,
}

/// What the indexer does next.
#[derive(Debug, PartialEq)]
pub enum Next {
    /// A pass over this account's folders (none: every folder).
    Run(String, Vec<String>),
    /// Nothing until then: every account asked for is put off (ms from now).
    Wait(f64),
    /// Nothing asked for.
    Idle,
}

impl Queue {
    /// Ask for `folders` of `account` (none: every folder) as well as what's asked of it already.
    pub fn want(&mut self, account: String, folders: Vec<String>) {
        let e = self.pending.entry(account).or_insert_with(|| Some(BTreeSet::new()));
        match (e.as_mut(), folders.is_empty()) {
            (_, true) => *e = None,
            (Some(set), false) => set.extend(folders),
            (None, false) => {}
        }
    }

    /// Stop `account`'s pass (where it is, what it added kept) and take it out of the queue.
    pub fn stop(&mut self, account: &str) {
        self.pending.remove(account);
        self.not_before.remove(account);
        let n = self.stops.entry(account.to_string()).or_default();
        *n = n.wrapping_add(1);
    }

    /// How many stops `account` has had: a pass notes it as it begins.
    pub fn stops(&self, account: &str) -> u64 {
        self.stops.get(account).copied().unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// `account`'s pass found its index held elsewhere: asked for again with `folders`, not
    /// before `at` (ms). Not when it was stopped since its pass began (`seen` stops then): asked
    /// for again regardless, a stop that came meanwhile was lost, and a removed account indexed.
    pub fn put_off(&mut self, account: String, folders: Vec<String>, at: f64, seen: u64) {
        if self.stops(&account) != seen {
            return;
        }
        self.not_before.insert(account.clone(), at);
        self.want(account, folders);
    }

    /// `account`'s pass found its index damaged, and it was set aside: made again over every
    /// folder at once, not left empty till the next ask, which may name one folder (found in
    /// review). Once in a row ([`Queue::ended_well`]): one damaged again as it's made again isn't
    /// made again and again. Not when it was stopped since its pass began (`seen` stops then).
    pub fn remake(&mut self, account: String, seen: u64) {
        if self.stops(&account) == seen && self.remade.insert(account.clone()) {
            self.want(account, Vec::new());
        }
    }

    /// `account`'s pass ended well: one found damaged later is made again.
    pub fn ended_well(&mut self, account: &str) {
        self.remade.remove(account);
    }

    /// What to do at `now` (ms): the first account asked for that isn't put off, else how long
    /// until one is.
    pub fn next(&mut self, now: f64) -> Next {
        let ready = self.pending.keys().find(|a| self.not_before.get(*a).is_none_or(|t| *t <= now)).cloned();
        if let Some(account) = ready {
            self.not_before.remove(&account);
            let folders = self.pending.remove(&account).flatten().map(|f| f.into_iter().collect()).unwrap_or_default();
            return Next::Run(account, folders);
        }
        let soonest = self.pending.keys().filter_map(|a| self.not_before.get(a)).copied().reduce(f64::min);
        soonest.map_or(Next::Idle, |t| Next::Wait((t - now).max(0.0)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(account: &str, folders: &[&str]) -> Next {
        Next::Run(account.into(), folders.iter().map(|f| f.to_string()).collect())
    }

    /// A busy index is put off without holding up the others, and runs once its time comes.
    #[test]
    fn a_busy_account_waits_behind_the_others() {
        let mut q = Queue::default();
        q.want("a1".into(), Vec::new());
        q.want("a2".into(), vec!["INBOX".into()]);
        assert_eq!(q.next(0.0), run("a1", &[]));
        let seen = q.stops("a1");
        q.put_off("a1".into(), Vec::new(), 5_000.0, seen);
        assert_eq!(q.next(10.0), run("a2", &["INBOX"]), "not held up");
        assert_eq!(q.next(20.0), Next::Wait(4_980.0));
        // Asked again while put off: still put off, its folders gathered.
        q.want("a1".into(), Vec::new());
        assert_eq!(q.next(30.0), Next::Wait(4_970.0));
        assert_eq!(q.next(5_000.0), run("a1", &[]));
        assert_eq!(q.next(5_001.0), Next::Idle);
        assert!(q.is_empty());
    }

    /// A stop while put off, or while the pass that found the index busy ran, is kept: the
    /// account isn't indexed again (a removed account's stop came this way).
    #[test]
    fn a_stop_is_kept_whenever_it_comes() {
        let mut q = Queue::default();
        q.want("a1".into(), Vec::new());
        assert_eq!(q.next(0.0), run("a1", &[]));
        let seen = q.stops("a1");
        q.put_off("a1".into(), Vec::new(), 5_000.0, seen);
        q.stop("a1");
        assert_eq!(q.next(6_000.0), Next::Idle, "stopped while put off");
        q.want("a1".into(), vec!["INBOX".into()]);
        assert_eq!(q.next(6_000.0), run("a1", &["INBOX"]), "asked for afresh");
        let seen = q.stops("a1");
        q.stop("a1");
        q.put_off("a1".into(), vec!["INBOX".into()], 11_000.0, seen);
        assert_eq!(q.next(12_000.0), Next::Idle, "stopped while its pass ran");
    }

    /// Found in review: an index found damaged was set aside and nothing more asked. It's made
    /// again over every folder at once; once in a row, till a pass ends well; not once stopped.
    #[test]
    fn a_damaged_index_is_made_again_once_in_a_row() {
        let mut q = Queue::default();
        q.want("a1".into(), vec!["INBOX".into()]);
        assert_eq!(q.next(0.0), run("a1", &["INBOX"]));
        q.remake("a1".into(), q.stops("a1"));
        assert_eq!(q.next(0.0), run("a1", &[]), "every folder");
        q.remake("a1".into(), q.stops("a1"));
        assert_eq!(q.next(0.0), Next::Idle, "damaged again: not again and again");
        q.ended_well("a1");
        let seen = q.stops("a1");
        q.stop("a1");
        q.remake("a1".into(), seen);
        assert_eq!(q.next(0.0), Next::Idle, "stopped while its pass ran");
        q.remake("a1".into(), q.stops("a1"));
        assert_eq!(q.next(0.0), run("a1", &[]));
    }
}
