//! Engine tests: searching mail (the search box, its scopes, the servers).

use super::*;

mod run;

/// Bring `account`'s search index up to its mail at once, as the indexer does in the
/// background: its generation.
pub(super) fn index_now(s: &Session, account: &str) -> u64 {
    let store = s.store.clone().unwrap();
    let mut p = mailcraft_store::search::Pass::start(&store, account, &[]).unwrap();
    let adds = p.adds().clone();
    for (i, m) in adds.iter().enumerate() {
        let doc = store.read_body(&m.key).map(|raw| mailcraft_mime::index_doc(&raw));
        p.add(&store, i, doc.as_ref()).unwrap();
    }
    p.finish(&store, true).unwrap();
    p.generation()
}

/// Tell the session a pass over `account`'s index is over.
pub(super) fn indexed(s: &mut Session, account: &str, generation: u64) {
    s.apply(Event::Indexed { account: account.into(), work: Default::default(), generation, failed: 0, finished: true });
}

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

/// Search reads accents away (both ways round), places `received:` days where the user is from
/// the session's clock and answers them from the summaries alone, and leaves a date that isn't
/// one out rather than looking for its digits.
#[test]
fn search_folds_accents_and_reads_dates() {
    let mut s = sample_session("search-fold");
    let all = |s: &mut Session, q: &str| s.run("search.run", &json!({"query": q, "scope": "all"})).unwrap()["results"].as_u64().unwrap() as usize;
    assert_eq!(all(&mut s, "PRÓJECTOR"), 1, "an accent typed finds the word written without");
    assert_eq!(all(&mut s, "projector"), 1);
    assert_eq!(all(&mut s, "turnaróunds -fóx"), 0, "a word left out, folded too");
    let c = mailcraft_model::dates::civil(s.clock.now, &s.clock);
    let day = format!("{:04}-{:02}-{:02}", c.year, c.month, c.day);
    let start = crate::parse_local(&day, &s.clock).map(|(t, _)| t).unwrap();
    let today = counted(&mut s, |m| (start..start + 86_400).contains(&m.date));
    let before = counted(&mut s, |m| m.date < start);
    assert!(today > 0 && before > 0, "the sample has mail today and before");
    let reads = s.store.as_ref().unwrap().reads();
    assert_eq!(all(&mut s, "received:today"), today);
    assert_eq!(all(&mut s, &format!("received:{day}")), today);
    assert_eq!(all(&mut s, &format!("before:{day}")), before);
    assert_eq!(all(&mut s, &format!("-received:{day}")), counted(&mut s, |m| !(start..start + 86_400).contains(&m.date)));
    assert_eq!(s.store.as_ref().unwrap().reads(), reads, "dates are the summaries': no message was read");
    assert_eq!(all(&mut s, "received:2026-13"), 0, "not a date: left out, nothing to search");
    done(s);
}

/// Typing in the search box on a big indexed mailbox reads no message: the index answers words,
/// and mail changing in a folder (a sync taking a message away) has that folder scanned again
/// with its index, still reading none.
#[test]
fn searching_a_big_mailbox_reads_no_message() {
    let store = Store::memory();
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
    st.validity = 1;
    for uid in 1..=3000u32 {
        let key = mailcraft_model::MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid };
        let word = if uid == 1234 { "zeppelin" } else { "word" };
        let html =
            format!("<html><body><table><tr><td><h1>Report {uid}</h1></td><td>{}</td></tr></table></body></html>", format!("{word} ").repeat(400));
        let raw = format!("From: a@x.example\r\nSubject: Report {uid}\r\nContent-Type: text/html\r\n\r\n{html}\r\n");
        store.write_body(&key, raw.as_bytes()).unwrap();
        st.messages.push(mailcraft_model::Summary { key, subject: format!("Report {uid}"), ..Default::default() });
    }
    st.messages[1234].subject = "needle here".into();
    store.save_folder("a1", "INBOX", &st).unwrap();
    let mut s = Session::open(Some(store.clone()), mailcraft_model::dates::Clock::new(NOW, 0));
    s.take_jobs();
    index_now(&s, "a1");
    let reads = store.reads();
    assert_eq!(s.run("search.run", &json!({"query": "needle"})).unwrap()["results"], 1);
    assert_eq!(s.run("search.run", &json!({"query": "zeppelin"})).unwrap()["results"], 1, "a word of the text, from the index");
    // Mail changes (as a sync does: a message goes), and the next key is typed.
    st.messages.pop();
    store.save_folder("a1", "INBOX", &st).unwrap();
    s.apply(Event::FolderChanged { account: "a1".into(), folder: "INBOX".into() });
    assert_eq!(s.run("search.run", &json!({"query": "zeppeli"})).unwrap()["results"], 1);
    assert_eq!(s.run("search.run", &json!({"query": "needl"})).unwrap()["results"], 1);
    assert_eq!(store.reads(), reads, "no message was read");
    assert_eq!(s.search_stats().sources, 0);
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

/// What every folder of the sample mailbox holds that `keep` keeps, counted from the summaries.
fn counted(s: &mut Session, keep: impl Fn(&mailcraft_model::Summary) -> bool) -> usize {
    let mut n = 0;
    for (a, f) in s.scope_folders(SearchScope::All) {
        n += s.folder_state(&a, &f).messages.iter().filter(|m| keep(m)).count();
    }
    n
}

/// Flags, senders, sizes: the summaries answer, and no message is read for them (a big data
/// file's every message was read again for `is:unread`), nor for a Search Folder asking them.
#[test]
fn summary_searches_read_no_message() {
    let mut s = sample_session("summary-search");
    let unread = counted(&mut s, |m| !m.flags.seen);
    let flagged = counted(&mut s, |m| m.flags.flagged);
    let from_zoe = counted(&mut s, |m| m.from.name.to_lowercase().contains("zoe") || m.from.email.contains("zoe"));
    assert!(unread > 0 && flagged > 0 && from_zoe > 0, "the sample has some of each");
    let reads = s.store.as_ref().unwrap().reads();
    let all = |s: &mut Session, q: &str| s.run("search.run", &json!({"query": q, "scope": "all"})).unwrap()["results"].as_u64().unwrap() as usize;
    assert_eq!(all(&mut s, "is:unread"), unread);
    assert_eq!(all(&mut s, "is:flagged"), flagged);
    assert_eq!(all(&mut s, "from:zoe"), from_zoe);
    all(&mut s, "has:attachment -is:read larger:1KB subject:storyboard");
    s.run("searchFolder.create", &json!({"name": "Unread", "query": "is:unread"})).unwrap();
    assert!(!s.list.is_empty());
    assert_eq!(s.store.as_ref().unwrap().reads(), reads, "no message was read");
    assert_eq!((s.search_stats().reader_pages, s.search_stats().sources), (0, 0), "nor any index");
    // Words anywhere need the text: with no index, a search that waits reads it.
    assert_eq!(all(&mut s, "projector"), 1);
    assert!(s.search_stats().sources > 0);
    // With one, it's the index that's read.
    index_now(&s, sample::SAMPLE_ACCOUNT);
    let (reads, sources) = (s.store.as_ref().unwrap().reads(), s.search_stats().sources);
    assert_eq!(all(&mut s, "projector"), 1);
    assert_eq!(all(&mut s, "is:unread from:zoe received:>=1970"), counted(&mut s, |m| !m.flags.seen && m.from.email.contains("zoe")));
    assert_eq!((s.store.as_ref().unwrap().reads(), s.search_stats().sources), (reads, sources), "no message read");
    done(s);
}

/// Marking a result read (opening it), unread or flagged weighs that message again, alone: the
/// search isn't run again and nothing is read for it. Mail moving has its two folders scanned
/// again, not the others.
#[test]
fn marking_a_result_read_weighs_it_alone() {
    let mut s = sample_session("search-mark-read");
    index_now(&s, sample::SAMPLE_ACCOUNT);
    s.run("search.run", &json!({"query": "storyboard", "scope": "all"})).unwrap();
    let found = s.list.len();
    assert!(found > 1);
    let before = s.search_stats();
    let unread = s.list.iter().find(|m| !m.flags.seen).map(|m| json!({"key": m.key.to_id()})).expect("an unread result");
    s.run("list.open", &unread).unwrap();
    assert!(s.reading.as_ref().is_some_and(|r| s.list.iter().any(|m| m.key == r.key && m.flags.seen)), "marked read");
    s.run("mail.markUnread", &json!({})).unwrap();
    s.run("mail.flag", &json!({})).unwrap();
    assert_eq!(s.list.len(), found, "the same results");
    let after = s.search_stats();
    assert_eq!(
        (after.summaries, after.reader_pages, after.sources, after.texts),
        (before.summaries, before.reader_pages, 0, 0),
        "nothing scanned or read again"
    );
    // Unread only: marked read, it goes from the results (the one being read stays).
    s.run("search.run", &json!({"query": "storyboard is:unread", "scope": "all"})).unwrap();
    let n = s.list.len();
    let k = s.list[0].key.clone();
    s.run("list.select", &json!({"key": k.to_id()})).unwrap();
    s.run("mail.markRead", &json!({})).unwrap();
    assert_eq!(s.list.len(), n - 1);
    assert!(!s.list.iter().any(|m| m.key == k));
    // Mail moving: the two folders scanned again, the others not. Deleted, it's still found,
    // in Deleted Items.
    s.run("search.run", &json!({"query": "storyboard", "scope": "all"})).unwrap();
    let k = s.list[0].key.clone();
    let from = s.folder_state(&k.account, &k.folder).messages.len();
    let trash = s.account(&k.account).and_then(|a| a.folder(FolderRole::Trash)).map(|f| f.path.clone()).unwrap();
    let to = s.folder_state(&k.account, &trash).messages.len();
    s.run("list.select", &json!({"key": k.to_id()})).unwrap();
    let before = s.search_stats().summaries;
    s.run("mail.delete", &json!({})).unwrap();
    assert_eq!(s.list.len(), found);
    assert!(s.search_stats().summaries - before <= (from + to + 1) as u64, "only its two folders");
    done(s);
}

/// Going to another folder and searching again folds no summary again: the texts are kept by
/// folder, whatever is on view. A folder read again with other mail has its own made again.
#[test]
fn another_view_keeps_the_summary_texts() {
    let mut s = sample_session("search-views");
    let a = sample::SAMPLE_ACCOUNT.to_string();
    assert_eq!(s.run("search.run", &json!({"query": "from:zoe", "scope": "mailbox"})).unwrap()["results"], 2);
    let made = s.search_stats().summary_texts;
    assert!(made > 0);
    s.open_view(View::Folder { account: a.clone(), folder: "Projects".into() });
    assert_eq!(s.run("search.run", &json!({"query": "from:zoe", "scope": "mailbox"})).unwrap()["results"], 2);
    s.open_view(View::Unified { role: FolderRole::Inbox });
    assert_eq!(s.run("search.run", &json!({"query": "subject:storyboard", "scope": "all"})).unwrap()["results"].as_u64().map(|n| n > 0), Some(true));
    assert_eq!(s.search_stats().summary_texts, made, "the folders' texts are kept whatever is on view");
    let store = s.store.clone().unwrap();
    let mut st = store.load_folder(&a, "INBOX").unwrap();
    st.messages.pop();
    store.save_folder(&a, "INBOX", &st).unwrap();
    s.apply(Event::FolderChanged { account: a.clone(), folder: "INBOX".into() });
    assert_eq!(s.run("search.run", &json!({"query": "from:zoe", "scope": "all"})).unwrap()["results"], 2);
    assert_eq!(s.search_stats().summary_texts, made + st.messages.len() as u64, "the Inbox's alone made again");
    done(s);
}

/// An account whose edits go to a server: marking a result read is recorded in the cache, and
/// the folder read again from it (`FolderChanged`): scanned again, the same results, nothing read.
#[test]
fn a_mark_read_sent_to_the_server_keeps_the_results() {
    let mut s = sample_session("search-remote-read");
    let a = sample::SAMPLE_ACCOUNT.to_string();
    index_now(&s, &a);
    s.run("search.run", &json!({"query": "storyboard", "scope": "all"})).unwrap();
    let found = s.list.len();
    let unread = s.list.iter().find(|m| !m.flags.seen).map(|m| m.key.clone()).expect("an unread result");
    let reads = s.store.as_ref().unwrap().reads();
    // What the worker does with the edit, then tells.
    let store = s.store.clone().unwrap();
    let op = mailcraft_model::Op::SetFlag { keys: vec![unread.clone()], flag: mailcraft_model::Flag::Seen, on: true };
    for folder in store.apply_op(&a, &op).unwrap() {
        s.apply(Event::FolderChanged { account: a.clone(), folder });
    }
    assert_eq!(s.list.len(), found);
    assert!(s.list.iter().any(|m| m.key == unread && m.flags.seen), "read");
    assert_eq!(store.reads(), reads, "nothing read");
    done(s);
}

/// A body downloaded after a search found nothing in it: once the index has it (a pass over,
/// `Event::Indexed`), the search shows it without being run again.
#[test]
fn a_body_indexed_later_joins_the_results() {
    let mut s = sample_session("search-ahead");
    let a = sample::SAMPLE_ACCOUNT.to_string();
    index_now(&s, &a);
    let store = s.store.clone().unwrap();
    let mut st = store.load_folder(&a, "INBOX").unwrap();
    let uid = st.messages.iter().map(|m| m.key.uid).max().unwrap_or(0) + 1;
    let key = mailcraft_model::MessageKey { account: a.clone(), folder: "INBOX".into(), validity: st.validity, uid };
    st.messages.insert(0, mailcraft_model::Summary { key: key.clone(), subject: "Headers only".into(), ..Default::default() });
    store.save_folder(&a, "INBOX", &st).unwrap();
    s.apply(Event::FolderChanged { account: a.clone(), folder: "INBOX".into() });
    assert_eq!(s.run("search.run", &json!({"query": "zeppelin", "scope": "all"})).unwrap()["results"], 0);
    store.write_body(&key, b"From: a@x.example\r\nSubject: Headers only\r\n\r\nThe zeppelin lands at noon.\r\n").unwrap();
    let generation = index_now(&s, &a);
    indexed(&mut s, &a, generation);
    assert_eq!(s.list.iter().map(|m| m.key.clone()).collect::<Vec<_>>(), [key], "found as the index has it");
    done(s);
}

/// A data file import ends folder after folder: a search open meanwhile scans each folder that
/// changes again, and what came in is found; what it read of other messages' text it doesn't
/// read again.
#[test]
fn a_search_during_an_import_follows_it() {
    let mut s = sample_session("search-import");
    let a = sample::SAMPLE_ACCOUNT.to_string();
    s.run("search.run", &json!({"query": "storyboard", "scope": "all"})).unwrap();
    let before = s.list.len();
    let sources = s.search_stats().sources;
    s.apply(Event::ImportProgress { account: a.clone(), progress: Default::default() });
    // A message comes into the Inbox as the import goes on.
    let store = s.store.clone().unwrap();
    let mut st = store.load_folder(&a, "INBOX").unwrap();
    let uid = st.messages.iter().map(|m| m.key.uid).max().unwrap_or(0) + 1;
    let key = mailcraft_model::MessageKey { account: a.clone(), folder: "INBOX".into(), validity: st.validity, uid };
    store.write_body(&key, b"From: a@x.example\r\nSubject: Imported\r\n\r\nThe storyboard, imported.\r\n").unwrap();
    st.messages.push(mailcraft_model::Summary { key, subject: "Imported".into(), ..Default::default() });
    store.save_folder(&a, "INBOX", &st).unwrap();
    let folders: Vec<String> = s.account(&a).unwrap().folders.iter().map(|f| f.path.clone()).collect();
    for n in 0..50 {
        let folder = folders[n % folders.len()].clone();
        s.apply(Event::FolderChanged { account: a.clone(), folder });
    }
    assert_eq!(s.list.len(), before + 1, "what came in is found");
    assert_eq!(s.search_stats().sources, sources + 1, "only what came in was read");
    // Nothing came in after all: the account goes, and the search finds nothing of it.
    s.apply(Event::Imported { account: a.clone(), messages: 0, skipped: 0, done: true, error: Some("no thread".into()), report: None });
    assert!(s.account(&a).is_none());
    assert_eq!(s.run("search.run", &json!({"query": "projector", "scope": "all"})).unwrap()["results"], 0);
    done(s);
}

/// Search events and queries that make no sense change nothing and break nothing.
#[test]
fn search_survives_junk_events_and_queries() {
    let mut s = sample_session("search-junk");
    for q in ["from:", "-", "\"", "is:", "larger:x", "from:,,", "-from:zoe -is:unread", "\"unterminated", "category:", "\u{0}\u{ffff}"] {
        let _ = s.run("search.run", &json!({"query": q, "scope": "all"}));
    }
    s.apply(Event::ImportProgress { account: "nobody".into(), progress: Default::default() });
    s.apply(Event::FolderChanged { account: "nobody".into(), folder: "INBOX".into() });
    s.apply(Event::FolderChanged { account: sample::SAMPLE_ACCOUNT.into(), folder: "No such folder".into() });
    s.apply(Event::Imported { account: "nobody".into(), messages: 1, skipped: 0, done: true, error: None, report: None });
    s.apply(Event::Imported { account: sample::SAMPLE_ACCOUNT.into(), messages: 0, skipped: 0, done: true, error: None, report: None });
    assert_eq!(s.run("search.run", &json!({"query": "projector", "scope": "all"})).unwrap()["results"], 1);
    done(s);
}

/// A list of file names (`attachment:a,b`) is one of them in the names: each message the index
/// offers is read for it, not taken as found (all of them matched, and `-attachment:a,b` none).
#[test]
fn a_list_of_file_names_reads_the_names() {
    let mut s = sample_session("names-list");
    let all = |s: &mut Session, q: &str| s.run("search.run", &json!({"query": q, "scope": "all"})).unwrap()["results"].as_u64().unwrap();
    let every = all(&mut s, "received:>=1970");
    assert!(every > 1);
    assert_eq!(all(&mut s, "attachment:order"), 1);
    assert_eq!(all(&mut s, "attachment:order,zzqqxx"), 1);
    assert_eq!(all(&mut s, "attachment:zzqqxx,yyqqww"), 0);
    assert_eq!(all(&mut s, "-attachment:zzqqxx,yyqqww"), every);
    assert_eq!(all(&mut s, "-attachment:order,zzqqxx"), every - 1);
    done(s);
}

/// The servers are asked for words, not dates: what they find outside a search's dates (or
/// other summary terms) isn't listed. The words they were asked are theirs to match: IMAP's FROM
/// and SUBJECT find them inside words, which the list's own search doesn't.
#[test]
fn server_finds_are_checked_against_the_dates() {
    use mailcraft_model::{EmailAddress, Folder, FolderRole, MessageKey, Summary};
    let mut s = sample_session("server-dates");
    let a = s
        .run("account.add", &json!({"email": "zoe@example.org", "password": "pw", "imapHost": "mail.example.org", "smtpHost": "mail.example.org"}))
        .unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    s.apply(Event::Folders {
        account: a.clone(),
        folders: vec![Folder { account: a.clone(), path: "INBOX".into(), role: FolderRole::Inbox, selectable: true, ..Default::default() }],
    });
    let store = s.store.clone().unwrap();
    let key = |uid| MessageKey { account: a.clone(), folder: "INBOX".into(), validity: 1, uid };
    let now = s.clock.now;
    let mut st = store.load_folder(&a, "INBOX").unwrap();
    st.validity = 1;
    st.messages = vec![
        Summary {
            key: key(7),
            subject: "Old figures".into(),
            from: EmailAddress::new("Joana Smith", "j.smith@corp.example"),
            date: 86_400,
            ..Default::default()
        },
        Summary { key: key(8), subject: "New one".into(), date: now, ..Default::default() },
    ];
    store.save_folder(&a, "INBOX", &st).unwrap();
    s.apply(Event::FolderChanged { account: a.clone(), folder: "INBOX".into() });
    let shown = |s: &Session| s.list.iter().map(|m| m.subject.clone()).collect::<Vec<_>>();
    for (q, keys, want) in [
        ("needle received:today", vec![7, 8], vec!["New one"]),
        ("needle -received:today", vec![7, 8], vec!["Old figures"]),
        // Found by the server inside a word: listed, as before the dates came in.
        ("from:oana", vec![7], vec!["Old figures"]),
        ("subject:ures -received:today", vec![7], vec!["Old figures"]),
        ("from:oana received:today", vec![7], vec![]),
        // What the server wasn't asked is still checked here.
        ("from:oana is:flagged", vec![7], vec![]),
    ] {
        s.run("search.run", &json!({"query": q, "scope": "all"})).unwrap();
        s.run("search.server", &json!({})).unwrap();
        s.take_jobs();
        let sent = s.server_search.as_ref().map(|x| x.sent.clone()).unwrap_or_default();
        assert_eq!(sent, q, "asked the server");
        s.apply(Event::ServerSearched { account: a.clone(), query: sent, keys: keys.into_iter().map(key).collect(), error: None, done: true });
        assert_eq!(shown(&s), want, "{q}");
    }
    done(s);
}

/// The search index kept up in the background: asked for as mail changes, once while jobs wait
/// to be taken (the folders adding up, every folder taking them all); nothing asked while it's
/// off (the CLI) or while a data file is imported; how a pass goes is kept.
#[test]
fn index_asks_are_one_while_they_wait() {
    let mut s = sample_session("index-asks");
    let a = sample::SAMPLE_ACCOUNT.to_string();
    let key = |folder: &str| mailcraft_model::MessageKey { account: a.clone(), folder: folder.into(), validity: 1, uid: 1 };
    s.apply(Event::FolderChanged { account: a.clone(), folder: "INBOX".into() });
    assert!(s.take_jobs().is_empty(), "off: nothing asked");

    s.set_indexing(true);
    assert_eq!(s.take_jobs(), [Job::Index { account: a.clone(), folders: Vec::new() }], "every account brought up");
    s.apply(Event::FolderChanged { account: a.clone(), folder: "INBOX".into() });
    // A local account's message is read from its data file: nothing new.
    s.apply(Event::Body { key: key("Sent Items"), raw: b"Subject: x\r\n\r\nx".to_vec() });
    s.apply(Event::FolderChanged { account: a.clone(), folder: "INBOX".into() });
    assert_eq!(s.take_jobs(), [Job::Index { account: a.clone(), folders: vec!["INBOX".into()] }]);
    // A local move asks for both folders (no server says they changed); a mark-read for none.
    s.run("folder.open", &json!({"account": a, "folder": "INBOX"})).unwrap();
    let k = key_of(&s, "Character turnarounds");
    s.run("list.select", &k).unwrap();
    s.run("mail.move", &json!({"folder": "Archive"})).unwrap();
    assert_eq!(s.take_jobs(), [Job::Index { account: a.clone(), folders: vec!["Archive".into(), "INBOX".into()] }]);
    s.run("mail.markRead", &json!({})).ok();
    assert!(s.take_jobs().is_empty(), "a flag changes nothing indexed");
    s.apply(Event::FolderChanged { account: a.clone(), folder: "INBOX".into() });
    let folders = s.account(&a).unwrap().folders.clone();
    s.apply(Event::Folders { account: a.clone(), folders });
    s.apply(Event::FolderChanged { account: a.clone(), folder: "Drafts".into() });
    assert_eq!(s.take_jobs(), [Job::Index { account: a.clone(), folders: Vec::new() }], "the folder list: every folder");

    let work = mailcraft_model::job::Work { done: 3, total: 9, ..Default::default() };
    s.apply(Event::Indexed { account: a.clone(), work, generation: 4, failed: 1, finished: false });
    assert_eq!(s.index_pass(&a), Some(crate::session::IndexPass { work, generation: 4, failed: 1, finished: false }));
    assert_eq!(s.index_pass("other"), None);
    done(s);
}

/// No index pass is asked for an account a data file is being imported into; its end asks for
/// every folder.
#[test]
fn an_import_asks_for_the_index_at_its_end() {
    let mut s = sample_session("index-import");
    s.set_indexing(true);
    s.take_jobs();
    s.import_pst("C:/Mail/old.pst", false).unwrap();
    let id = match s.take_jobs().as_slice() {
        [Job::ImportPst { account, .. }] => account.clone(),
        jobs => panic!("{jobs:?}"),
    };
    s.apply(Event::FolderChanged { account: id.clone(), folder: "INBOX".into() });
    assert!(s.take_jobs().is_empty(), "not while it imports");
    s.apply(Event::Imported { account: id.clone(), messages: 2, skipped: 0, done: true, error: None, report: None });
    assert!(s.take_jobs().contains(&Job::Index { account: id, folders: Vec::new() }));
    done(s);
}
