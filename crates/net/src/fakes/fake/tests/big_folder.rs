//! A big folder kept whole, against the in-memory server: what its syncs cost.

use super::*;

/// MEASURE: a folder of 50,000 messages, all kept ("keep all mail"), on a server without
/// CONDSTORE: the first sync, then one that asks every message's flags (some changed, some gone,
/// some new). The digest is of the folder's list after, to tell a change of result.
#[test]
#[ignore = "a measurement"]
fn measure_a_big_folder_synced_without_condstore() {
    const N: u32 = 50_000;
    let st = State::new();
    {
        let mut s = st.lock().unwrap();
        s.caps = "IMAP4rev1 UIDPLUS MOVE";
        for n in 1..=N {
            s.add_raw("INBOX", format!("Subject: Message {n}\r\nMessage-ID: <b{n}@x>\r\n\r\nbody {n}\r\n").into_bytes());
        }
    }
    let store = store("big-folder");
    let mut c = login(&st);
    let opts = SyncOptions { window: usize::MAX, prefetch: 0, ..Default::default() };
    let began = Instant::now();
    let first = sync_folder(&mut c, &store, "a1", "INBOX", &opts).unwrap();
    let first_took = began.elapsed();
    {
        let mut s = st.lock().unwrap();
        for n in (1..=N).step_by(100) {
            s.set_flags("INBOX", n, &["\\Seen", "\\Flagged"]);
        }
        for n in (50..=N).step_by(1000) {
            s.expunge("INBOX", n);
        }
        for n in 1..=10 {
            s.add_raw("INBOX", format!("Subject: New {n}\r\nMessage-ID: <n{n}@x>\r\n\r\nnew\r\n").into_bytes());
        }
    }
    let began = Instant::now();
    let again = sync_folder(&mut c, &store, "a1", "INBOX", &opts).unwrap();
    let again_took = began.elapsed();
    let fs = store.load_folder("a1", "INBOX").unwrap();
    assert_eq!((first.new, again.new, again.changed, again.removed, fs.messages.len()), (50_000, 10, 500, 50, 49_960));
    let digest = mailcraft_model::hash::fnv1a(format!("{:?}", fs.messages).as_bytes());
    println!(
        "MEASURE sync of {N} kept without CONDSTORE: first {:.0} ms, every flag asked {:.0} ms (list {digest:016x})",
        first_took.as_secs_f64() * 1000.0,
        again_took.as_secs_f64() * 1000.0
    );
}
