//! Big data files, on demand: making one, and importing it the way the app does, timed. Both are
//! ignored tests (they take minutes and gigabytes), run by hand:
//!
//! ```text
//! MC_PST=D:\big.pst MC_MESSAGES=200000 MC_ATTACH_KB=300 cargo test -p mailcraft-net --release --lib pst_bench::make -- --ignored --nocapture
//! MC_PST=D:\big.pst MC_CACHE=D:\cache cargo test -p mailcraft-net --release --lib pst_bench::import -- --ignored --nocapture
//! ```
//!
//! Memory is watched from outside (the test process's working set), so the numbers are the
//! process's, allocator and all.

use std::path::{Path, PathBuf};
use std::time::Instant;

use pst_rw::{UnicodePstAttachment, UnicodePstBatchMessage, UnicodePstFile, UnicodePstMessage, UnicodePstRecipient, UnicodePstRecipientType};

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

/// A cheap repeatable stream of bytes (xorshift), so attachments aren't all zeros.
struct Noise(u64);

impl Noise {
    fn bytes(&mut self, n: usize) -> Vec<u8> {
        let mut out = Vec::with_capacity(n);
        while out.len() < n {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            out.extend_from_slice(&self.0.to_le_bytes());
        }
        out.truncate(n);
        out
    }

    fn below(&mut self, n: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % n.max(1)
    }
}

const WORDS: [&str; 24] = [
    "storyboard",
    "budget",
    "review",
    "meeting",
    "poster",
    "invoice",
    "schedule",
    "draft",
    "mix",
    "camera",
    "festival",
    "proof",
    "deadline",
    "client",
    "studio",
    "render",
    "colour",
    "sound",
    "edit",
    "shoot",
    "contract",
    "travel",
    "lunch",
    "plan",
];

fn sentence(n: &mut Noise, words: usize) -> String {
    (0..words).map(|_| WORDS[n.below(WORDS.len() as u64) as usize]).collect::<Vec<_>>().join(" ")
}

/// Make a data file of `MC_MESSAGES` messages (default 20 000) across a few folders, one in three
/// with an attachment of about `MC_ATTACH_KB` (default 200 KB), at `MC_PST`.
#[test]
#[ignore]
fn make() {
    let path = PathBuf::from(env("MC_PST").expect("MC_PST: where to write the data file"));
    let total: usize = env("MC_MESSAGES").and_then(|v| v.parse().ok()).unwrap_or(20_000);
    let attach_kb: usize = env("MC_ATTACH_KB").and_then(|v| v.parse().ok()).unwrap_or(200);
    // Messages per write: the writer rewrites a folder's table each time, so few and big.
    let batch: usize = env("MC_BATCH").and_then(|v| v.parse().ok()).unwrap_or(5000);
    let _ = std::fs::remove_file(&path);
    let folders: [(&[&str], usize); 5] =
        [(&["Inbox"], 55), (&["Sent Items"], 15), (&["Inbox", "Projects"], 15), (&["Archive", "2024"], 10), (&["Archive", "2023"], 5)];
    let mut noise = Noise(0x9E37_79B9_7F4A_7C15);
    let began = Instant::now();
    let mut written = 0usize;
    let mut created = false;
    for (folder, share) in folders {
        let count = total * share / 100;
        let mut done = 0;
        while done < count {
            let n = (count - done).min(batch);
            // Owned first, borrowed by the writer's batch.
            let mut owned = Vec::with_capacity(n);
            for i in 0..n {
                let k = written + i;
                let subject = format!("{} #{k}", sentence(&mut noise, 4));
                let body = format!("Hello,\r\n\r\n{}\r\n\r\nThanks,\r\nRiley\r\n", sentence(&mut noise, 120));
                let html = format!("<html><body><p>Hello,</p><p>{}</p><p>Thanks,<br>Riley</p></body></html>", sentence(&mut noise, 120));
                let file = k.is_multiple_of(3).then(|| {
                    let size = attach_kb * 1024 / 2 + noise.below((attach_kb * 1024) as u64) as usize;
                    (format!("file-{k}.bin"), noise.bytes(size))
                });
                // Delivery times are Windows file times (100 ns since 1601).
                let unix = 1_700_000_000 + k as i64 * 60;
                owned.push((subject, body, html, format!("<m{k}@bench.example>"), file, (unix + 11_644_473_600) * 10_000_000));
            }
            let recipients = [UnicodePstRecipient { name: "Zoe Park", email: "zoe@studio.example", recipient_type: UnicodePstRecipientType::To }];
            let files: Vec<Vec<UnicodePstAttachment<'_>>> = owned
                .iter()
                .map(|o| {
                    o.4.iter()
                        .map(|(name, data)| UnicodePstAttachment { filename: name, mime_type: "application/octet-stream", content_id: None, data })
                        .collect()
                })
                .collect();
            let batch: Vec<UnicodePstBatchMessage<'_>> = owned
                .iter()
                .zip(&files)
                .map(|(o, f)| UnicodePstBatchMessage {
                    message: UnicodePstMessage {
                        subject: &o.0,
                        sender_name: "Ana Lima",
                        sender_email: "ana@soundlab.example",
                        recipients: &recipients,
                        body: &o.1,
                        html_body: Some(&o.2),
                        message_id: &o.3,
                        delivery_time: o.5,
                    },
                    attachments: f,
                })
                .collect();
            let (first, rest) = if created { (None, batch.as_slice()) } else { (batch.first(), batch.get(1..).unwrap_or(&[])) };
            if let Some(first) = first {
                UnicodePstFile::create_in_folder_with_attachments(&path, folder, &first.message, first.attachments).unwrap();
                created = true;
            }
            if !rest.is_empty() {
                UnicodePstFile::append_many_in_folder_with_attachments(&path, folder, rest).unwrap();
            }
            done += n;
            written += n;
            if written.is_multiple_of(10_000) || done == count {
                let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                println!("{written} messages, {:.2} GB, {:.0} s", size as f64 / 1e9, began.elapsed().as_secs_f64());
            }
        }
    }
    let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    println!("done: {written} messages, {:.2} GB in {:.0} s", size as f64 / 1e9, began.elapsed().as_secs_f64());
}

/// Import the data file at `MC_PST` into a fresh packed cache at `MC_CACHE` (as the app keeps
/// accounts), printing progress with the time.
#[test]
#[ignore]
fn import() {
    let path = PathBuf::from(env("MC_PST").expect("MC_PST: the data file to import"));
    let cache = PathBuf::from(env("MC_CACHE").expect("MC_CACHE: an empty folder for the cache"));
    // `MC_KEEP`: over what's there (an import stopped part-way, say).
    if env("MC_KEEP").is_none() {
        let _ = std::fs::remove_dir_all(&cache);
    }
    let store = mailcraft_store::Store::open_packed(cache.join("root"), cache.join("files")).unwrap();
    let began = Instant::now();
    let mut last = Instant::now();
    let mut progress = |n: usize| {
        if last.elapsed().as_secs_f64() >= 5.0 {
            last = Instant::now();
            println!("{n} messages, {:.0} s, {:.0}/s", began.elapsed().as_secs_f64(), n as f64 / began.elapsed().as_secs_f64());
        }
    };
    let done = crate::pst::import(&path, &store, "d1", &mut progress).unwrap();
    let secs = began.elapsed().as_secs_f64();
    let listed: usize = done.folders.iter().map(|f| store.load_folder("d1", &f.path).map(|st| st.messages.len()).unwrap_or(0)).sum();
    assert_eq!(listed, done.messages, "every imported message listed once");
    println!(
        "done: {} messages ({} skipped) in {:.0} s, {:.0}/s; cache {:.2} GB",
        done.messages,
        done.skipped,
        secs,
        done.messages as f64 / secs,
        dir_size(&cache) as f64 / 1e9
    );
}

fn dir_size(p: &Path) -> u64 {
    let Ok(rd) = std::fs::read_dir(p) else { return std::fs::metadata(p).map(|m| m.len()).unwrap_or(0) };
    rd.flatten().map(|e| if e.path().is_dir() { dir_size(&e.path()) } else { e.metadata().map(|m| m.len()).unwrap_or(0) }).sum()
}

/// A data file with a folder bigger than a page of the list: imported, the folder lists every
/// message once, newest first, with the right counts; its list is kept in pages (the import never
/// held it whole), and changing it afterwards writes as any folder does.
#[test]
fn a_big_folder_comes_in_a_page_at_a_time() {
    let dir = std::env::temp_dir().join(format!("mailcraft-pst-pages-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let pst = dir.join("big.pst");
    let n = mailcraft_store::PAGE_MESSAGES * 2 + 808;
    let owned: Vec<(String, String, i64)> =
        (0..n).map(|k| (format!("Report #{k}"), format!("<p{k}@bench.example>"), 1_700_000_000 + k as i64 * 60)).collect();
    let recipients = [UnicodePstRecipient { name: "Zoe", email: "zoe@studio.example", recipient_type: UnicodePstRecipientType::To }];
    let batch: Vec<UnicodePstBatchMessage<'_>> = owned
        .iter()
        .map(|o| UnicodePstBatchMessage {
            message: UnicodePstMessage {
                subject: &o.0,
                sender_name: "Ana",
                sender_email: "ana@soundlab.example",
                recipients: &recipients,
                body: "Hello.",
                html_body: None,
                message_id: &o.1,
                delivery_time: o.2,
            },
            attachments: &[],
        })
        .collect();
    UnicodePstFile::create_in_folder_with_attachments(&pst, &["Inbox"], &batch[0].message, batch[0].attachments).unwrap();
    for part in batch[1..].chunks(4000) {
        UnicodePstFile::append_many_in_folder_with_attachments(&pst, &["Inbox"], part).unwrap();
    }
    let store = mailcraft_store::Store::open_packed(dir.join("root"), dir.join("files")).unwrap();
    let done = crate::pst::import(&pst, &store, "d1", &mut |_| {}).unwrap();
    assert_eq!(done.messages, n);
    let inbox = done.folders.iter().find(|f| f.path == "INBOX").unwrap();
    let st = store.load_folder("d1", "INBOX").unwrap();
    assert_eq!(st.messages.len(), n);
    assert_eq!((inbox.total as usize, inbox.unread), (n, st.unread()), "counted as they went out");
    assert_eq!(st.messages[0].subject, format!("Report #{}", n - 1), "newest first");
    let mut uids: Vec<u32> = st.messages.iter().map(|m| m.key.uid).collect();
    uids.sort_unstable();
    uids.dedup();
    assert_eq!(uids.len(), n, "every message once");
    assert!(st.uid_next as usize > n);
    assert!(store.read_body(&st.messages[n / 2].key).is_some());
    // Changed afterwards: a flag, saved and read back.
    let mut st = st;
    st.messages[5].flags.seen = true;
    let key = st.messages[5].key.clone();
    store.save_folder("d1", "INBOX", &st).unwrap();
    assert!(store.load_folder("d1", "INBOX").unwrap().find(key.uid).unwrap().flags.seen);
    let _ = std::fs::remove_dir_all(&dir);
}
