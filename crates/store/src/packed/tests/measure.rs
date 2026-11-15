//! What the measurement tests share: times in milliseconds, percentiles, and what the process has
//! read, written and held, as the system counts it.

use std::time::Duration;

/// `d` in milliseconds.
pub(super) fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// The value at `q` (0.5 the median, 0.99 the 99th percentile) of `v`, sorted here: the nearest
/// rank, `round((len - 1) × q)`. 0 when `v` is empty.
pub(super) fn percentile(v: &mut [f64], q: f64) -> f64 {
    v.sort_by(f64::total_cmp);
    v.get(((v.len() as f64 - 1.0) * q).round() as usize).copied().unwrap_or(0.0)
}

/// (median, mean, 90th percentile) of `v`.
pub(super) fn spread(mut v: Vec<f64>) -> (f64, f64, f64) {
    let median = percentile(&mut v, 0.5);
    (median, v.iter().sum::<f64>() / v.len().max(1) as f64, percentile(&mut v, 0.9))
}

/// What the process has read and written (every read and write it asked for, the system's cache
/// included) and the most memory it has held, in bytes: its working set, and its private bytes
/// (on Linux, its peak virtual size).
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct Process {
    pub(super) read: u64,
    pub(super) written: u64,
    pub(super) peak: u64,
    pub(super) peak_private: u64,
}

impl Process {
    /// The counts so far, or `None` when the system's answer can't be read.
    #[cfg(windows)]
    pub(super) fn now() -> Option<Process> {
        let script = format!(
            "$p = Get-CimInstance Win32_Process -Filter 'ProcessId={}'; \
             \"$($p.ReadTransferCount) $($p.WriteTransferCount) $($p.PeakWorkingSetSize) $($p.PeakPageFileUsage)\"",
            std::process::id()
        );
        let out = std::process::Command::new("powershell").args(["-NoProfile", "-Command", &script]).output().ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        let n: Vec<u64> = text.split_whitespace().map(|w| w.parse().ok()).collect::<Option<_>>()?;
        // The peaks are in KiB.
        let [read, written, peak, private] = n[..] else { return None };
        Some(Process { read, written, peak: peak << 10, peak_private: private << 10 })
    }

    /// The counts so far, or `None` when the system's answer can't be read.
    #[cfg(not(windows))]
    pub(super) fn now() -> Option<Process> {
        let field =
            |text: &str, name: &str| text.lines().find_map(|l| l.strip_prefix(name)).and_then(|v| v.split_whitespace().next()?.parse::<u64>().ok());
        let io = std::fs::read_to_string("/proc/self/io").ok()?;
        let status = std::fs::read_to_string("/proc/self/status").ok()?;
        // The peaks are in kB.
        Some(Process {
            read: field(&io, "rchar:")?,
            written: field(&io, "wchar:")?,
            peak: field(&status, "VmHWM:")? << 10,
            peak_private: field(&status, "VmPeak:")? << 10,
        })
    }

    /// What was read and written since `before`, and the peaks now.
    pub(super) fn since(self, before: Process) -> Process {
        Process { read: self.read.saturating_sub(before.read), written: self.written.saturating_sub(before.written), ..self }
    }
}

#[test]
fn percentiles_are_nearest_ranks() {
    let mut v: Vec<f64> = (1..=100).rev().map(f64::from).collect();
    assert_eq!(percentile(&mut v, 0.99), 99.0);
    assert_eq!(percentile(&mut v, 0.5), 51.0);
    assert_eq!(percentile(&mut [], 0.5), 0.0);
    assert_eq!(spread(vec![3.0, 1.0, 2.0]), (2.0, 2.0, 3.0));
    assert_eq!(spread(Vec::new()), (0.0, 0.0, 0.0));
}

/// The system's counts of this process are read (PowerShell on Windows, `/proc` on Linux): what
/// the measurements print. Not run with the tests: a system that doesn't answer isn't a fault here.
#[test]
#[ignore = "asks the system: cargo test -p mailcraft-store --lib packed::tests::measure::the_system_counts_the_process -- --ignored"]
fn the_system_counts_the_process() {
    if cfg!(any(windows, target_os = "linux")) {
        let p = Process::now().expect("the system's counts");
        assert!(p.peak > 0 && p.peak_private > 0, "{p:?}");
    }
}
