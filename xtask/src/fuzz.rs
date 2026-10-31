//! `cargo xtask fuzz`: every fuzz target has what the weekly run gives it (fuzz/README.md,
//! `.github/workflows/fuzz.yml`): its source, at least one seed and a dictionary. libFuzzer stops
//! at a dictionary that isn't there, and nothing but the weekly run would notice.

use std::path::Path;

/// The targets `fuzz/Cargo.toml` lists, by name.
fn targets(manifest: &str) -> Vec<String> {
    manifest
        .lines()
        .filter(|l| l.contains("path = \"fuzz_targets/"))
        .filter_map(|l| l.split("name = \"").nth(1)?.split('"').next().map(str::to_string))
        .collect()
}

/// What each target lacks, one line each.
fn missing(fuzz: &Path, names: &[String]) -> Vec<String> {
    let has_file = |p: &Path| std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.len() > 0);
    let has_seed = |p: &Path| std::fs::read_dir(p).is_ok_and(|mut d| d.any(|e| e.is_ok_and(|e| has_file(&e.path()))));
    let mut out = Vec::new();
    for name in names {
        if !has_file(&fuzz.join("fuzz_targets").join(format!("{name}.rs"))) {
            out.push(format!("fuzz/fuzz_targets/{name}.rs"));
        }
        if !has_seed(&fuzz.join("seeds").join(name)) {
            out.push(format!("fuzz/seeds/{name}/ (a seed)"));
        }
        if !has_file(&fuzz.join("dicts").join(format!("{name}.dict"))) {
            out.push(format!("fuzz/dicts/{name}.dict"));
        }
    }
    out
}

pub fn run(root: &Path) -> Result<(), String> {
    let fuzz = root.join("fuzz");
    let manifest = std::fs::read_to_string(fuzz.join("Cargo.toml")).map_err(|e| format!("fuzz/Cargo.toml: {e}"))?;
    let names = targets(&manifest);
    if names.is_empty() {
        return Err("fuzz/Cargo.toml lists no fuzz targets".into());
    }
    let missing = missing(&fuzz, &names);
    if missing.is_empty() {
        println!("fuzz: {} targets, each with its seeds and dictionary", names.len());
        Ok(())
    } else {
        Err(format!("fuzz targets lack what the weekly run needs:\n  {}", missing.join("\n  ")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_targets_and_finds_whats_missing() {
        let manifest = "[package]\nname = \"mailcraft-fuzz\"\n[[bin]]\nbin = [\n    { name = \"photo\", path = \"fuzz_targets/photo.rs\" }, # photos\n    { name = \"nothing\", path = \"fuzz_targets/nothing.rs\" },\n]\n";
        let names = targets(manifest);
        assert_eq!(names, ["photo", "nothing"]);
        let fuzz = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("fuzz");
        let missing = missing(&fuzz, &names);
        assert_eq!(missing, ["fuzz/fuzz_targets/nothing.rs", "fuzz/seeds/nothing/ (a seed)", "fuzz/dicts/nothing.dict"]);
    }
}
