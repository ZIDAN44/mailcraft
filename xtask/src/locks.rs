//! `cargo xtask locks`: the fuzz workspace builds the versions the app does. `fuzz/Cargo.lock`
//! is a lock of its own, which an update to `Cargo.lock` doesn't touch: left behind, the weekly
//! run fuzzes dependencies older than the ones MailCraft ships. Every package both name is at a
//! version `Cargo.lock` has; `locks --update` moves the fuzz workspace's to it.

use std::collections::BTreeMap;
use std::path::Path;

/// The versions a lock file has of each package, by name.
type Locked = BTreeMap<String, Vec<String>>;

/// A package the fuzz workspace has at a version the main one doesn't.
#[derive(Debug, PartialEq)]
struct Apart {
    name: String,
    fuzz: String,
    main: Vec<String>,
}

fn packages(lock: &str) -> Locked {
    let mut out = Locked::new();
    for block in lock.split("[[package]]").skip(1) {
        let field = |key: &str| block.lines().find_map(|l| l.trim().strip_prefix(key)?.strip_prefix(" = \"")?.strip_suffix('"'));
        if let (Some(name), Some(version)) = (field("name"), field("version")) {
            out.entry(name.to_string()).or_default().push(version.to_string());
        }
    }
    out
}

/// What the fuzz lock has at a version the main lock doesn't. A package only one of them has
/// (the fuzzer's own, or the app's) isn't compared, and the main lock may hold more versions.
fn apart(main: &Locked, fuzz: &Locked) -> Vec<Apart> {
    let mut out = Vec::new();
    for (name, versions) in fuzz {
        let Some(kept) = main.get(name) else { continue };
        for v in versions.iter().filter(|v| !kept.contains(v)) {
            out.push(Apart { name: name.clone(), fuzz: v.clone(), main: kept.clone() });
        }
    }
    out
}

/// The part of a version that others must share to stand in for it: `1` of `1.4.2`, `0.8` of
/// `0.8.4`.
fn line(version: &str) -> &str {
    let mut dots = version.match_indices('.').map(|(i, _)| i);
    let (first, second) = (dots.next(), dots.next());
    let end = if version.starts_with("0.") { second } else { first };
    end.and_then(|e| version.get(..e)).unwrap_or(version)
}

/// The main lock's version the fuzz one moves to: the one that can stand in for it, or the only
/// one there is.
fn wanted(a: &Apart) -> Option<&str> {
    let alike = a.main.iter().find(|m| line(m) == line(&a.fuzz));
    alike.or(if a.main.len() == 1 { a.main.first() } else { None }).map(String::as_str)
}

fn read(root: &Path) -> Result<Vec<Apart>, String> {
    let read = |lock: &str| std::fs::read_to_string(root.join(lock)).map_err(|e| format!("{lock}: {e}"));
    Ok(apart(&packages(&read("Cargo.lock")?), &packages(&read("fuzz/Cargo.lock")?)))
}

/// Move each package to the main lock's version. One may have to wait for another (a family
/// released together), so it's gone over again while that gets any further.
fn update(root: &Path) -> Result<(), String> {
    let mut left = read(root)?;
    while !left.is_empty() {
        for a in &left {
            let Some(to) = wanted(a) else { continue };
            let mut c = super::cargo();
            c.args(["update", "--quiet", "--manifest-path", "fuzz/Cargo.toml", "--package", &format!("{}@{}", a.name, a.fuzz), "--precise", to]);
            // Refused for now when it waits for another: the next round says.
            let _ = c.stderr(std::process::Stdio::null()).status();
        }
        let now = read(root)?;
        if now.len() >= left.len() {
            break;
        }
        left = now;
    }
    Ok(())
}

pub fn run(root: &Path, args: &[&str]) -> Result<(), String> {
    if args.contains(&"--update") {
        update(root)?;
    }
    let apart = read(root)?;
    if apart.is_empty() {
        println!("locks: fuzz/Cargo.lock has the versions Cargo.lock has");
        return Ok(());
    }
    let lines: Vec<String> =
        apart.iter().map(|a| format!("{}: {} in fuzz/Cargo.lock, {} in Cargo.lock", a.name, a.fuzz, a.main.join(" and "))).collect();
    Err(format!("the fuzz workspace builds other versions than the app (`cargo xtask locks --update` moves them):\n  {}", lines.join("\n  ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAIN: &str = "version = 4\r\n\r\n[[package]]\r\nname = \"redb\"\r\nversion = \"4.5.0\"\r\nsource = \"registry+https://x\"\r\ndependencies = [\r\n \"libc\",\r\n]\r\n\r\n[[package]]\r\nname = \"syn\"\r\nversion = \"1.0.109\"\r\n\r\n[[package]]\r\nname = \"syn\"\r\nversion = \"2.0.90\"\r\n\r\n[[package]]\r\nname = \"egui\"\r\nversion = \"0.33.0\"\r\n";

    fn fuzz(redb: &str, syn: &str) -> String {
        format!(
            "version = 4\n\n[[package]]\nname = \"libfuzzer-sys\"\nversion = \"0.4.9\"\n\n[[package]]\nname = \"redb\"\nversion = \"{redb}\"\n\n[[package]]\nname = \"syn\"\nversion = \"{syn}\"\n"
        )
    }

    #[test]
    fn a_package_at_another_version_is_found() {
        let main = packages(MAIN);
        assert_eq!(main.get("syn").map(Vec::len), Some(2), "{main:?}");
        assert_eq!(main.get("redb"), Some(&vec!["4.5.0".to_string()]), "a dependency's name isn't the package's");
        // The same, and one of the two the main lock has: alike. The fuzzer's own isn't compared.
        assert!(apart(&main, &packages(&fuzz("4.5.0", "2.0.90"))).is_empty());
        assert!(apart(&main, &packages(&fuzz("4.5.0", "1.0.109"))).is_empty());
        let found = apart(&main, &packages(&fuzz("4.4.0", "2.0.87")));
        assert_eq!(
            found.iter().map(|a| (a.name.as_str(), a.fuzz.as_str(), wanted(a))).collect::<Vec<_>>(),
            [("redb", "4.4.0", Some("4.5.0")), ("syn", "2.0.87", Some("2.0.90")),]
        );
        // None that can stand in for it among several: left to be sorted out by hand.
        let odd = apart(&main, &packages(&fuzz("4.5.0", "3.0.0")));
        assert_eq!(odd.first().map(wanted), Some(None));
        // The only one there is, a major version on.
        let on = apart(&main, &packages(&fuzz("3.1.0", "2.0.90")));
        assert_eq!(on.first().and_then(wanted), Some("4.5.0"));
    }

    #[test]
    fn versions_that_stand_in_for_each_other_share_a_line() {
        assert_eq!([line("1.4.2"), line("0.8.4"), line("0.0.3"), line("12"), line("")], ["1", "0.8", "0.0", "12", ""]);
    }

    #[test]
    fn the_fuzz_workspace_builds_what_the_app_does() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        assert_eq!(read(&root), Ok(Vec::new()));
    }
}
