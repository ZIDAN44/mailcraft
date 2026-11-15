//! An import run: the account's data file made ready, the planner, the lanes and the writer
//! started and run to the writer's end, and what that came to.

use super::*;

/// [`import_with`], telling `sink` how it goes and of each folder as it's done.
pub fn import_into(path: &Path, store: &Store, account: &str, opts: &ImportOptions, sink: &mut dyn ImportSink) -> Result<Imported, String> {
    let mut out = Out::new(store, account, opts, sink);
    let skip = if opts.resume {
        match resume::load(store, account) {
            resume::Found::Nothing => None,
            resume::Found::Unsound => return Ok(blocked(mailcraft_model::job::Blocked::SourceChanged)),
            resume::Found::Some(cp) if cp.state == resume::CheckState::Done => return Ok(cp.imported(store, account)),
            resume::Found::Some(cp) => Some(out.resume_from(*cp)),
        }
    } else {
        None
    };
    // `!BDN` then the client magic: `SM` a data file, `SO` Outlook's offline copy of a server
    // account (MS-PST 2.2.2.6).
    let mut head = [0u8; 10];
    if std::fs::File::open(path).and_then(|mut f| io::Read::read_exact(&mut f, &mut head)).is_ok()
        && head.starts_with(b"!BDN")
        && head.get(8..10) == Some(b"SO")
    {
        return Err("this is Outlook's offline copy of a server account (.ost), not a data file: add the account itself to see its mail".into());
    }
    // Too little room left to go on: it waits for more, with nothing read.
    if let Some(free) = out.short_of_room() {
        return Ok(blocked(mailcraft_model::job::Blocked::DiskFull { free }));
    }
    // The account's data file has a bigger cache while the import writes, and lets go of it
    // however the import ends.
    let cache = match opts.tuning().cache_bytes {
        0 => mailcraft_store::BULK_CACHE,
        n => n,
    };
    let _bulk = Bulk::begin(store, account, cache);
    // Kept from writers from here until the import is done, whichever handles are reading it
    // meanwhile (they all go before the writer is). A file this can't open, the planner can't
    // either, and says why.
    let _held = open::open_read(path).ok();
    // The messages go into the account's search index as they're written, unless another
    // writer holds it (then they're indexed after, as mail from anywhere is).
    if !opts.no_index {
        out.open_index();
    }
    let mut ctx = lanes::Ctx::new(path, opts, account, &out);
    ctx.resume = skip;
    let lanes = ctx.lanes();
    out.lanes = u64::try_from(lanes).unwrap_or(u64::MAX);
    let (planned, wrote) = run_threads(&mut out, &ctx, lanes);
    conclude(out, &ctx, opts, path, planned, wrote)
}

/// What the planner came to (`None`: it couldn't start; `Err`: it panicked), and whether the
/// writer went to its end.
type Ran = (Option<std::thread::Result<io::Result<()>>>, std::thread::Result<()>);

/// The planner, the readers (`lanes` of them) and the writer, run until the writer is done.
fn run_threads(out: &mut Out<'_>, ctx: &lanes::Ctx<'_>, lanes: usize) -> Ran {
    let (job_tx, job_rx) = std::sync::mpsc::sync_channel(plan::JOBS);
    let jobs: lanes::Jobs = std::sync::Mutex::new(job_rx);
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::scope(|s| {
        let jobs = &jobs;
        // Whatever becomes of the writer (or of starting the threads), every other thread stops
        // when it does.
        struct StopAll<'c, 'a>(&'c lanes::Ctx<'a>);
        impl Drop for StopAll<'_, '_> {
            fn drop(&mut self) {
                self.0.stop();
            }
        }
        let stop_all = StopAll(ctx);
        let feed = plan::Feed { seq: 0, jobs: job_tx, results: tx.clone(), handed: 0, capped: false };
        let planner = std::thread::Builder::new()
            .spawn_scoped(s, move || std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| planner(ctx, feed))))
            .map_err(|e| log::warn!("import: the planner could not start: {e}"))
            .ok();
        ctx.live.store(lanes, Ordering::Relaxed);
        let mut started = 0;
        if planner.is_some() {
            for _ in 0..lanes {
                let tx = tx.clone();
                let lane = std::thread::Builder::new().spawn_scoped(s, move || {
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| lanes::run(ctx, jobs, &tx)));
                    lanes::gone(ctx, jobs, 1);
                });
                if let Err(e) = lane {
                    log::warn!("import: a reader could not start: {e}");
                    break;
                }
                started += 1;
            }
        }
        // The lanes that didn't start are gone too: none at all, and the import stops.
        if started < lanes {
            lanes::gone(ctx, jobs, lanes - started);
        }
        drop(tx);
        let wrote = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| order::write(out, ctx, &rx)));
        drop(stop_all);
        drop(rx);
        (planner.map(|p| p.join().unwrap_or_else(Err)), wrote)
    })
}

/// How an import ended, from what its threads came to: blocked going on, stopped, short of
/// what the file holds (said once for each reason), or failed; else what it brought in.
fn conclude(
    mut out: Out<'_>,
    ctx: &lanes::Ctx<'_>,
    opts: &ImportOptions,
    path: &Path,
    planned: Option<std::thread::Result<io::Result<()>>>,
    wrote: std::thread::Result<()>,
) -> Result<Imported, String> {
    let Some(planned) = planned else {
        out.seal();
        return Err("the import could not start: the computer is short of resources; try again".into());
    };
    // Asked to stop, the file's reads fail from then on: whatever the planner came to then is a
    // stop, not a damaged file (it's planned whole again going on). Stopped before the file was
    // planned, the writer had nothing to see the stop by: it's a stop all the same.
    let planned = if opts.stop.raised() { Ok(Ok(())) } else { planned };
    if opts.stop.raised() && !out.note_ready() {
        out.note_stop();
    }
    // Going on, a file that isn't the one it was (or can't be read now) waits on the user, with
    // nothing written.
    if ctx.resume.is_some() && !out.note_ready() {
        let why = match &planned {
            _ if ctx.changed.load(Ordering::Relaxed) => Some(mailcraft_model::job::Blocked::SourceChanged),
            Ok(Err(e)) => Some(resume::blocked_by(e)),
            Err(_) => Some(mailcraft_model::job::Blocked::SourceChanged),
            Ok(Ok(())) => None,
        };
        if let Some(why) = why {
            out.seal();
            return Ok(blocked(why));
        }
    }
    // Cut off as if the process had ended: nothing an end writes is written.
    #[cfg(test)]
    if wrote.is_err() && matches!(opts.fault, Some(lanes::tests::Fault::CutOffAfter(_))) {
        return Err("cut off".into());
    }
    if wrote.is_err() {
        log::warn!("{}: the import stopped writing; what was written is kept", path.display());
        out.lose(SkipReason::StoppedEarly, 1);
    }
    if ctx.orphaned.load(Ordering::Relaxed) {
        log::warn!("{}: no reader was left to read the data file; stopped reading", path.display());
        out.lose(SkipReason::StoppedEarly, 1);
    }
    if ctx.damaged.load(Ordering::Relaxed) {
        log::warn!("{}: the data file is damaged; stopped reading", path.display());
        out.lose(SkipReason::StoppedEarly, 1);
    }
    let failed: Option<String> = match planned {
        Ok(Ok(())) => None,
        // Outlook (or another program) has it open.
        Ok(Err(e)) if open::locked_by_another(&e) => Some(LOCKED.into()),
        // Nothing found at all: say why; otherwise keep what was read.
        Ok(Err(e)) if out.folders.is_empty() => Some(format!("not an Outlook data file, or it is damaged ({e})")),
        Ok(Err(e)) => {
            log::warn!("{}: the rest of the data file could not be read: {e}", path.display());
            None
        }
        Err(_) if out.folders.is_empty() => Some("the data file is damaged".into()),
        Err(_) => {
            log::warn!("{}: the data file is damaged; stopped reading", path.display());
            out.lose(SkipReason::StoppedEarly, 1);
            None
        }
    };
    match failed {
        // Batches may have gone in already (a lock met part-way): on the disk and sealed all the
        // same, as at the end.
        Some(e) => {
            out.seal();
            Err(e)
        }
        None => Ok(out.finish()),
    }
}

/// An import that couldn't go on from where it stopped, and why.
fn blocked(why: mailcraft_model::job::Blocked) -> Imported {
    Imported { blocked: Some(why), ..Imported::default() }
}

/// An account's bulk write ([`Store::bulk_begin`]), ended when dropped, on every way out of an
/// import (a panic included: the file then keeps its cache until it's next asked for).
struct Bulk<'a> {
    store: &'a Store,
    account: &'a str,
}

impl<'a> Bulk<'a> {
    fn begin(store: &'a Store, account: &'a str, cache_bytes: usize) -> Bulk<'a> {
        store.bulk_begin(account, cache_bytes);
        Bulk { store, account }
    }
}

impl Drop for Bulk<'_> {
    fn drop(&mut self) {
        self.store.bulk_end(self.account);
    }
}

/// The planner: opens the file, plans it (unless it was planned when it was looked at), and
/// hands out its work.
fn planner(ctx: &lanes::Ctx<'_>, mut feed: plan::Feed) -> io::Result<()> {
    let began = Instant::now();
    let (store, source) = opened(ctx)?;
    let fresh;
    let planned = match ctx.opts.planned.as_deref() {
        Some(p) => &p.plan,
        None => {
            let planning = plan::plan(&store, ctx);
            ctx.tally.since(Phase::Plan, began);
            fresh = planning?;
            &fresh
        }
    };
    // Stopped as it was planned: the plan may be part of the file, so nothing is handed out (and
    // the file isn't taken for another). Going on: only with the file it began on.
    if ctx.stopped() {
        return Ok(());
    }
    if let Some(skip) = &ctx.resume
        && !skip.same_file(&planned.record_key, planned.fingerprint())
    {
        ctx.changed.store(true, Ordering::Relaxed);
        return Ok(());
    }
    plan::stream(&store, planned, &mut feed, ctx);
    ctx.tally.reading(&source);
    Ok(())
}

/// The file opened to be planned and read. Read only, and kept from writers while it's read.
/// Locked by Outlook, this is what fails, and the import says so.
pub(crate) fn opened(ctx: &lanes::Ctx<'_>) -> io::Result<(Rc<PstStore>, Shared)> {
    let source = open::source(ctx.path, ctx.opts)?;
    let pst = PstFile::read_from(open::handle(&source))?;
    pst.set_max_value_size(MAX_VALUE);
    Ok((PstStore::read(Rc::new(pst))?, source))
}
