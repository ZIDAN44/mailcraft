//! Engine tests: searching mail (the search box, its scopes, the servers).

use super::*;

/// Issue #4: the search box looks in the folder, it and its subfolders, its mailbox (at first)
/// or every mailbox, the one picked kept for the next searches; the recent searches are kept,
/// newest first, once each, ten of them, and can be cleared.
#[test]
fn search_scopes_and_recent_searches() {
    let mut s = sample_session("scopes");
    let a = sample::SAMPLE_ACCOUNT.to_string();
    assert_eq!(s.prefs.search_scope, SearchScope::Mailbox, "Current Mailbox at first");
    s.accounts[0].folders.push(mailcraft_model::Folder {
        account: a.clone(),
        path: "Projects old".into(),
        delimiter: Some('/'),
        selectable: true,
        ..Default::default()
    });
    s.open_view(View::Folder { account: a.clone(), folder: "Projects".into() });
    let folders = |s: &Session, scope| s.scope_folders(scope).into_iter().map(|(_, f)| f).collect::<Vec<_>>();
    assert_eq!(folders(&s, SearchScope::Current), ["Projects"]);
    assert_eq!(folders(&s, SearchScope::Subfolders), ["Projects", "Projects/Short film"], "not a folder whose name only starts the same");
    assert_eq!(folders(&s, SearchScope::Mailbox).len(), 9);
    // "shot list" is in Projects/Short film.
    let found = |s: &mut Session, scope: Option<&str>| {
        let mut v = json!({"query": "shot list"});
        if let Some(sc) = scope {
            v["scope"] = json!(sc);
        }
        s.run("search.run", &v).unwrap()["results"].as_u64().unwrap()
    };
    assert_eq!(found(&mut s, Some("current")), 0);
    assert_eq!(s.prefs.search_scope, SearchScope::Current, "kept for the next searches");
    assert_eq!(found(&mut s, None), 0);
    assert_eq!(found(&mut s, Some("subfolders")), 1);
    assert_eq!(found(&mut s, Some("mailbox")), 1);
    assert_eq!(found(&mut s, Some("all")), 1);
    assert_eq!(s.search.as_ref().map(|x| x.1), Some(SearchScope::All));
    assert!(s.run("search.run", &json!({"query": "x", "scope": "everywhere"})).is_err());
    // From a unified view, the mailbox is every account's.
    s.open_view(View::Unified { role: FolderRole::Inbox });
    assert_eq!(s.scope_folders(SearchScope::Mailbox).len(), s.scope_folders(SearchScope::All).len());

    // Recent searches: only those asked to be kept (Enter, a pick, leaving the box).
    assert!(s.prefs.recent_searches.is_empty(), "typing doesn't keep each word typed");
    assert!(s.run("search.forgetRecent", &json!({})).is_err(), "nothing to clear");
    s.run("search.run", &json!({"query": "budget", "remember": true})).unwrap();
    s.run("search.remember", &json!({"query": "  storyboard "})).unwrap();
    s.run("search.run", &json!({"query": "budget", "remember": true})).unwrap();
    assert_eq!(s.prefs.recent_searches, ["budget", "storyboard"]);
    for n in 0..12 {
        s.remember_search(&format!("q{n}"));
    }
    assert_eq!(s.prefs.recent_searches.len(), 10);
    assert_eq!(s.prefs.recent_searches.first().map(String::as_str), Some("q11"));
    s.remember_search("   ");
    assert_eq!(s.prefs.recent_searches.len(), 10);
    s.run("search.forgetRecent", &json!({})).unwrap();
    assert!(s.prefs.recent_searches.is_empty());

    // An account kept online only: what's searched for stays in memory (issue #4), and what was
    // kept with the settings before goes from them.
    s.remember_search("on disk");
    s.accounts[0].config.imap.host = "imap.studio.example".into();
    s.accounts[0].config.online_only = true;
    assert!(s.accounts[0].config.keeps_online());
    s.remember_search("private words");
    assert!(s.prefs.recent_searches.is_empty(), "nothing of it in the settings");
    assert_eq!(s.recent_searches(), ["private words", "on disk"]);
    s.run("search.forgetRecent", &json!({})).unwrap();
    assert!(s.recent_searches().is_empty());
    done(s);
}

#[test]
fn search_finds_bodies_fields_and_attachment_names() {
    let mut s = sample_session("search");
    assert_eq!(s.run("search.run", &json!({"query": "projector"})).unwrap()["results"], 1);
    assert_eq!(s.list[0].subject, "Re: Storyboard review on Thursday");
    assert_eq!(s.run("search.run", &json!({"query": "from:zoe"})).unwrap()["results"], 2);
    assert_eq!(s.run("search.run", &json!({"query": "turnaround"})).unwrap()["results"], 1);
    assert_eq!(s.run("search.run", &json!({"query": "shot list"})).unwrap()["results"], 1, "other folders are searched too");
    assert_eq!(s.run("search.run", &json!({"query": "shot list", "scope": "current"})).unwrap()["results"], 0);
    assert_eq!(s.run("search.run", &json!({"query": "has:attachment from:jun"})).unwrap()["results"], 1);
    assert!(s.view_title().contains("turnaround") || s.view_title().contains("jun"));
    s.run("search.clear", &json!({})).unwrap();
    assert!(s.search.is_none());
    assert_eq!(s.list.len(), 7);
    let _ = SearchScope::All;
    done(s);
}

/// Typing in the search box stays quick on a big mailbox: a search after mail changes (which
/// rebuilds the index) doesn't parse every message again.
#[test]
fn searching_a_big_mailbox_stays_quick() {
    let store = temp("big-search");
    store.save_accounts(&[mailcraft_model::AccountConfig { id: "a1".into(), email: "me@studio.example".into(), ..Default::default() }]).unwrap();
    store
        .save_folders(
            "a1",
            &[mailcraft_model::Folder {
                account: "a1".into(),
                path: "INBOX".into(),
                role: FolderRole::Inbox,
                selectable: true,
                ..Default::default()
            }],
        )
        .unwrap();
    let mut st = store.load_folder("a1", "INBOX").unwrap();
    for uid in 1..=3000u32 {
        let key = mailcraft_model::MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid };
        let html = format!("<html><body><table><tr><td><h1>Report {uid}</h1></td><td>{}</td></tr></table></body></html>", "word ".repeat(400));
        let raw = format!("From: a@x.example\r\nSubject: Report {uid}\r\nContent-Type: text/html\r\n\r\n{html}\r\n");
        store.write_body(&key, raw.as_bytes()).unwrap();
        st.messages.push(mailcraft_model::Summary { key, subject: format!("Report {uid}"), ..Default::default() });
    }
    st.messages[1234].subject = "needle here".into();
    store.save_folder("a1", "INBOX", &st).unwrap();
    let mut s = Session::open(Some(store.clone()), mailcraft_model::dates::Clock::new(NOW, 0));
    s.take_jobs();
    let t = std::time::Instant::now();
    assert_eq!(s.run("search.run", &json!({"query": "needle"})).unwrap()["results"], 1);
    let first = t.elapsed();
    // Mail changes (as a sync does), and the next key is typed: the index is built again.
    s.open_view(View::Folder { account: "a1".into(), folder: "INBOX".into() });
    let t = std::time::Instant::now();
    assert_eq!(s.run("search.run", &json!({"query": "needl"})).unwrap()["results"], 1);
    let again = t.elapsed();
    println!("first {first:?}, again {again:?}");
    assert!(again < first / 2 || again < std::time::Duration::from_millis(300), "first {first:?}, again {again:?}");
    let _ = std::fs::remove_dir_all(store.root());
}

/// Issue #4: mail past what is kept. A folder with more on its server says how many and shows
/// more when asked; a search asks the servers, and what they find is in the results whether or
/// not its words are in what is downloaded here.
#[test]
fn more_on_the_server_and_searching_it() {
    use mailcraft_model::{Folder, FolderRole, MessageKey, Summary};
    let mut s = sample_session("server-search");
    let a = s
        .run("account.add", &json!({"email": "zoe@example.org", "password": "pw", "imapHost": "mail.example.org", "smtpHost": "mail.example.org"}))
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    s.take_jobs();
    s.apply(Event::Folders {
        account: a.clone(),
        folders: vec![Folder {
            account: a.clone(),
            path: "INBOX".into(),
            role: FolderRole::Inbox,
            total: 50,
            selectable: true,
            ..Default::default()
        }],
    });
    let store = s.store.clone().unwrap();
    let key = |uid| MessageKey { account: a.clone(), folder: "INBOX".into(), validity: 1, uid };
    let mut st = store.load_folder(&a, "INBOX").unwrap();
    st.validity = 1;
    st.messages = (41..=50).map(|n| Summary { key: key(n), subject: format!("Message {n}"), ..Default::default() }).collect();
    store.save_folder(&a, "INBOX", &st).unwrap();
    s.view = crate::View::Folder { account: a.clone(), folder: "INBOX".into() };
    s.apply(Event::FolderChanged { account: a.clone(), folder: "INBOX".into() });
    assert_eq!(s.more_on_server(), Some((a.clone(), "INBOX".to_string(), 40)));
    s.run("folder.more", &json!({})).unwrap();
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::MoreMail { account, folder, more: 1000 } if *account == a && folder == "INBOX")));
    assert_eq!(s.showing_more, Some((a.clone(), "INBOX".to_string())));
    s.apply(Event::Status { account: a.clone(), status: SyncStatus::Online { at: 0 } });
    assert_eq!(s.showing_more, None, "settled, it can be asked again");
    assert_eq!(s.more_on_server().map(|m| m.2), Some(40));

    // Searching: nothing here has "needle"; the server finds an old message.
    assert!(s.run("search.server", &json!({})).is_err(), "search for something first");
    s.run("search.run", &json!({"query": "needle", "scope": "all"})).unwrap();
    assert!(s.list.is_empty());
    s.run("search.server", &json!({})).unwrap();
    let jobs = s.take_jobs();
    assert!(
        jobs.iter().any(
            |j| matches!(j, Job::SearchServer { account, query, folders, search: 1 } if *account == a && query == "needle" && folders == &["INBOX".to_string()])
        ),
        "{jobs:?}"
    );
    assert!(!jobs.iter().any(|j| matches!(j, Job::SearchServer { account, .. } if account == "sample")), "the sample mailbox has no server");
    assert_eq!(s.server_search.as_ref().map(|x| x.pending), Some(1));
    let mut st = store.load_folder(&a, "INBOX").unwrap();
    st.messages.push(Summary { key: key(7), subject: "Old one".into(), ..Default::default() });
    store.save_folder(&a, "INBOX", &st).unwrap();
    s.apply(Event::FolderChanged { account: a.clone(), folder: "INBOX".into() });
    // Shown as soon as it's found, the account still searching.
    s.apply(Event::ServerSearched { account: a.clone(), query: "needle".into(), keys: vec![key(7)], error: None, done: false });
    assert_eq!(s.list.iter().map(|m| m.subject.as_str()).collect::<Vec<_>>(), ["Old one"]);
    assert_eq!(s.server_search.as_ref().map(|x| (x.pending, x.found.len())), Some((1, 1)));
    s.apply(Event::ServerSearched { account: a.clone(), query: "needle".into(), keys: Vec::new(), error: None, done: true });
    assert_eq!(s.server_search.as_ref().map(|x| (x.pending, x.found.len())), Some((0, 1)));
    // An answer for another query is not this search's; a new query starts again.
    s.apply(Event::ServerSearched { account: a.clone(), query: "other".into(), keys: vec![key(41)], error: None, done: true });
    assert_eq!(s.list.len(), 1);
    s.run("search.run", &json!({"query": "something else", "scope": "all"})).unwrap();
    assert!(s.server_search.is_none());
    assert!(!s.take_jobs().contains(&Job::StopSearch), "nothing was still searching");
    // Clearing the search box (Esc, or its ×) stops a search still running: a new id each time.
    s.run("search.server", &json!({})).unwrap();
    assert!(s.take_jobs().iter().any(|j| matches!(j, Job::SearchServer { search: 2, .. })));
    s.run("search.clear", &json!({})).unwrap();
    assert!(s.server_search.is_none());
    assert_eq!(s.take_jobs(), vec![Job::StopSearch]);
    // Leaving the folder stops one too.
    s.run("search.run", &json!({"query": "needle", "scope": "all"})).unwrap();
    s.run("search.server", &json!({})).unwrap();
    s.take_jobs();
    s.open_view(crate::View::Folder { account: a.clone(), folder: "INBOX".into() });
    assert_eq!(s.take_jobs(), vec![Job::StopSearch]);
    done(s);
}
