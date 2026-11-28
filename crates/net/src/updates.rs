//! File › About › Check for updates: MailCraft's releases, as GitHub lists them; and, for the
//! test builds' updater (`apps/mailcraft/src/self_update.rs`), fetching an installer (resumably,
//! [`download`]) and proving it is the one the release lists.
//!
//! MailCraft's releases are test builds (`v0.19.0-test.15`, pre-releases, which GitHub's "latest
//! release" leaves out) and releases (`v0.19.0`), so the recent ones are listed and the engine
//! picks (`newer_release`, by the build's channel).
//! The answer is untrusted: its size is capped, and only addresses on the project's own releases
//! pages are kept (anything else falls back to the release's page, or nothing). A download is
//! accepted only if its SHA-256 matches the release's record of it: GitHub's per-file `digest`,
//! else the release's `SHA256SUMS.txt`. Once MailCraft is built with the project's public key
//! ([`signed::KEYS`]), only the release's `SHA256SUMS.txt`, signed with that key, is: [`signed`].

use mailcraft_model::job::Release;
use serde_json::Value;

use crate::http::{Request, send};
use crate::{NetError, Result};

mod download;
pub mod signed;
pub use download::{Progress, content_range, download_installer, file_name, part_path, sha256_file};

/// Where MailCraft is published.
pub const REPO: &str = "ZIDAN44/mailcraft";

/// Where releases are listed and their files come from: MailCraft's GitHub releases, or a copy of
/// them served on this computer, for trying the updater out (debug builds only, see
/// `apps/mailcraft/src/self_update.rs`). Every address a listing gives is kept only when it's
/// under [`Feed::files`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Feed {
    /// The list of releases, newest first, as GitHub's API gives it.
    pub list: String,
    /// The start of every release page and file address taken from it.
    pub files: String,
}

impl Feed {
    /// MailCraft's own releases on GitHub (thirty: enough to reach the last release past a run
    /// of test builds).
    pub fn github() -> Feed {
        Feed { list: format!("https://api.github.com/repos/{REPO}/releases?per_page=30"), files: format!("https://github.com/{REPO}/releases/") }
    }

    /// A copy served at `base` on this computer (`http://127.0.0.1:8123/`): its list at
    /// `<base>releases`, its files under `<base>`. `None` for any other address.
    pub fn local(base: &str) -> Option<Feed> {
        let base = base.trim();
        let rest = base.strip_prefix("http://").or_else(|| base.strip_prefix("https://"))?;
        let host =
            if rest.starts_with('[') { rest.split(']').next().map(|h| format!("{h}]")) } else { rest.split(['/', ':']).next().map(str::to_string) };
        if !matches!(host.as_deref(), Some("127.0.0.1" | "localhost" | "[::1]")) || base.contains(['\r', '\n', ' ']) {
            return None;
        }
        let base = if base.ends_with('/') { base.to_string() } else { format!("{base}/") };
        Some(Feed { list: format!("{base}releases"), files: base })
    }

    /// The feed to use: GitHub's; in a debug build, `MAILCRAFT_UPDATE_FEED` may name a local copy
    /// instead (`http://127.0.0.1:8123/`), for trying the updater and File › About out against
    /// releases made up for it. A release build never reads it.
    pub fn current() -> Feed {
        if cfg!(debug_assertions)
            && let Some(local) = std::env::var("MAILCRAFT_UPDATE_FEED").ok().and_then(|v| Feed::local(&v))
        {
            return local;
        }
        Feed::github()
    }

    /// `url` when it's on this feed's pages, else `None`.
    pub fn ours(&self, url: &str) -> Option<String> {
        (url.starts_with(&self.files) && url.len() < 1024 && !url.contains(['\r', '\n', ' '])).then(|| url.to_string())
    }

    /// The one address the file `name` of the release tagged `tag` may have, as GitHub makes them:
    /// `<files>download/<tag>/<name>`, a `+` written `%2B`. None for a tag or name with anything
    /// but ASCII letters, digits and `._+-` in it, or that is `.` or `..`: so a file whose address
    /// leads to another release's, or out of the project's releases (by `..`, or a slash or dot
    /// written `%2f`, `%2e`, which GitHub reads as one), isn't taken.
    pub fn file_address(&self, tag: &str, name: &str) -> Option<String> {
        let plain = |s: &str| {
            !s.is_empty() && s.len() <= 200 && s != "." && s != ".." && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"._+-".contains(&b))
        };
        (plain(tag) && plain(name)).then(|| format!("{}download/{}/{}", self.files, tag.replace('+', "%2B"), name.replace('+', "%2B")))
    }
}

/// The largest answer read (thirty releases with their files lists).
const MAX_ANSWER: usize = 8 << 20;

/// The largest installer accepted.
const MAX_INSTALLER: usize = 400 << 20;
/// The largest `SHA256SUMS.txt` read.
const MAX_SUMS: usize = 256 << 10;
/// What MailCraft calls itself to GitHub (which refuses a request without a User-Agent).
const USER_AGENT: &str = concat!("MailCraft/", env!("CARGO_PKG_VERSION"));

/// The most recent releases (drafts left out), newest first as GitHub gives them.
pub fn releases(feed: &Feed) -> Result<Vec<Release>> {
    match releases_since(feed, asset_suffix(), None)? {
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
pub fn releases_since(feed: &Feed, suffix: Option<&str>, etag: Option<&str>) -> Result<Listing> {
    let mut req = Request::get(&feed.list, MAX_ANSWER)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .header("User-Agent", USER_AGENT);
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
    Ok(Listing::Changed(parse_from(feed, &String::from_utf8_lossy(&resp.body), suffix)?, etag))
}

/// The SHA-256 a `SHA256SUMS.txt` (as `sha256sum` writes it) gives `file`, in lowercase hex. A
/// line repeated counts once; a file listed with two different sums has none, whichever line
/// comes first, so it isn't installed (as craft-libs' updater reads the file).
pub fn sum_for(sums: &str, file: &str) -> Option<String> {
    let mut found: Option<String> = None;
    for line in sums.lines().take(10_000) {
        let Some((hash, name)) = line.trim().split_once(char::is_whitespace) else { continue };
        let name = name.trim_start().trim_start_matches('*');
        if name != file || !is_sha256(hash) {
            continue;
        }
        let hash = hash.to_ascii_lowercase();
        match &found {
            None => found = Some(hash),
            Some(h) if *h == hash => {}
            Some(_) => {
                log::warn!("updates: SHA256SUMS.txt lists {file} with different checksums; it isn't installed");
                return None;
            }
        }
    }
    found
}

fn is_sha256(h: &str) -> bool {
    h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The SHA-256 of `bytes`, in lowercase hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    crate::hex(ring::digest::digest(&ring::digest::SHA256, bytes).as_ref())
}

/// The checksum to hold `release`'s installer to: see [`expected_sum_with`], with the public keys
/// MailCraft is built with.
pub fn expected_sum(release: &Release) -> Result<String> {
    expected_sum_with(signed::KEYS, release)
}

/// The checksum to hold `release`'s installer to. With no `keys` (releases not signed yet):
/// GitHub's own record, else its line in the release's `SHA256SUMS.txt`. With keys: its line in
/// the release's `SHA256SUMS.txt` once that file's signature checks out against one of them
/// ([`signed::verify_sums`]), and GitHub's record, when there is one, must say the same. An error
/// when it can't be told, so nothing is installed.
pub fn expected_sum_with(keys: &[&str], release: &Release) -> Result<String> {
    if keys.is_empty() && is_sha256(&release.sha256) {
        return Ok(release.sha256.to_ascii_lowercase());
    }
    // The installer's name as the checksums list it: its address writes a `+` as `%2B`.
    let file = unescaped(release.download.rsplit('/').next().unwrap_or_default());
    if release.sums.is_empty() || file.is_empty() {
        return Err(NetError::No(format!("{} lists no checksum for its installer", release.tag)));
    }
    if !keys.is_empty() && release.sums_sig.is_empty() {
        return Err(NetError::No(format!("{} isn't signed; it isn't installed", release.tag)));
    }
    let sums = fetch(&release.sums, MAX_SUMS, "checksums file")?;
    if !keys.is_empty() {
        let sig = fetch(&release.sums_sig, signed::MAX_SIG, "checksums signature")?;
        // The release named whole, from the address the checksums came from (`Release::tag` is
        // cut short for showing).
        let tag = unescaped(release.sums.rsplit('/').nth(1).unwrap_or_default());
        signed::verify_sums(keys, &tag, &sums, &String::from_utf8_lossy(&sig))?;
    }
    checked_sum(keys, release, &String::from_utf8_lossy(&sums), &file)
}

/// A part of a release file's address as it's named: [`Feed::file_address`] writes `+` as `%2B`.
fn unescaped(part: &str) -> String {
    part.replace("%2B", "+").replace("%2b", "+")
}

/// `file`'s line in `sums` (signed, when there are `keys`), held to GitHub's record of it too then.
fn checked_sum(keys: &[&str], release: &Release, sums: &str, file: &str) -> Result<String> {
    let sum = sum_for(sums, file).ok_or_else(|| NetError::No(format!("{}'s checksums don't list {file}", release.tag)))?;
    if !keys.is_empty() && is_sha256(&release.sha256) && !release.sha256.eq_ignore_ascii_case(&sum) {
        return Err(NetError::Protocol(format!("{}'s installer isn't the one its signed checksums list; it isn't installed", release.tag)));
    }
    Ok(sum)
}

/// The file at `url` (at most `max` bytes); `what` names it in an error.
fn fetch(url: &str, max: usize, what: &str) -> Result<Vec<u8>> {
    let resp = send(&Request::get(url, max).header("User-Agent", USER_AGENT))?;
    if !resp.ok() {
        return Err(NetError::Protocol(format!("the {what} answered HTTP {}", resp.status)));
    }
    Ok(resp.body)
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
    let exe_dir = std::env::current_exe().ok().and_then(|exe| exe.parent().map(std::path::Path::to_path_buf));
    asset_suffix_for(exe_dir.is_some_and(|dir| mailcraft_store::location::portable_data(&dir).is_some()))
}

/// [`asset_suffix`], for a portable copy or not: on Windows a portable copy is offered the
/// portable `.zip`, as it updates itself from (issue #260: Check for updates offered it the
/// `.msi`, which installs a second MailCraft in Program Files rather than updating this one).
fn asset_suffix_for(portable: bool) -> Option<&'static str> {
    if cfg!(target_os = "windows") {
        Some(match (portable, cfg!(target_arch = "aarch64"), cfg!(target_arch = "x86")) {
            (true, true, _) => "-windows-arm64-portable.zip",
            (true, _, true) => "-windows-x86-portable.zip",
            (true, ..) => "-windows-x64-portable.zip",
            (false, true, _) => "-windows-arm64.msi",
            (false, _, true) => "-windows-x86.msi",
            (false, ..) => "-windows-x64.msi",
        })
    } else if cfg!(target_os = "macos") {
        Some("-macos-universal.dmg")
    } else {
        None
    }
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
    parse_from(&Feed::github(), body, suffix)
}

/// [`parse`] for a listing from `feed`: only addresses on its pages are kept.
pub fn parse_from(feed: &Feed, body: &str, suffix: Option<&str>) -> Result<Vec<Release>> {
    let ours = |url: &str| feed.ours(url);
    let v: Value = serde_json::from_str(body).map_err(|e| NetError::Protocol(format!("GitHub's answer can't be read: {e}")))?;
    let list = v.as_array().ok_or_else(|| NetError::Protocol("GitHub's answer isn't a list of releases".into()))?;
    let text = |r: &Value, k: &str| r.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let mut out = Vec::new();
    for r in list.iter().take(30) {
        if r.get("draft").and_then(Value::as_bool).unwrap_or(false) {
            continue;
        }
        let tag = text(r, "tag_name");
        let Some((version, build)) = tag_version(&tag) else { continue };
        // A test build (or another pre-release) by its tag: `v0.19.0-test.15`, not `v0.19.0`.
        let pre = tag.trim().trim_start_matches('v').contains('-');
        let page = ours(&text(r, "html_url")).unwrap_or_else(|| feed.files.trim_end_matches('/').to_string());
        let assets: &[Value] = r.get("assets").and_then(Value::as_array).map_or(&[], |a| a.as_slice());
        let assets = assets.get(..assets.len().min(100)).unwrap_or_default();
        // A file's address is taken only when it's the one its release gives it.
        let address = |a: &Value| {
            let url = text(a, "browser_download_url");
            feed.file_address(&tag, &text(a, "name")).filter(|own| *own == url)
        };
        let installer = suffix.and_then(|s| assets.iter().find(|a| text(a, "name").ends_with(s)));
        let download = installer.and_then(address).unwrap_or_default();
        // GitHub records each file's SHA-256 as `"digest": "sha256:<hex>"`.
        let sha256 = installer
            .filter(|_| !download.is_empty())
            .and_then(|a| text(a, "digest").strip_prefix("sha256:").filter(|h| is_sha256(h)).map(str::to_ascii_lowercase))
            .unwrap_or_default();
        let size = installer.filter(|_| !download.is_empty()).and_then(|a| a.get("size")?.as_u64()).unwrap_or(0);
        let sums = assets.iter().find(|a| text(a, "name") == "SHA256SUMS.txt").and_then(address).unwrap_or_default();
        let sums_sig = assets.iter().find(|a| text(a, "name") == "SHA256SUMS.txt.minisig").and_then(address).unwrap_or_default();
        let commit = text(r, "target_commitish");
        out.push(Release {
            tag: tag.chars().take(64).collect(),
            version,
            build,
            pre,
            commit: if commit.len() == 40 && commit.bytes().all(|b| b.is_ascii_hexdigit()) { commit } else { String::new() },
            page,
            download,
            sha256,
            size,
            sums,
            sums_sig,
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

    /// A file is taken only at the address its own release gives it: not another release's, not
    /// one leaving the project's releases by `..` or a slash or dot written as `%2f`, `%2e`.
    #[test]
    fn a_files_address_is_its_own_releases() {
        let release = |name: &str, url: &str| {
            format!(
                r#"[{{"tag_name": "v0.40.0", "draft": false, "html_url": "https://github.com/ZIDAN44/mailcraft/releases/tag/v0.40.0",
                    "assets": [{{"name": "{name}", "browser_download_url": "{url}"}},
                               {{"name": "SHA256SUMS.txt", "browser_download_url": "{url}"}}]}}]"#
            )
        };
        let base = "https://github.com/ZIDAN44/mailcraft/releases/download";
        let name = "mailcraft-0.40.0-windows-x64.msi";
        let own = format!("{base}/v0.40.0/{name}");
        let r = &parse(&release(name, &own), Some("-windows-x64.msi")).unwrap()[0];
        assert_eq!(r.download, own);
        assert_eq!(r.sums, "", "the checksums file's address must be its own too");
        for elsewhere in [
            format!("{base}/v0.39.0/{name}"),
            format!("{base}/v0.40.0/../../../../other/repo/releases/download/v1/{name}"),
            format!("{base}/v0.40.0/%2e%2e/%2e%2e/{name}"),
            format!("{base}/v0.40.0/x%2f..%2f{name}"),
            format!("{base}/v0.40.0/{name}?x=1"),
            format!("{base}/v0.40.0/{name}#x"),
        ] {
            assert_eq!(parse(&release(name, &elsewhere), Some("-windows-x64.msi")).unwrap()[0].download, "", "{elsewhere}");
        }
        // A name or tag GitHub would write otherwise has no address of its own here.
        let feed = Feed::github();
        assert_eq!(feed.file_address("v1.0.0", "a b.msi"), None);
        assert_eq!(feed.file_address("..", "x.msi"), None);
        assert_eq!(feed.file_address("v1.0.0", ".."), None);
        assert_eq!(feed.file_address("v1.0.0+1", "x.msi").as_deref(), Some(format!("{base}/v1.0.0%2B1/x.msi").as_str()));
    }

    /// What a build without a key (before 0.42.0) holds a download to: GitHub's digest, else the
    /// checksums file's line; only for an installer on the project's page, with its size. This
    /// build, with the project's key, takes neither alone: the release isn't signed.
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
        assert_eq!(expected_sum_with(&[], msi).unwrap(), msi.sha256);
        assert!(expected_sum(msi).unwrap_err().to_string().contains("isn't signed"), "built with the project's key");
        let zip = &parse(body, Some("-windows-x64-portable.zip")).unwrap()[0];
        // What Check for updates offers: a portable copy, the portable .zip (#260: the .msi).
        if cfg!(all(windows, target_arch = "x86_64")) {
            assert!(parse(body, asset_suffix_for(true)).unwrap()[0].download.ends_with("-windows-x64-portable.zip"));
            assert!(parse(body, asset_suffix_for(false)).unwrap()[0].download.ends_with("-windows-x64.msi"));
        }
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
        // A file listed twice: the same sum (in either case) is one; two sums are none, in
        // either order, and the other files keep theirs.
        let (a, b) = ("a".repeat(64), "b".repeat(64));
        assert_eq!(sum_for(&format!("{a}  x.msi\n{} *x.msi\n", a.to_ascii_uppercase()), "x.msi"), Some(a.clone()));
        for text in [format!("{a}  x.msi\n{b}  x.msi\n{a}  y.zip\n"), format!("{b}  x.msi\n{a}  y.zip\n{a}  x.msi\n")] {
            assert_eq!(sum_for(&text, "x.msi"), None, "{text}");
            assert_eq!(sum_for(&text, "y.zip"), Some(a.clone()), "{text}");
        }
        assert_eq!(sha256_hex(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        // Nothing to check against: refused before anything is fetched.
        let bare = Release { download: msi.download.clone(), ..Default::default() };
        assert!(expected_sum_with(&[], &bare).is_err());
        assert!(expected_sum(&bare).is_err());
    }

    /// With the project's key built in, an installer is held to its line in the release's signed
    /// checksums, fetched with their signature; a release without a signature, or one whose
    /// checksums aren't signed with the key, or whose GitHub record says otherwise, isn't installed.
    #[test]
    fn signed_releases_are_held_to_their_signed_checksums() {
        use signed::tests::{KEY_A, KEY_B, SIG, SUMS};
        let ok = |body: &str| format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).into_bytes();
        let release = |base: &str, sha256: &str| Release {
            tag: "v0.43.0-test.5".into(),
            download: format!("{base}/download/v0.43.0-test.5/mailcraft-0.43.0-windows-x64.msi"),
            sha256: sha256.into(),
            sums: format!("{base}/download/v0.43.0-test.5/SHA256SUMS.txt"),
            sums_sig: format!("{base}/download/v0.43.0-test.5/SHA256SUMS.txt.minisig"),
            ..Default::default()
        };
        let msi = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08";
        // Signed by the key: the line for the installer, the same as GitHub's record or without one.
        for digest in [msi, ""] {
            let (base, seen) = crate::http::tests::serve(vec![ok(SUMS), ok(SIG)]);
            assert_eq!(expected_sum_with(&[KEY_A], &release(&base, digest)).unwrap(), msi);
            let seen = seen.join().unwrap();
            assert!(seen[0].starts_with("GET /download/v0.43.0-test.5/SHA256SUMS.txt ") && seen[1].contains("SHA256SUMS.txt.minisig"), "{seen:?}");
        }
        // Signed by another key: refused, as it is when the checksums were changed on the way.
        let (base, _) = crate::http::tests::serve(vec![ok(SUMS), ok(SIG)]);
        assert!(expected_sum_with(&[KEY_B], &release(&base, msi)).unwrap_err().to_string().contains("aren't signed"));
        let (base, _) = crate::http::tests::serve(vec![ok(&SUMS.replacen("9f86", "0f86", 1)), ok(SIG)]);
        assert!(expected_sum_with(&[KEY_A], &release(&base, "")).unwrap_err().to_string().contains("aren't signed"));
        // GitHub's record of the file says otherwise than the signed checksums: not installed.
        let (base, _) = crate::http::tests::serve(vec![ok(SUMS), ok(SIG)]);
        let other = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        assert!(expected_sum_with(&[KEY_A], &release(&base, other)).unwrap_err().to_string().contains("isn't the one"));
        // No signature, or no checksums: refused before anything is fetched, GitHub's record or not.
        let unsigned = Release { sums_sig: String::new(), ..release("http://127.0.0.1:9", msi) };
        assert!(expected_sum_with(&[KEY_A], &unsigned).unwrap_err().to_string().contains("isn't signed"));
        let bare = Release { sums: String::new(), ..release("http://127.0.0.1:9", msi) };
        assert!(expected_sum_with(&[KEY_A], &bare).is_err());
        // Without a key built in (releases not signed yet), GitHub's record is enough, as before.
        assert_eq!(expected_sum_with(&[], &unsigned).unwrap(), msi);
    }

    /// A release whose tag is longer than the 64 characters `Release::tag` keeps, and an installer
    /// named with a `+` (written `%2B` in its address), are still found in their checksums and
    /// their signature checked against the whole tag. Signed for these tests with rsign2, as the
    /// fixtures in `signed`.
    #[test]
    fn long_tags_and_plus_signs_are_named_as_the_release_names_them() {
        use signed::tests::KEY_A;
        let tag = "v0.43.0-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa+build.7";
        let sums = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08  mailcraft-0.43.0+build.7-windows-x64.msi\n\
                    e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855  mailcraft-0.43.0-windows-x64-portable.zip\n";
        let sig = "untrusted comment: signature from rsign secret key\n\
                   RUQliLDa3ZytQjPv9pgRJf1oVpualWrfQX67o2mkQOyvyrBVRe0ObFAAaLkSxUyUdL2HU/kk+Dtl0FAontRvPK/VZE9Rqw2zOwE=\n\
                   trusted comment: MailCraft v0.43.0-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa+build.7\n\
                   9Ta/3BAMKpnpueYI6UTmDT1/9gxo75cNahZzxDbWJZCNdJpq5KLfEkf6tONA907Bu8U3AqruubDy0i52QHR1DA==\n";
        let ok = |body: &str| format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).into_bytes();
        for (name, sum) in [
            ("mailcraft-0.43.0+build.7-windows-x64.msi", "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"),
            ("mailcraft-0.43.0-windows-x64-portable.zip", "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"),
        ] {
            let (base, _) = crate::http::tests::serve(vec![ok(sums), ok(sig)]);
            let feed = Feed::local(&format!("{base}/")).unwrap();
            let release = Release {
                tag: tag.chars().take(64).collect(),
                download: feed.file_address(tag, name).unwrap(),
                sums: feed.file_address(tag, "SHA256SUMS.txt").unwrap(),
                sums_sig: feed.file_address(tag, "SHA256SUMS.txt.minisig").unwrap(),
                ..Default::default()
            };
            assert!(release.download.contains("%2B") && release.tag.len() < tag.len());
            assert_eq!(expected_sum_with(&[KEY_A], &release).unwrap(), sum, "{name}");
        }
        // Unsigned, with no GitHub record: the `+` name is still found in the checksums.
        let (base, _) = crate::http::tests::serve(vec![ok(sums)]);
        let feed = Feed::local(&format!("{base}/")).unwrap();
        let release = Release {
            tag: tag.into(),
            download: feed.file_address(tag, "mailcraft-0.43.0+build.7-windows-x64.msi").unwrap(),
            sums: feed.file_address(tag, "SHA256SUMS.txt").unwrap(),
            ..Default::default()
        };
        assert_eq!(expected_sum_with(&[], &release).unwrap(), "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08");
    }

    /// The signature is found beside the checksums, at the address its own release gives it.
    #[test]
    fn the_checksums_signature_is_found_beside_them() {
        let base = "https://github.com/ZIDAN44/mailcraft/releases/download/v0.43.0";
        let body = format!(
            r#"[{{"tag_name": "v0.43.0", "draft": false, "html_url": "https://github.com/ZIDAN44/mailcraft/releases/tag/v0.43.0",
                 "assets": [{{"name": "SHA256SUMS.txt", "browser_download_url": "{base}/SHA256SUMS.txt"}},
                            {{"name": "SHA256SUMS.txt.minisig", "browser_download_url": "{base}/SHA256SUMS.txt.minisig"}}]}},
               {{"tag_name": "v0.42.0", "draft": false, "html_url": "https://github.com/ZIDAN44/mailcraft/releases/tag/v0.42.0",
                 "assets": [{{"name": "SHA256SUMS.txt.minisig", "browser_download_url": "{base}/SHA256SUMS.txt.minisig"}}]}}]"#
        );
        let r = parse(&body, None).unwrap();
        assert_eq!(r[0].sums_sig, format!("{base}/SHA256SUMS.txt.minisig"));
        assert_eq!(r[1].sums_sig, "", "another release's signature isn't this one's");
    }

    /// A release (`v0.33.0`) and a test build (`v0.33.0-test.31`) are told apart; a feed served
    /// on this computer takes only its own addresses, and only one on this computer is a feed.
    #[test]
    fn releases_and_test_builds_and_a_local_feed() {
        let body = r#"[
          {"tag_name": "v0.33.0", "draft": false, "html_url": "https://github.com/ZIDAN44/mailcraft/releases/tag/v0.33.0", "assets": []},
          {"tag_name": "v0.33.0-test.31", "draft": false, "html_url": "https://github.com/ZIDAN44/mailcraft/releases/tag/v0.33.0-test.31", "assets": []}
        ]"#;
        let r = parse(body, None).unwrap();
        assert_eq!(r.iter().map(|r| (r.version.as_str(), r.build, r.pre)).collect::<Vec<_>>(), [("0.33.0", 0, false), ("0.33.0", 31, true)]);
        let local = Feed::local("http://127.0.0.1:8123").unwrap();
        assert_eq!(local, Feed { list: "http://127.0.0.1:8123/releases".into(), files: "http://127.0.0.1:8123/".into() });
        assert!(Feed::local("http://localhost:9/").is_some() && Feed::local("http://[::1]:9/").is_some());
        for elsewhere in ["https://example.com/", "http://127.0.0.1.example.com/", "http://10.0.0.1/", "file:///tmp/", "http://127.0.0.1/a b"] {
            assert_eq!(Feed::local(elsewhere), None, "{elsewhere}");
        }
        let served = r#"[{"tag_name": "v0.34.0", "draft": false, "html_url": "http://127.0.0.1:8123/tag/v0.34.0",
          "assets": [{"name": "mailcraft-0.34.0-linux-x86_64.tar.gz", "browser_download_url": "http://127.0.0.1:8123/download/v0.34.0/mailcraft-0.34.0-linux-x86_64.tar.gz"},
                     {"name": "mailcraft-0.34.0-linux-x86_64.deb", "browser_download_url": "https://evil.example/m.deb"}]}]"#;
        let r = parse_from(&local, served, Some("-linux-x86_64.tar.gz")).unwrap();
        assert!(r[0].download.starts_with("http://127.0.0.1:8123/download/") && r[0].page.ends_with("/tag/v0.34.0"));
        assert_eq!(parse_from(&local, served, Some("-linux-x86_64.deb")).unwrap()[0].download, "", "another site's file");
        assert_eq!(parse(served, Some("-linux-x86_64.tar.gz")).unwrap()[0].download, "", "GitHub's feed doesn't take a local file");
        if !cfg!(debug_assertions) {
            assert_eq!(Feed::current(), Feed::github(), "a release build reads no feed of its own");
        }
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
