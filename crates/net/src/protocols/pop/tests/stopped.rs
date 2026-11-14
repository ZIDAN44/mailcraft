//! A download stopped part-way (the connection cut, the cache not written, the app killed) loses
//! nothing and lists nothing twice, now that new mail is saved in batches.

use std::sync::{Arc, Mutex};

use super::*;

/// More than two batches ([`SAVE_EVERY`]).
const COUNT: usize = 1_200;

fn maildrop() -> Vec<(String, Vec<u8>)> {
    (0..COUNT).map(|i| (format!("u{i}"), mail(&format!("M{i}"), ""))).collect()
}

fn account(port: u16) -> AccountConfig {
    let cfg = ServerConfig { host: "127.0.0.1".into(), port, security: Security::None };
    AccountConfig { id: "p1".into(), email: "me@pop.example".into(), pop: Some(cfg), ..Default::default() }
}

fn fetch(store: &Store, acc: &AccountConfig, keep: &Keep) -> Result<SyncReport> {
    let mut c = open(acc, "pw").unwrap();
    let r = fetch_new(&mut c, store, "p1", "INBOX", keep, 0)?;
    c.quit().unwrap();
    Ok(r)
}

fn in_inbox(store: &Store) -> usize {
    store.load_folder("p1", "INBOX").unwrap().messages.len()
}

/// Every message of the maildrop in the Inbox once, with its source, and recorded once.
fn whole(store: &Store) {
    let inbox = store.load_folder("p1", "INBOX").unwrap();
    let mut subjects: Vec<&str> = inbox.messages.iter().map(|m| m.subject.as_str()).collect();
    subjects.sort_unstable();
    subjects.dedup();
    assert_eq!((inbox.messages.len(), subjects.len()), (COUNT, COUNT));
    assert!(inbox.messages.iter().all(|m| store.has_body(&m.key)), "every source here");
    let state: State = store.load_account_doc("p1", STATE_DOC).unwrap();
    assert_eq!(state.len(), COUNT);
    assert!(state.values().all(|k| !k.pending));
}

/// The connection cut part-way: what came before is kept, nothing goes from the server (the
/// session never ended), and the next check brings the rest, once.
#[test]
fn a_download_cut_off_keeps_what_came_and_gets_the_rest_once() {
    let server = fake::serve(maildrop());
    let (store, acc) = (Store::memory(), account(server.port));
    folders(&store, "p1").unwrap();
    let keep = Keep { remove_now: true, ..Keep::default() };
    *server.on_retr.lock().unwrap() = Some(Box::new(|n| n != 700));
    assert!(fetch(&store, &acc, &keep).is_err());
    assert_eq!(in_inbox(&store), 699, "what came before the cut is kept");
    assert_eq!(server.drop.lock().unwrap().len(), COUNT, "nothing removed from the server");
    *server.on_retr.lock().unwrap() = None;
    assert_eq!(fetch(&store, &acc, &keep).unwrap().new, COUNT - 699);
    whole(&store);
    assert!(server.drop.lock().unwrap().is_empty(), "removed once saved here");
}

/// A batch's sources, or the Inbox's list, can't be written after its record was: the record
/// names messages the list doesn't, and the next check brings them again, once.
#[test]
fn a_save_stopped_between_the_record_and_the_list_downloads_nothing_twice() {
    let probe = Store::failing();
    let key = MessageKey { account: "p1".into(), folder: "INBOX".into(), validity: 1, uid: 1 };
    let path = probe.body_path(&key);
    let dir = path.parent().and_then(std::path::Path::parent).and_then(std::path::Path::file_name).unwrap().to_string_lossy();
    let list = format!("{dir}/state.json");
    for fails in [".eml", list.as_str()] {
        let server = fake::serve(maildrop());
        let (store, acc) = (Store::failing(), account(server.port));
        folders(&store, "p1").unwrap();
        let (s, fails_now) = (store.clone(), fails.to_string());
        *server.on_retr.lock().unwrap() = Some(Box::new(move |n| {
            if n == 600 {
                s.fail(None, Some(&fails_now));
            }
            true
        }));
        assert!(fetch(&store, &acc, &Keep::default()).is_err(), "{fails}");
        store.fail(None, None);
        let state: State = store.load_account_doc("p1", STATE_DOC).unwrap();
        assert_eq!((state.len(), state.values().filter(|k| k.pending).count()), (2 * SAVE_EVERY, SAVE_EVERY), "{fails}");
        assert_eq!(in_inbox(&store), SAVE_EVERY, "{fails}");
        *server.on_retr.lock().unwrap() = None;
        assert_eq!(fetch(&store, &acc, &Keep::default()).unwrap().new, COUNT - SAVE_EVERY, "{fails}");
        whole(&store);
    }
}

/// The app killed at any point between saves (the cache as it was then): the next check brings
/// what wasn't saved, once.
#[test]
fn a_download_killed_anywhere_loses_nothing_and_lists_nothing_twice() {
    let server = fake::serve(maildrop());
    let (store, acc) = (Store::memory(), account(server.port));
    folders(&store, "p1").unwrap();
    let shots: Arc<Mutex<Vec<Vec<(String, Vec<u8>)>>>> = Arc::default();
    let (s, taken) = (store.clone(), shots.clone());
    // Asked before each answer, while the download waits for it: the cache as a kill then left it.
    *server.on_retr.lock().unwrap() = Some(Box::new(move |n| {
        if [1, 250, 500, 501, 999, 1001, 1200].contains(&n) {
            taken.lock().unwrap().push(s.snapshot(|_| true));
        }
        true
    }));
    assert_eq!(fetch(&store, &acc, &Keep::default()).unwrap().new, COUNT);
    whole(&store);
    *server.on_retr.lock().unwrap() = None;
    let shots = std::mem::take(&mut *shots.lock().unwrap());
    let saved: Vec<usize> = shots
        .into_iter()
        .map(|files| {
            let store = Store::memory_from(files);
            let before = in_inbox(&store);
            assert_eq!(fetch(&store, &acc, &Keep::default()).unwrap().new, COUNT - before);
            whole(&store);
            before
        })
        .collect();
    assert_eq!(saved, [0, 0, 0, 500, 500, 1000, 1000], "saved a batch at a time");
}

/// Stopped after recording a batch, before clearing its mark: each marked message the list names
/// stays, as does one moved or deleted for good since; one it doesn't name comes again, its
/// source (written before the stop) removed.
#[test]
fn a_record_marked_as_saving_is_settled() {
    let server = fake::serve(["listed", "lost", "gone", "moved"].map(|id| (id.to_string(), mail(id, ""))).to_vec());
    let (store, acc) = (Store::memory(), account(server.port));
    folders(&store, "p1").unwrap();
    let key = |folder: &str, uid: u32| MessageKey { account: "p1".into(), folder: folder.into(), validity: 1, uid };
    let raw = mail("listed", "");
    let listed = mime::summary_from_full(&raw, key("INBOX", 1), Flags::default(), 1, 0);
    store.write_body(&key("INBOX", 1), &raw).unwrap();
    store.write_body(&key("INBOX", 2), &mail("lost", "")).unwrap();
    store.save_folder("p1", "INBOX", &FolderState { validity: 1, uid_next: 5, messages: vec![listed.into()], ..Default::default() }).unwrap();
    let marked = |k: MessageKey, gone: bool| Kept { at: 0, key: Some(k.to_id()), gone, pending: true };
    let state: State = [
        ("listed", marked(key("INBOX", 1), false)),
        ("lost", marked(key("INBOX", 2), false)),
        ("gone", marked(key("INBOX", 3), true)),
        ("moved", marked(key("Archive", 7), false)),
    ]
    .into_iter()
    .map(|(id, k)| (id.to_string(), k))
    .collect();
    store.save_account_doc("p1", STATE_DOC, &state).unwrap();
    assert_eq!(fetch(&store, &acc, &Keep::default()).unwrap().new, 1, "only the one the list didn't name");
    assert!(!store.has_body(&key("INBOX", 2)), "its source from before, named by nothing, removed");
    let inbox = store.load_folder("p1", "INBOX").unwrap();
    let mut subjects: Vec<&str> = inbox.messages.iter().map(|m| m.subject.as_str()).collect();
    subjects.sort_unstable();
    assert_eq!(subjects, ["listed", "lost"]);
    assert!(inbox.messages.iter().all(|m| m.key.uid == 1 || m.key.uid >= 5), "no number given twice");
    let state: State = store.load_account_doc("p1", STATE_DOC).unwrap();
    assert!(state.values().all(|k| !k.pending));
    assert!(state["gone"].gone);
    assert_eq!(state["moved"].key, Some(key("Archive", 7).to_id()));
}
