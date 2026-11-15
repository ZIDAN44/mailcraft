//! Message files and copies of mail kept here, into a folder of each kind of server; and an IMAP
//! server that won't keep what a copy is marked with.

use super::*;

/// The key of a message kept here (a data file's), its source written.
fn kept_here(store: &Store) -> MessageKey {
    let key = MessageKey { account: "file".into(), folder: "Kept".into(), validity: 1, uid: 1 };
    let raw = b"From: Zoe <zoe@studio.test>\r\nTo: me@studio.test\r\nSubject: From the file\r\nMessage-ID: <kept@studio.test>\r\nDate: Mon, 2 Nov 2026 10:00:00 +0000\r\n\r\nHi.\r\n";
    store.write_body(&key, raw).unwrap();
    key
}

fn marked() -> Flags {
    Flags { seen: true, flagged: true, keywords: vec!["work".into()], ..Default::default() }
}

/// How many messages and copies went in.
fn both((added, copied, _): (usize, usize, Vec<Event>)) -> (usize, usize) {
    (added, copied)
}

/// Account `account` started on `pool`, then the message files and a copy of a message kept here
/// put into `folder`: what was said of each (messages added, copies filed), and everything said.
/// A wait that times out says what `server` tells of where its side stood.
fn file_both(
    pool: &mut Pool,
    store: &Store,
    account: AccountConfig,
    secret: &str,
    folder: &str,
    name: &str,
    server: &dyn Fn() -> String,
) -> (usize, usize, Vec<Event>) {
    let id = account.id.clone();
    pool.submit(Job::Start { account, password: Some(Secret(secret.into())) });
    let mut said = until_telling(pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }), server);
    let (dir, paths) = files(name);
    pool.submit(Job::FileMessages { account: id.clone(), folder: folder.into(), paths });
    said.extend(until_telling(pool, "filed", |e| matches!(e, Event::MessagesFiled { .. }), server));
    let Some(Event::MessagesFiled { added, .. }) = said.last().cloned() else { panic!() };
    pool.submit(Job::FileCopies { account: id.clone(), folder: folder.into(), mail: vec![(kept_here(store), marked())], moved: false });
    said.extend(until_telling(pool, "copied", |e| matches!(e, Event::CopiesFiled { .. }), server));
    let Some(Event::CopiesFiled { done, failed, .. }) = said.last().cloned() else { panic!() };
    assert!(failed.is_empty(), "{failed:?}");
    pool.submit(Job::Remove { account: id });
    let _ = std::fs::remove_dir_all(dir);
    (added, done.len(), said)
}

/// JMAP, Microsoft Graph, Exchange (EWS) and POP each take both, as received; JMAP keeps how
/// each was marked.
#[test]
fn every_kind_of_server_takes_message_files_and_copies() {
    use crate::fakes::{fake_ews, fake_graph, fake_jmap};
    // JMAP.
    let mut d = fake_jmap::Data::default();
    d.mailbox("Inbox", None, Some("inbox"));
    let server = fake_jmap::serve(d);
    let store = crate::fake::store("filing-kinds-jmap");
    let mut pool = Pool::new(store.clone(), std::sync::Arc::new(MemoryCredentials::default()));
    let account = fake_jmap::account("j1", &server.base);
    assert_eq!(both(file_both(&mut pool, &store, account, fake_jmap::PASSWORD, "INBOX", "kinds-jmap", &String::new)), (2, 1));
    {
        let data = server.data.lock().unwrap();
        let mut keywords: Vec<Vec<&str>> = data.emails.iter().map(|e| e.keywords.iter().map(String::as_str).collect()).collect();
        keywords.sort();
        assert_eq!(keywords, [vec!["$flagged", "$seen", "work"], vec!["$seen"], vec!["$seen"]]);
    }

    // Microsoft Graph.
    let mut d = fake_graph::Data::default();
    d.folder("Inbox", None, Some("inbox"));
    d.folder("Drafts", None, Some("drafts"));
    let server = fake_graph::serve(d);
    let store = crate::fake::store("filing-kinds-graph");
    let mut pool = Pool::new(store.clone(), std::sync::Arc::new(MemoryCredentials::default()));
    let account = fake_graph::account("g1", &server.base);
    assert_eq!(both(file_both(&mut pool, &store, account, fake_graph::TOKEN, "INBOX", "kinds-graph", &String::new)), (2, 1));
    assert_eq!(server.data.lock().unwrap().messages.len(), 3);

    // Exchange.
    let mut d = fake_ews::Data::default();
    let inbox = d.folder("Inbox", None, Some("inbox"), "IPF.Note");
    let server = fake_ews::serve(d);
    let store = crate::fake::store("filing-kinds-ews");
    let mut pool = Pool::new(store.clone(), std::sync::Arc::new(MemoryCredentials::default()));
    let account = fake_ews::account("e1", &server.url);
    assert_eq!(
        both(file_both(&mut pool, &store, account, fake_ews::PASSWORD, "INBOX", "kinds-ews", &|| server
            .data
            .lock()
            .map(|d| d.where_it_stopped())
            .unwrap_or_default())),
        (2, 1)
    );
    assert_eq!(server.data.lock().unwrap().items.iter().filter(|i| i.folder == inbox).count(), 3);

    // POP: its folders are on this computer, said to have changed.
    let server = crate::pop::fake::serve(Vec::new());
    let store = crate::fake::store("filing-kinds-pop");
    crate::fakes::fake_dav::looked_for_today(&store, "p1");
    let mut pool = Pool::new(store.clone(), std::sync::Arc::new(MemoryCredentials::default()));
    let account = AccountConfig { smtp: no_smtp(), ..pop_account(server.port) };
    let (added, copied, said) = file_both(&mut pool, &store, account, "pw", "Archive", "kinds-pop", &String::new);
    assert_eq!((added, copied), (2, 1));
    let changed = said.iter().filter(|e| matches!(e, Event::FolderChanged { folder, .. } if folder == "Archive")).count();
    assert_eq!(changed, 2, "once for the files, once for the copy");
    let kept = store.load_folder("p1", "Archive").unwrap();
    assert_eq!(kept.messages.len(), 3);
    assert!(kept.messages.iter().any(|m| m.flags.seen && m.flags.flagged && m.flags.keywords == ["work"]), "as it was marked");
}

/// An IMAP server that won't keep a copy's keywords is asked again without them; one that won't
/// keep the day it came either, without that too; one that won't take it at all, refuses it.
/// What it was asked is never asked again word for word, and a message file (no keywords, no
/// day) is asked for once.
#[test]
fn a_copy_imap_wont_keep_as_marked_goes_in_without_what_it_wont_keep() {
    let st = State::new();
    let store = crate::fake::store("filing-refused");
    crate::fakes::fake_dav::looked_for_today(&store, "a1");
    let mut pool = Pool::new(store.clone(), std::sync::Arc::new(MemoryCredentials::default()));
    pool.opts.prefetch = 0;
    let account = AccountConfig { smtp: no_smtp(), ..imap_account("a1", serve_tcp(&st)) };
    pool.submit(Job::Start { account, password: Some(Secret("pw".into())) });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    let key = kept_here(&store);
    let appends = |st: &std::sync::Arc<std::sync::Mutex<State>>| -> Vec<String> {
        let mut s = st.lock().unwrap();
        let a = s.log.iter().filter(|l| l.starts_with("APPEND")).cloned().collect();
        s.log.clear();
        a
    };
    let copy = |pool: &mut Pool, flags: Flags| -> Vec<(MessageKey, String)> {
        pool.submit(Job::FileCopies { account: "a1".into(), folder: "Archive".into(), mail: vec![(key.clone(), flags)], moved: false });
        let seen = until(pool, "copied", |e| matches!(e, Event::CopiesFiled { .. }));
        let Some(Event::CopiesFiled { failed, .. }) = seen.last().cloned() else { panic!() };
        failed
    };
    let archived = |st: &std::sync::Arc<std::sync::Mutex<State>>| st.lock().unwrap().mailbox("Archive").unwrap().msgs.last().map(|m| m.flags.clone());
    appends(&st);
    let dated = "\"02-Nov-2026 10:00:00 +0000\"";

    // Its keyword refused: in, marked as it was otherwise, on the day it came.
    st.lock().unwrap().refuse_appends = Some("work");
    assert!(copy(&mut pool, marked()).is_empty());
    let asked = appends(&st);
    assert_eq!(asked.len(), 2, "{asked:?}");
    assert!(asked[1].contains(r"(\Seen \Flagged)") && asked[1].contains(dated), "{asked:?}");
    assert_eq!(archived(&st), Some(vec![r"\Seen".to_string(), r"\Flagged".to_string()]));

    // The day refused too: in without it.
    st.lock().unwrap().refuse_appends = Some("+0000");
    assert!(copy(&mut pool, marked()).is_empty());
    let asked = appends(&st);
    assert_eq!(asked.len(), 3, "{asked:?}");
    assert!(!asked[2].contains(dated), "{asked:?}");

    // Nothing taken: refused, each different request asked once.
    st.lock().unwrap().refuse_appends = Some("Archive");
    assert_eq!(copy(&mut pool, marked()).len(), 1);
    assert_eq!(appends(&st).len(), 3);
    assert_eq!(copy(&mut pool, Flags::default()).len(), 1);
    let asked = appends(&st);
    assert_eq!(asked.len(), 2, "with and without the day: {asked:?}");

    // A message file, refused: asked once.
    let (dir, paths) = files("refused");
    pool.submit(Job::FileMessages { account: "a1".into(), folder: "Archive".into(), paths: paths[1..2].to_vec() });
    let seen = until(&pool, "filed", |e| matches!(e, Event::MessagesFiled { .. }));
    assert!(matches!(seen.last(), Some(Event::MessagesFiled { added: 0, .. })), "{seen:?}");
    assert_eq!(appends(&st).len(), 1);
    pool.submit(Job::Remove { account: "a1".into() });
    let _ = std::fs::remove_dir_all(dir);
}
