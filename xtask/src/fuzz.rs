//! `cargo xtask fuzz`: every fuzz target has what the weekly run gives it (fuzz/README.md,
//! `.github/workflows/fuzz.yml`): its source, at least one seed and a dictionary. libFuzzer stops
//! at a dictionary that isn't there, and nothing but the weekly run would notice.

use std::path::Path;

/// The targets `fuzz/Cargo.toml` lists: name and source (under `fuzz_targets/`, or in a folder
/// of its role there once that's full: `store/search_index.rs`).
fn targets(manifest: &str) -> Vec<(String, String)> {
    let field = |l: &str, f: &str| l.split(&format!("{f} = \"")).nth(1)?.split('"').next().map(str::to_string);
    manifest.lines().filter(|l| l.contains("path = \"fuzz_targets/")).filter_map(|l| Some((field(l, "name")?, field(l, "path")?))).collect()
}

/// The lines (from 1) of a dictionary libFuzzer can't read, as its `ParseDictionaryFile` reads
/// them: past spaces at either end, a line is empty, a `#` comment, or a `"…"` not empty (a name
/// and `=` may come first) of printable ASCII, whose only escapes are `\\`, `\"` and `\xHH`.
/// libFuzzer stops at the first it can't, before fuzzing anything: a byte written raw, or a
/// line end in a value, which cuts it in two.
fn unreadable(dict: &[u8]) -> Vec<usize> {
    let entry = |line: &[u8]| -> bool {
        let line = line.trim_ascii();
        if line.is_empty() || line.first() == Some(&b'#') {
            return true;
        }
        let Some(start) = line.iter().position(|&b| b == b'"') else { return false };
        let Some(value) = line.get(start + 1..).and_then(|v| v.strip_suffix(b"\"")) else { return false };
        if value.is_empty() {
            return false;
        }
        let mut bytes = value.iter();
        while let Some(&b) = bytes.next() {
            if !((0x20..=0x7e).contains(&b) || matches!(b, b'\t' | b'\n' | 0x0b | 0x0c | b'\r')) {
                return false;
            }
            if b == b'\\' {
                match bytes.next() {
                    Some(b'\\' | b'"') => {}
                    Some(b'x') if bytes.next().is_some_and(u8::is_ascii_hexdigit) && bytes.next().is_some_and(u8::is_ascii_hexdigit) => {}
                    _ => return false,
                }
            }
        }
        true
    };
    dict.split(|&b| b == b'\n').enumerate().filter(|(_, l)| !entry(l)).map(|(i, _)| i + 1).collect()
}

/// What each target lacks, one line each.
fn missing(fuzz: &Path, targets: &[(String, String)]) -> Vec<String> {
    let has_file = |p: &Path| std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.len() > 0);
    let has_seed = |p: &Path| std::fs::read_dir(p).is_ok_and(|mut d| d.any(|e| e.is_ok_and(|e| has_file(&e.path()))));
    let mut out = Vec::new();
    for (name, path) in targets {
        if !has_file(&fuzz.join(path)) {
            out.push(format!("fuzz/{path}"));
        }
        if !has_seed(&fuzz.join("seeds").join(name)) {
            out.push(format!("fuzz/seeds/{name}/ (a seed)"));
        }
        let dict = fuzz.join("dicts").join(format!("{name}.dict"));
        if !has_file(&dict) {
            out.push(format!("fuzz/dicts/{name}.dict"));
        } else if let Some(&line) = std::fs::read(&dict).map(|d| unreadable(&d)).unwrap_or_default().first() {
            out.push(format!("fuzz/dicts/{name}.dict line {line} (libFuzzer can't read it)"));
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
        let manifest = "[package]\nname = \"mailcraft-fuzz\"\n[[bin]]\nbin = [\n    { name = \"photo\", path = \"fuzz_targets/photo.rs\" }, # photos\n    { name = \"search_index\", bench = false, path = \"fuzz_targets/store/search_index.rs\" },\n    { name = \"nothing\", path = \"fuzz_targets/nothing.rs\" },\n]\n";
        let found = targets(manifest);
        assert_eq!(found.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), ["photo", "search_index", "nothing"]);
        assert_eq!(found[1].1, "fuzz_targets/store/search_index.rs", "a target in a folder of its role");
        let fuzz = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("fuzz");
        let missing = missing(&fuzz, &found);
        assert_eq!(missing, ["fuzz/fuzz_targets/nothing.rs", "fuzz/seeds/nothing/ (a seed)", "fuzz/dicts/nothing.dict"]);
    }

    /// The backup target's dictionary as it was: its first line's end written raw cut the entry
    /// in two, and the weekly run stopped there, reading `"MAILCRAFT-BACKUP`; its version and
    /// flags were raw bytes too.
    #[test]
    fn a_dictionary_libfuzzer_cant_read_is_found() {
        assert_eq!(unreadable(b"# first line\r\n\"MAILCRAFT-BACKUP\r\n\"\r\n\"\x01\x00\"\r\n"), [2, 3, 4]);
        assert_eq!(unreadable(b"  # spaces\n\n\"MAILCRAFT-BACKUP\\x0A\"\nkw=\"a\\\\b\\\"c\" \n\"\\x01\\x00\"\r\n"), [] as [usize; 0]);
        assert_eq!(unreadable(b"no quotes\n\"\\q\"\n\"\\x1\"\n\"\"\n\"\n\"caf\xc3\xa9\""), [1, 2, 3, 4, 5, 6]);
    }
}
