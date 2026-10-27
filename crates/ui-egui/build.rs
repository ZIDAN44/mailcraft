//! Fonts from storytold/craft-fonts, a fully optional build input: with `CRAFT_FONTS_DIR` set to
//! a checkout, the fonts in its `fonts/manifest.txt` (and their licences) are compiled in as
//! fallbacks; without it, `CRAFT_FONTS` is empty and nothing changes. See craft-fonts'
//! `docs/integration.md`.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo::rerun-if-env-changed=CRAFT_FONTS_DIR");
    println!("cargo::rerun-if-env-changed=CRAFT_FONTS_REQUIRED");
    // Release builds set CRAFT_FONTS_REQUIRED=1: then fonts not built in are an error, a
    // CRAFT_FONTS_DIR left unset or empty included (that built a release without its fallback
    // fonts, saying nothing). Only "1" turns it on.
    let required = std::env::var_os("CRAFT_FONTS_REQUIRED").is_some_and(|v| v == "1");
    let mut src = String::from("pub static CRAFT_FONTS: &[CraftFont] = &[\n");
    match std::env::var_os("CRAFT_FONTS_DIR").filter(|d| !d.is_empty()).map(PathBuf::from) {
        Some(dir) => match craft_fonts(&dir) {
            Ok(entries) => src.push_str(&entries),
            Err(e) if required => println!("cargo::error=CRAFT_FONTS_DIR={}: {e}", dir.display()),
            Err(e) => println!("cargo::warning=building without craft-fonts: CRAFT_FONTS_DIR={}: {e}", dir.display()),
        },
        None if required => {
            println!("cargo::error=CRAFT_FONTS_REQUIRED=1 but CRAFT_FONTS_DIR is unset or empty: point it at a craft-fonts checkout");
        }
        None => {}
    }
    src.push_str("];\n");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap_or_default()).join("craft_fonts.rs");
    if let Err(e) = std::fs::write(&out, src) {
        println!("cargo::error=writing {}: {e}", out.display());
    }
}

/// One `CraftFont { .. }` initialiser per manifest line.
fn craft_fonts(dir: &Path) -> Result<String, String> {
    let manifest = dir.join("fonts/manifest.txt");
    println!("cargo::rerun-if-changed={}", manifest.display());
    let text = std::fs::read_to_string(&manifest).map_err(|e| format!("{}: {e}", manifest.display()))?;
    // The browser build has a download to keep small: the Japanese and Korean interface fonts and
    // the small Arabic, Hebrew and Thai ones (the browser has no system fonts to fall back on).
    // NanumGothic adds about 2 MB to the .wasm (0.4 MB with brotli), Noto Sans Hebrew 66 KB.
    const WEB_FONTS: &[(&str, &str)] = &[
        ("BIZ UDPGothic", "Regular"),
        ("NanumGothic", "Regular"),
        ("Noto Sans Arabic", "Regular"),
        ("Noto Sans Hebrew", "Regular"),
        ("Noto Sans Thai", "Regular"),
    ];
    let wasm = std::env::var("CARGO_CFG_TARGET_ARCH").is_ok_and(|a| a == "wasm32");
    let mut out = String::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        let f: Vec<&str> = line.split(" | ").map(str::trim).collect();
        let [family, style, file, scripts, licence, licence_file, ..] = f.as_slice() else {
            return Err(format!("malformed manifest line: {line}"));
        };
        if wasm && !WEB_FONTS.contains(&(*family, *style)) {
            continue;
        }
        let path = dir.join(file).canonicalize().map_err(|e| format!("{file}: {e}"))?;
        let licence_path = dir.join(licence_file).canonicalize().map_err(|e| format!("{licence_file}: {e}"))?;
        println!("cargo::rerun-if-changed={}", path.display());
        println!("cargo::rerun-if-changed={}", licence_path.display());
        let scripts: Vec<String> = scripts.split(',').map(|s| format!("{:?}", s.trim())).collect();
        let _ = writeln!(
            out,
            "    CraftFont {{ family: {family:?}, style: {style:?}, scripts: &[{}], licence: {licence:?}, licence_text: include_str!({:?}), bytes: include_bytes!({:?}) }},",
            scripts.join(", "),
            licence_path.display().to_string(),
            path.display().to_string(),
        );
    }
    Ok(out)
}
