//! What the measurement tests read off: a percentile of the times they took, and this process's
//! peak working set.

/// The `q`th percentile of `sorted` (sorted already, smallest first): the value `q`% of the way
/// along, the last for 100; `None` when there's none.
pub(crate) fn percentile<T: Copy>(sorted: &[T], q: usize) -> Option<T> {
    sorted.get((sorted.len().saturating_mul(q) / 100).min(sorted.len().saturating_sub(1))).copied()
}

/// This process's peak working set, in bytes (Windows' count, asked of PowerShell; Linux's
/// `VmHWM`); `None` where neither is there.
pub(crate) fn peak_working_set() -> Option<u64> {
    if cfg!(windows) {
        let script = format!("(Get-Process -Id {}).PeakWorkingSet64", std::process::id());
        let out = std::process::Command::new("powershell").args(["-NoProfile", "-Command", &script]).output().ok()?;
        return String::from_utf8_lossy(&out.stdout).trim().parse().ok();
    }
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let kb = status.lines().find_map(|l| l.strip_prefix("VmHWM:"))?.trim().trim_end_matches("kB").trim().parse::<u64>().ok()?;
    Some(kb << 10)
}
