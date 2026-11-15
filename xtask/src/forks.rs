//! `cargo xtask ci`'s fork check: a third-party crate built from a fork with fixes of ours is
//! pinned by revision in `[patch.crates-io]` (each fork's `PATCHES.md` lists its fixes, and its
//! own tests prove them). Where the fuzz workspace builds that crate too, its manifest pins the
//! same fork at the same revision: a fix missing from the fuzz build would leave the weekly run
//! fuzzing the code it replaced. Which crates are forked is read from the manifests themselves,
//! so a fork added (or one the fuzz workspace starts to build) is checked without a list to
//! keep up.

use std::collections::BTreeSet;
use std::path::Path;

use crate::locks::{Locked, packages};

/// The lines of a manifest's `[patch.crates-io]`, when it has one (the section's own heading at
/// the start of a line: a comment may name it too).
fn patch_section(manifest: &str) -> Option<&str> {
    const HEADING: &str = "[patch.crates-io]";
    let at = if manifest.starts_with(HEADING) { 0 } else { manifest.find(&format!("\n{HEADING}"))? + 1 };
    let rest = manifest.get(at + HEADING.len()..)?;
    Some(rest.split("\n[").next().unwrap_or(rest))
}

/// The section's entries: each crate's name (quoted or not) and what it's taken from.
fn entries(manifest: &str) -> impl Iterator<Item = (&str, &str)> {
    patch_section(manifest)
        .into_iter()
        .flat_map(str::lines)
        .map(str::trim)
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| l.split_once('='))
        .map(|(name, from)| (name.trim().trim_matches('"'), from))
        .filter(|(name, _)| !name.is_empty())
}

/// The crates a manifest's `[patch.crates-io]` takes from elsewhere.
fn patched(manifest: &str) -> BTreeSet<String> {
    entries(manifest).map(|(name, _)| name.to_string()).collect()
}

/// The fork and revision a manifest's `[patch.crates-io]` takes `name` from, as `<git>@<rev>`.
fn pin(manifest: &str, name: &str) -> Option<String> {
    let (_, line) = entries(manifest).find(|(n, _)| *n == name)?;
    let field = |key: &str| line.split(&format!("{key} = \"")).nth(1).and_then(|v| v.split('"').next());
    Some(format!("{}@{}", field("git")?, field("rev")?))
}

/// Each crate either manifest patches that both workspaces build, and that isn't taken from the
/// same fork at the same revision by both: what's wrong with each.
fn unlike(main: &str, fuzz: &str, main_builds: &Locked, fuzz_builds: &Locked) -> Vec<String> {
    patched(main)
        .union(&patched(fuzz))
        .filter(|name| main_builds.contains_key(*name) && fuzz_builds.contains_key(*name))
        .filter_map(|name| match (pin(main, name), pin(fuzz, name)) {
            (Some(a), Some(b)) if a == b => None,
            (a, b) => Some(format!("{name}: {a:?} in Cargo.toml, {b:?} in fuzz/Cargo.toml")),
        })
        .collect()
}

pub fn run(root: &Path) -> Result<(), String> {
    let read = |file: &str| std::fs::read_to_string(root.join(file)).map_err(|e| format!("{file}: {e}"));
    let (main, fuzz) = (read("Cargo.toml")?, read("fuzz/Cargo.toml")?);
    let wrong = unlike(&main, &fuzz, &packages(&read("Cargo.lock")?), &packages(&read("fuzz/Cargo.lock")?));
    if wrong.is_empty() {
        println!("forks: what both workspaces build of the {} forked crates is pinned alike", patched(&main).len());
        return Ok(());
    }
    Err(format!("forked crates not pinned alike in [patch.crates-io]:\n  {}", wrong.join("\n  ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pin_is_read_from_the_patch_section() {
        let ok = "# [patch.crates-io] below\n[patch.crates-io]\negui = { git = \"x\" }\nforked-rw = { git = \"https://f.git\", rev = \"1a\" }\n";
        assert_eq!(pin(ok, "forked-rw").as_deref(), Some("https://f.git@1a"));
        assert_eq!(pin(&ok.replace("1a", "2b"), "forked-rw").as_deref(), Some("https://f.git@2b"), "another revision");
        assert_eq!(pin(&ok.replace(", rev = \"1a\"", ""), "forked-rw"), None, "a branch's head isn't a pin");
        let elsewhere = "[dependencies]\nforked-rw = { git = \"https://f.git\", rev = \"1a\" }\n[patch.crates-io]\negui = \"1\"\n";
        assert_eq!(pin(elsewhere, "forked-rw"), None, "only [patch.crates-io] counts");
        // A crate whose name begins another's has its own pin.
        let both = format!("{ok}forked = {{ git = \"https://f.git\", rev = \"3c\" }}\n");
        assert_eq!(pin(&both, "forked-rw").as_deref(), Some("https://f.git@1a"));
        assert_eq!(pin(&both, "forked").as_deref(), Some("https://f.git@3c"));
        assert_eq!(pin(ok, "forked"), None, "another crate's pin isn't this one's");
    }

    /// The forked crates are the patch section's, comments and other sections left out, with
    /// Windows line ends too.
    #[test]
    fn the_forked_crates_are_read_from_the_manifest() {
        let m = "[dependencies]\nserde = \"1\"\n\n[patch.crates-io]\r\n# egui's family\r\negui = { git = \"https://e.git\", rev = \"6d\" }\r\n\"winit\" = { git = \"https://w.git\", rev = \"a5\" }\r\n\r\n[profile.release]\r\nlto = true\r\n";
        assert_eq!(patched(m).into_iter().collect::<Vec<_>>(), ["egui", "winit"]);
        assert_eq!(pin(m, "winit").as_deref(), Some("https://w.git@a5"), "a quoted name's pin");
        assert!(patched("[dependencies]\nserde = \"1\"\n").is_empty());
    }

    /// What both build must be pinned alike; what only one builds isn't compared.
    #[test]
    fn what_both_build_is_pinned_alike() {
        let main = "[patch.crates-io]\negui = { git = \"https://e.git\", rev = \"6d\" }\nredb = { git = \"https://r.git\", rev = \"1a\" }\n";
        let lock = |names: &[&str]| -> Locked { names.iter().map(|n| (n.to_string(), vec!["1.0.0".to_string()])).collect() };
        let builds = lock(&["egui", "redb", "serde"]);
        let fuzz_ok = "[dependencies]\n[patch.crates-io]\nredb = { git = \"https://r.git\", rev = \"1a\" }\n";
        // The fuzz workspace builds no egui: only redb is compared.
        assert!(unlike(main, fuzz_ok, &builds, &lock(&["redb", "libfuzzer-sys"])).is_empty());
        // Not patched there, at another revision, or patched there only: each found.
        let none = unlike(main, "[dependencies]\n", &builds, &lock(&["redb"]));
        assert_eq!(none, ["redb: Some(\"https://r.git@1a\") in Cargo.toml, None in fuzz/Cargo.toml"]);
        assert_eq!(unlike(main, &fuzz_ok.replace("1a", "2b"), &builds, &lock(&["redb"])).len(), 1);
        let only_fuzz = "[patch.crates-io]\nserde = { git = \"https://s.git\", rev = \"9f\" }\nredb = { git = \"https://r.git\", rev = \"1a\" }\n";
        assert_eq!(
            unlike(main, only_fuzz, &builds, &lock(&["redb", "serde"])),
            ["serde: None in Cargo.toml, Some(\"https://s.git@9f\") in fuzz/Cargo.toml"]
        );
        // The fuzz workspace starting to build egui: it must take the same fork.
        assert_eq!(unlike(main, fuzz_ok, &builds, &lock(&["redb", "egui"])).len(), 1);
    }

    #[test]
    fn the_workspaces_are_pinned_alike() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        assert_eq!(run(&root), Ok(()));
        let main = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
        assert!(patched(&main).contains("egui"), "the forks are found: {:?}", patched(&main));
    }
}
