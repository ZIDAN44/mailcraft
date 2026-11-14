//! The search index kept up in the browser ([`Job::Index`]): the desktop's pass
//! ([`search::Pass`]), run a few messages at a time between frames, one account after another.
//! It gives the page back after [`UNITS`] messages or [`BYTES`] of them read, and between merge
//! steps; [`Job::StopIndex`] stops it there, what it added kept. Summaries it finds to say
//! otherwise than the index about a message's files are put right, as on the desktop.

use mailcraft_engine::mime;
use mailcraft_engine::model::{Event, Job};
use mailcraft_engine::store::search::{self, Pass};
use mailcraft_engine::store::{Result as StoreResult, Store, StoreError};

use super::{Rcs, emit, now, sleep, store};
use crate::index_queue::{Next, Queue};

/// Messages read before the page is given back.
const UNITS: usize = 16;
/// Bytes of messages read before the page is given back.
const BYTES: usize = 1 << 20;
/// How long before an index held elsewhere is tried again.
const RETRY_MS: f64 = 5_000.0;
/// The longest the runner waits at once while every account asked for is put off, so one asked
/// for meanwhile soon has its turn.
const WAIT_MS: f64 = 250.0;

/// What's asked of the indexer, and whether its runner is going.
#[derive(Default)]
pub(super) struct Indexing {
    queue: Queue,
    running: bool,
}

/// [`Job::Index`] and [`Job::StopIndex`].
pub(super) fn job(sh: &Rcs, j: Job) {
    let start = {
        let mut s = sh.borrow_mut();
        let ix = &mut s.indexing;
        match j {
            Job::Index { account, folders } => ix.queue.want(account, folders),
            Job::StopIndex { account } => ix.queue.stop(&account),
            _ => {}
        }
        !ix.queue.is_empty() && !std::mem::replace(&mut ix.running, true)
    };
    if start {
        wasm_bindgen_futures::spawn_local(run(sh.clone()));
    }
}

/// Each account asked for, one pass at a time, until none is. One whose index is held elsewhere
/// is put off ([`Queue::put_off`]) while the others go on: waited for in the loop, a stop that
/// came meanwhile was lost (a removed account was indexed again), and every other account waited.
async fn run(sh: Rcs) {
    loop {
        let next = {
            let mut s = sh.borrow_mut();
            let next = s.indexing.queue.next(js_sys::Date::now());
            if next == Next::Idle {
                s.indexing.running = false;
            }
            next
        };
        let (account, folders) = match next {
            Next::Idle => return,
            Next::Wait(ms) => {
                sleep(ms.clamp(1.0, WAIT_MS) as i32).await;
                continue;
            }
            Next::Run(account, folders) => (account, folders),
        };
        let seen = sh.borrow().indexing.queue.stops(&account);
        match pass(&sh, &account, &folders).await {
            Ok(()) => sh.borrow_mut().indexing.queue.ended_well(&account),
            Err(StoreError::Busy { .. }) => {
                let at = js_sys::Date::now() + RETRY_MS;
                sh.borrow_mut().indexing.queue.put_off(account, folders, at, seen);
            }
            Err(e) if search::is_damaged(&e) => {
                log::warn!("{account}: the search index is made again: {e}");
                match search::set_aside(&store(&sh), &account) {
                    Ok(()) => sh.borrow_mut().indexing.queue.remake(account, seen),
                    Err(e) => log::warn!("{account}: {e}"),
                }
            }
            Err(e) => log::warn!("{account}: the search index wasn't brought up to the mail: {e}"),
        }
    }
}

fn indexed(account: &str, p: &Pass, finished: bool) -> Event {
    Event::Indexed { account: account.to_string(), work: p.work(), generation: p.generation(), failed: p.failed(), finished }
}

/// One pass over `account`'s index, the page given back as it goes; its end is said however it
/// ends, once it has started.
async fn pass(sh: &Rcs, account: &str, folders: &[String]) -> StoreResult<()> {
    let store = store(sh);
    let mut p = Pass::start(&store, account, folders)?;
    emit(sh, indexed(account, &p, false));
    settle_files(sh, &store, account, &mut p);
    let ended = walk(sh, &store, account, &mut p).await;
    settle_files(sh, &store, account, &mut p);
    emit(sh, indexed(account, &p, true));
    ended
}

/// Put right in their folders' lists the summaries the pass found to say otherwise than the
/// index about carrying files, and tell the window of each folder changed. A folder put right,
/// or that couldn't be, isn't marked: the next pass looks at it again ([`Pass::unmark`]).
fn settle_files(sh: &Rcs, store: &Store, account: &str, p: &mut Pass) {
    for (folder, found) in p.take_files() {
        match store.set_has_files(account, &folder, &found) {
            Ok(0) => {}
            Ok(_) => {
                p.unmark(&folder);
                emit(sh, Event::AttachmentsKnown { account: account.to_string(), folder, with: found.with, without: found.without });
            }
            Err(e) => {
                log::warn!("{account}: whether {} messages carry files wasn't kept: {e}", found.with.len().saturating_add(found.without.len()));
                p.unmark(&folder);
            }
        }
    }
}

/// A pass's messages read and added, then written and merged, until a stop.
async fn walk(sh: &Rcs, store: &Store, account: &str, p: &mut Pass) -> StoreResult<()> {
    let stops = |sh: &Rcs| sh.borrow().indexing.queue.stops(account);
    let began = stops(sh);
    let stopped = |sh: &Rcs| stops(sh) != began;
    let (mut units, mut bytes) = (0usize, 0usize);
    let mut i = 0;
    while i < p.adds().len() && !stopped(sh) {
        let doc = p.adds().get(i).and_then(|m| store.read_body(&m.key)).map(|raw| {
            bytes = bytes.saturating_add(raw.len());
            mime::index_doc(&raw)
        });
        p.add(store, i, doc.as_ref())?;
        (i, units) = (i + 1, units + 1);
        if units >= UNITS || bytes >= BYTES {
            emit(sh, indexed(account, p, false));
            sleep(0).await;
            (units, bytes) = (0, 0);
        }
    }
    let complete = !stopped(sh) && i >= p.adds().len();
    // Put right before the folders are marked, so one that couldn't be isn't.
    settle_files(sh, store, account, p);
    p.finish(store, complete)?;
    let at = u64::try_from(now()).unwrap_or(0);
    while complete && !stopped(sh) && p.merge_step(store, at)? {
        sleep(0).await;
    }
    Ok(())
}
