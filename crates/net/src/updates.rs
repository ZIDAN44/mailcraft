//! File › About › Check for updates: MailCraft's releases, as GitHub lists them; and, for the
//! test builds' updater (`apps/mailcraft/src/self_update.rs`), fetching an installer (resumably,
//! [`download`]) and proving it is the one the release lists.
//!
//! MailCraft's releases are pre-releases (`v0.19.0-test.15`), which GitHub's "latest release"
//! leaves out, so the recent ones are listed and the engine picks (`Session::check_updates`).
//! The answer is untrusted: its size is capped, and only addresses on the project's own releases
//! pages are kept (anything else falls back to the release's page, or nothing). A download is
//! accepted only if its SHA-256 matches the release's record of it: GitHub's per-file `digest`,
//! else the release's `SHA256SUMS.txt`.

use mailcraft_model::job::Release;
use serde_json::Value;

use crate::http::{Request, send};
use crate::{NetError, Result};

mod download;
pub use download::{Progress, content_range, download_installer, file_name, part_path, sha256_file};

/// Where MailCraft is published.
pub const REPO: &str = "ZIDAN44/mailcraft";

/// The largest answer read (ten releases with their files lists).
const MAX_ANSWER: usize = 2 << 20;

/// The largest installer accepted.
const MAX_INSTALLER: usize = 400 << 20;
/// The largest `SHA256SUMS.txt` read.
const MAX_SUMS: usize = 256 << 10;

/// The most recent releases (drafts left out), newest first as GitHub gives them.
pub fn releases() -> Result<Vec<Release>> {
    match releases_since(asset_suffix(), None)? {
        Listing::Changed(list, _) => Ok(list),
        Listing::Unchanged => Ok(Vec::new()),
    }
}

/// What a conditional listing found.
#[derive(Debug)]
pub enum Listing {
    /// The list, and its `ETag` to ask with next time.
    Changed(Vec<Release>, Option<String>),
    /// Nothing new since the `ETag` given (GitHub doesn't count this against the quota).
    Unchanged,
}

/// The recent releases with `suffix` picking each one's installer, unless they're unchanged
/// since `etag` (from an earlier answer).
pub fn releases_since(suffix: Option<&str>, etag: Option<&str>) -> Result<Listing> {
    let mut req = Request::get(&format!("https://api.github.com/repos/{REPO}/releases?per_page=10"), MAX_ANSWER)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .header("User-Agent", concat!("MailCraft/", env!("CARGO_PKG_VERSION")));
    if let Some(tag) = etag.filter(|t| t.len() < 200 && t.is_ascii() && !t.contains(['\r', '\n'])) {
        req = req.header("If-None-Match", tag);
    }
    let resp = send(&req)?;
    if resp.status == 304 {
        return Ok(Listing::Unchanged);
    }
    // GitHub answers 403 (or 429) when its anonymous quota ran out (60 an hour for everyone
    // behind the same address); when it refills rides along.
    if matches!(resp.status, 403 | 429) {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        return Err(NetError::Later(rate_limited(resp.header("x-ratelimit-reset"), now)));
    }
    if !resp.ok() {
        return Err(NetError::Protocol(format!("GitHub answered HTTP {}", resp.status)));
    }
    let etag = resp.header("ETag").filter(|t| t.len() < 200).map(str::to_string);
    Ok(Listing::Changed(parse(&String::from_utf8_lossy(&resp.body), suffix)?, etag))
}

/// The SHA-256 a `SHA256SUMS.txt` (as `sha256sum` writes it) gives `file`, in lowercase hex.
pub fn sum_for(sums: &str, file: &str) -> Option<String> {
    sums.lines().take(10_000).find_map(|line| {
        let (hash, name) = line.trim().split_once(char::is_whitespace)?;
        let name = name.trim_start().trim_start_matches('*');
        (name == file && is_sha256(hash)).then(|| hash.to_ascii_lowercase())
    })
}

fn is_sha256(h: &str) -> bool {
    h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The SHA-256 of `bytes`, in lowercase hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    ring::digest::digest(&ring::digest::SHA256, bytes).as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

/// The checksum to hold `release`'s installer to: GitHub's own record, else its line in the
/// release's `SHA256SUMS.txt`. An error when neither says.
pub fn expected_sum(release: &Release) -> Result<String> {
    if is_sha256(&release.sha256) {
        return Ok(release.sha256.to_ascii_lowercase());
    }
    let file = release.download.rsplit('/').next().unwrap_or_default();
    if release.sums.is_empty() || file.is_empty() {
        return Err(NetError::No(format!("{} lists no checksum for its installer", release.tag)));
    }
    let resp = send(&Request::get(&release.sums, MAX_SUMS).header("User-Agent", concat!("MailCraft/", env!("CARGO_PKG_VERSION"))))?;
    if !resp.ok() {
        return Err(NetError::Protocol(format!("the checksums file answered HTTP {}", resp.status)));
    }
    sum_for(&String::from_utf8_lossy(&resp.body), file).ok_or_else(|| NetError::No(format!("{}'s checksums don't list {file}", release.tag)))
}

/// What to say when GitHub's quota ran out: `reset` is its `x-ratelimit-reset` header (the Unix
/// time it refills), `now` the time now.
pub fn rate_limited(reset: Option<&str>, now: u64) -> String {
    let fallback = "GitHub is limiting update checks for now (an hourly quota shared by everyone on this network); try again later".to_string();
    let Some(at) = reset.and_then(|r| r.trim().parse::<u64>().ok()).filter(|at| *at > now) else { return fallback };
    let minutes = (at - now).div_ceil(60);
    if minutes > 24 * 60 {
        return fallback;
    }
    format!(
        "GitHub is limiting update checks for now (an hourly quota shared by everyone on this network); try again in {minutes} minute{}",
        if minutes == 1 { "" } else { "s" }
    )
}

/// The installer for this system among a release's files (by the end of its name), when there
/// is one to offer: Linux and the BSDs install in many ways, so their users get the page.
fn asset_suffix() -> Option<&'static str> {
    if cfg!(target_os = "windows") {
        Some(if cfg!(target_arch = "aarch64") {
            "-windows-arm64.msi"
        } else if cfg!(target_arch = "x86") {
            "-windows-x86.msi"
        } else {
            "-windows-x64.msi"
        })
    } else if cfg!(target_os = "macos") {
        Some("-macos-universal.dmg")
    } else {
        None
    }
}

/// An address on the project's releases pages, else `None`.
fn ours(url: &str) -> Option<String> {
    let base = format!("https://github.com/{REPO}/releases/");
    (url.starts_with(&base) && url.len() < 1024 && !url.contains(['\r', '\n', ' '])).then(|| url.to_string())
}

/// A tag as a version and test build: `v0.19.0-test.15` → ("0.19.0", 15); `v1.2.3` → ("1.2.3", 0).
pub fn tag_version(tag: &str) -> Option<(String, u32)> {
    let t = tag.trim().strip_prefix('v').unwrap_or(tag.trim());
    let (version, rest) = t.split_once('-').unwrap_or((t, ""));
    let parts: Vec<&str> = version.split('.').collect();
    if parts.len() != 3 || parts.iter().any(|p| p.is_empty() || p.len() > 6 || !p.bytes().all(|b| b.is_ascii_digit())) {
        return None;
    }
    let build = rest.rsplit('.').next().and_then(|n| n.parse::<u32>().ok()).unwrap_or(0);
    Some((version.to_string(), build))
}

/// The releases in GitHub's answer; `suffix` picks each one's installer for this system.
pub fn parse(body: &str, suffix: Option<&str>) -> Result<Vec<Release>> {
    let v: Value = serde_json::from_str(body).map_err(|e| NetError::Protocol(format!("GitHub's answer can't be read: {e}")))?;
    let list = v.as_array().ok_or_else(|| NetError::Protocol("GitHub's answer isn't a list of releases".into()))?;
    let text = |r: &Value, k: &str| r.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let mut out = Vec::new();
    for r in list.iter().take(20) {
        if r.get("draft").and_then(Value::as_bool).unwrap_or(false) {
            continue;
        }
        let tag = text(r, "tag_name");
        let Some((version, build)) = tag_version(&tag) else { continue };
        let page = ours(&text(r, "html_url")).unwrap_or_else(|| format!("https://github.com/{REPO}/releases"));
        let assets: &[Value] = r.get("assets").and_then(Value::as_array).map_or(&[], |a| a.as_slice());
        let assets = assets.get(..assets.len().min(100)).unwrap_or_default();
        let installer = suffix.and_then(|s| assets.iter().find(|a| text(a, "name").ends_with(s)));
        let download = installer.and_then(|a| ours(&text(a, "browser_download_url"))).unwrap_or_default();
        // GitHub records each file's SHA-256 as `"digest": "sha256:<hex>"`.
        let sha256 = installer
            .filter(|_| !download.is_empty())
            .and_then(|a| text(a, "digest").strip_prefix("sha256:").filter(|h| is_sha256(h)).map(str::to_ascii_lowercase))
            .unwrap_or_default();
        let size = installer.filter(|_| !download.is_empty()).and_then(|a| a.get("size")?.as_u64()).unwrap_or(0);
        let sums =
            assets.iter().find(|a| text(a, "name") == "SHA256SUMS.txt").and_then(|a| ours(&text(a, "browser_download_url"))).unwrap_or_default();
        let commit = text(r, "target_commitish");
        out.push(Release {
            tag: tag.chars().take(64).collect(),
            version,
            build,
            commit: if commit.len() == 40 && commit.bytes().all(|b| b.is_ascii_hexdigit()) { commit } else { String::new() },
            page,
            download,
            sha256,
            size,
            sums,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_are_versions_and_test_builds() {
        assert_eq!(tag_version("v0.19.0-test.15"), Some(("0.19.0".into(), 15)));
        assert_eq!(tag_version("v1.2.3"), Some(("1.2.3".into(), 0)));
        assert_eq!(tag_version("0.20.0-rc.2"), Some(("0.20.0".into(), 2)));
        for bad in ["web-0.15.0-2d69edf", "v1.2", "vx.y.z", "", "v1.2.3.4", "v99999999.0.0"] {
            assert_eq!(tag_version(bad), None, "{bad}");
        }
    }

    /// GitHub's answer, as it lists MailCraft's pre-releases: drafts and other tags left out, the
    /// right installer picked, and nothing but the project's own release pages kept.
    #[test]
    fn releases_are_read_from_githubs_answer() {
        let body = r#"[
          {"tag_name": "v0.20.0-test.16", "draft": true, "html_url": "https://github.com/ZIDAN44/mailcraft/releases/tag/v0.20.0-test.16", "assets": []},
          {"tag_name": "v0.19.0-test.15", "draft": false, "prerelease": true, "target_commitish": "8bdba5ceb26a8b910102731c634d6998b8ab25d1",
           "html_url": "https://github.com/ZIDAN44/mailcraft/releases/tag/v0.19.0-test.15",
           "assets": [
             {"name": "mailcraft-0.19.0-windows-arm64.msi", "browser_download_url": "https://github.com/ZIDAN44/mailcraft/releases/download/v0.19.0-test.15/mailcraft-0.19.0-windows-arm64.msi"},
             {"name": "mailcraft-0.19.0-windows-x64.msi", "browser_download_url": "https://github.com/ZIDAN44/mailcraft/releases/download/v0.19.0-test.15/mailcraft-0.19.0-windows-x64.msi"},
             {"name": "mailcraft-0.19.0-macos-universal.dmg", "browser_download_url": "https://evil.example/mailcraft.dmg"}
           ]},
          {"tag_name": "web-0.15.0-2d69edf", "draft": false, "html_url": "https://github.com/ZIDAN44/mailcraft/releases/tag/web-0.15.0-2d69edf", "assets": []},
          {"tag_name": "v0.18.0-test.14", "draft": false, "target_commitish": "release", "html_url": "https://evil.example/", "assets": []}
        ]"#;
        let r = parse(body, Some("-windows-x64.msi")).unwrap();
        assert_eq!(r.len(), 2);
        let first = &r[0];
        assert_eq!((first.version.as_str(), first.build), ("0.19.0", 15));
        assert_eq!(first.commit, "8bdba5ceb26a8b910102731c634d6998b8ab25d1");
        assert!(first.download.ends_with("/mailcraft-0.19.0-windows-x64.msi"), "{}", first.download);
        assert_eq!(r[1].page, "https://github.com/ZIDAN44/mailcraft/releases", "a page elsewhere isn't offered");
        assert_eq!(r[1].commit, "", "a branch name isn't a commit");
        // Another site's file is never offered for download.
        assert_eq!(parse(body, Some("-macos-universal.dmg")).unwrap()[0].download, "");
        assert_eq!(parse(body, None).unwrap()[0].download, "");
        assert!(parse("{\"message\": \"Not Found\"}", None).is_err());
        assert!(parse("not json", None).is_err());
    }

    /// What the updater holds a download to: GitHub's digest, else the checksums file's line;
    /// only for an installer on the project's page, with its size.
    #[test]
    fn installers_come_with_what_to_check_them_against() {
        let body = r#"[{"tag_name": "v0.21.0-test.19", "draft": false, "target_commitish": "8180a31aa25d1ea4c7e8b4bd94e0f0f2c3a1b2c3",
          "html_url": "https://github.com/ZIDAN44/mailcraft/releases/tag/v0.21.0-test.19",
          "assets": [
            {"name": "mailcraft-0.21.0-windows-x64.msi", "size": 31457280,
             "digest": "sha256:9F86D081884C7D659A2FEAA0C55AD015A3BF4F1B2B0B822CD15D6C15B0F00A08",
             "browser_download_url": "https://github.com/ZIDAN44/mailcraft/releases/download/v0.21.0-test.19/mailcraft-0.21.0-windows-x64.msi"},
            {"name": "mailcraft-0.21.0-windows-x64-portable.zip", "size": 9, "digest": "md5:abc",
             "browser_download_url": "https://github.com/ZIDAN44/mailcraft/releases/download/v0.21.0-test.19/mailcraft-0.21.0-windows-x64-portable.zip"},
            {"name": "SHA256SUMS.txt", "browser_download_url": "https://github.com/ZIDAN44/mailcraft/releases/download/v0.21.0-test.19/SHA256SUMS.txt"}
          ]}]"#;
        let msi = &parse(body, Some("-windows-x64.msi")).unwrap()[0];
        assert_eq!((msi.sha256.as_str(), msi.size), ("9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08", 31_457_280));
        assert!(msi.sums.ends_with("/v0.21.0-test.19/SHA256SUMS.txt"));
        assert_eq!(expected_sum(msi).unwrap(), msi.sha256);
        let zip = &parse(body, Some("-windows-x64-portable.zip")).unwrap()[0];
        assert_eq!((zip.sha256.as_str(), zip.size), ("", 9), "a digest of another kind isn't taken");
        let sums = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08  mailcraft-0.21.0-windows-x64.msi\n\
                    E3B0C44298FC1C149AFBF4C8996FB92427AE41E4649B934CA495991B7852B855 *mailcraft-0.21.0-windows-x64-portable.zip\n\
                    nothexnothexnothexnothexnothexnothexnothexnothexnothexnothexnoth  mailcraft-x.dmg\n";
        assert_eq!(
            sum_for(sums, "mailcraft-0.21.0-windows-x64-portable.zip").as_deref(),
            Some("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
        );
        assert_eq!(sum_for(sums, "mailcraft-x.dmg"), None, "not a checksum");
        assert_eq!(sum_for(sums, "portable.zip"), None, "names match whole");
        assert_eq!(sha256_hex(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        // Nothing to check against: refused before anything is fetched.
        let bare = Release { download: msi.download.clone(), ..Default::default() };
        assert!(expected_sum(&bare).is_err());
    }

    #[test]
    fn a_used_up_quota_says_how_long_to_wait() {
        assert!(rate_limited(Some("1800001200"), 1_800_000_000).ends_with("try again in 20 minutes"));
        assert!(rate_limited(Some("1800000060"), 1_800_000_000).ends_with("try again in 1 minute"));
        for odd in [None, Some("soon"), Some("1700000000"), Some("1900000000")] {
            assert!(rate_limited(odd, 1_800_000_000).ends_with("try again later"), "{odd:?}");
        }
    }
}
