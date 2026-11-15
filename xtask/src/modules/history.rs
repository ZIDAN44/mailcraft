//! Change coupling: Rust files committed together, again and again (Gall et al.'s logical
//! coupling; Tornhill's behavioural code analysis). It shows what changes for the same reasons,
//! which the code's own references don't always say. Files are followed through renames.

use std::collections::BTreeMap;
use std::path::Path;

/// A commit touching more Rust files than this is a sweep (a rename, a lint, a format) and says
/// nothing about what belongs together (Mondal et al., ICPC 2013, saw no coupling past ~20).
const SWEEP: usize = 20;
/// Pairs seen together fewer times than this are chance.
const MIN_TOGETHER: usize = 2;

/// For files under `prefix`: each pair's share of the commits either is in that have both
/// (the Jaccard index: a file in every commit doesn't tie itself to everything), for pairs
/// committed together at least [`MIN_TOGETHER`] times.
pub fn coupling(root: &Path, prefix: &str) -> Result<BTreeMap<(String, String), f64>, String> {
    let out = std::process::Command::new("git")
        .current_dir(root)
        .args(["log", "--no-merges", "--name-status", "-M", "--format=format:@commit"])
        .output()
        .map_err(|e| format!("git log: {e}"))?;
    if !out.status.success() {
        return Err(format!("git log: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut alone: BTreeMap<String, usize> = BTreeMap::new();
    let mut together: BTreeMap<(String, String), usize> = BTreeMap::new();
    // A file moved since is known by where it is now: newest commits come first, so a rename is
    // seen before the commits that named the file where it was.
    let mut now_at: BTreeMap<String, String> = BTreeMap::new();
    for commit in text.split("@commit") {
        let mut rust: Vec<String> = Vec::new();
        for line in commit.lines() {
            let fields: Vec<&str> = line.split('\t').collect();
            let path = match fields.as_slice() {
                [status, from, to] if status.starts_with('R') => {
                    let current = now_at.get(*to).cloned().unwrap_or_else(|| (*to).to_string());
                    now_at.insert((*from).to_string(), current.clone());
                    current
                }
                [_, path] => now_at.get(*path).cloned().unwrap_or_else(|| (*path).to_string()),
                _ => continue,
            };
            if path.ends_with(".rs") {
                rust.push(path);
            }
        }
        let rust: Vec<&str> = rust.iter().map(String::as_str).collect();
        if rust.is_empty() || rust.len() > SWEEP {
            continue;
        }
        let mut mine: Vec<&str> = rust.into_iter().filter(|f| f.starts_with(prefix)).collect();
        mine.sort_unstable();
        mine.dedup();
        for f in &mine {
            *alone.entry((*f).to_string()).or_default() += 1;
        }
        for (i, a) in mine.iter().enumerate() {
            for b in mine.iter().skip(i + 1) {
                *together.entry(((*a).to_string(), (*b).to_string())).or_default() += 1;
            }
        }
    }
    Ok(together
        .into_iter()
        .filter(|(_, n)| *n >= MIN_TOGETHER)
        .map(|((a, b), n)| {
            let either = alone.get(&a).copied().unwrap_or(n) + alone.get(&b).copied().unwrap_or(n) - n;
            let share = n as f64 / either.max(1) as f64;
            ((a, b), share)
        })
        .collect())
}
