//! Waiting in the tests: for the pool to say something, or for something to be so. Every wait
//! has the one deadline, a guard against hanging and never a measure: a wait that passes it fails
//! the test, saying what it waited for and what the pool said meanwhile.

use std::time::{Duration, Instant};

use mailcraft_model::Event;

use crate::worker::Pool;

/// How long a wait goes on before it fails the test: long, as a busy computer (a dozen builds at
/// once, a slow VM) takes its time. It only ever ends a test that would hang otherwise.
pub(crate) const DEADLINE: Duration = Duration::from_secs(60);

/// How often a condition is looked at again.
const EVERY: Duration = Duration::from_millis(20);

/// How many of the events seen a wait that failed shows: the last ones.
const SHOWN: usize = 30;

/// Every event the pool says until `f` takes one: all of them, that one last.
pub(crate) fn until(pool: &Pool, what: &str, f: impl FnMut(&Event) -> bool) -> Vec<Event> {
    until_within(pool, DEADLINE, what, f)
}

/// [`until`], with a deadline of its own (a real server's long work, run by hand).
pub(crate) fn until_within(pool: &Pool, deadline: Duration, what: &str, mut f: impl FnMut(&Event) -> bool) -> Vec<Event> {
    let end = Instant::now() + deadline;
    let mut seen = Vec::new();
    loop {
        let left = end.saturating_duration_since(Instant::now());
        if left.is_zero() {
            timed_out(deadline, what, &seen);
        }
        if let Some(e) = pool.wait(left.min(Duration::from_millis(100))) {
            let hit = f(&e);
            seen.push(e);
            if hit {
                return seen;
            }
        }
    }
}

/// Every event the pool says until `done` is so (looked at between them, given the events so far).
pub(crate) fn events_until(pool: &Pool, what: &str, done: impl FnMut(&[Event]) -> bool) -> Vec<Event> {
    events_until_within(pool, DEADLINE, what, done)
}

/// [`events_until`], with a deadline of its own (a real server's long work, run by hand).
pub(crate) fn events_until_within(pool: &Pool, deadline: Duration, what: &str, mut done: impl FnMut(&[Event]) -> bool) -> Vec<Event> {
    let end = Instant::now() + deadline;
    let mut seen = Vec::new();
    while !done(&seen) {
        if Instant::now() >= end {
            timed_out(deadline, what, &seen);
        }
        seen.extend(pool.wait(EVERY));
    }
    seen
}

/// Every event the pool says until `account`'s thread waits (for a job, for mail, or out a wait
/// of its own), and what was said by then: what it was doing is all done, and nothing more is
/// said until something wakes it. For what must not be said after what a test waited for.
pub(crate) fn until_idle(pool: &Pool, account: &str) -> Vec<Event> {
    let mut seen = events_until(pool, "the account's thread to wait", |_| pool.idle(account));
    seen.extend(std::iter::from_fn(|| pool.wait(Duration::ZERO)));
    seen
}

/// Wait until `done` is so, with no pool to listen to (or its events not wanted).
pub(crate) fn eventually(what: &str, done: impl FnMut() -> bool) {
    eventually_within(DEADLINE, what, done);
}

/// [`eventually`], with a deadline of its own (a real server's long work, a measurement).
pub(crate) fn eventually_within(deadline: Duration, what: &str, mut done: impl FnMut() -> bool) {
    let end = Instant::now() + deadline;
    while !done() {
        if Instant::now() >= end {
            timed_out(deadline, what, &[]);
        }
        std::thread::sleep(EVERY);
    }
}

fn timed_out(deadline: Duration, what: &str, seen: &[Event]) -> ! {
    let last = &seen[seen.len().saturating_sub(SHOWN)..];
    panic!("timed out after {deadline:?} waiting for {what}; the last {} of {} events said: {last:#?}", last.len(), seen.len());
}

/// Each kind of wait: past its deadline it fails, saying what it waited for; one whose
/// condition is so ends at once, with what was said.
#[test]
fn a_wait_past_its_deadline_fails_saying_what_it_waited_for() {
    let pool = Pool::new(mailcraft_store::Store::memory(), std::sync::Arc::new(crate::worker::MemoryCredentials::default()));
    let short = Duration::from_millis(50);
    let message = |wait: &dyn Fn()| {
        let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(wait));
        failed.err().and_then(|p| p.downcast::<String>().ok()).map(|s| *s).unwrap_or_default()
    };
    let said = message(&|| drop(until_within(&pool, short, "the moon", |_| false)));
    assert!(said.contains("timed out after 50ms waiting for the moon") && said.contains("the last 0 of 0 events"), "{said}");
    let said = message(&|| drop(events_until_within(&pool, short, "the tide", |_| false)));
    assert!(said.contains("timed out after 50ms waiting for the tide"), "{said}");
    let said = message(&|| eventually_within(short, "the stars", || false));
    assert!(said.contains("timed out after 50ms waiting for the stars"), "{said}");
    // So already: no wait, and nothing fails.
    let mut looks = 0;
    eventually("nothing", || {
        looks += 1;
        true
    });
    assert_eq!(looks, 1);
    assert!(events_until(&pool, "nothing", |seen| seen.is_empty()).is_empty());
}
