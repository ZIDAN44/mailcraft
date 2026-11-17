//! A download says how it goes, as Send / Receive shows it (Outlook's "Receiving message 10 of
//! 158 (27.6 MB of 210 MB)"), and stops before a message when asked (Cancel), what came kept.

use super::*;

fn account(port: u16) -> AccountConfig {
    let cfg = ServerConfig { host: "127.0.0.1".into(), port, security: Security::None };
    AccountConfig { id: "p1".into(), email: "me@pop.example".into(), pop: Some(cfg), ..Default::default() }
}

#[test]
fn a_download_says_how_it_goes_and_stops_when_asked() {
    let drop: Vec<(String, Vec<u8>)> = (0..5).map(|i| (format!("u{i}"), mail(&format!("M{i}"), &"x".repeat(100 * (i + 1))))).collect();
    let sizes: Vec<u64> = drop.iter().map(|(_, m)| m.len() as u64).collect();
    let server = fake::serve(drop);
    let dir = std::env::temp_dir().join(format!("mailcraft-pop-progress-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    let acc = account(server.port);
    folders(&store, "p1").unwrap();

    // Stopped before the fourth: three came, and were kept.
    let mut heard: Vec<Fetching> = Vec::new();
    let mut c = open(&acc, "pw").unwrap();
    let r = fetch_new_as(&mut c, &store, "p1", "INBOX", &Keep::default(), 1_000, 0, &mut |f| {
        heard.push(*f);
        f.done < 3
    })
    .unwrap();
    c.quit().unwrap();
    assert_eq!(r.new, 3);
    let all: u64 = sizes.iter().sum();
    assert_eq!(
        heard.first(),
        Some(&Fetching { done: 0, total: 5, bytes_done: 0, bytes_total: all, saved: 0 }),
        "told before the first, with all that's to come"
    );
    let three: u64 = sizes.iter().take(3).sum();
    assert_eq!(heard.last(), Some(&Fetching { done: 3, total: 5, bytes_done: three, bytes_total: all, saved: 3 }), "and as it stopped");
    assert!(heard.windows(2).all(|w| w[0].done <= w[1].done && w[0].bytes_done <= w[1].bytes_done), "only ever further on");
    assert_eq!(store.load_folder("p1", "INBOX").unwrap().messages.len(), 3);

    // Gone on with: the other two, counted as what's new now.
    heard.clear();
    let mut c = open(&acc, "pw").unwrap();
    assert_eq!(
        fetch_new_as(&mut c, &store, "p1", "INBOX", &Keep::default(), 1_000, 0, &mut |f| {
            heard.push(*f);
            true
        })
        .unwrap()
        .new,
        2
    );
    c.quit().unwrap();
    let two: u64 = sizes.iter().skip(3).sum();
    assert_eq!(heard.last(), Some(&Fetching { done: 2, total: 2, bytes_done: two, bytes_total: two, saved: 2 }));
    assert_eq!(store.load_folder("p1", "INBOX").unwrap().messages.len(), 5, "none twice");

    // Nothing new: told once, at the end, with nothing to come.
    heard.clear();
    let mut c = open(&acc, "pw").unwrap();
    fetch_new_as(&mut c, &store, "p1", "INBOX", &Keep::default(), 1_000, 0, &mut |f| {
        heard.push(*f);
        true
    })
    .unwrap();
    c.quit().unwrap();
    assert_eq!(heard, [Fetching::default()]);
    let _ = std::fs::remove_dir_all(dir);
}

/// A big download is in the Inbox's list a batch at a time, and says so as each is saved: the
/// window shows them as they come (a 210 MB download's Inbox stayed empty until its end).
#[test]
fn a_big_download_says_each_batch_it_saves() {
    // Two of these pass a batch's size ([`SAVE_BYTES`]).
    let big = SAVE_BYTES / 2 + 1;
    let drop: Vec<(String, Vec<u8>)> = (0..3).map(|i| (format!("u{i}"), mail(&format!("B{i}"), &"y".repeat(big)))).collect();
    let server = fake::serve(drop);
    let dir = std::env::temp_dir().join(format!("mailcraft-pop-batches-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    let acc = account(server.port);
    folders(&store, "p1").unwrap();
    let mut heard: Vec<Fetching> = Vec::new();
    let mut c = open(&acc, "pw").unwrap();
    let mut listed = Vec::new();
    fetch_new_as(&mut c, &store, "p1", "INBOX", &Keep::default(), 1_000, 0, &mut |f| {
        // What the Inbox's list has as it's told.
        listed.push(store.load_folder("p1", "INBOX").unwrap().messages.len());
        heard.push(*f);
        true
    })
    .unwrap();
    c.quit().unwrap();
    let saved: Vec<u32> = heard.iter().map(|f| f.saved).collect();
    assert_eq!(saved, [0, 0, 2, 3], "the first two saved before the third came, the third at the end");
    assert_eq!(listed, [0, 0, 2, 3], "said once they're in the list");
    let _ = std::fs::remove_dir_all(dir);
}
