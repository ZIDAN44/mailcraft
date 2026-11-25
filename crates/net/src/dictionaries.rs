//! Downloading Hunspell `.aff`/`.dic` dictionary pairs cleanly over HTTPS and saving them
//! atomically on disk.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::http::{Request, Url, get_into};
use crate::{NetError, Result};

/// Maximum size accepted for a single Hunspell `.aff` or `.dic` file (32 MiB).
pub const MAX_FILE_BYTES: u64 = 32 * 1024 * 1024;
const ALLOWED_HOST: &str = "raw.githubusercontent.com";

/// Fetch a `.aff`/`.dic` pair over HTTPS from the allowlisted dictionary host, updating `done` and
/// `total` as bytes arrive.
pub fn fetch_pair(aff_url: &str, dic_url: &str, done: &AtomicU64, total: &AtomicU64) -> Result<(Vec<u8>, Vec<u8>)> {
    check_url(aff_url)?;
    check_url(dic_url)?;
    fetch_pair_unchecked(aff_url, dic_url, done, total)
}

fn check_url(raw: &str) -> Result<()> {
    let url = Url::parse(raw)?;
    if !url.https || !url.host.eq_ignore_ascii_case(ALLOWED_HOST) || url.port != 443 {
        return Err(NetError::Refused("dictionary downloads must use HTTPS on raw.githubusercontent.com".into()));
    }
    Ok(())
}

/// Inner fetcher separated for localhost unit tests against `crate::http::tests::serve`.
fn fetch_pair_unchecked(aff_url: &str, dic_url: &str, done: &AtomicU64, total: &AtomicU64) -> Result<(Vec<u8>, Vec<u8>)> {
    done.store(0, Ordering::Relaxed);
    total.store(0, Ordering::Relaxed);
    let aff = fetch_one(aff_url, done, total, 0)?;
    let base = aff.len() as u64;
    done.store(base, Ordering::Relaxed);
    total.store(base, Ordering::Relaxed);
    let dic = fetch_one(dic_url, done, total, base)?;
    done.store(base.saturating_add(dic.len() as u64), Ordering::Relaxed);
    total.store(base.saturating_add(dic.len() as u64), Ordering::Relaxed);
    Ok((aff, dic))
}

struct ProgressWriter<'a> {
    buf: &'a mut Vec<u8>,
    done: &'a AtomicU64,
    base: u64,
}

impl Write for ProgressWriter<'_> {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.buf.extend_from_slice(data);
        self.done.store(self.base.saturating_add(self.buf.len() as u64), Ordering::Relaxed);
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn fetch_one(url: &str, done: &AtomicU64, total: &AtomicU64, base: u64) -> Result<Vec<u8>> {
    let max = usize::try_from(MAX_FILE_BYTES).unwrap_or(usize::MAX);
    let req = Request::get(url, max).header("Accept-Encoding", "identity");
    let mut out = Vec::new();
    let mut bad_status = None;
    let resp = get_into(&req, |head| {
        if !head.ok() {
            bad_status = Some(head.status);
            return Ok(None);
        }
        if let Some(len) = head.header("Content-Length").and_then(|v| v.trim().parse::<u64>().ok()) {
            total.store(base.saturating_add(len.min(MAX_FILE_BYTES)), Ordering::Relaxed);
        }
        Ok(Some(ProgressWriter { buf: &mut out, done, base }))
    })?;
    if let Some(status) = bad_status {
        return Err(NetError::Protocol(format!("dictionary download answered HTTP {status}")));
    }
    if !resp.ok() {
        return Err(NetError::Protocol(format!("dictionary download answered HTTP {}", resp.status)));
    }
    Ok(out)
}

/// Atomically write `<locale>.aff` and `<locale>.dic` into `dir` using scratch files + rename.
pub fn save_pair_atomic(dir: &Path, locale: &str, aff: &[u8], dic: &[u8]) -> Result<(PathBuf, PathBuf)> {
    let safe = !locale.is_empty() && locale.len() <= 32 && locale.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    if !safe {
        return Err(NetError::Protocol("invalid dictionary locale name".into()));
    }
    std::fs::create_dir_all(dir)?;
    let aff_path = dir.join(format!("{locale}.aff"));
    let dic_path = dir.join(format!("{locale}.dic"));
    mailcraft_store::write_atomic(&aff_path, aff)?;
    mailcraft_store::write_atomic(&dic_path, dic)?;
    Ok((aff_path, dic_path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::tests::serve;

    #[test]
    fn downloads_and_writes_dictionary_pair_atomically() {
        let (base, h) = serve(vec![
            b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nSET UTF-8\n".to_vec(),
            b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\n\r\n1\nhello\n".to_vec(),
        ]);
        let done = AtomicU64::new(0);
        let total = AtomicU64::new(0);
        let (aff, dic) = fetch_pair_unchecked(&format!("{base}/en_US.aff"), &format!("{base}/en_US.dic"), &done, &total).unwrap();
        assert_eq!(aff, b"SET UTF-8\n");
        assert_eq!(dic, b"1\nhello\n");
        assert_eq!(done.load(Ordering::Relaxed), 18);
        assert_eq!(total.load(Ordering::Relaxed), 18);

        let dir = std::env::temp_dir().join(format!("mailcraft-net-dict-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (aff_p, dic_p) = save_pair_atomic(&dir, "en_US", &aff, &dic).unwrap();
        assert_eq!(std::fs::read(&aff_p).unwrap(), aff);
        assert_eq!(std::fs::read(&dic_p).unwrap(), dic);
        let leftovers: Vec<_> =
            std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()).filter(|e| e.file_name().to_string_lossy().ends_with(".tmp")).collect();
        assert!(leftovers.is_empty(), "atomic writes leave no .tmp files");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = h.join();
    }

    #[test]
    fn refuses_bad_urls_http_errors_and_unsafe_locales() {
        let done = AtomicU64::new(0);
        let total = AtomicU64::new(0);
        assert!(fetch_pair("http://raw.githubusercontent.com/a.aff", "https://raw.githubusercontent.com/a.dic", &done, &total).is_err());
        assert!(fetch_pair("https://example.com/a.aff", "https://raw.githubusercontent.com/a.dic", &done, &total).is_err());

        let (base, h) = serve(vec![b"HTTP/1.1 404 Not Found\r\nContent-Length: 9\r\n\r\nnot found".to_vec()]);
        let err = fetch_pair_unchecked(&format!("{base}/missing.aff"), &format!("{base}/missing.dic"), &done, &total).unwrap_err();
        assert!(matches!(err, NetError::Protocol(m) if m.contains("404")));
        let _ = h.join();

        let dir = std::env::temp_dir().join("mailcraft-net-dict-bad");
        assert!(save_pair_atomic(&dir, "../evil", b"a", b"b").is_err());
        assert!(save_pair_atomic(&dir, "", b"a", b"b").is_err());
    }
}
