//! The writer: what the planner and the lanes send, applied in the order the planner numbered
//! it, whichever lane finished first. Messages get their numbers here, in that order, so an
//! import comes out the same however many lanes read it.

use std::sync::mpsc::{Receiver, RecvTimeoutError};

use super::lanes::{Ctx, Outcome};
use super::*;

/// The writer writes what it holds, and puts the folders finished in the folder list, at least
/// this often, whatever comes.
const FLUSH_EVERY: Duration = Duration::from_secs(1);

/// One numbered thing for the writer to apply, with what it holds of the budget.
pub(super) struct Done {
    pub(super) seq: u64,
    pub(super) step: Step,
    pub(super) charge: u64,
}

pub(super) enum Step {
    /// What the file holds, for the progress.
    Totals { items: u64, bytes: u64, folders: usize },
    /// A mail folder begins (`index` in the plan), under the plan's folder `parent`.
    Folder { index: usize, parent: Option<usize>, name: String },
    /// …and is done.
    FolderEnd { index: usize },
    /// Things left out: items (`items`), or parts of the file that aren't.
    Lost { reason: SkipReason, n: usize, items: bool },
    /// A folder of a kind that isn't brought in, holding `items`.
    LeftOut { items: usize },
    /// An item of the plan's folder `folder`, of `size` bytes as the file gives it.
    Item { folder: usize, size: u64, pim: bool, outcome: Outcome },
}

/// Apply what comes through `results` to `out`, in order, until every sender is gone or the
/// import is stopped.
pub(super) fn write(out: &mut Out<'_>, ctx: &Ctx<'_>, results: &Receiver<Done>) {
    let mut waiting: BTreeMap<u64, Done> = BTreeMap::new();
    let mut next = 0u64;
    let mut paths: Vec<Option<String>> = Vec::new();
    let cap = ctx.budget.cap();
    let mut flushed = Instant::now();
    loop {
        let began = Instant::now();
        let got = results.recv_timeout(FLUSH_EVERY);
        ctx.tally.idle(began);
        match got {
            Ok(done) => {
                waiting.insert(done.seq, done);
            }
            Err(RecvTimeoutError::Timeout) => {
                out.flush();
                out.publish(false);
                flushed = Instant::now();
                continue;
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
        while let Some(done) = waiting.remove(&next) {
            if out.stopped() {
                return;
            }
            apply(out, ctx, &mut paths, done);
            next = next.saturating_add(1);
        }
        ctx.budget.advance(next);
        // What's held goes to the disk when it's half the budget, or sooner when a lane waits
        // for room (not item by item, unless the budget is that small).
        if out.pending_charge > cap / 2 || (out.pending_charge >= cap / 8 && ctx.budget.waiting()) {
            out.flush();
        }
        // Results coming without a pause: the second's flush all the same.
        if flushed.elapsed() >= FLUSH_EVERY {
            out.flush();
            out.publish(false);
            flushed = Instant::now();
        }
    }
    // Everyone is gone: the rest in order. A number never sent is an item lost with its lane.
    for (seq, done) in std::mem::take(&mut waiting) {
        if out.stopped() {
            return;
        }
        if seq > next {
            out.skip(SkipReason::MessageUnreadable, usize::try_from(seq - next).unwrap_or(usize::MAX));
        }
        apply(out, ctx, &mut paths, done);
        next = seq.saturating_add(1);
    }
}

fn apply(out: &mut Out<'_>, ctx: &Ctx<'_>, paths: &mut Vec<Option<String>>, done: Done) {
    #[cfg(test)]
    if let Some(super::lanes::tests::Fault::WriterAfter(n)) = ctx.opts.fault
        && out.read as u64 >= n
    {
        panic!("a test's writer fault");
    }
    let path_of = |paths: &Vec<Option<String>>, i: usize| paths.get(i).cloned().flatten();
    match done.step {
        Step::Totals { items, bytes, folders } => out.totals(items, bytes, folders),
        Step::Folder { index, parent, name } => {
            let parent = parent.and_then(|p| path_of(paths, p));
            let path = out.add_folder(parent.as_deref(), &name);
            if paths.len() <= index {
                paths.resize(index.saturating_add(1), None);
            }
            if let Some(slot) = paths.get_mut(index) {
                *slot = Some(path);
            }
        }
        Step::FolderEnd { index } => {
            if let Some(path) = path_of(paths, index) {
                out.save_folder(&path);
            }
        }
        Step::LeftOut { items } => out.leave_out_folder(items),
        Step::Lost { reason, n, items } => {
            if items {
                out.skip(reason, n);
            } else {
                out.lose(reason, n);
            }
        }
        Step::Item { folder, size, pim, outcome } => {
            let mut held = done.charge;
            match outcome {
                Outcome::Built(b) => {
                    if let Some(path) = path_of(paths, folder) {
                        out.add_built(&path, *b, held);
                        held = 0;
                    }
                }
                Outcome::Pim(it) => out.add_pim(*it),
                Outcome::NotMail => out.skip(SkipReason::ItemNotUsable, 1),
                Outcome::Unreadable if pim => out.skip(SkipReason::ItemUnreadable, 1),
                Outcome::Unreadable => out.skip(SkipReason::MessageUnreadable, 1),
                Outcome::NotRebuilt => out.skip(SkipReason::MessageNotRebuilt, 1),
                // Not read: the import is stopping, and it isn't counted.
                Outcome::Stopped => {
                    ctx.budget.release(held);
                    return;
                }
            }
            ctx.budget.release(held);
            out.item_read(size);
        }
    }
}
