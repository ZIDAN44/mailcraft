//! Downloading an update's installer so that nothing fetched is lost: the bytes go to a partial
//! file as they come, and a download cut off (the connection gone, MailCraft closed) carries on
//! from there with an HTTP range request (RFC 9110 §14), in this run or the next.
//!
//! The partial file is named after the installer and the size the release gives it
//! (`mailcraft-0.25.0-windows-x64.msi.31457280.part`); a server that answers a range with the
//! whole file again starts it over, and the finished file is renamed into place only once its
//! SHA-256 is the release's. A resumed file that doesn't match (a proxy that spliced it wrongly)
//! is fetched once more from the start before the download gives up. The way craft-launcher
//! fetches the Craft apps.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use mailcraft_model::job::Release;

use super::{MAX_INSTALLER, ours};
use crate::http::{Request, get_into};
use crate::{NetError, Result};

/// Tries in a row that get nowhere before the download waits for a later check.
const MAX_FRUITLESS: u32 = 5;

/// How a download is going, for whoever shows it.
#[derive(Debug, Default)]
pub struct Progress {
    /// Bytes fetched so far (on disk), and the installer's size (0 until known).
    pub done: AtomicU64,
    pub total: AtomicU64,
}

impl Progress {
    /// The bytes fetched and the size, as now.
    pub fn get(&self) -> (u64, u64) {
        (self.done.load(Ordering::Relaxed), self.total.load(Ordering::Relaxed))
    }
}

/// Fetch `release`'s installer into `dir`, checked against `expected` (lowercase hex SHA-256):
/// the finished file, or `None` when `stop` was set (what came stays, to carry on from). A file
/// already there and right is taken as it is.
pub fn download_installer(release: &Release, expected: &str, dir: &Path, progress: &Progress, stop: &AtomicBool) -> Result<Option<PathBuf>> {
    if ours(&release.download).is_none() {
        return Err(NetError::No("the release has no installer on MailCraft's releases page".into()));
    }
    let name = file_name(&release.download).ok_or_else(|| NetError::Protocol("the installer has no usable file name".into()))?;
    let size = release.size.min(MAX_INSTALLER as u64);
    fetch_to(&release.download, &dir.join(name), size, expected, progress, stop, Duration::from_secs(1))
}

/// The partial file `dest` grows in while it downloads.
pub fn part_path(dest: &Path, size: u64) -> PathBuf {
    let name = dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    dest.with_file_name(if size > 0 { format!("{name}.{size}.part") } else { format!("{name}.part") })
}

/// The last part of a download address, if it's a plain file name.
pub fn file_name(url: &str) -> Option<&str> {
    let name = url.rsplit('/').next()?;
    (!name.is_empty()
        && name.len() < 200
        && !name.starts_with('.')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_')))
    .then_some(name)
}

/// The SHA-256 of a file, in lowercase hex, read a piece at a time.
pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path)?;
    let mut ctx = ring::digest::Context::new(&ring::digest::SHA256);
    let mut buf = vec![0u8; 256 << 10];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        ctx.update(buf.get(..n).unwrap_or_default());
    }
    Ok(ctx.finish().as_ref().iter().map(|b| format!("{b:02x}")).collect())
}

/// [`download_installer`] from any address (the tests' local server): `size` 0 when unknown;
/// `pause` the first wait between tries (doubling, at most 30 s).
fn fetch_to(url: &str, dest: &Path, size: u64, expected: &str, progress: &Progress, stop: &AtomicBool, pause: Duration) -> Result<Option<PathBuf>> {
    let dir = dest.parent().ok_or_else(|| NetError::Protocol("nowhere to download to".into()))?;
    std::fs::create_dir_all(dir)?;
    progress.total.store(size, Ordering::Relaxed);
    if dest.is_file() && sha256_file(dest).is_ok_and(|h| h == expected) {
        progress.done.store(size, Ordering::Relaxed);
        return Ok(Some(dest.to_path_buf()));
    }
    let part = part_path(dest, size);
    let mut again = false;
    loop {
        if !resume(url, &part, size, progress, stop, pause)? {
            return Ok(None);
        }
        if sha256_file(&part).is_ok_and(|h| h == expected) {
            std::fs::rename(&part, dest)?;
            return Ok(Some(dest.to_path_buf()));
        }
        let _ = std::fs::remove_file(&part);
        if again {
            return Err(NetError::Protocol("the download doesn't match the release's checksum, so it isn't installed".into()));
        }
        log::warn!("updates: {url} didn't match its checksum; downloading it again from the start");
        again = true;
    }
}

/// Fill `part` from where it is: true when it's whole (the checksum still to decide), false when
/// stopped. Tries again while tries make progress, waiting longer each time one doesn't.
fn resume(url: &str, part: &Path, size: u64, progress: &Progress, stop: &AtomicBool, pause: Duration) -> Result<bool> {
    let mut fruitless = 0u32;
    loop {
        if stop.load(Ordering::Relaxed) {
            return Ok(false);
        }
        let have = std::fs::metadata(part).map(|m| m.len()).unwrap_or(0);
        progress.done.store(have, Ordering::Relaxed);
        if size > 0 && have == size {
            return Ok(true);
        }
        if size > 0 && have > size {
            std::fs::remove_file(part)?;
            continue;
        }
        let error = match attempt(url, part, have, size, progress, stop) {
            Ok(()) => return Ok(true),
            Err(Try::Fatal(e)) => return Err(e),
            Err(Try::Again(e)) => e,
        };
        if stop.load(Ordering::Relaxed) {
            return Ok(false);
        }
        let now = std::fs::metadata(part).map(|m| m.len()).unwrap_or(0);
        fruitless = if now > have { 1 } else { fruitless + 1 };
        if fruitless >= MAX_FRUITLESS {
            return Err(error);
        }
        log::info!("updates: {error}; trying again");
        let wait = pause.saturating_mul(1 << (fruitless - 1).min(5)).min(Duration::from_secs(30));
        let until = std::time::Instant::now() + wait;
        while std::time::Instant::now() < until {
            if stop.load(Ordering::Relaxed) {
                return Ok(false);
            }
            std::thread::sleep(Duration::from_millis(50).min(wait));
        }
    }
}

/// Why a try ended.
enum Try {
    /// Worth trying again.
    Again(NetError),
    /// Not: the file isn't what the release says, or can't be written.
    Fatal(NetError),
}

/// One request, carrying `part` on from byte `have`.
fn attempt(url: &str, part: &Path, have: u64, size: u64, progress: &Progress, stop: &AtomicBool) -> std::result::Result<(), Try> {
    // The whole file at most: a server may answer a range with all of it.
    let cap = usize::try_from(size).ok().filter(|n| *n > 0).map_or(MAX_INSTALLER, |n| n.min(MAX_INSTALLER));
    let mut req = Request::get(url, cap)
        .header("User-Agent", concat!("MailCraft/", env!("CARGO_PKG_VERSION")))
        // The installer is compressed already: the bytes exactly as published.
        .header("Accept-Encoding", "identity");
    if have > 0 {
        log::info!("updates: carrying on from byte {have} of {size}");
        req = req.header("Range", &format!("bytes={have}-"));
    }
    let mut refused: Option<Try> = None;
    let mut whole = false;
    let result = get_into(&req, |head| {
        let (start, total) = match head.status {
            200 => (0, head.header("Content-Length").and_then(|v| v.trim().parse::<u64>().ok()).unwrap_or(size)),
            206 => match head.header("Content-Range").and_then(content_range) {
                Some((start, total)) => (start, total.unwrap_or(size)),
                None => {
                    refused = Some(Try::Again(NetError::Protocol("the server's part of the file can't be read".into())));
                    return Ok(None);
                }
            },
            // Asked from past the end: the file is whole already (or wrong: the checksum decides).
            416 if have > 0 => {
                whole = true;
                return Ok(None);
            }
            404 | 410 => {
                refused = Some(Try::Fatal(NetError::No("the installer is no longer on the release page".into())));
                return Ok(None);
            }
            s => {
                refused = Some(Try::Again(NetError::Protocol(format!("the download answered HTTP {s}"))));
                return Ok(None);
            }
        };
        if size > 0 && total != size {
            refused = Some(Try::Fatal(NetError::Protocol(format!("the server offers {total} bytes, the release says {size}"))));
            return Ok(None);
        }
        let file = if head.status == 206 && start == have {
            std::fs::OpenOptions::new().create(true).append(true).open(part)
        } else if head.status == 206 {
            // Another part than asked for: start again from the beginning.
            let _ = std::fs::remove_file(part);
            refused = Some(Try::Again(NetError::Protocol(format!("the server sent the file from byte {start}, not {have}"))));
            return Ok(None);
        } else {
            std::fs::File::create(part)
        };
        let file = file.map_err(|e| {
            refused = Some(Try::Fatal(NetError::Io(std::io::Error::new(e.kind(), format!("{}: {e}", part.display())))));
            NetError::Refused(String::new())
        })?;
        progress.done.store(if head.status == 206 { have } else { 0 }, Ordering::Relaxed);
        Ok(Some(Counted { file, progress, stop }))
    });
    if let Some(r) = refused {
        return Err(r);
    }
    match result {
        Ok(_) if whole || size == 0 => Ok(()),
        Ok(_) if std::fs::metadata(part).map(|m| m.len()).unwrap_or(0) == size => Ok(()),
        Ok(_) => Err(Try::Again(NetError::Closed("the download ended early".into()))),
        Err(NetError::Io(e)) if e.kind() == std::io::ErrorKind::StorageFull => Err(Try::Fatal(NetError::Io(e))),
        Err(e) => Err(Try::Again(e)),
    }
}

/// `bytes 100-199/1000` (or `/*`): where the part starts, and the whole size when given.
pub fn content_range(v: &str) -> Option<(u64, Option<u64>)> {
    let rest = v.trim().strip_prefix("bytes")?.trim_start();
    let (range, total) = rest.split_once('/')?;
    let (start, end) = range.split_once('-')?;
    let (start, end) = (start.trim().parse::<u64>().ok()?, end.trim().parse::<u64>().ok()?);
    let total = match total.trim() {
        "*" => None,
        t => Some(t.parse::<u64>().ok()?),
    };
    (end >= start && total.is_none_or(|t| end < t)).then_some((start, total))
}

/// The partial file, counting what's written and stopping when asked.
struct Counted<'a> {
    file: std::fs::File,
    progress: &'a Progress,
    stop: &'a AtomicBool,
}

impl Write for Counted<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.stop.load(Ordering::Relaxed) {
            // Not `Interrupted`, which `write_all` tries again.
            return Err(std::io::Error::other("stopped"));
        }
        let n = self.file.write(buf)?;
        self.progress.done.fetch_add(n as u64, Ordering::Relaxed);
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::tests::serve;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mailcraft-update-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sum(bytes: &[u8]) -> String {
        super::super::sha256_hex(bytes)
    }

    #[test]
    fn content_ranges_are_read_strictly() {
        assert_eq!(content_range("bytes 100-199/1000"), Some((100, Some(1000))));
        assert_eq!(content_range("bytes 0-9/*"), Some((0, None)));
        for bad in ["", "bytes */1000", "bytes 9-1/10", "bytes 0-10/10", "items 0-1/2", "bytes x-1/2"] {
            assert_eq!(content_range(bad), None, "{bad}");
        }
    }

    #[test]
    fn only_plain_file_names_are_downloaded_to() {
        let base = "https://github.com/ZIDAN44/mailcraft/releases/download/v0.21.0-test.19/";
        assert_eq!(file_name(&format!("{base}mailcraft-0.21.0-windows-x64.msi")), Some("mailcraft-0.21.0-windows-x64.msi"));
        for bad in ["", "..", ".hidden.msi", "a%2F..%2Fb.msi", "x?y=1", "spaces in.msi"] {
            assert_eq!(file_name(&format!("{base}{bad}")), None, "{bad}");
        }
        assert_eq!(part_path(Path::new("/u/m.msi"), 42), PathBuf::from("/u/m.msi.42.part"));
    }

    /// A download cut off part way carries on from its partial file with a range request, even
    /// in a later run; one stopped keeps what came.
    #[test]
    fn a_download_cut_off_carries_on_where_it_stopped() {
        let body = b"0123456789abcdefghij".to_vec();
        let expected = sum(&body);
        let (base, h) = serve(vec![
            // The first run: half the file, then the connection drops.
            b"HTTP/1.1 200 OK\r\nContent-Length: 20\r\n\r\n0123456789".to_vec(),
            // Trying again: the rest, as asked.
            b"HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 10-19/20\r\nContent-Length: 10\r\n\r\nabcdefghij".to_vec(),
        ]);
        let dir = scratch("resume");
        let dest = dir.join("m.msi");
        let progress = Progress::default();
        let stop = AtomicBool::new(false);
        let got = fetch_to(&format!("{base}/m.msi"), &dest, 20, &expected, &progress, &stop, Duration::from_millis(1)).unwrap();
        assert_eq!(got.as_deref(), Some(dest.as_path()));
        assert_eq!(std::fs::read(&dest).unwrap(), body);
        assert_eq!(progress.get(), (20, 20));
        assert!(!part_path(&dest, 20).exists(), "the partial file became the installer");
        let seen = h.join().unwrap();
        assert!(!seen[0].contains("Range:"), "{}", seen[0]);
        assert!(seen[1].contains("Range: bytes=10-\r\n"), "{}", seen[1]);
        // Already here: nothing fetched.
        assert!(fetch_to("http://127.0.0.1:1/m.msi", &dest, 20, &expected, &progress, &stop, Duration::ZERO).unwrap().is_some());

        // A later run finds what an earlier one left; a server that sends it all again starts over.
        let dest = dir.join("n.msi");
        std::fs::write(part_path(&dest, 20), b"01234").unwrap();
        let (base, h) = serve(vec![b"HTTP/1.1 200 OK\r\nContent-Length: 20\r\n\r\n0123456789abcdefghij".to_vec()]);
        assert!(fetch_to(&format!("{base}/n.msi"), &dest, 20, &expected, &progress, &stop, Duration::ZERO).unwrap().is_some());
        assert_eq!(std::fs::read(&dest).unwrap(), body);
        assert!(h.join().unwrap()[0].contains("Range: bytes=5-"));

        // Stopped (MailCraft closing): nothing more asked for, the partial file kept.
        let dest = dir.join("o.msi");
        std::fs::write(part_path(&dest, 20), b"0123").unwrap();
        stop.store(true, Ordering::Relaxed);
        assert_eq!(fetch_to("http://127.0.0.1:1/o.msi", &dest, 20, &expected, &progress, &stop, Duration::ZERO).unwrap(), None);
        assert_eq!(std::fs::read(part_path(&dest, 20)).unwrap(), b"0123");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Some servers answer a plain GET with 206 and the whole file as its one part: taken, with no
    /// partial file there yet to carry on.
    #[test]
    fn a_part_from_the_start_is_taken_without_a_partial_file() {
        let body = b"0123456789".to_vec();
        let dir = scratch("from-start");
        let (base, h) = serve(vec![b"HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-9/10\r\nContent-Length: 10\r\n\r\n0123456789".to_vec()]);
        let dest = dir.join("c.msi");
        let (progress, stop) = (Progress::default(), AtomicBool::new(false));
        assert!(fetch_to(&format!("{base}/c.msi"), &dest, 10, &sum(&body), &progress, &stop, Duration::ZERO).unwrap().is_some());
        assert_eq!(std::fs::read(&dest).unwrap(), body);
        h.join().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A spliced file is fetched once more from the start; a second mismatch, a size other than
    /// the release's, or a file gone from the release ends it without an installer.
    #[test]
    fn a_wrong_file_is_never_kept() {
        let body = b"0123456789".to_vec();
        let expected = sum(&body);
        let dir = scratch("wrong");
        let progress = Progress::default();
        let stop = AtomicBool::new(false);
        let (base, h) = serve(vec![
            b"HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 4-9/10\r\nContent-Length: 6\r\n\r\nXXXXXX".to_vec(),
            b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\n0123456789".to_vec(),
        ]);
        let dest = dir.join("a.msi");
        std::fs::write(part_path(&dest, 10), b"0123").unwrap();
        assert!(fetch_to(&format!("{base}/a.msi"), &dest, 10, &expected, &progress, &stop, Duration::ZERO).unwrap().is_some());
        assert_eq!(std::fs::read(&dest).unwrap(), body, "fetched again from the start");
        let seen = h.join().unwrap();
        assert!(!seen[1].contains("Range:"), "{}", seen[1]);

        let (base, h) = serve(vec![
            b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nXXXXXXXXXX".to_vec(),
            b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nYYYYYYYYYY".to_vec(),
            b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\n\r\n01234567890".to_vec(),
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_vec(),
        ]);
        let dest = dir.join("b.msi");
        assert!(fetch_to(&format!("{base}/b.msi"), &dest, 10, &expected, &progress, &stop, Duration::ZERO).is_err());
        assert!(!dest.exists() && !part_path(&dest, 10).exists(), "nothing wrong is left");
        assert!(fetch_to(&format!("{base}/b.msi"), &dest, 10, &expected, &progress, &stop, Duration::ZERO).is_err(), "the wrong size");
        assert!(fetch_to(&format!("{base}/b.msi"), &dest, 10, &expected, &progress, &stop, Duration::ZERO).is_err(), "gone");
        h.join().unwrap();
        // Not on MailCraft's own releases page: refused before anything is fetched.
        let elsewhere = Release { download: "https://evil.example/mailcraft.msi".into(), size: 10, ..Default::default() };
        assert!(download_installer(&elsewhere, &expected, &dir, &progress, &stop).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
