//! The data file's reader: the same bytes and positions as the file read plainly, whatever the
//! seeks and reads; ranges read once; read-ahead only where reads follow on.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use super::super::chunked::{ChunkReader, PAGE, Shared, Source};
use super::super::open::open_read;

/// A file of `len` bytes that don't repeat in any short run.
fn file_of(name: &str, len: usize) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("mailcraft-pst-chunked-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("data.bin");
    let mut x = 0x2545_f491_4f6c_dd1du64;
    let bytes: Vec<u8> = (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect();
    std::fs::write(&path, bytes).unwrap();
    (dir, path)
}

fn source(path: &Path, chunk_kb: usize) -> Shared {
    Source::new(open_read(path).unwrap(), chunk_kb, super::super::Stop::default()).unwrap()
}

/// Ten thousand seeks and reads of every size, at random, from both: the same bytes, the same
/// positions, and an error where the file gives one (a seek before the start).
#[test]
fn the_reader_reads_what_the_file_holds() {
    let len = 1024 * 1024 + 7;
    let (dir, path) = file_of("same", len);
    for chunk_kb in [0, 4] {
        let shared = source(&path, chunk_kb);
        let mut ours = ChunkReader::new(&shared);
        let mut plain = std::fs::File::open(&path).unwrap();
        let mut x = 0x9e37_79b9_7f4a_7c15u64 ^ chunk_kb as u64;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        for step in 0..10_000 {
            let r = next();
            match r % 5 {
                0 => {
                    let to = SeekFrom::Start(next() % (len as u64 + 20_000));
                    assert_eq!(ours.seek(to).unwrap(), plain.seek(to).unwrap(), "step {step}");
                }
                1 => {
                    let to = SeekFrom::Current((next() % 300_000) as i64 - 150_000);
                    match (ours.seek(to), plain.seek(to)) {
                        (Ok(a), Ok(b)) => assert_eq!(a, b, "step {step}"),
                        (Err(_), Err(_)) => {}
                        (a, b) => panic!("step {step}: {a:?} against {b:?}"),
                    }
                }
                2 => {
                    let to = SeekFrom::End(-((next() % 200_000) as i64) + 100);
                    assert_eq!(ours.seek(to).unwrap(), plain.seek(to).unwrap(), "step {step}");
                }
                _ => {
                    let n = match next() % 6 {
                        0 => PAGE,
                        1 => 0,
                        2 => 70_000 + (next() % 200_000) as usize,
                        _ => (next() % 9000) as usize,
                    };
                    let (mut a, mut b) = (vec![0; n], vec![0; n]);
                    let got = ours.read(&mut a).unwrap();
                    // The file may give fewer bytes in one read: read the same from it.
                    let mut want = 0;
                    while want < got {
                        let k = plain.read(&mut b[want..got]).unwrap();
                        assert!(k > 0, "step {step}");
                        want += k;
                    }
                    assert_eq!(a[..got], b[..got], "step {step}");
                    let pos = ours.stream_position().unwrap();
                    assert_eq!(pos, plain.stream_position().unwrap(), "step {step}");
                    if n > 0 && pos < len as u64 {
                        assert!(got > 0, "step {step}: nothing read before the end");
                    }
                }
            }
        }
        // Whatever it read, it holds no more than its 64 chunks.
        let (held, _) = shared.borrow().held();
        assert!(held <= 64 * if chunk_kb == 0 { 64 * 1024 } else { chunk_kb * 1024 }, "{held}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Reads at and past the end give nothing; seeks before the start, or past what a position holds,
/// are errors and leave the position where it was.
#[test]
fn the_end_and_before_the_start() {
    let (dir, path) = file_of("ends", 10_000);
    let shared = source(&path, 0);
    let mut r = ChunkReader::new(&shared);
    let mut buf = [0; 64];
    assert_eq!(r.seek(SeekFrom::End(-10)).unwrap(), 9990);
    assert_eq!(r.read(&mut buf).unwrap(), 10);
    assert_eq!(r.read(&mut buf).unwrap(), 0);
    assert_eq!(r.seek(SeekFrom::Start(50_000)).unwrap(), 50_000);
    assert_eq!(r.read(&mut buf).unwrap(), 0);
    assert!(r.read_exact(&mut buf).is_err());
    assert!(r.seek(SeekFrom::Current(-60_000)).is_err());
    assert!(r.seek(SeekFrom::End(-10_001)).is_err());
    assert!(r.seek(SeekFrom::Current(i64::MAX)).and_then(|_| r.seek(SeekFrom::Current(i64::MAX))).is_err());
    r.seek(SeekFrom::Start(3)).unwrap();
    assert!(r.seek(SeekFrom::Current(-4)).is_err());
    assert_eq!(r.stream_position().unwrap(), 3);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Pages read here and there are read exactly, once each however often they're asked for (as
/// the root page is before every block); reads that follow on read ahead, four chunks at a time;
/// and a second handle starts with what the first read.
#[test]
fn ranges_are_read_once_and_ahead_only_in_order() {
    let (dir, path) = file_of("ahead", 8 * 1024 * 1024);
    let shared = source(&path, 0);
    let mut r = ChunkReader::new(&shared);
    let mut page = [0; PAGE];
    for round in 0..50 {
        for k in 0..40u64 {
            r.seek(SeekFrom::Start(k * 97 * 1024 + 512)).unwrap();
            r.read_exact(&mut page).unwrap();
        }
        // The root page, again and again.
        r.seek(SeekFrom::Start(0x4400)).unwrap();
        r.read_exact(&mut page).unwrap();
        assert_eq!(shared.borrow().bytes_read, 41 * 512, "round {round}: scattered pages read exactly, once");
    }

    // 2 MiB in blocks of 8 KiB from one place on: read ahead.
    let before = (shared.borrow().reads, shared.borrow().bytes_read);
    r.seek(SeekFrom::Start(5 * 1024 * 1024)).unwrap();
    let mut block = [0; 8192];
    for _ in 0..256 {
        r.read_exact(&mut block).unwrap();
    }
    let reads = shared.borrow().reads - before.0;
    assert!(reads <= 10, "{reads} reads for 2 MiB in order");
    assert!(shared.borrow().bytes_read - before.1 <= 2 * 1024 * 1024 + 256 * 1024);

    // Another handle on the same file: what's cached is read from the cache.
    let mut again = ChunkReader::new(&shared);
    let read = shared.borrow().bytes_read;
    again.seek(SeekFrom::Start(0x4400)).unwrap();
    again.read_exact(&mut page).unwrap();
    assert_eq!(shared.borrow().bytes_read, read);
    let _ = std::fs::remove_dir_all(&dir);
}

/// An attachment's blocks close together in no order: read by the chunk, not one by one; a block
/// far from the last is read exactly, and so are pages, near or not.
#[test]
fn blocks_near_the_last_read_their_chunk() {
    let (dir, path) = file_of("near", 16 * 1024 * 1024);
    let shared = source(&path, 0);
    let mut r = ChunkReader::new(&shared);
    let mut plain = std::fs::File::open(&path).unwrap();
    let mut block = [0; 8192];
    let mut want = [0; 8192];
    // 64 blocks of the 512 KiB from 4 MiB on, in a scattered order (37 is prime to 64).
    let base = 4 * 1024 * 1024u64;
    for i in 0..64u64 {
        let at = base + (i * 37 % 64) * 8192;
        r.seek(SeekFrom::Start(at)).unwrap();
        r.read_exact(&mut block).unwrap();
        plain.seek(SeekFrom::Start(at)).unwrap();
        plain.read_exact(&mut want).unwrap();
        assert_eq!(block, want, "block {i}");
    }
    let (reads, bytes) = (shared.borrow().reads, shared.borrow().bytes_read);
    assert!(reads <= 9, "{reads} reads for 64 blocks in 8 chunks");
    assert!(bytes <= 512 * 1024 + 8192, "{bytes} bytes read for 512 KiB");

    // Far from the last: exactly the block.
    r.seek(SeekFrom::Start(12 * 1024 * 1024 + 100)).unwrap();
    r.read_exact(&mut block).unwrap();
    assert_eq!(shared.borrow().bytes_read - bytes, 8192);
    // A page near it (not following on): exactly the page.
    let mut page = [0; PAGE];
    r.seek(SeekFrom::Start(13 * 1024 * 1024)).unwrap();
    r.read_exact(&mut page).unwrap();
    assert_eq!(shared.borrow().bytes_read - bytes, 8192 + PAGE as u64);
    let _ = std::fs::remove_dir_all(&dir);
}
