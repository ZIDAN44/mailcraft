//! `cargo xtask version [set X.Y.Z[-pre]]`: the workspace version, the single source of truth.
//!
//! The version lives in `[workspace.package] version` in the root `Cargo.toml`; every crate
//! inherits it with `version.workspace = true`, and the packaging scripts read it from here.

use std::path::Path;

/// The `version = "…"` value inside `[workspace.package]`: what's between its quotes (a comment
/// after them was read as part of it).
pub fn read(manifest: &str) -> Result<String, String> {
    let (_, line) = find_line(manifest)?;
    let v = quoted(line).and_then(|r| line.get(r)).unwrap_or_default();
    if v.is_empty() {
        return Err("empty [workspace.package] version".into());
    }
    Ok(v.to_string())
}

/// Where the value is in a `version = "…"` line: between the quotes after the `=`.
fn quoted(line: &str) -> Option<std::ops::Range<usize>> {
    let eq = line.find('=')?;
    let after = line.get(eq + 1..)?;
    let open = eq + 1 + after.find('"')?;
    if !line.get(eq + 1..open)?.trim().is_empty() {
        return None;
    }
    let close = open + 1 + line.get(open + 1..)?.find('"')?;
    Some(open + 1..close)
}

/// `manifest` with the workspace version replaced; everything else is byte-for-byte unchanged,
/// the version's own line too (its indentation and a comment after it were lost).
pub fn replace(manifest: &str, new: &str) -> Result<String, String> {
    validate(new)?;
    let (idx, _) = find_line(manifest)?;
    let mut out = String::with_capacity(manifest.len() + 8);
    for (i, line) in manifest.split_inclusive('\n').enumerate() {
        if i == idx {
            let at = quoted(line).ok_or("the [workspace.package] version isn't a quoted string")?;
            let mut line = line.to_string();
            line.replace_range(at, new);
            out.push_str(&line);
        } else {
            out.push_str(line);
        }
    }
    Ok(out)
}

/// Semver without build metadata: `MAJOR.MINOR.PATCH` plus an optional `-pre.release` tag.
pub fn validate(v: &str) -> Result<(), String> {
    let bad = || Err(format!("`{v}` is not a version like 1.2.3 or 1.2.3-rc.1"));
    let (core, pre) = match v.split_once('-') {
        Some((c, p)) => (c, Some(p)),
        None => (v, None),
    };
    let nums: Vec<&str> = core.split('.').collect();
    // Each a number Cargo takes (a u64): a larger one was written into Cargo.toml, which Cargo
    // then refused, every build with it.
    if nums.len() != 3
        || nums
            .iter()
            .any(|n| n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) || (n.len() > 1 && n.starts_with('0')) || n.parse::<u64>().is_err())
    {
        return bad();
    }
    if let Some(p) = pre
        && (p.is_empty()
            || p.split('.').any(|id| {
                // A number among the pre-release parts has no leading zero (semver; as
                // PhotoCraft's xtask): `rc.01` sorts unlike what it says.
                id.is_empty()
                    || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                    || (id.len() > 1 && id.starts_with('0') && id.bytes().all(|b| b.is_ascii_digit()))
            }))
    {
        return bad();
    }
    Ok(())
}

/// Index and text of the `version` line in `[workspace.package]`.
fn find_line(manifest: &str) -> Result<(usize, &str), String> {
    let mut in_section = false;
    for (i, line) in manifest.lines().enumerate() {
        let t = line.trim();
        // A section's heading, with a comment after it or not.
        if t.starts_with('[') {
            in_section = t.split('#').next().map(str::trim) == Some("[workspace.package]");
            continue;
        }
        if in_section
            && let Some(rest) = t.strip_prefix("version")
            && rest.trim_start().starts_with('=')
        {
            return Ok((i, line));
        }
    }
    Err("no `version = \"…\"` in [workspace.package] of the root Cargo.toml".into())
}

pub fn run(root: &Path, args: &[&str]) -> Result<(), String> {
    let path = root.join("Cargo.toml");
    let text = std::fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    match args {
        [] => {
            println!("{}", read(&text)?);
            Ok(())
        }
        ["set", new] => {
            let old = read(&text)?;
            let updated = replace(&text, new)?;
            // Write atomically: a half-written root manifest breaks every build in the workspace.
            let tmp = root.join("target").join("Cargo.toml.xtask-version");
            std::fs::create_dir_all(root.join("target")).map_err(|e| format!("create target/: {e}"))?;
            std::fs::write(&tmp, updated).map_err(|e| format!("write {}: {e}", tmp.display()))?;
            std::fs::rename(&tmp, &path).map_err(|e| format!("replace {}: {e}", path.display()))?;
            // Refresh the workspace members' entries in Cargo.lock (no dependency upgrades).
            let mut c = crate::cargo();
            c.args(["update", "--workspace", "--offline"]);
            if crate::run(c, "cargo update --workspace --offline").is_err() {
                let mut c = crate::cargo();
                c.args(["update", "--workspace"]);
                crate::run(c, "cargo update --workspace")?;
            }
            println!("version: {old} -> {new}");
            Ok(())
        }
        _ => Err("usage: cargo xtask version [set X.Y.Z[-pre]]".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = "[workspace]\nmembers = [\"a\"]\n\n[workspace.package]\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[workspace.dependencies]\nfoo = { version = \"1\" }\n";

    #[test]
    fn reads_the_workspace_version() {
        assert_eq!(read(MANIFEST).unwrap(), "0.1.0");
        assert!(read("[package]\nversion = \"1.0.0\"\n").is_err());
    }

    #[test]
    fn replaces_only_the_workspace_version() {
        let out = replace(MANIFEST, "1.2.3-rc.1").unwrap();
        assert_eq!(read(&out).unwrap(), "1.2.3-rc.1");
        assert_eq!(out, MANIFEST.replace("version = \"0.1.0\"", "version = \"1.2.3-rc.1\""));
        assert!(out.contains("foo = { version = \"1\" }"));
    }

    #[test]
    fn keeps_crlf_line_endings() {
        let crlf = MANIFEST.replace('\n', "\r\n");
        let out = replace(&crlf, "2.0.0").unwrap();
        assert_eq!(out, crlf.replace("\"0.1.0\"", "\"2.0.0\""));
    }

    #[test]
    fn prerelease_numbers_have_no_leading_zero() {
        for bad in ["1.0.0-01", "1.0.0-rc.01", "1.0.0-00.1"] {
            assert!(validate(bad).is_err(), "{bad}");
        }
        for ok in ["1.0.0-0", "1.0.0-rc.0", "1.0.0-01a", "1.0.0-01-2", "1.0.0-rc.10"] {
            assert!(validate(ok).is_ok(), "{ok}");
        }
    }

    #[test]
    fn validates_versions() {
        for ok in ["0.1.0", "1.20.300", "1.0.0-rc.1", "1.0.0-alpha", "1.0.0-x-y.2"] {
            assert!(validate(ok).is_ok(), "{ok}");
        }
        for bad in ["1.0", "1.0.0.0", "v1.0.0", "01.0.0", "1.0.0-", "1.0.0-a..b", "1.0.0+meta", "1.a.0", ""] {
            assert!(validate(bad).is_err(), "{bad}");
        }
        assert!(replace(MANIFEST, "nope").is_err());
    }

    /// A comment after the section's heading or the version, and the line's indentation, are
    /// read past and kept (as PhotoCraft's xtask); a number Cargo can't take is refused before
    /// Cargo.toml is written (as DesignCraft's).
    #[test]
    fn comments_and_indentation_are_kept() {
        let noted = "[workspace.package] # shared by every crate\n  version = \"0.1.0\"  # the release\nedition = \"2024\"\n";
        assert_eq!(read(noted).unwrap(), "0.1.0");
        assert_eq!(replace(noted, "0.2.0").unwrap(), noted.replace("\"0.1.0\"", "\"0.2.0\""));
        assert!(validate("18446744073709551615.0.0").is_ok());
        for too_big in ["18446744073709551616.0.0", "1.99999999999999999999.0"] {
            assert!(validate(too_big).is_err(), "{too_big}");
        }
    }

    #[test]
    fn the_real_manifest_has_a_valid_version() {
        let text = std::fs::read_to_string(crate::root().join("Cargo.toml")).unwrap();
        validate(&read(&text).unwrap()).unwrap();
    }
}
