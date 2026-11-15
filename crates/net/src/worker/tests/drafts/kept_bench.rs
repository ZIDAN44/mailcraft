//! Keeping n drafts on this computer and filing them all at the next connection, timed: an ignored
//! test, run by hand.
//!
//! ```text
//! cargo test -p mailcraft-net --release --lib kept_drafts_filed_timed -- --ignored --nocapture
//! ```
//!
//! `MC_KEPT` sets how many drafts (40), `MC_KEPT_KB` how big each is (1024 KB). Each is kept as
//! offline into a store kept as the app keeps it (each account one data file), and then all are
//! filed on the fake IMAP server at one connection. It prints a `MEASURE` line: how long keeping
//! them took, how long filing them took, and how big the data file was with them kept.

use super::kept_texts;
use super::*;

fn env(name: &str, or: usize) -> usize {
    std::env::var(name).ok().and_then(|v| v.trim().parse().ok()).unwrap_or(or)
}

/// A draft of about `size` bytes: text lines, as an attachment's base64 would be.
fn draft(n: usize, size: usize) -> Vec<u8> {
    let mut raw = format!("From: me@studio.example\r\nSubject: Plans {n}\r\nMessage-ID: <bench{n}@studio.example>\r\n\r\n").into_bytes();
    let line: Vec<u8> = (0..76u8).map(|i| b'A' + i.wrapping_add(n as u8) % 26).chain(*b"\r\n").collect();
    while raw.len() < size {
        raw.extend_from_slice(&line);
    }
    raw
}

#[test]
#[ignore = "a measurement, run by hand"]
fn kept_drafts_filed_timed() {
    let (n, kb) = (env("MC_KEPT", 40), env("MC_KEPT_KB", 1024));
    let st = crate::fake::State::new();
    st.lock().unwrap().boxes.push(crate::fake::Mailbox::new("Drafts", r"\Drafts", 600));
    let port = crate::fake::serve_tcp(&st);
    let account = imap_account("a1", port);
    // The app's store: each account one data file.
    let root = crate::data_files::scratch::scratch("kept-bench");
    let files = root.join("files");
    let store = Store::open_packed(&root, &files).unwrap();
    crate::fakes::fake_dav::looked_for_today(&store, "a1");
    let drafts: Vec<Vec<u8>> = (0..n).map(|i| draft(i, kb << 10)).collect();
    let start = Instant::now();
    for d in &drafts {
        keep_draft(&store, "a1", "Drafts", None, d.clone(), false, &Order::default()).unwrap();
    }
    let keeping = start.elapsed();
    let on_disk = std::fs::read_dir(&files).unwrap().filter_map(|e| e.ok()?.metadata().ok()).map(|m| m.len()).sum::<u64>();
    let start = Instant::now();
    let mut pool = Pool::new(store.clone(), Arc::new(MemoryCredentials::default()));
    pool.submit(Job::Start { account, password: Some(Secret("pw".into())) });
    // The last one is kept no more just after it's filed.
    crate::waiting::eventually_within(Duration::from_secs(600), "every kept draft filed, and kept no more", || {
        st.lock().unwrap().mailbox("Drafts").map_or(0, |b| b.msgs.len()) >= n && kept_texts(&store).is_empty()
    });
    let filing = start.elapsed();
    println!(
        "MEASURE kept drafts: {n} of {kb} KB; keeping all {:.0} ms; filing all at one connection {:.0} ms; data file {} MB",
        keeping.as_secs_f64() * 1000.0,
        filing.as_secs_f64() * 1000.0,
        on_disk >> 20
    );
    pool.submit(Job::Remove { account: "a1".into() });
}
