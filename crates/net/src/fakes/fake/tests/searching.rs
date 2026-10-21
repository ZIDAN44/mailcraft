//! Issue #4: searching the server and showing more, against the in-memory server.

use super::*;

/// Issue #4: the search box's words as IMAP SEARCH keys; what IMAP can't ask is left out.
#[test]
fn a_query_becomes_imap_search_keys() {
    use crate::sync::{SearchArg, imap_criteria};
    let a = |s: &str| SearchArg::Atom(s.into());
    let st = |s: &str| SearchArg::Str(s.into());
    assert_eq!(imap_criteria("quarterly"), Some(vec![a("TEXT"), st("quarterly")]));
    assert_eq!(
        imap_criteria("from:ana subject:\"the plan\" -draft is:unread"),
        Some(vec![a("FROM"), st("ana"), a("SUBJECT"), st("the plan"), a("NOT"), a("TEXT"), st("draft"), a("UNSEEN")])
    );
    assert_eq!(imap_criteria("is:flagged -is:unread"), Some(vec![a("FLAGGED"), a("SEEN")]));
    assert_eq!(imap_criteria("has:attachment"), None, "nothing IMAP can ask");
    assert_eq!(imap_criteria(""), None);
    assert_eq!(imap_criteria("Grüße"), Some(vec![a("TEXT"), st("grüße")]));
    // Sizes; a list as nested ORs (left to the list when one name is several words).
    assert_eq!(imap_criteria("larger:1KB -smaller:10"), Some(vec![a("LARGER"), a("1024"), a("NOT"), a("SMALLER"), a("10")]));
    assert_eq!(
        imap_criteria("from:ana,jun,\"zoe park\""),
        Some(vec![a("OR"), a("FROM"), st("ana"), a("OR"), a("FROM"), st("jun"), a("FROM"), st("zoe park")])
    );
    assert_eq!(imap_criteria("-to:ana,jun"), Some(vec![a("NOT"), a("OR"), a("TO"), st("ana"), a("TO"), st("jun")]));
    assert_eq!(
        imap_criteria("from:ana,jun lee"),
        Some(vec![a("TEXT"), st("lee")]).map(|v| [vec![a("OR"), a("FROM"), st("ana"), a("FROM"), st("jun")], v].concat())
    );
}

/// Gap #7: the query as Exchange's KQL/AQS words with the read, flag and attachment
/// conditions apart, and as a JMAP filter. What a user types can't break out of a word: quotes
/// and backslashes are dropped, a word spelled like an operator is quoted.
#[test]
fn a_query_becomes_exchange_words_and_a_jmap_filter() {
    use crate::sync::{WordQuery, word_query};
    use serde_json::json;
    let w = |text: &str| WordQuery { text: text.into(), ..Default::default() };
    assert_eq!(word_query("quarterly"), Some(w("quarterly")));
    assert_eq!(
        word_query("from:ana subject:\"the plan\" -draft is:unread"),
        Some(WordQuery { text: "from:ana subject:\"the plan\" NOT draft".into(), unread: Some(true), ..Default::default() })
    );
    assert_eq!(
        word_query("is:flagged -is:unread has:attachment"),
        Some(WordQuery { unread: Some(false), flagged: Some(true), attachment: Some(true), ..Default::default() })
    );
    assert_eq!(word_query(""), None);
    assert_eq!(word_query("is:important"), None, "nothing Exchange's searches ask");
    assert_eq!(word_query("OR NOT"), Some(w("or not")), "words, lowercased by the parser: not operators");
    assert_eq!(word_query("a(b) ana@x.example").map(|q| q.text), Some("a b ana x.example".into()), "the parser's own words");
    for hostile in ["subject:\"a\\\" OR from:x\" a(b)", "\"\"", "\\\"", "from:\"", "x\" OR \"y", "\u{0}\"\r\n", "-\"-\""] {
        let Some(q) = word_query(hostile) else { continue };
        assert_eq!(q.text.matches('"').count() % 2, 0, "{hostile:?} → {:?}", q.text);
        assert!(!q.text.contains('\\') && !q.text.contains("\"\"") && !q.text.chars().any(char::is_control), "{hostile:?} → {:?}", q.text);
    }
    let long = "x".repeat(5000);
    assert!(word_query(&long).unwrap().text.len() <= 202);
    let read = WordQuery { unread: Some(true), ..Default::default() };
    assert!(read.admits(&mailcraft_model::Flags::default(), false));
    assert!(!read.admits(&mailcraft_model::Flags { seen: true, ..Default::default() }, false));

    assert_eq!(
        mailcraft_jmap::search_filter("from:ana -draft is:unread is:flagged"),
        Some(json!({"operator": "AND", "conditions": [
            {"from": "ana"},
            {"operator": "NOT", "conditions": [{"text": "draft"}]},
            {"notKeyword": "$seen"},
            {"hasKeyword": "$flagged"},
        ]}))
    );
    assert_eq!(
        mailcraft_jmap::search_filter("-is:unread has:attachment"),
        Some(json!({"operator": "AND", "conditions": [{"hasKeyword": "$seen"}, {"hasAttachment": true}]}))
    );
    assert_eq!(mailcraft_jmap::search_filter("is:important"), None);
    assert_eq!(mailcraft_jmap::search_filter(""), None);
    // Sizes (minSize is at least, so one more byte), and a list as OR.
    assert_eq!(
        mailcraft_jmap::search_filter("larger:1KB smaller:2KB from:ana,\"zoe park\""),
        Some(json!({"operator": "AND", "conditions": [
            {"minSize": 1025},
            {"maxSize": 2048},
            {"operator": "OR", "conditions": [{"from": "ana"}, {"from": "zoe park"}]},
        ]}))
    );
}

/// Issue #4: mail past what's kept is found on the server. A folder keeps its newest 10 of
/// 30; searching the server finds an old one by a word of its body (and one by a word in
/// UTF-8, sent as a literal), brings it into the list, and then shows more fills in the
/// newest of the rest without skipping what's around the found one.
#[test]
fn mail_past_whats_kept_is_found_and_shown_more() {
    let st = State::new();
    {
        let mut s = st.lock().unwrap();
        for n in 1..=30 {
            let body = if n == 3 { "Grüße und needle-three".to_string() } else { format!("body {n}") };
            s.add_raw("INBOX", format!("Subject: Message {n}\r\nMessage-ID: <m{n}@x>\r\n\r\n{body}\r\n").into_bytes());
        }
    }
    let store = store("past-kept");
    let mut c = login(&st);
    sync_folder(&mut c, &store, "a1", "INBOX", &SyncOptions { window: 10, prefetch: 0, ..Default::default() }).unwrap();
    assert_eq!(store.load_folder("a1", "INBOX").unwrap().messages.len(), 10);

    let find = |c: &mut Client<Fake>, criteria: &[crate::sync::SearchArg]| {
        let (mut keys, mut new) = (Vec::new(), 0);
        crate::sync::find_on_server(c, &store, "a1", "INBOX", criteria, 500, &mut |k, n| {
            keys.extend(k);
            new += n;
            true
        })
        .unwrap();
        (keys, new)
    };
    let criteria = crate::sync::imap_criteria("needle-three").unwrap();
    let (keys, new) = find(&mut c, &criteria);
    assert_eq!((keys.iter().map(|k| k.uid).collect::<Vec<_>>(), new), (vec![3], 1));
    let kept = store.load_folder("a1", "INBOX").unwrap();
    assert!(kept.messages.iter().any(|m| m.subject == "Message 3") && kept.messages.len() == 11);
    // Found again: nothing new to bring in.
    assert_eq!(find(&mut c, &criteria), (keys.clone(), 0));
    // A word in UTF-8: sent as a literal with CHARSET UTF-8.
    let (keys, _) = find(&mut c, &crate::sync::imap_criteria("Grüße").unwrap());
    assert_eq!(keys.iter().map(|k| k.uid).collect::<Vec<_>>(), [3]);
    assert!(st.lock().unwrap().log.iter().any(|l| l.ends_with("UID SEARCH CHARSET UTF-8 1:30 TEXT ")), "a literal follows");
    // Nothing found: nothing changes.
    assert_eq!(find(&mut c, &crate::sync::imap_criteria("absent").unwrap()), (vec![], 0));

    // Show more: 10 more, the newest of what isn't kept (20 down to 11), the found one kept.
    sync_folder(&mut c, &store, "a1", "INBOX", &SyncOptions { window: 20, prefetch: 0, ..Default::default() }).unwrap();
    let mut uids: Vec<u32> = store.load_folder("a1", "INBOX").unwrap().messages.iter().map(|m| m.key.uid).collect();
    uids.sort_unstable();
    assert_eq!(uids, [3].into_iter().chain(12..=30).collect::<Vec<u32>>(), "the newest of the rest, up to 20 in all");
    // The usual window again: nothing is dropped.
    sync_folder(&mut c, &store, "a1", "INBOX", &SyncOptions { window: 10, prefetch: 0, ..Default::default() }).unwrap();
    assert_eq!(store.load_folder("a1", "INBOX").unwrap().messages.len(), 20);
    let _ = std::fs::remove_dir_all(store.root());
}

/// Issue #4: a large folder is searched newest first, a range of message numbers at a time
/// (the ranges growing while the server is quick), each range's finds handed on at once, and
/// the search can stop between two ranges.
#[test]
fn a_large_folder_is_searched_a_range_at_a_time_and_can_stop() {
    let st = State::new();
    {
        let mut s = st.lock().unwrap();
        for n in 1..=3000 {
            let body = if n % 100 == 0 { format!("needle {n}") } else { format!("body {n}") };
            s.add_raw("INBOX", format!("Subject: Message {n}\r\nMessage-ID: <r{n}@x>\r\n\r\n{body}\r\n").into_bytes());
        }
    }
    let store = store("search-ranges");
    let mut c = login(&st);
    sync_folder(&mut c, &store, "a1", "INBOX", &SyncOptions { window: 10, prefetch: 0, ..Default::default() }).unwrap();
    let criteria = crate::sync::imap_criteria("needle").unwrap();
    let searches = |st: &Arc<Mutex<State>>| -> Vec<String> {
        st.lock().unwrap().log.iter().filter(|l| l.contains("UID SEARCH") && l.contains("TEXT")).cloned().collect()
    };

    let mut batches: Vec<Vec<u32>> = Vec::new();
    let total = crate::sync::find_on_server(&mut c, &store, "a1", "INBOX", &criteria, 500, &mut |k, _| {
        batches.push(k.iter().map(|k| k.uid).collect());
        true
    })
    .unwrap();
    assert_eq!(total, 30);
    let asked = searches(&st);
    assert!(asked[0].contains(" 2501:3000 TEXT ") && asked[1].contains(" 1501:2500 TEXT ") && asked[2].contains(" 1:1500 TEXT "), "{asked:?}");
    assert_eq!(batches.iter().map(Vec::len).collect::<Vec<_>>(), [5, 10, 15], "each range's finds as they come");
    assert_eq!(batches[0], [2600, 2700, 2800, 2900, 3000]);
    assert_eq!(store.load_folder("a1", "INBOX").unwrap().messages.len(), 10 + 30 - 1, "3000 was kept already");

    // Stopped after the first range: nothing more is asked.
    st.lock().unwrap().log.clear();
    let mut calls = 0;
    crate::sync::find_on_server(&mut c, &store, "a1", "INBOX", &criteria, 500, &mut |_, _| {
        calls += 1;
        false
    })
    .unwrap();
    assert_eq!((calls, searches(&st).len()), (1, 1));
    // The cap: the newest 12 only, and no range asked past it.
    st.lock().unwrap().log.clear();
    let mut uids = Vec::new();
    crate::sync::find_on_server(&mut c, &store, "a1", "INBOX", &criteria, 12, &mut |k, _| {
        uids.extend(k.iter().map(|k| k.uid));
        true
    })
    .unwrap();
    uids.sort_unstable();
    assert_eq!(uids, (19..=30).map(|n| n * 100).collect::<Vec<u32>>());
    assert_eq!(searches(&st).len(), 2);
    let _ = std::fs::remove_dir_all(store.root());
}

/// Issue #4: Esc (the search box cleared) stops a server search that's still going: the
/// account answers that it's done, stops asking, and goes on with its other work.
#[test]
fn a_server_search_stops_when_its_no_longer_wanted() {
    let st = State::new();
    {
        let mut s = st.lock().unwrap();
        for n in 1..=20_000 {
            s.add_raw("INBOX", format!("Subject: M{n}\r\nMessage-ID: <s{n}@x>\r\n\r\nbody {n}\r\n").into_bytes());
        }
    }
    let port = serve_tcp(&st);
    let store = store("search-stop");
    let mut pool = crate::worker::Pool::new(store.clone(), std::sync::Arc::new(crate::worker::MemoryCredentials::default()));
    pool.submit(Job::SetSync { messages: 10, offline: 0 });
    let account = AccountConfig {
        id: "a1".into(),
        email: "me@studio.example".into(),
        username: "me@studio.example".into(),
        imap: ServerConfig { host: "127.0.0.1".into(), port, security: Security::None },
        smtp: ServerConfig { host: "127.0.0.1".into(), port: 1, security: Security::None },
        ..Default::default()
    };
    pool.submit(Job::Start { account, password: Some(Secret("pw".into())) });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    // Slow searching, as a server reading every message.
    {
        let mut s = st.lock().unwrap();
        s.search_delay = Duration::from_millis(1000);
        s.log.clear();
    }
    pool.submit(Job::SearchServer { account: "a1".into(), folders: vec!["INBOX".into()], query: "nowhere".into(), search: 7 });
    let end = Instant::now() + Duration::from_secs(20);
    while !st.lock().unwrap().log.iter().any(|l| l.contains("TEXT")) {
        assert!(Instant::now() < end, "never searched");
        let _ = pool.wait(Duration::from_millis(20));
    }
    pool.submit(Job::StopSearch);
    until(&pool, "done", |e| matches!(e, Event::ServerSearched { done: true, error: None, .. }));
    let asked = st.lock().unwrap().log.iter().filter(|l| l.contains("TEXT")).count();
    // A second range may have gone before the stop came; never the rest (a second each, the
    // ranges not growing: forty of them).
    assert!(asked <= 3, "stopped between two ranges, not after the folder's 20 000: {asked}");
    // The account goes on.
    st.lock().unwrap().search_delay = Duration::ZERO;
    pool.submit(Job::SyncFolder { account: "a1".into(), folder: "INBOX".into() });
    until(&pool, "synced", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    pool.submit(Job::Remove { account: "a1".into() });
    let _ = std::fs::remove_dir_all(store.root());
}

/// Issue #4 through the worker: Show more keeps the folder's larger window across syncs
/// (windows.json), and a server search answers with what it found, in the list.
#[test]
fn show_more_and_search_the_server_through_the_worker() {
    show_more_and_search("IMAP4rev1 UIDPLUS MOVE CONDSTORE ENABLE QUOTA");
}

/// The same against what Dovecot offers (issue #4: Show more did nothing there): with
/// QRESYNC, a folder's sync takes its quick path, which must keep the larger window too.
#[test]
fn show_more_and_search_with_qresync() {
    show_more_and_search("IMAP4rev1 UIDPLUS MOVE CONDSTORE QRESYNC ENABLE QUOTA ESEARCH");
}

fn show_more_and_search(caps: &'static str) {
    let st = State::new();
    {
        let mut s = st.lock().unwrap();
        s.caps = caps;
        for n in 1..=25 {
            s.add_raw("INBOX", format!("Subject: Message {n}\r\nMessage-ID: <w{n}@x>\r\n\r\nbody needle-{n}\r\n").into_bytes());
        }
    }
    let port = serve_tcp(&st);
    let store = store(if caps.contains("QRESYNC") { "worker-more-qresync" } else { "worker-more" });
    let mut pool = crate::worker::Pool::new(store.clone(), std::sync::Arc::new(crate::worker::MemoryCredentials::default()));
    pool.submit(Job::SetSync { messages: 10, offline: 0 });
    let account = AccountConfig {
        id: "a1".into(),
        email: "me@studio.example".into(),
        username: "me@studio.example".into(),
        imap: ServerConfig { host: "127.0.0.1".into(), port, security: Security::None },
        smtp: ServerConfig { host: "127.0.0.1".into(), port: 1, security: Security::None },
        ..Default::default()
    };
    pool.submit(Job::Start { account, password: Some(Secret("pw".into())) });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    assert_eq!(store.load_folder("a1", "INBOX").unwrap().messages.len(), 10);

    pool.submit(Job::SearchServer { account: "a1".into(), folders: vec!["INBOX".into()], query: "needle-2".into(), search: 1 });
    let seen = until(&pool, "searched", |e| matches!(e, Event::ServerSearched { done: true, .. }));
    let mut found: Vec<u32> = seen
        .iter()
        .flat_map(|e| match e {
            Event::ServerSearched { keys, error: None, query, .. } if query == "needle-2" => keys.iter().map(|k| k.uid).collect(),
            _ => Vec::new(),
        })
        .collect();
    found.sort_unstable();
    let found = Some(found);
    // "needle-2" is the words "needle" and "2", each found anywhere as IMAP's TEXT finds them
    // (so "needle-12" too).
    assert_eq!(found, Some(vec![2, 12, 20, 21, 22, 23, 24, 25]), "{seen:?}");
    assert!(seen.iter().any(|e| matches!(e, Event::FolderChanged { folder, .. } if folder == "INBOX")));
    assert!(store.load_folder("a1", "INBOX").unwrap().messages.iter().any(|m| m.key.uid == 2));

    pool.submit(Job::MoreMail { account: "a1".into(), folder: "INBOX".into(), more: 5 });
    let end = Instant::now() + Duration::from_secs(20);
    while store.load_folder("a1", "INBOX").unwrap().messages.len() < 17 {
        assert!(Instant::now() < end, "never showed more");
        let _ = pool.wait(Duration::from_millis(100));
    }
    let windows: std::collections::HashMap<String, usize> = store.load_account_doc("a1", "windows.json").unwrap();
    assert_eq!(windows.get("INBOX"), Some(&17), "12 kept (10, and 2 found), 5 more");
    // A later sync keeps them.
    pool.submit(Job::SyncFolder { account: "a1".into(), folder: "INBOX".into() });
    until(&pool, "synced", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    assert_eq!(store.load_folder("a1", "INBOX").unwrap().messages.len(), 17);
    pool.submit(Job::Remove { account: "a1".into() });
    // Offline: answered at once.
    pool.submit(Job::SearchServer { account: "a1".into(), folders: vec!["INBOX".into()], query: "x".into(), search: 2 });
    until(&pool, "offline answer", |e| matches!(e, Event::ServerSearched { error: Some(_), .. }));
    let _ = std::fs::remove_dir_all(store.root());
}
