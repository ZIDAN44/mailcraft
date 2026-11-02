//! The log file, for working out what happened when something misbehaves on an account (its
//! servers' errors, its statuses, how long syncing took). `mailcraft.log` in the data folder:
//! a new one each time MailCraft starts (the last run's renamed `mailcraft.old.log`), up to
//! [`MAX`] a run. MailCraft's own lines from Info up, other
//! libraries' warnings. Lines name accounts, folders and errors; never a message's content or a
//! password (MailCraft doesn't log either anywhere).
//!
//! Lines are written on a thread of the log's own: a line logged is handed over and the caller
//! goes on. Written where it was logged, a line from the window's thread (a slow wait it reports,
//! what a search spent its time on) waited there for the disk, which a big data file being
//! flushed can hold for hundreds of milliseconds. Lines are written in the order they were
//! logged; past [`WAITING_MAX`] waiting, more are left out and counted (the count written after)
//! rather than held. Closing ([`log::Log::flush`]) waits for what was handed over.

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Condvar, Mutex, PoisonError};
use std::time::Duration;

use log::{Level, LevelFilter, Metadata, Record};

/// Most written in one run (the rest of the run goes unlogged, and says so).
const MAX: u64 = 4 << 20;
/// Most text waiting for the log's thread: past it, lines are left out and counted.
const WAITING_MAX: usize = 1 << 20;
/// How long a flush waits for the log's thread at most.
const FLUSH_WAIT: Duration = Duration::from_secs(2);

pub const NAME: &str = "mailcraft.log";
const OLD: &str = "mailcraft.old.log";

struct FileLog {
    /// The open file and how much has gone into it.
    out: Mutex<Option<(File, u64)>>,
    /// Lines also go to the terminal (a debug build, or `--log`).
    echo: bool,
    /// What waits for the log's thread.
    waiting: Mutex<Waiting>,
    /// Lines were handed over, or the thread wrote what it took.
    moved: Condvar,
}

#[derive(Default)]
struct Waiting {
    /// The log's thread runs (without it, lines are written where they're logged).
    threaded: bool,
    /// Lines handed over and not taken yet.
    text: String,
    /// Lines left out since the last taken, too many waiting.
    dropped: u64,
    /// The thread is writing what it took.
    writing: bool,
}

/// A line to the terminal; a terminal gone (standard error closed, or a broken pipe once the
/// program that started MailCraft has ended) is passed over. `eprint!` panicked there, and
/// the next line logged ended MailCraft.
fn echo(to: &mut dyn Write, line: &str) {
    let _ = to.write_all(line.as_bytes());
}

impl FileLog {
    fn new(file: Option<File>, echo: bool) -> FileLog {
        FileLog { out: Mutex::new(file.map(|f| (f, 0))), echo, waiting: Mutex::default(), moved: Condvar::new() }
    }

    fn waiting(&self) -> std::sync::MutexGuard<'_, Waiting> {
        self.waiting.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Lines written on a thread of the log's own from now on (where they're logged, when none
    /// could be started).
    fn start_thread(&'static self) {
        let started = std::thread::Builder::new().name("log".into()).spawn(move || self.write_on()).is_ok();
        self.waiting().threaded = started;
    }

    /// The log's thread: what's handed over, written in order, for as long as MailCraft runs.
    fn write_on(&self) {
        loop {
            let (text, dropped) = {
                let mut w = self.waiting();
                while w.text.is_empty() && w.dropped == 0 {
                    w = self.moved.wait(w).unwrap_or_else(PoisonError::into_inner);
                }
                w.writing = true;
                (std::mem::take(&mut w.text), std::mem::take(&mut w.dropped))
            };
            self.write_line(&text);
            if dropped > 0 {
                self.write_line(&format!("... {dropped} lines left out here: more came than the disk took\n"));
            }
            self.waiting().writing = false;
            self.moved.notify_all();
        }
    }

    /// `line` handed to the log's thread (written here when there's none).
    fn hand_over(&self, line: &str) {
        let mut w = self.waiting();
        if !w.threaded {
            drop(w);
            self.write_line(line);
            return;
        }
        if w.text.len().saturating_add(line.len()) > WAITING_MAX {
            w.dropped = w.dropped.saturating_add(1);
        } else {
            w.text.push_str(line);
        }
        drop(w);
        self.moved.notify_all();
    }

    fn write_line(&self, line: &str) {
        let mut out = self.out.lock().unwrap_or_else(PoisonError::into_inner);
        let Some((file, written)) = out.as_mut() else { return };
        if *written >= MAX {
            return;
        }
        let bytes = line.as_bytes();
        let ok = file.write_all(bytes).is_ok();
        *written = written.saturating_add(bytes.len() as u64);
        if !ok {
            *out = None;
        } else if *written >= MAX {
            let _ = file.write_all(b"... the log is full: the rest of this run isn't written\n");
            let _ = file.flush();
        }
    }

    /// Wait ([`FLUSH_WAIT`] at most) for the lines handed over to be written.
    fn wait_written(&self) {
        let began = std::time::Instant::now();
        let mut w = self.waiting();
        while w.threaded && (!w.text.is_empty() || w.dropped > 0 || w.writing) {
            let left = FLUSH_WAIT.saturating_sub(began.elapsed());
            if left.is_zero() {
                return;
            }
            w = match self.moved.wait_timeout(w, left) {
                Ok((w, _)) => w,
                Err(e) => e.into_inner().0,
            };
        }
    }
}

impl log::Log for FileLog {
    fn enabled(&self, m: &Metadata) -> bool {
        enabled(m.target(), m.level())
    }

    fn log(&self, r: &Record) {
        if !self.enabled(r.metadata()) {
            return;
        }
        let line = line(&stamp(), r.level(), r.target(), &r.args().to_string());
        if self.echo {
            echo(&mut std::io::stderr(), &line);
        }
        self.hand_over(&line);
    }

    fn flush(&self) {
        self.wait_written();
        if let Some((f, _)) = self.out.lock().unwrap_or_else(PoisonError::into_inner).as_mut() {
            let _ = f.flush();
        }
    }
}

/// Whether a line is kept: MailCraft's from Info up, everyone else's warnings.
fn enabled(target: &str, level: Level) -> bool {
    if target.starts_with("mailcraft") { level <= Level::Info } else { level <= Level::Warn }
}

/// One line as written: when (UTC), how serious, where from, what (a message over several lines
/// indented under it).
fn line(stamp: &str, level: Level, target: &str, msg: &str) -> String {
    let msg = msg.trim_end().replace("\r\n", "\n").replace('\n', "\n    ");
    format!("{stamp} {level:<5} {target}: {msg}\n")
}

/// Now, as `2026-10-19 14:03:07` (UTC).
fn stamp() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let secs = i64::try_from(secs).unwrap_or(0);
    let (y, m, d) = mailcraft_engine::model::dates::civil_from_days(secs.div_euclid(86_400));
    let t = secs.rem_euclid(86_400);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}", t / 3600, t % 3600 / 60, t % 60)
}

/// Start logging into `dir` (the previous run's log kept as the old one). Without a folder, or
/// when the file can't be made, lines go to the terminal only if `echo`.
pub fn start(dir: Option<&Path>, echo: bool) -> Option<PathBuf> {
    let path = dir.map(|d| d.join(NAME));
    let file = path.as_ref().and_then(|p| {
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
            let _ = std::fs::rename(p, d.join(OLD));
        }
        File::create(p).ok()
    });
    let opened = file.is_some();
    let logger: &'static FileLog = Box::leak(Box::new(FileLog::new(file, echo)));
    if log::set_logger(logger).is_ok() {
        log::set_max_level(LevelFilter::Info);
        logger.start_thread();
    }
    path.filter(|_| opened)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A terminal that's gone doesn't end MailCraft (found when its starter exited).
    #[test]
    fn a_closed_terminal_is_passed_over() {
        struct Gone;
        impl Write for Gone {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe))
            }
        }
        echo(
            &mut Gone, "a line
",
        );
        let log = FileLog::new(None, true);
        log.hand_over(
            "still here
",
        );
    }

    #[test]
    fn lines_and_levels() {
        assert!(enabled("mailcraft_net::worker", Level::Info));
        assert!(!enabled("mailcraft_net::worker", Level::Debug));
        assert!(!enabled("rustls::client", Level::Info));
        assert!(enabled("rustls::client", Level::Warn));
        assert_eq!(line("2026-10-19 14:03:07", Level::Warn, "x", "a\r\nb\n"), "2026-10-19 14:03:07 WARN  x: a\n    b\n");
        let s = stamp();
        assert_eq!((s.len(), s.as_bytes()[4], s.as_bytes()[13]), (19, b'-', b':'));
    }

    #[test]
    fn a_full_log_stops_and_says_so() {
        let dir = std::env::temp_dir().join(format!("mailcraft-log-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(NAME);
        let log = FileLog::new(Some(File::create(&path).unwrap()), false);
        if let Some(out) = log.out.lock().unwrap().as_mut() {
            out.1 = MAX - 10;
        }
        log.write_line("0123456789abcdef\n");
        log.write_line("never written\n");
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("0123456789abcdef\n... the log is full"), "{text}");
        assert!(!text.contains("never written"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A line logged while the disk holds the log's writes (a write that takes seconds while a
    /// big data file is flushed) doesn't wait for it: it's written after, in order, and a flush
    /// waits for it.
    #[test]
    fn a_line_logged_never_waits_for_the_disk() {
        use log::Log;
        let dir = std::env::temp_dir().join(format!("mailcraft-log-wait-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(NAME);
        let log: &'static FileLog = Box::leak(Box::new(FileLog::new(Some(File::create(&path).unwrap()), false)));
        log.start_thread();
        let record = |msg: &str| log.hand_over(&line("2026-10-19 14:03:07", Level::Warn, "mailcraft::x", msg));
        record("first");
        log.flush();
        let held = log.out.lock().unwrap();
        let began = std::time::Instant::now();
        record("while the disk is busy");
        record("and another");
        assert!(began.elapsed() < Duration::from_millis(500), "logging waited {:?}", began.elapsed());
        drop(held);
        log.flush();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains("x: first\n") && text.ends_with("x: while the disk is busy\n2026-10-19 14:03:07 WARN  mailcraft::x: and another\n"),
            "{text}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Past what may wait, lines are left out and counted rather than held.
    #[test]
    fn too_many_lines_waiting_are_counted_not_kept() {
        use log::Log;
        let dir = std::env::temp_dir().join(format!("mailcraft-log-full-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(NAME);
        let log: &'static FileLog = Box::leak(Box::new(FileLog::new(Some(File::create(&path).unwrap()), false)));
        log.start_thread();
        let held = log.out.lock().unwrap();
        let long = "x".repeat(WAITING_MAX / 2 + 1);
        for _ in 0..3 {
            log.hand_over(&format!("{long}\n"));
        }
        // The first taken by the thread (held by the disk) or waiting, then at most one more.
        assert!(log.waiting().text.len() <= WAITING_MAX);
        drop(held);
        log.flush();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("lines left out here: more came than the disk took"), "{} bytes", text.len());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
