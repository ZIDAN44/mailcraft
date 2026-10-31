//! Mail kept by the app's own build is compressed: the store's test of this runs in the store's
//! dependency graph, but the app's has another crate turning flate2's zlib-rs backend on, whose
//! fast level kept base64 attachments at 0.995 of their size (so they were stored raw, and an
//! imported `.pst` took a third more disk than it needed). This one runs where the app does.

use mailcraft_engine::model::{AccountConfig, MessageKey};
use mailcraft_engine::store::Store;

/// Bytes that don't compress (xorshift).
fn noise(n: usize, seed: u64) -> Vec<u8> {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect()
}

/// Base64 (as attachments travel in mail) in lines of 76.
fn base64(bytes: &[u8]) -> Vec<u8> {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::new();
    for (i, c) in bytes.chunks(3).enumerate() {
        let n = c.iter().enumerate().fold(0u32, |n, (j, b)| n | (u32::from(*b) << (16 - 8 * j)));
        for j in 0..4 {
            out.push(if j <= c.len() { ABC[((n >> (18 - 6 * j)) & 63) as usize] } else { b'=' });
        }
        if i % 19 == 18 {
            out.extend_from_slice(b"\r\n");
        }
    }
    out
}

/// How much mail goes into an account's data file before the file passes `size`: messages
/// made by `body`, written as an import writes them. The database grows its file by doubling, so
/// a file's size only says what it holds within a factor of two; how much it takes to fill a
/// given size, against mail that can't be compressed, says what each byte was kept in.
fn written_to_fill(s: &Store, account: &str, size: u64, body: impl Fn(u32) -> Vec<u8>) -> u64 {
    let key = |uid: u32| MessageKey { account: account.into(), folder: "INBOX".into(), validity: 1, uid };
    let raw = body(0);
    let mut written = raw.len() as u64;
    s.write_bodies(&[(key(0), raw)]).unwrap();
    let file = s.data_file(account).unwrap();
    for uid in 1..1000 {
        if std::fs::metadata(&file).map_or(0, |m| m.len()) > size {
            return written;
        }
        let raw = body(uid);
        written += raw.len() as u64;
        s.write_bodies(&[(key(uid), raw)]).unwrap();
    }
    panic!("{account}: {written} bytes written and the data file is still under {size}");
}

/// Messages carrying base64 attachments are kept in at most 0.80 of their size.
#[test]
fn attachments_are_kept_compressed_in_the_apps_build() {
    let t = std::env::temp_dir().join(format!("mailcraft-app-deflate-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&t);
    let s = Store::open_packed(t.join("app"), t.join("Files")).unwrap();
    let account = |id: &str| AccountConfig { id: id.into(), email: format!("{id}@studio.example"), ..Default::default() };
    s.save_accounts(&[account("a1"), account("a2")]).unwrap();
    let head = b"Subject: figures\r\nContent-Type: application/octet-stream\r\nContent-Transfer-Encoding: base64\r\n\r\n";
    let mail = |uid: u32| [head.as_slice(), &base64(&noise(64 << 10, u64::from(uid)))].concat();
    let size = 16 << 20;
    let attachments = written_to_fill(&s, "a1", size, mail);
    // The same sizes of bytes that don't compress at all (kept as they are, with no compression).
    let random = written_to_fill(&s, "a2", size, |uid| noise(mail(uid).len(), u64::from(uid) + 1000));
    let first = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid: 1 };
    assert_eq!(s.read_body(&first), Some(mail(1)));
    assert!(random * 100 <= attachments * 80, "{attachments} bytes of mail with attachments filled what {random} bytes that don't compress did");
    drop(s);
    let _ = std::fs::remove_dir_all(t);
}
