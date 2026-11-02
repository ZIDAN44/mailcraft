//! A whole index pass over a data folder as the app keeps it, timed, with read marks made the
//! app's way meanwhile: an ignored test, run by hand on a copy (the marks change it).
//!
//! ```text
//! MC_DATA=D:\copy cargo test -p mailcraft-net --release --lib index_bench -- --ignored --nocapture
//! ```
//!
//! `MC_FRESH=1` sets the account's index aside first, so the pass reads every message;
//! `MC_READERS` sets how many threads read (unset: the app's). While the pass runs a message of
//! the biggest folder is marked read and unread every 20 ms through the store's thread for marks
//! ([`Store::mark_later`]), each timed from being handed over until it's on disk, as 50 are first
//! with nothing running. It prints how big the sources are (a `sources:` line), then the pass's
//! own log line (where its time went), a `marks:` line with those over 60 ms and when, and on
//! Windows the working set before the pass and the process's peak (asked of PowerShell, from
//! outside), whose difference is what the pass added; the copy is sealed as it ends, so the next
//! run opens it without a check. Counts and timings only.

use std::path::PathBuf;

use mailcraft_model::Flags;
use mailcraft_store::FolderMarks;

use super::*;

/// The pass's own lines (where its time went), printed.
struct Print;

impl log::Log for Print {
    fn enabled(&self, m: &log::Metadata<'_>) -> bool {
        m.level() <= log::Level::Info
    }

    fn log(&self, r: &log::Record<'_>) {
        if self.enabled(r.metadata()) {
            println!("{} {}", r.level(), r.args());
        }
    }

    fn flush(&self) {}
}

static PRINT: Print = Print;

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

/// This process's working set and peak working set, in MB, as Windows counts them.
fn working_set() -> Option<(u64, u64)> {
    if !cfg!(windows) {
        return None;
    }
    let pid = std::process::id();
    let script = format!("$w = Get-Process -Id {pid}; '' + $w.WorkingSet64 + ' ' + $w.PeakWorkingSet64");
    let out = std::process::Command::new("powershell").args(["-NoProfile", "-Command", &script]).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let mut n = text.split_whitespace().filter_map(|v| v.parse::<u64>().ok());
    Some((n.next()? >> 20, n.next()? >> 20))
}

/// Times in ms: the median, p99 and most.
fn spread(took: &mut [f64]) -> (f64, f64, f64) {
    took.sort_by(f64::total_cmp);
    let at = |q: usize| took.get((took.len() * q / 100).min(took.len().saturating_sub(1))).copied().unwrap_or(0.0);
    (at(50), at(99), took.last().copied().unwrap_or(0.0))
}

#[test]
#[ignore = "a measurement on a copy of a data folder: MC_DATA, run alone with --ignored --nocapture"]
fn index_bench() {
    if log::set_logger(&PRINT).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
    }
    let data = PathBuf::from(env("MC_DATA").expect("MC_DATA: a copy of a data folder"));
    let s = Store::open_packed(data.clone(), data.join("MailCraft Files")).unwrap();
    let account = s.load_accounts().unwrap().first().map(|a| a.id.clone()).expect("an account in MC_DATA");
    if env("MC_FRESH").is_some() {
        search::set_aside(&s, &account).unwrap();
    }
    let readers = env("MC_READERS").and_then(|v| v.parse().ok()).unwrap_or_else(indexing::readers);
    // The biggest folder's first message, marked read and unread.
    let folders = s.load_folders(&account).unwrap();
    let biggest = folders.iter().max_by_key(|f| f.total).map(|f| f.path.clone()).expect("a folder");
    let list = s.load_folder(&account, &biggest).unwrap();
    let first = list.messages.first().cloned().expect("a message");
    let (validity, uid) = (list.validity, first.key.uid);
    let mark = |on: bool| {
        let flags = Flags { seen: on, ..first.flags.clone() };
        let t0 = Instant::now();
        s.mark_later(&account, vec![FolderMarks { folder: biggest.clone(), validity, flags: vec![(uid, flags)] }], Vec::new());
        assert!(s.flush_marks(), "a mark not written in 10 s");
        t0.elapsed().as_secs_f64() * 1000.0
    };
    // How big the sources are, by their summaries: what a reader holds at once.
    let mut sizes: Vec<u32> =
        folders.iter().filter_map(|f| s.load_folder(&account, &f.path).ok()).flat_map(|l| l.messages.into_iter().map(|m| m.size)).collect();
    sizes.sort_unstable();
    let (most, over) = (sizes.last().copied().unwrap_or(0), sizes.iter().filter(|n| **n > 16 << 20).count());
    let big = sizes.iter().filter(|n| **n > 4 << 20).map(|n| u64::from(*n)).sum::<u64>();
    let all = sizes.iter().map(|n| u64::from(*n)).sum::<u64>();
    println!("sources: {}, the biggest {} MB, {over} over 16 MB; those over 4 MB hold {} of {} MB", sizes.len(), most >> 20, big >> 20, all >> 20);
    let mut alone: Vec<f64> = (0..50).map(|i| mark(i % 2 == 0)).collect();
    let before = working_set();
    let stop = AtomicBool::new(false);
    let began = Instant::now();
    let (mut took, mut reports, mut slow) = (Vec::new(), 0u32, Vec::new());
    std::thread::scope(|sc| {
        let pass = std::thread::Builder::new()
            .name("mail-index".into())
            .spawn_scoped(sc, || {
                indexing::pass(&s, &account, &[], &stop, readers, &mailcraft_mime::index_doc, &mut |_| reports += 1).unwrap();
            })
            .unwrap();
        while !pass.is_finished() {
            let at = began.elapsed();
            let ms = mark(took.len() % 2 == 0);
            if ms > 60.0 {
                slow.push(format!("{:.1} s: {ms:.0} ms", at.as_secs_f64()));
            }
            took.push(ms);
            std::thread::sleep(Duration::from_millis(20));
        }
    });
    let whole = began.elapsed();
    let after = working_set();
    mark(first.flags.seen);
    // Closed sealed, so the next run on the copy opens it without a check.
    s.seal_all().unwrap();
    let ((a50, a99, amax), (d50, d99, dmax)) = (spread(&mut alone), spread(&mut took));
    println!("pass: {readers} readers, {whole:?}, {reports} reports");
    println!(
        "marks: alone median {a50:.1} ms, p99 {a99:.1}, most {amax:.1}; {} during the pass median {d50:.1} ms, p99 {d99:.1}, most {dmax:.1}",
        took.len()
    );
    if !slow.is_empty() {
        println!("marks over 60 ms: {}", slow.join(", "));
    }
    if let (Some((ws, _)), Some((_, peak))) = (before, after) {
        println!("memory: {ws} MB before the pass, peak {peak} MB: +{} MB", peak.saturating_sub(ws));
    }
}
