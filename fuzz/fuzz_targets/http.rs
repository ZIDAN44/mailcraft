//! What a web server sends: an HTTP answer read whole and read as it streams (lengths, chunks, to
//! the end of the connection), a download's `Content-Range`; and what the updater reads from
//! GitHub: the release list, a `SHA256SUMS.txt`, the quota header and release tags.
#![no_main]

use std::io::{Read, Write};

use libfuzzer_sys::fuzz_target;
use mailcraft_net::http::{self, Request, Url};
use mailcraft_net::updates;

/// The server's side of a connection: the input, a few bytes at a time; what's sent is dropped.
struct Server<'a> {
    data: &'a [u8],
    step: usize,
}

impl Read for Server<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.step.min(self.data.len()).min(buf.len());
        let (now, rest) = self.data.split_at(n);
        buf.get_mut(..n).unwrap_or_default().copy_from_slice(now);
        self.data = rest;
        Ok(n)
    }
}

impl Write for Server<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fuzz_target!(|data: &[u8]| {
    let Ok(url) = Url::parse("https://github.com/ZIDAN44/mailcraft/releases/download/v1/m.msi") else { return };
    let req = Request::get(&url.path, 4096);
    // The step the bytes come in at, from the first byte: chunk sizes and lines split anywhere.
    let step = usize::from(data.first().copied().unwrap_or(1)).max(1);
    let body = data.get(1..).unwrap_or_default();
    let _ = http::exchange(&mut Server { data: body, step }, &req, &url);
    let mut got = Vec::new();
    let _ = http::exchange_into(&mut Server { data: body, step }, &req, &url, |_| Ok(Some(&mut got)));
    assert!(got.len() <= 4096, "a body past the cap was written");
    if let Ok(text) = std::str::from_utf8(body) {
        let _ = updates::content_range(text);
        let _ = updates::parse(text, Some("-windows-x64.msi"));
        let _ = updates::sum_for(text, "mailcraft-0.25.0-windows-x64.msi");
        let _ = updates::tag_version(text);
        let _ = updates::rate_limited(Some(text), 1_800_000_000);
    }
});
