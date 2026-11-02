//! The writer: what the planner and the lanes send, applied in the order the planner numbered
//! it, whichever lane finished first. Messages get their numbers here, in that order, so an
//! import comes out the same however many lanes read it.

use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender};

use super::lanes::{Ctx, Outcome};
use super::out::sent_ids::SentIds;
use super::*;

/// The writer writes what it holds, and puts the folders finished in the folder list, at least
/// this often, whatever comes.
const FLUSH_EVERY: Duration = Duration::from_secs(1);

/// A mail folder of the plan (its `index` there, under the plan's folder `parent`), as the
/// outline of what comes in says it: its node, its name in the file and the items it holds.
pub(super) struct Outlined {
    pub(super) index: usize,
    pub(super) parent: Option<usize>,
    pub(super) node: u32,
    pub(super) name: String,
    pub(super) items: u64,
}

/// One numbered thing for the writer to apply, with what it holds of the budget.
pub(super) struct Done {
    pub(super) seq: u64,
    pub(super) step: Step,
    pub(super) charge: u64,
}

pub(super) enum Step {
    /// What the file holds, for the progress, its record key and fingerprint, for its
    /// checkpoints, and its mail folders, for the outline the window shows ahead.
    Totals { items: u64, bytes: u64, folders: usize, record_key: String, fingerprint: u64, outline: Vec<Outlined> },
    /// A folder of the file (its `node`) begins (`index` in the plan): a mail folder under the
    /// plan's folder `parent`, or contacts or a calendar (`pim`). Going on, a mail folder done
    /// already begins only to say where it went (its folders under it go there too). The Sent
    /// Items folder begins twice: in its place, made but not read (`deferred`), and after every
    /// other, to be read.
    Folder { index: usize, node: u32, parent: Option<usize>, name: String, pim: bool, deferred: bool },
    /// Everything before it is applied, and the sent items come next: the replies read are sent
    /// back to the planner, for the lanes reading them.
    Freeze { reply: SyncSender<Arc<SentIds>> },
    /// …and is done.
    FolderEnd { index: usize, node: u32, pim: bool },
    /// Things left out: items (`items`), or parts of the file that aren't.
    Lost { reason: SkipReason, n: usize, items: bool },
    /// A folder of a kind that isn't brought in (at `node`), holding `items`.
    LeftOut { node: u32, items: usize },
    /// An item of the plan's folder `folder` (the file's at `node`, at `row` in it), of `size`
    /// bytes as the file gives it.
    Item { folder: usize, size: u64, pim: bool, node: u32, row: u64, outcome: Outcome },
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
                // Nothing to write for a while (contacts read, or items left out): the
                // checkpoint goes on its own.
                if out.checkpoint_due() {
                    out.checkpoint();
                }
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
        // A checkpoint due goes here, between items, with what's held (the batches an item
        // fills as it's added carry none: its row isn't done yet).
        if out.checkpoint_due() {
            out.checkpoint();
        }
        // Results coming without a pause: the second's flush all the same.
        if flushed.elapsed() >= FLUSH_EVERY {
            out.flush();
            out.publish(false);
            if out.checkpoint_due() {
                out.checkpoint();
            }
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
    if let Some(super::lanes::tests::Fault::WriterAfter(n) | super::lanes::tests::Fault::CutOffAfter(n)) = ctx.opts.fault
        && out.read as u64 >= n
    {
        panic!("a test's writer fault");
    }
    let path_of = |paths: &Vec<Option<String>>, i: usize| paths.get(i).cloned().flatten();
    match done.step {
        Step::Totals { items, bytes, folders, record_key, fingerprint, outline } => {
            out.totals(items, bytes, folders);
            out.source_known(ctx.path, record_key, fingerprint);
            out.outline(&outline);
        }
        Step::Folder { node, pim: true, .. } => out.begin_pim_folder(node),
        Step::Folder { index, node, parent, name, pim: false, deferred } => {
            let parent = parent.and_then(|p| path_of(paths, p));
            let path = out.begin_folder(node, parent.as_deref(), &name, deferred);
            if paths.len() <= index {
                paths.resize(index.saturating_add(1), None);
            }
            if let Some(slot) = paths.get_mut(index) {
                *slot = path;
            }
        }
        Step::FolderEnd { node, pim: true, .. } => out.other_folder_done(node),
        Step::FolderEnd { index, pim: false, .. } => {
            if let Some(path) = path_of(paths, index) {
                out.save_folder(&path);
            }
        }
        Step::Freeze { reply } => {
            // The planner gone, nothing waits for it.
            let _ = reply.send(out.freeze_sent());
        }
        Step::LeftOut { node, items } => {
            out.leave_out_folder(items);
            out.other_folder_done(node);
        }
        Step::Lost { reason, n, items } => {
            if items {
                out.skip(reason, n);
            } else {
                out.lose(reason, n);
            }
        }
        Step::Item { folder, size, pim, node, row, outcome } => {
            let mut held = done.charge;
            let failed = matches!(outcome, Outcome::Failed);
            match outcome {
                Outcome::Built(mut b) => {
                    out.sent_ids(&mut b);
                    if let Some(path) = path_of(paths, folder) {
                        out.add_built(&path, *b, held);
                        held = 0;
                    }
                }
                Outcome::Pim(it) => out.add_pim(*it),
                Outcome::NotMail => out.skip(SkipReason::ItemNotUsable, 1),
                Outcome::Unreadable | Outcome::Failed if pim => out.skip(SkipReason::ItemUnreadable, 1),
                Outcome::Unreadable | Outcome::Failed => out.skip(SkipReason::MessageUnreadable, 1),
                Outcome::NotRebuilt => out.skip(SkipReason::MessageNotRebuilt, 1),
                // Not read: the import is stopping, and it isn't counted.
                Outcome::Stopped => {
                    ctx.budget.release(held);
                    return;
                }
            }
            ctx.budget.release(held);
            out.item_read(size);
            out.row_done(node, row, failed);
        }
    }
}
