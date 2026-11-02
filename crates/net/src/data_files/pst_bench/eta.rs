//! How long an import's items and bytes took, fitted over its progress and its folders, and how
//! far off the time left the window says would have been (`pst_bench::import`'s `fit` and `eta`
//! lines, and `replay`).

use mailcraft_model::job::{Eta, ITEM_WEIGHT, Work};

use super::{Sample, env};

/// Spans shorter than this are left out of the fit: one big item can take most of a second.
const FIT_SPAN_MS: u64 = 2000;

/// What an item and a byte took over an import, fitted: a span of it took `item_ms` for each
/// item read in it and `byte_ns` for each byte; `r2` is the share of the spans' times it
/// explains (through the origin), over `spans` spans.
#[derive(Clone, Copy, Debug)]
struct Fit {
    item_ms: f64,
    byte_ns: f64,
    r2: f64,
    spans: usize,
}

impl Fit {
    /// The bytes an item weighs: those that take as long to import as an item does besides them
    /// (`ITEM_WEIGHT`, fitted). 0 when the fit says nothing sensible.
    fn weight(&self) -> u64 {
        let w = self.item_ms * 1e6 / self.byte_ns;
        if w.is_finite() && w > 0.0 { w as u64 } else { 0 }
    }
}

/// Least squares, through the origin, over the import cut into spans of at least `span_ms` from
/// one sample to a later one: the time each took against the items and bytes read in it. None
/// with fewer than two spans, or when items and bytes went up together throughout.
fn fit(samples: &[Sample], span_ms: u64) -> Option<Fit> {
    let (first, rest) = samples.split_first()?;
    fit_spans(&spans(*first, rest, span_ms))
}

/// The items, bytes and milliseconds from `from` to each of `to` in turn that is `span_ms` or
/// more after the last taken.
fn spans(mut from: Sample, to: &[Sample], span_ms: u64) -> Vec<(f64, f64, f64)> {
    let mut spans = Vec::new();
    for s in to {
        if s.ms.saturating_sub(from.ms) >= span_ms {
            let d = |a: u64, b: u64| a.saturating_sub(b) as f64;
            spans.push((d(s.items, from.items), d(s.bytes, from.bytes), d(s.ms, from.ms)));
            from = *s;
        }
    }
    spans
}

/// The design's own fit, printed: each folder's items, bytes and time, from where the one before
/// it ended (`ends`; the first from the first progress, after the walk), a folder that read
/// nothing taken with the next.
pub(super) fn judge_folders(samples: &[Sample], ends: Vec<Sample>) {
    let Some(first) = samples.first() else { return };
    if let Some(f) = fit_spans(&spans(*first, &by_folder(*first, ends), 0)) {
        println!(
            "fit by folder: {:.3} ms an item, {:.2} ns a byte, item weight {} bytes, r2 {:.3}, {} folders",
            f.item_ms,
            f.byte_ns,
            f.weight(),
            f.r2,
            f.spans
        );
    }
}

/// Where folders ended past `from`, a folder that read nothing gone with the next.
fn by_folder(from: Sample, mut ends: Vec<Sample>) -> Vec<Sample> {
    ends.retain(|e| e.items > from.items);
    ends.dedup_by_key(|e| e.items);
    ends
}

/// Least squares, through the origin, of each span's time against its items and bytes.
fn fit_spans(spans: &[(f64, f64, f64)]) -> Option<Fit> {
    let (mut xx, mut xy, mut yy, mut xt, mut yt, mut tt) = (0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
    for &(x, y, t) in spans {
        (xx, xy, yy) = (xx + x * x, xy + x * y, yy + y * y);
        (xt, yt, tt) = (xt + x * t, yt + y * t, tt + t * t);
    }
    let det = xx * yy - xy * xy;
    if spans.len() < 2 || det <= 1e-9 * xx * yy {
        return None;
    }
    let (a, b) = ((xt * yy - yt * xy) / det, (yt * xx - xt * xy) / det);
    let sse: f64 = spans.iter().map(|&(x, y, t)| (t - a * x - b * y).powi(2)).sum();
    Some(Fit { item_ms: a, byte_ns: b * 1e6, r2: if tt > 0.0 { 1.0 - sse / tt } else { 0.0 }, spans: spans.len() })
}

/// How far off the time left would have been: the worst share it was off by from 10% done and
/// from 50% (by the bar), and the bar and the time when it was first shown.
#[derive(Clone, Copy, Debug, Default)]
struct EtaError {
    from_10: f64,
    from_50: f64,
    first_permille: u16,
    first_ms: u64,
}

/// Below a minute left the window says "less than a minute" whatever the estimate, so only
/// times of a minute or more, said or true, are judged.
const JUDGED_FROM_S: f64 = 60.0;

/// The time left as the window works it out (`Eta`, items weighing `weight` bytes besides their
/// own) from an import's samples, with `totals` (items, bytes), against the time it really took
/// to end at `end_ms`. The window's clock is taken to be the worker's.
fn eta_error(samples: &[Sample], totals: (u64, u64), end_ms: u64, weight: u64) -> EtaError {
    let mut eta = Eta::default();
    let mut out = EtaError::default();
    let mut shown = false;
    // `MC_TRACE`: what was said and what was true at every 5% (by the bar).
    let (trace, mut next_trace) = (env("MC_TRACE").is_some(), 0u16);
    for s in samples {
        let work = Work { done: s.items, total: totals.0, bytes_done: s.bytes, bytes_total: totals.1 };
        eta.sample(s.ms, work.weighted_done(weight), work.weighted_total(weight), 0, s.ms);
        let Some(left) = eta.remaining_s(s.ms) else { continue };
        let permille = work.permille(weight).unwrap_or(0);
        if !shown {
            (shown, out.first_permille, out.first_ms) = (true, permille, s.ms);
        }
        let truth = end_ms.saturating_sub(s.ms) as f64 / 1000.0;
        if trace && permille >= next_trace {
            println!("  {:.1}% done, {} s in: {left} s left said, {truth:.0} s true", permille as f64 / 10.0, s.ms / 1000);
            next_trace = (permille / 50 + 1) * 50;
        }
        out.count(permille, left as f64, truth);
    }
    out
}

impl EtaError {
    /// `left` seconds said with `truth` left, at `permille` done (by the bar).
    fn count(&mut self, permille: u16, left: f64, truth: f64) {
        if truth < JUDGED_FROM_S && left < JUDGED_FROM_S {
            return;
        }
        let off = (left - truth).abs() / truth.max(1.0);
        if permille >= 100 {
            self.from_10 = self.from_10.max(off);
        }
        if permille >= 500 {
            self.from_50 = self.from_50.max(off);
        }
    }
}

/// The best the time left could have been from a rate alone: said from the import's own
/// average over the whole of it (its work over its time, which nothing knows ahead), with items
/// weighing `weight`, from where the window first may say it (judged as [`eta_error`] judges).
/// What's off then is the import going faster and slower than its average, not the estimate.
fn best_case(samples: &[Sample], totals: (u64, u64), end_ms: u64, weight: u64) -> EtaError {
    let mut out = EtaError::default();
    let work = |s: &Sample| Work { done: s.items, total: totals.0, bytes_done: s.bytes, bytes_total: totals.1 };
    let total = work(&Sample { ms: 0, items: 0, bytes: 0 }).weighted_total(weight) as f64;
    // Work a millisecond.
    let rate = total / end_ms.max(1) as f64;
    for s in samples.iter().filter(|s| s.ms >= BEST_FROM_MS) {
        let w = work(s);
        let left = (total - w.weighted_done(weight) as f64).max(0.0) / rate / 1000.0;
        out.count(w.permille(weight).unwrap_or(0), left, end_ms.saturating_sub(s.ms) as f64 / 1000.0);
    }
    out
}

/// The window says no time left before this far into an import.
const BEST_FROM_MS: u64 = 20_000;

/// What an item and a byte cost over an import's progress, and how far off the time left would
/// have been with the app's item weight, bytes alone, the fitted weight and those in `MC_WEIGHTS`
/// (a list of bytes), and at best from a rate (`best_case`).
pub(super) fn judge(samples: &[Sample], totals: (u64, u64), end_ms: u64) {
    let fitted = fit(samples, FIT_SPAN_MS);
    if let Some(f) = fitted {
        println!("fit: {:.3} ms an item, {:.2} ns a byte, item weight {} bytes, r2 {:.3}, {} spans", f.item_ms, f.byte_ns, f.weight(), f.r2, f.spans);
    }
    let mut weights = vec![("app".to_string(), ITEM_WEIGHT), ("bytes only".to_string(), 0)];
    weights.extend(fitted.map(|f| ("fitted".to_string(), f.weight())));
    let asked = env("MC_WEIGHTS").unwrap_or_default();
    weights.extend(asked.split(',').filter_map(|w| w.trim().parse().ok()).map(|w| ("asked".to_string(), w)));
    for (name, weight) in weights {
        let e = eta_error(samples, totals, end_ms, weight);
        let best = best_case(samples, totals, end_ms, weight);
        println!(
            "eta ({name}, item weight {weight}): worst {:.1}% off from 10% done, {:.1}% from 50%; first shown at {:.1}% done, {} s in; \
             from the run's own average rate {:.1}% and {:.1}%",
            e.from_10 * 100.0,
            e.from_50 * 100.0,
            e.first_permille as f64 / 10.0,
            e.first_ms / 1000,
            best.from_10 * 100.0,
            best.from_50 * 100.0
        );
    }
}

/// Judge again the progress an import kept in `MC_SAMPLES` (made by `import` with it set), with
/// `MC_WEIGHTS` and `MC_TRACE` as there.
#[test]
#[ignore]
fn replay() {
    let text = std::fs::read_to_string(env("MC_SAMPLES").expect("MC_SAMPLES: the progress an import kept")).unwrap();
    let mut lines = text.lines().map(|l| l.split(',').map(|n| n.trim().parse::<u64>().unwrap()).collect::<Vec<_>>());
    let head = lines.next().unwrap();
    let samples: Vec<Sample> = lines.map(|l| Sample { ms: l[0], items: l[1], bytes: l[2] }).collect();
    judge(&samples, (head[0], head[1]), head[2]);
}

/// Samples of an import every 200 items of `items` (sizes in bytes), where an item takes
/// `item_ms` and a byte `byte_ns`.
fn simulated(items: &[u64], item_ms: f64, byte_ns: f64) -> Vec<Sample> {
    let (mut t, mut bytes, mut out) = (0.0f64, 0u64, Vec::new());
    for (k, size) in items.iter().enumerate() {
        t += item_ms + *size as f64 * byte_ns / 1e6;
        bytes += size;
        if (k + 1) % 200 == 0 {
            out.push(Sample { ms: t as u64, items: k as u64 + 1, bytes });
        }
    }
    out
}

/// Folders of many small messages, then of few big ones, then small again.
fn mixed_items() -> Vec<u64> {
    let mut items = vec![4_000u64; 20_000];
    items.extend(std::iter::repeat_n(3_000_000u64, 2_000));
    items.extend(std::iter::repeat_n(6_000u64, 20_000));
    items
}

/// The fit finds what an item and a byte cost from the progress alone, however the folders mix
/// small and big items.
#[test]
fn the_fit_finds_what_an_item_and_a_byte_cost() {
    let f = fit(&simulated(&mixed_items(), 2.0, 18.0), FIT_SPAN_MS).unwrap();
    assert!((f.item_ms - 2.0).abs() < 0.05, "{f:?}");
    assert!((f.byte_ns - 18.0).abs() < 0.5, "{f:?}");
    assert!((105_000..118_000).contains(&f.weight()) && f.r2 > 0.99, "{f:?}");
    // Every item alike: nothing tells an item's cost from its bytes'.
    assert!(fit(&simulated(&[5_000; 10_000], 2.0, 18.0), FIT_SPAN_MS).is_none());
    assert!(fit(&[], FIT_SPAN_MS).is_none());
}

/// The time left is judged against when the import really ended: right with items weighed as
/// they cost, and far off when they weigh nothing and small items come first.
#[test]
fn the_time_left_is_judged_against_the_real_end() {
    let items = mixed_items();
    let samples = simulated(&items, 2.0, 18.0);
    let totals = (items.len() as u64, items.iter().sum());
    let end = samples.last().unwrap().ms;
    let right = eta_error(&samples, totals, end, 111_111);
    assert!(right.from_10 < 0.05 && right.from_50 < 0.05, "{right:?}");
    assert!(right.first_ms >= 20_000 && right.first_permille >= 20, "nothing said early: {right:?}");
    let bytes_only = eta_error(&samples, totals, end, 0);
    assert!(bytes_only.from_10 > 0.2, "{bytes_only:?}");
    assert_eq!(eta_error(&[], totals, end, 0).first_ms, 0);
    // At best, from the run's average: right with items weighed as they cost, but off by the
    // folders' speeds alone with bytes only, which no estimate from the speed can do better than.
    let best = best_case(&samples, totals, end, 111_111);
    assert!(best.from_10 < 0.02 && best.from_50 < 0.02, "{best:?}");
    let best_bytes = best_case(&samples, totals, end, 0);
    assert!(best_bytes.from_10 > 0.1, "{best_bytes:?}");
    assert_eq!(best_case(&[], totals, end, 0).from_10, 0.0);
}

/// The design's fit, by folder: where each folder ended, a folder that read nothing taken with
/// the next, and those ended before the first progress left out.
#[test]
fn the_fit_by_folder_finds_what_an_item_and_a_byte_cost() {
    let samples = simulated(&mixed_items(), 2.0, 18.0);
    let at = |items: u64| *samples.iter().find(|s| s.items >= items).unwrap();
    // Folders ending at 10,000, 20,000 (and an empty one there), 21,000, 22,000, 32,000 and the end.
    let mut ends: Vec<Sample> = [10_000, 20_000, 20_000, 21_000, 22_000, 32_000, 42_000].iter().map(|n| at(*n)).collect();
    ends.insert(0, Sample { ms: 1, items: 0, bytes: 0 });
    let folders = by_folder(samples[0], ends);
    assert_eq!(folders.len(), 6);
    let f = fit_spans(&spans(samples[0], &folders, 0)).unwrap();
    assert!((f.item_ms - 2.0).abs() < 0.05 && (f.byte_ns - 18.0).abs() < 0.5, "{f:?}");
}
