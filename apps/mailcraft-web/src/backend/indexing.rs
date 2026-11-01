//! The search index kept up in the browser ([`Job::Index`]): the desktop's pass
//! ([`search::Pass`]), run a few messages at a time between frames, one account after another.
//! It gives the page back after [`UNITS`] messages or [`BYTES`] of them read, and between merge
//! steps; [`Job::StopIndex`] stops it there, what it added kept. Summaries it finds to say
//! otherwise than the index about a message's files are put right, as on the desktop.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use mailcraft_engine::mime;
use mailcraft_engine::model::{Event, Job};
use mailcraft_engine::store::search::{self, Pass};
use mailcraft_engine::store::{Result as StoreResult, Store, StoreError};

use super::{Rcs, emit, now, sleep, store};

/// Messages read before the page is given back.
const UNITS: usize = 16;
/// Bytes of messages read before the page is given back.
const BYTES: usize = 1 << 20;
/// How long before an index held elsewhere is tried again.
const RETRY_MS: i32 = 5_000;

/// What's asked of the indexer.
#[derive(Default)]
pub(super) struct Indexing {
    /// Each account asked for and its folders (`None`: every folder).
    pending: BTreeMap<String, Option<BTreeSet<String>>>,
    running: bool,
    /// Bumped by a stop: a pass that sees its account's change stops.
    stops: HashMap<String, u64>,
}

/// [`Job::Index`] and [`Job::StopIndex`].
pub(super) fn job(sh: &Rcs, j: Job) {
    let start = {
        let mut s = sh.borrow_mut();
        let ix = &mut s.indexing;
        match j {
            Job::Index { account, folders } => want(&mut ix.pending, account, folders),
            Job::StopIndex { account } => {
                ix.pending.remove(&account);
                let n = ix.stops.entry(account).or_default();
                *n = n.wrapping_add(1);
            }
            _ => {}
        }
        !ix.pending.is_empty() && !std::mem::replace(&mut ix.running, true)
    };
    if start {
        wasm_bindgen_futures::spawn_local(run(sh.clone()));
    }
}

/// Ask for `folders` of `account` (none: every folder) as well as what's asked of it already.
fn want(pending: &mut BTreeMap<String, Option<BTreeSet<String>>>, account: String, folders: Vec<String>) {
    let e = pending.entry(account).or_insert_with(|| Some(BTreeSet::new()));
    match (e.as_mut(), folders.is_empty()) {
        (_, true) => *e = None,
        (Some(set), false) => set.extend(folders),
        (None, false) => {}
    }
}

/// Each account asked for, one pass at a time, until none is.
async fn run(sh: Rcs) {
    loop {
        let next = {
            let mut s = sh.borrow_mut();
            let next = s.indexing.pending.pop_first();
            if next.is_none() {
                s.indexing.running = false;
            }
            next
        };
        let Some((account, folders)) = next else { return };
        let folders: Vec<String> = folders.map(|f| f.into_iter().collect()).unwrap_or_default();
        match pass(&sh, &account, &folders).await {
            Ok(()) => {}
            Err(StoreError::Busy { .. }) => {
                sleep(RETRY_MS).await;
                want(&mut sh.borrow_mut().indexing.pending, account, folders);
            }
            Err(e) if search::is_damaged(&e) => {
                log::warn!("{account}: the search index is made again: {e}");
                if let Err(e) = search::set_aside(&store(&sh), &account) {
                    log::warn!("{account}: {e}");
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
/// index about carrying files, and tell the window of each folder changed.
fn settle_files(sh: &Rcs, store: &Store, account: &str, p: &mut Pass) {
    for (folder, found) in p.take_files() {
        match store.set_has_files(account, &folder, &found) {
            Ok(0) => {}
            Ok(_) => emit(sh, Event::AttachmentsKnown { account: account.to_string(), folder, with: found.with, without: found.without }),
            Err(e) => log::warn!("{account}: whether {} messages carry files wasn't kept: {e}", found.with.len().saturating_add(found.without.len())),
        }
    }
}

/// A pass's messages read and added, then written and merged, until a stop.
async fn walk(sh: &Rcs, store: &Store, account: &str, p: &mut Pass) -> StoreResult<()> {
    let stops = |sh: &Rcs| sh.borrow().indexing.stops.get(account).copied().unwrap_or(0);
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
    p.finish(store, complete)?;
    let at = u64::try_from(now()).unwrap_or(0);
    while complete && !stopped(sh) && p.merge_step(store, at)? {
        sleep(0).await;
    }
    Ok(())
}
