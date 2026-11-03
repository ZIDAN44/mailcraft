//! Workspace tooling: `cargo xtask <command>`.
//!
//! Pure Rust (std, serde_json, png, icu_normalizer). External tools (`cargo`, `git`) run through
//! `std::process::Command`.

mod fold;
mod fuzz;
mod ico;
mod icons;
mod sizes;
mod version;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use serde_json::Value;

const USAGE: &str = "\
usage: cargo xtask <command>

commands:
  ci       fmt --check, sizes, fold --check, clippy -D warnings, tests, forks pinned alike, layers,
           assets, fuzz, wasm (stops at the first failure)
  layers   enforce the crate layering (docs/architecture.md)
  assets   check that every asset file is attributed in ATTRIBUTION.md
  fuzz     check that every fuzz target has its seeds and dictionary (fuzz/README.md)
  sizes    files stay under 1000 lines and folders under 16 Rust files (xtask/sizes.txt holds
           what was over then, which may shrink but never grow); `sizes --update` lowers it
  fold     write search's fold table (crates/model/src/search/fold_table.rs); `fold --check`
           fails when the one committed differs
  wasm     cargo check --target wasm32-unknown-unknown for the portable crates and the web app
  icons    render the app icon into assets/app-icon/ (PNGs, SVG, .ico, .icns)
  ico      pack PNGs into a Windows .ico: ico <out.ico> <in.png>…
  version  print the workspace version; `version set X.Y.Z[-pre]` changes it (release builds)
";

/// Crate layers. A crate may depend only on crates in lower layers.
const LAYERS: &[(&str, u8)] = &[
    ("mailcraft-model", 0),
    ("mailcraft-mime", 1),
    ("mailcraft-store", 1),
    ("mailcraft-print", 1),
    ("mailcraft-smime", 1),
    ("mailcraft-jmap", 2),
    ("mailcraft-keys", 2),
    ("mailcraft-net", 3),
    ("mailcraft-engine", 4),
    ("mailcraft-ui-egui", 6),
];

/// Third-party crates built from a fork with fixes of ours, pinned by revision in
/// `[patch.crates-io]`. Each fork's `PATCHES.md` lists its fixes, and its own tests prove them.
const FORKED: &[&str] = &["outlook-pst-rw"];

/// Crates that never build for the web (sockets, threads, TLS).
const NATIVE_ONLY: &[&str] = &["mailcraft-keys", "mailcraft-net"];

/// External crates that belong to one layer only.
const RESTRICTED: &[(&str, &[&str])] = &[
    // UI toolkits: only the UI crate and the apps.
    ("egui", &["mailcraft-ui-egui"]),
    ("eframe", &[]),
    ("winit", &[]),
    ("rfd", &[]),
    // The network layer is wired up by the apps only: the engine stays sans-I/O.
    ("mailcraft-net", &[]),
    // Network stacks: only the network layer.
    ("lettre", &["mailcraft-net"]),
    ("rustls", &["mailcraft-net"]),
    ("imap-proto", &["mailcraft-net"]),
];

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."))
}

fn cargo() -> Command {
    let mut c = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    c.current_dir(root());
    c
}

fn run(mut cmd: Command, what: &str) -> Result<(), String> {
    eprintln!("$ {what}");
    let status = cmd.status().map_err(|e| format!("{what}: failed to start: {e}"))?;
    if status.success() { Ok(()) } else { Err(format!("{what}: exited with {status}")) }
}

fn metadata() -> Result<Value, String> {
    let out = cargo().args(["metadata", "--format-version", "1", "--no-deps"]).output().map_err(|e| format!("cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err(format!("cargo metadata failed:\n{}", String::from_utf8_lossy(&out.stderr)));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| format!("cargo metadata: {e}"))
}

fn layer(name: &str) -> Option<u8> {
    LAYERS.iter().find(|(n, _)| *n == name).map(|(_, l)| *l)
}

/// Layering violations in `(crate, [(dependency, is_dev)])` form.
fn violations(crates: &[(String, Vec<(String, bool)>)]) -> Vec<String> {
    let mut out = Vec::new();
    for (name, deps) in crates {
        let Some(own) = layer(name) else { continue };
        for (dep, dev) in deps {
            if *dev {
                continue;
            }
            if let Some(l) = layer(dep)
                && l >= own
            {
                out.push(format!("{name} (L{own}) depends on {dep} (L{l})"));
            }
            if let Some((_, allowed)) = RESTRICTED.iter().find(|(c, _)| c == dep)
                && !allowed.contains(&name.as_str())
            {
                out.push(format!("{name} depends on {dep}, which only {allowed:?} may use"));
            }
        }
    }
    out
}

fn cmd_layers() -> Result<(), String> {
    let meta = metadata()?;
    let crates: Vec<(String, Vec<(String, bool)>)> = meta["packages"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|p| {
            let deps = p["dependencies"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|d| (d["name"].as_str().unwrap_or("").to_string(), d["kind"].as_str() == Some("dev")))
                .collect();
            (p["name"].as_str().unwrap_or("").to_string(), deps)
        })
        .collect();
    for (name, deps) in &crates {
        let ws: Vec<&str> = deps.iter().filter(|(d, dev)| !dev && d.starts_with("mailcraft-")).map(|(d, _)| d.as_str()).collect();
        let l = layer(name).map_or("app".to_string(), |l| format!("L{l}"));
        println!("{name:<20} {l:<4} {}", ws.join(", "));
    }
    let v = violations(&crates);
    if v.is_empty() {
        println!("OK: no layering violations");
        Ok(())
    } else {
        Err(format!("{} layering violation(s):\n  {}", v.len(), v.join("\n  ")))
    }
}

const ASSET_EXT: &[&str] =
    &["png", "jpg", "jpeg", "gif", "webp", "bmp", "ico", "icns", "svg", "pdf", "ttf", "otf", "woff", "woff2", "mp3", "wav", "mp4", "eml"];

fn cmd_assets() -> Result<(), String> {
    let out = Command::new("git")
        .current_dir(root())
        .args(["ls-files", "--cached", "--others", "--exclude-standard"])
        .output()
        .map_err(|e| format!("git ls-files: {e}"))?;
    let md = std::fs::read_to_string(root().join("ATTRIBUTION.md")).map_err(|e| format!("ATTRIBUTION.md: {e}"))?;
    let missing: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|f| {
            let ext = Path::new(f).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
            ASSET_EXT.contains(&ext.as_str()) || f.starts_with("assets/")
        })
        .filter(|f| !md.contains(&format!("`{f}`")))
        .map(str::to_string)
        .collect();
    if missing.is_empty() {
        println!("assets: every asset file is attributed");
        Ok(())
    } else {
        Err(format!("{} asset file(s) lack an ATTRIBUTION.md row:\n  {}", missing.len(), missing.join("\n  ")))
    }
}

fn cmd_wasm() -> Result<(), String> {
    let mut c = cargo();
    c.args(["check", "--target", "wasm32-unknown-unknown"]);
    for (name, _) in LAYERS.iter().filter(|(n, _)| !NATIVE_ONLY.contains(n)) {
        c.args(["-p", name]);
    }
    // The browser app itself.
    c.args(["-p", "mailcraft-web"]);
    run(c, "cargo check --target wasm32-unknown-unknown (portable crates)")
}

/// Each forked crate is what both workspaces build, at the same revision: the main one and the
/// fuzzing one (a fix missing from the fuzz build would leave the weekly run fuzzing the code it
/// replaced).
fn patched(root: &Path) -> Result<(), String> {
    let read = |manifest: &str| std::fs::read_to_string(root.join(manifest)).map_err(|e| format!("{manifest}: {e}"));
    let (main, fuzz) = (read("Cargo.toml")?, read("fuzz/Cargo.toml")?);
    let wrong: Vec<String> = FORKED
        .iter()
        .filter_map(|name| match (pin(&main, name), pin(&fuzz, name)) {
            (Some(a), Some(b)) if a == b => None,
            (a, b) => Some(format!("{name}: {a:?} in Cargo.toml, {b:?} in fuzz/Cargo.toml")),
        })
        .collect();
    if wrong.is_empty() { Ok(()) } else { Err(format!("forked crates not pinned alike in [patch.crates-io]:\n  {}", wrong.join("\n  "))) }
}

/// The fork and revision a manifest's `[patch.crates-io]` takes `name` from, as `<git>@<rev>`.
fn pin(manifest: &str, name: &str) -> Option<String> {
    // The section itself, at the start of a line (a comment may name it too).
    let patch = manifest.split("\n[patch.crates-io]").nth(1).map(|p| p.split("\n[").next().unwrap_or(p))?;
    let line = patch.lines().find(|l| l.trim_start().starts_with(&format!("{name} ")))?;
    let field = |key: &str| line.split(&format!("{key} = \"")).nth(1).and_then(|v| v.split('"').next());
    Some(format!("{}@{}", field("git")?, field("rev")?))
}

fn cmd_ci() -> Result<(), String> {
    let mut fmt = cargo();
    fmt.args(["fmt", "--all", "--", "--check"]);
    run(fmt, "cargo fmt --check")?;
    sizes::run(&root(), &[])?;
    fold::run(&root(), &["--check"])?;
    let mut clippy = cargo();
    clippy.args(["clippy", "--workspace", "--all-targets", "--", "-D", "warnings"]);
    run(clippy, "cargo clippy -D warnings")?;
    let mut test = cargo();
    test.args(["test", "--workspace"]);
    run(test, "cargo test")?;
    patched(&root())?;
    cmd_layers()?;
    cmd_assets()?;
    fuzz::run(&root())?;
    cmd_wasm()
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let r = match args.first().map(String::as_str) {
        Some("ci") => cmd_ci(),
        Some("layers") => cmd_layers(),
        Some("assets") => cmd_assets(),
        Some("fuzz") => fuzz::run(&root()),
        Some("sizes") => sizes::run(&root(), &args.iter().skip(1).map(String::as_str).collect::<Vec<_>>()),
        Some("fold") => fold::run(&root(), &args.iter().skip(1).map(String::as_str).collect::<Vec<_>>()),
        Some("wasm") => cmd_wasm(),
        Some("icons") => icons::run(&root()),
        Some("ico") => ico::run(&args.iter().skip(1).map(String::as_str).collect::<Vec<_>>()),
        Some("version") => version::run(&root(), &args.iter().skip(1).map(String::as_str).collect::<Vec<_>>()),
        _ => {
            print!("{USAGE}");
            Ok(())
        }
    };
    match r {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catches_upward_and_restricted_dependencies() {
        let crates = vec![
            ("mailcraft-engine".to_string(), vec![("mailcraft-net".to_string(), false), ("mailcraft-model".to_string(), false)]),
            ("mailcraft-model".to_string(), vec![("egui".to_string(), false)]),
            ("mailcraft-store".to_string(), vec![("mailcraft-engine".to_string(), true)]),
        ];
        let v = violations(&crates);
        assert_eq!(v.len(), 2, "{v:?}");
        assert!(v[0].contains("mailcraft-net"));
        assert!(v[1].contains("egui"));
    }

    #[test]
    fn forked_crates_are_pinned_alike() {
        let ok = "# [patch.crates-io] below\n[patch.crates-io]\negui = { git = \"x\" }\noutlook-pst-rw = { git = \"https://f.git\", rev = \"1a\" }\n";
        assert_eq!(pin(ok, "outlook-pst-rw").as_deref(), Some("https://f.git@1a"));
        assert_eq!(pin(&ok.replace("1a", "2b"), "outlook-pst-rw").as_deref(), Some("https://f.git@2b"), "another revision");
        assert_eq!(pin(&ok.replace(", rev = \"1a\"", ""), "outlook-pst-rw"), None, "a branch's head isn't a pin");
        let elsewhere = "[dependencies]\noutlook-pst-rw = { git = \"https://f.git\", rev = \"1a\" }\n[patch.crates-io]\negui = \"1\"\n";
        assert_eq!(pin(elsewhere, "outlook-pst-rw"), None, "only [patch.crates-io] counts");
        // A crate whose name begins another's has its own pin.
        let both = format!("{ok}outlook-pst = {{ git = \"https://f.git\", rev = \"3c\" }}\n");
        assert_eq!(pin(&both, "outlook-pst-rw").as_deref(), Some("https://f.git@1a"));
        assert_eq!(pin(&both, "outlook-pst").as_deref(), Some("https://f.git@3c"));
        assert_eq!(pin(ok, "outlook-pst"), None, "another crate's pin isn't this one's");
        assert!(patched(&root()).is_ok(), "{:?}", patched(&root()));
    }
}
