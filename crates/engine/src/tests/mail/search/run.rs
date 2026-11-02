//! Engine tests: a search run a step at a time, on its index (search.md §10, the engine's).

use super::*;
use std::collections::HashSet;

use crate::session::{SearchStats, SearchWork};

/// A mailbox of `folders` folders of `per` messages each (the first `big` more), in memory: every
/// message's text holds common words, every seventh "zanzibar" and every eleventh the phrase
/// "quarterly budget review". Indexed when `index`.
fn generated(folders: usize, per: u32, big: u32, index: bool) -> Session {
    let store = Store::memory();
    store.save_accounts(&[mailcraft_model::AccountConfig { id: "a1".into(), email: "me@studio.example".into(), ..Default::default() }]).unwrap();
    let list: Vec<mailcraft_model::Folder> = (0..folders)
        .map(|i| mailcraft_model::Folder {
            account: "a1".into(),
            path: if i == 0 { "INBOX".into() } else { format!("Folder {i}") },
            role: if i == 0 { FolderRole::Inbox } else { FolderRole::Other },
            selectable: true,
            ..Default::default()
        })
        .collect();
    store.save_folders("a1", &list).unwrap();
    let mut n = 0u32;
    for (i, f) in list.iter().enumerate() {
        let mut st = mailcraft_store::FolderState { validity: 1, ..Default::default() };
        let count = if i == 0 { per + big } else { per };
        let mut bodies = Vec::new();
        for uid in 1..=count {
            n += 1;
            let key = mailcraft_model::MessageKey { account: "a1".into(), folder: f.path.clone(), validity: 1, uid };
            let mut text = format!("Hello team, notes {n} on the common plan.");
            if n.is_multiple_of(7) {
                text.push_str(" Ship it to zanzibar.");
            }
            if n.is_multiple_of(11) {
                text.push_str(" The quarterly budget review is on Monday.");
            } else {
                text.push_str(" The budget is quarterly; a review is due.");
            }
            bodies.push((key.clone(), format!("From: a@x.example\r\nSubject: Notes {n}\r\n\r\n{text}\r\n").into_bytes()));
            st.messages.push(mailcraft_model::Summary {
                key,
                subject: format!("Notes {n}"),
                date: NOW - i64::from(n) * 60,
                size: 200,
                flags: mailcraft_model::Flags { seen: !n.is_multiple_of(3), ..Default::default() },
                ..Default::default()
            });
        }
        for (k, raw) in bodies {
            store.write_body(&k, &raw).unwrap();
        }
        mailcraft_model::sort_newest_first(&mut st.messages);
        store.save_folder("a1", &f.path, &st).unwrap();
    }
    let mut s = Session::open(Some(store), mailcraft_model::dates::Clock::new(NOW, 0));
    s.take_jobs();
    if index {
        index_now(&s, "a1");
    }
    s
}

/// Step the search until it's done: how many steps it took.
fn settle(s: &mut Session, units: u32) -> usize {
    let mut steps = 0;
    while !s.search_step(units).done {
        steps += 1;
        assert!(steps < 100_000, "a search that never ends");
    }
    steps
}

fn keys(s: &Session) -> std::collections::BTreeSet<mailcraft_model::MessageKey> {
    s.list.iter().map(|m| m.key.clone()).collect()
}

/// In the background, what's found streams into the list a step at a time: never fewer, the
/// list's revision moving with it, the selection kept; a new query meanwhile takes its place.
#[test]
fn results_stream_in_a_step_at_a_time() {
    let mut s = generated(6, 1500, 0, true);
    s.set_background_search(true);
    let r = s.run("search.run", &json!({"query": "zanzibar", "scope": "all"})).unwrap();
    assert_eq!(r["pending"], true, "not all of it in the first steps");
    let mut last = s.list.len();
    let mut chosen = None;
    let mut steps = 0;
    loop {
        let rev = s.revision;
        let p = s.search_step(1);
        steps += 1;
        assert!(s.list.len() >= last, "never fewer");
        assert_eq!(p.found, s.list.len());
        if s.list.len() != last {
            assert_ne!(s.revision, rev, "the list's revision moves with it");
            assert!(p.changed);
        }
        last = s.list.len();
        if chosen.is_none() && !s.list.is_empty() {
            let k = s.list[0].key.clone();
            s.run("list.select", &json!({"key": k.to_id()})).unwrap();
            chosen = Some(k);
        }
        if let Some(k) = &chosen {
            assert_eq!(s.selection, std::slice::from_ref(k), "the selection is kept");
        }
        if p.done {
            break;
        }
    }
    // (The list put together at the end takes one unit: what's found is in it already.)
    assert!(steps > 4, "{steps}");
    assert_eq!(s.list.len(), 9000 / 7);
    assert!(s.list.windows(2).all(|w| w[0].when() >= w[1].when()), "newest first");
    // A new query mid-run takes its place.
    s.run("search.run", &json!({"query": "zanzibar", "scope": "all"})).unwrap();
    s.search_step(3);
    s.run("search.run", &json!({"query": "subject:\"notes 42\"", "scope": "all"})).unwrap();
    settle(&mut s, 1);
    assert_eq!(s.list.len(), 1 + 10 + 100, "only the new query's: Notes 42, 420…, 4200…");
    assert!(s.list.iter().all(|m| m.subject.starts_with("Notes 42")));
    drop(s);
}

/// Each step is bounded however big the mailbox: `search_step(8)` reads at most 8 pages of
/// folders' lists, 16 of the index, checks at most 8 × 512 summaries and 8 texts, and reads no
/// message. Flags read nothing of the index at all.
#[test]
fn each_step_is_bounded() {
    let mut s = generated(4, 300, 5000, true);
    s.set_background_search(true);
    s.set_cache_limit(10);
    for q in ["zanzibar", "\"quarterly budget review\"", "budg -zanzibar", "is:unread"] {
        let reads = s.store.as_ref().unwrap().reads();
        s.start_search(q, SearchScope::All, false);
        let mut steps = 0;
        loop {
            let before = s.search_stats();
            let p = s.search_step(8);
            let d = |f: fn(&SearchStats) -> u64| f(&s.search_stats()) - f(&before);
            assert!(d(|x| x.folder_pages) <= 8, "{q}: folder pages");
            assert!(d(|x| x.reader_pages) <= 16, "{q}: index pages {}", d(|x| x.reader_pages));
            assert!(d(|x| x.summaries) <= 8 * 512, "{q}: summaries");
            assert!(d(|x| x.texts) <= 8, "{q}: texts");
            assert!(d(|x| x.units) <= 8, "{q}: units");
            steps += 1;
            if p.done {
                break;
            }
        }
        assert!(steps > 2, "{q}: {steps} steps");
        assert_eq!(s.store.as_ref().unwrap().reads(), reads, "{q}: no message read");
        assert_eq!(s.search_progress().map(|p| p.missing), Some(0), "{q}: everything indexed");
    }
    let n = s.search_progress().map(|p| p.found).unwrap();
    assert_eq!(n, (1..=6_200u32).filter(|n| n.is_multiple_of(3)).count(), "is:unread");
    // The phrase: only where its words are in that order.
    s.start_search("\"quarterly budget review\"", SearchScope::All, false);
    settle(&mut s, 8);
    assert_eq!(s.list.len(), (1..=6_200u32).filter(|n| n.is_multiple_of(11)).count());
    let before = s.search_stats().reader_pages;
    s.start_search("is:unread", SearchScope::All, false);
    settle(&mut s, 8);
    assert_eq!(s.search_stats().reader_pages, before, "flags read nothing of the index");
    drop(s);
}

/// An index still being built: what isn't in it yet may be missing (nothing is read for it);
/// once a pass is over, the search shows it without being run again.
#[test]
fn an_index_pass_adds_what_may_be_missing() {
    let mut s = generated(2, 200, 0, false);
    s.set_background_search(true);
    let reads = s.store.as_ref().unwrap().reads();
    s.run("search.run", &json!({"query": "zanzibar", "scope": "all"})).unwrap();
    settle(&mut s, 4);
    let p = s.search_progress().unwrap();
    assert_eq!((p.found, p.missing), (0, 400), "no index: every message may be missing");
    assert_eq!(s.store.as_ref().unwrap().reads(), reads, "and none was read");
    let generation = index_now(&s, "a1");
    indexed(&mut s, "a1", generation);
    settle(&mut s, 4);
    let p = s.search_progress().unwrap();
    assert_eq!((p.found, p.missing, s.list.len()), (400 / 7, 0, 400 / 7));
    drop(s);
}

/// A Search Folder opened on an account with no index yet reads no message: its summaries are
/// searched, the rest may be missing.
#[test]
fn a_search_folder_on_an_empty_index_reads_no_message() {
    let mut s = sample_session("search-folder-empty");
    s.set_background_search(true);
    let reads = s.store.as_ref().unwrap().reads();
    s.run("searchFolder.create", &json!({"name": "Projector", "query": "projector"})).unwrap();
    settle(&mut s, 4);
    s.run("searchFolder.open", &json!({"name": "Projector"})).unwrap();
    settle(&mut s, 4);
    assert_eq!(s.store.as_ref().unwrap().reads(), reads, "no message read");
    assert!(s.search_progress().is_some_and(|p| p.missing > 0));
    assert_eq!(s.search_stats().sources, 0);
    done(s);
}

/// Waiting with no index (each message read) finds what the index finds, query after query.
#[test]
fn waiting_without_an_index_finds_what_the_index_finds() {
    let queries = [
        "projector",
        "storyboard",
        "shot list",
        "\"shot list\"",
        "\"list shot\"",
        "-storyboard",
        "attachment:order",
        "-attachment:order",
        "from:zoe storyboard",
        "PRÓJECTOR",
        "turnaround -fox",
        "body:projector",
        "storyboard is:unread",
        "studio.example",
    ];
    let mut s = sample_session("search-same");
    let mut without = Vec::new();
    for q in queries {
        s.run("search.run", &json!({"query": q, "scope": "all"})).unwrap();
        without.push(keys(&s));
    }
    assert!(s.search_stats().sources > 0);
    index_now(&s, sample::SAMPLE_ACCOUNT);
    let sources = s.search_stats().sources;
    for (q, want) in queries.into_iter().zip(without) {
        s.run("search.run", &json!({"query": q, "scope": "all"})).unwrap();
        assert_eq!(keys(&s), want, "{q}");
    }
    assert_eq!(s.search_stats().sources, sources, "with the index, no message is read");
    done(s);
}

/// `has:attachment` counts files, not pictures the text shows: as the index read the message,
/// over what its summary says.
#[test]
fn a_picture_shown_in_the_text_is_no_attachment() {
    let mut s = sample_session("search-inline");
    let a = sample::SAMPLE_ACCOUNT.to_string();
    let store = s.store.clone().unwrap();
    let mut st = store.load_folder(&a, "INBOX").unwrap();
    let uid = st.messages.iter().map(|m| m.key.uid).max().unwrap_or(0) + 1;
    let key = mailcraft_model::MessageKey { account: a.clone(), folder: "INBOX".into(), validity: st.validity, uid };
    let raw = "From: a@x.example\r\nSubject: Logo inside\r\nContent-Type: multipart/related; boundary=b\r\n\r\n--b\r\nContent-Type: text/html\r\n\r\n<p>Our logo <img src=\"cid:logo@x\"></p>\r\n--b\r\nContent-Type: image/png\r\nContent-ID: <logo@x>\r\nContent-Disposition: inline; filename=logo.png\r\nContent-Transfer-Encoding: base64\r\n\r\niVBORw0KGgo=\r\n--b--\r\n";
    store.write_body(&key, raw.as_bytes()).unwrap();
    st.messages.insert(0, mailcraft_model::Summary { key: key.clone(), subject: "Logo inside".into(), has_attachments: true, ..Default::default() });
    store.save_folder(&a, "INBOX", &st).unwrap();
    s.apply(Event::FolderChanged { account: a.clone(), folder: "INBOX".into() });
    let found = |s: &mut Session| {
        s.run("search.run", &json!({"query": "has:attachment", "scope": "all"})).unwrap();
        s.list.iter().any(|m| m.key == key)
    };
    assert!(found(&mut s), "no index: as its summary says");
    index_now(&s, &a);
    assert!(!found(&mut s), "the index read it: no file");
    s.run("search.run", &json!({"query": "-has:attachment logo", "scope": "all"})).unwrap();
    assert!(s.list.iter().any(|m| m.key == key));
    done(s);
}

/// A summary that said a picture the text shows was a file (an old `.pst` import's) is put
/// right once a pass finds it: in the folder's list kept, and in what the window holds, so the
/// Has files filter, the paperclip and `has:attachment` agree; a list shown otherwise is patched
/// where it stands.
#[test]
fn the_has_files_filter_agrees_with_has_attachment() {
    let mut s = sample_session("search-files-known");
    let a = sample::SAMPLE_ACCOUNT.to_string();
    let store = s.store.clone().unwrap();
    let mut st = store.load_folder(&a, "INBOX").unwrap();
    let uid = st.messages.iter().map(|m| m.key.uid).max().unwrap_or(0) + 1;
    let key = mailcraft_model::MessageKey { account: a.clone(), folder: "INBOX".into(), validity: st.validity, uid };
    let raw = "From: a@x.example\r\nSubject: Logo inside\r\nContent-Type: multipart/related; boundary=b\r\n\r\n--b\r\nContent-Type: text/html\r\n\r\n<p>Our logo <img src=\"cid:logo@x\"></p>\r\n--b\r\nContent-Type: image/png\r\nContent-ID: <logo@x>\r\nContent-Disposition: inline; filename=logo.png\r\nContent-Transfer-Encoding: base64\r\n\r\niVBORw0KGgo=\r\n--b--\r\n";
    store.write_body(&key, raw.as_bytes()).unwrap();
    st.messages.insert(0, mailcraft_model::Summary { key: key.clone(), subject: "Logo inside".into(), has_attachments: true, ..Default::default() });
    store.save_folder(&a, "INBOX", &st).unwrap();
    s.apply(Event::FolderChanged { account: a.clone(), folder: "INBOX".into() });
    s.run("list.filter", &json!({"value": "attachments"})).unwrap();
    assert!(s.list.iter().any(|m| m.key == key), "as its summary says");
    // A pass, as the indexer runs it: what it found put right in the list kept, then told.
    let mut p = mailcraft_store::search::Pass::start(&store, &a, &[]).unwrap();
    let adds = p.adds().clone();
    for (i, m) in adds.iter().enumerate() {
        let doc = store.read_body(&m.key).map(|raw| mailcraft_mime::index_doc(&raw));
        p.add(&store, i, doc.as_ref()).unwrap();
    }
    let found = p.take_files();
    let inbox = found.get("INBOX").cloned().unwrap_or_default();
    assert_eq!(inbox.without, [uid], "{found:?}");
    assert_eq!(store.set_has_files(&a, "INBOX", &inbox).unwrap(), 1);
    p.finish(&store, true).unwrap();
    let generation = p.generation();
    drop(p);
    s.apply(Event::AttachmentsKnown { account: a.clone(), folder: "INBOX".into(), with: inbox.with, without: inbox.without });
    assert!(!s.list.iter().any(|m| m.key == key), "the filter reads it as the index did");
    assert!(!s.folder_state(&a, "INBOX").find(uid).unwrap().has_attachments);
    indexed(&mut s, &a, generation);
    let has: std::collections::HashSet<_> = s.list.iter().map(|m| m.key.clone()).collect();
    s.run("search.run", &json!({"query": "has:attachment", "scope": "current"})).unwrap();
    let searched: std::collections::HashSet<_> = s.list.iter().map(|m| m.key.clone()).collect();
    assert_eq!(has, searched, "Has files and has:attachment find the same mail");
    s.run("search.clear", &json!({})).unwrap();
    // Shown unfiltered: its row patched where it stands.
    s.run("list.filter", &json!({"value": "all"})).unwrap();
    let revision = s.revision;
    s.apply(Event::AttachmentsKnown { account: a.clone(), folder: "INBOX".into(), with: vec![uid], without: Vec::new() });
    assert!(s.summary(&key).unwrap().has_attachments && s.revision != revision);
    done(s);
}

/// A word past the first 50,000 characters of a message is found, from the index and from the
/// source; a phrase across the slices a long text is checked in is found too, the text checked
/// over several units.
#[test]
fn long_texts_are_searched_whole() {
    let mut s = sample_session("search-long");
    let a = sample::SAMPLE_ACCOUNT.to_string();
    let store = s.store.clone().unwrap();
    let mut st = store.load_folder(&a, "INBOX").unwrap();
    let uid = st.messages.iter().map(|m| m.key.uid).max().unwrap_or(0) + 1;
    let key = mailcraft_model::MessageKey { account: a.clone(), folder: "INBOX".into(), validity: st.validity, uid };
    // "needle haystack" across the first slice's end (256 KiB of text).
    let mut text = "lorem ".repeat(262_140 / 6);
    text.push_str("needle haystack ");
    text.push_str(&"ipsum ".repeat(20_000));
    text.push_str("zanzibar");
    store.write_body(&key, format!("From: a@x.example\r\nSubject: Long\r\n\r\n{text}\r\n").as_bytes()).unwrap();
    st.messages.insert(0, mailcraft_model::Summary { key: key.clone(), subject: "Long".into(), ..Default::default() });
    store.save_folder(&a, "INBOX", &st).unwrap();
    s.apply(Event::FolderChanged { account: a.clone(), folder: "INBOX".into() });
    let hits = |s: &mut Session, q: &str| {
        s.run("search.run", &json!({"query": q, "scope": "all"})).unwrap();
        s.list.iter().any(|m| m.key == key)
    };
    assert!(hits(&mut s, "zanzibar"), "read from its source");
    index_now(&s, &a);
    assert!(hits(&mut s, "zanzibar"), "from the index");
    let texts = s.search_stats().texts;
    assert!(hits(&mut s, "\"needle haystack\""));
    assert!(s.search_stats().texts - texts > 2, "checked a slice a unit");
    assert!(!hits(&mut s, "\"haystack needle\""));
    done(s);
}

/// What the index settles isn't checked again, nor told as waiting for it: a message of
/// pictures only has no word, so `-word` finds it (658 of the 41 GB file's weren't found); one
/// cut short (past 1,000,000 characters) may hold a word past its cut, which nothing kept can
/// tell, so it's counted apart, not as "still indexing", and its kept text isn't read for a word.
/// The part of a dotted word (`xx.quokka`) is a word. A phrase its kept text was read for and
/// didn't have stays cut short when the message is weighed again (a flag on it, its folder
/// scanned again): it went back to "still indexing".
#[test]
fn what_the_index_settles_is_not_checked_again() {
    let mut s = sample_session("search-settled");
    let a = sample::SAMPLE_ACCOUNT.to_string();
    let store = s.store.clone().unwrap();
    let mut st = store.load_folder(&a, "INBOX").unwrap();
    let uid = st.messages.iter().map(|m| m.key.uid).max().unwrap_or(0) + 1;
    let key = |uid: u32| mailcraft_model::MessageKey { account: a.clone(), folder: "INBOX".into(), validity: st.validity, uid };
    let (pictures, long, dotted) = (key(uid), key(uid + 1), key(uid + 2));
    let raw = "From: a@x.example\r\nSubject: Pictures\r\nContent-Type: multipart/related; boundary=b\r\n\r\n--b\r\nContent-Type: text/html\r\n\r\n<p><img src=\"cid:logo@x\"></p>\r\n--b\r\nContent-Type: image/png\r\nContent-ID: <logo@x>\r\nContent-Disposition: inline; filename=logo.png\r\nContent-Transfer-Encoding: base64\r\n\r\niVBORw0KGgo=\r\n--b--\r\n";
    store.write_body(&pictures, raw.as_bytes()).unwrap();
    let text = format!("alpha beta {}zanzibar", "lorem ".repeat(200_000));
    store.write_body(&long, format!("From: a@x.example\r\nSubject: Long\r\n\r\n{text}\r\n").as_bytes()).unwrap();
    store.write_body(&dotted, b"From: a@x.example\r\nSubject: Dotted\r\n\r\nSee xx.quokka here.\r\n").unwrap();
    for (k, subject) in [(&pictures, "Pictures"), (&long, "Long"), (&dotted, "Dotted")] {
        st.messages.insert(0, mailcraft_model::Summary { key: k.clone(), subject: subject.into(), ..Default::default() });
    }
    store.save_folder(&a, "INBOX", &st).unwrap();
    s.apply(Event::FolderChanged { account: a.clone(), folder: "INBOX".into() });
    index_now(&s, &a);
    let search = |s: &mut Session, q: &str| {
        s.run("search.run", &json!({"query": q, "scope": "all"})).unwrap();
        let p = s.search_progress().unwrap();
        let listed = |k: &mailcraft_model::MessageKey| s.list.iter().any(|m| &m.key == k);
        ((listed(&pictures), listed(&long), listed(&dotted)), (p.missing, p.cut_short))
    };
    assert_eq!(search(&mut s, "-quokka"), ((true, false, false), (0, 1)), "no text: no word; the long one may have it");
    assert_eq!(search(&mut s, "quokka"), ((false, false, true), (0, 1)));
    let texts = s.search_stats().texts;
    assert_eq!(search(&mut s, "zanzibar"), ((false, false, false), (0, 1)), "past the cut");
    assert_eq!(s.search_stats().texts, texts, "no kept text read for a word");
    assert_eq!(search(&mut s, "subject:pictures"), ((true, false, false), (0, 0)));
    let texts = s.search_stats().texts;
    assert_eq!(search(&mut s, "\"beta alpha\""), ((false, false, false), (0, 1)));
    assert!(s.search_stats().texts > texts, "its kept text read for the phrase");
    let progress = |s: &Session| s.search_progress().map(|p| (p.missing, p.cut_short));
    s.search_patched(std::slice::from_ref(&long));
    settle(&mut s, 1);
    assert_eq!(progress(&s), Some((0, 1)), "after a flag on it");
    s.search_folder_changed(&a, "INBOX");
    settle(&mut s, 1);
    assert_eq!(progress(&s), Some((0, 1)), "after its folder is scanned again");
    done(s);
}

/// With an index, a word that must be missing, an address and a word in the files' names read
/// no message.
#[test]
fn negated_words_read_no_message_with_an_index() {
    let mut s = sample_session("search-negated");
    index_now(&s, sample::SAMPLE_ACCOUNT);
    let reads = s.store.as_ref().unwrap().reads();
    for q in ["-storyboard", "zoe@studio.example", "attachment:order", "-projector is:unread"] {
        s.run("search.run", &json!({"query": q, "scope": "all"})).unwrap();
    }
    assert_eq!(s.store.as_ref().unwrap().reads(), reads);
    assert_eq!(s.search_stats().sources, 0);
    done(s);
}

/// A message marked read at any point of a search, the list being put together at its end
/// included, shows read once the search is done: what was gathered before isn't shown over it.
#[test]
fn a_flag_changed_as_a_search_ends_stays_changed() {
    let mut s = generated(30, 20, 0, true);
    s.set_background_search(true);
    // Notes 21: unread, with "zanzibar".
    let key = mailcraft_model::MessageKey { account: "a1".into(), folder: "Folder 1".into(), validity: 1, uid: 1 };
    let seen = |s: &Session| s.list.iter().find(|m| m.key == key).map(|m| m.flags.seen);
    let mut marked = 0;
    for n in 0.. {
        let r = s.run("search.run", &json!({"query": "zanzibar", "scope": "all"})).unwrap();
        assert_eq!(r["pending"], true, "not all of it in the first steps");
        let mut finished = false;
        for _ in 0..n {
            finished = s.search_step(1).done;
        }
        if finished {
            break;
        }
        if seen(&s) != Some(false) {
            continue;
        }
        s.run("list.select", &json!({"key": key.to_id()})).unwrap();
        s.run("mail.markRead", &json!({})).unwrap();
        settle(&mut s, 1);
        assert_eq!(seen(&s), Some(true), "marked read after {n} steps");
        s.run("mail.markUnread", &json!({})).unwrap();
        assert_eq!(seen(&s), Some(false));
        marked += 1;
    }
    assert!(marked > 2, "{marked}");
    drop(s);
}

/// The folders a search reads stay in the cache while it runs, not once it's done; scanning
/// one again keeps them again.
#[test]
fn a_done_search_lets_its_folders_go() {
    let mut s = generated(3, 40, 0, true);
    s.set_background_search(true);
    s.start_search("zanzibar", SearchScope::All, false);
    assert_eq!(s.search_pinned(), 3, "kept while it runs");
    settle(&mut s, 1);
    assert!(s.search_pinned() == 0, "let go once it's done");
    s.search_folder_changed("a1", "Folder 1");
    assert!(!s.search_step(1).done);
    assert_eq!(s.search_pinned(), 3, "kept again while it scans again");
    settle(&mut s, 1);
    assert!(s.search_pinned() == 0);
    drop(s);
}

/// The window stops a step when its time is up: asked after each unit, it goes no further, and
/// what was found by then is in the list.
#[test]
fn a_step_stops_when_asked() {
    let mut s = generated(4, 600, 0, true);
    s.set_background_search(true);
    s.start_search("zanzibar", SearchScope::All, false);
    let units = s.search_stats().units;
    let mut asked = 0;
    let p = s.search_step_while(64, &mut || {
        asked += 1;
        false
    });
    assert_eq!((asked, s.search_stats().units - units), (1, 1), "one unit, then asked and stopped");
    assert!(!p.done);
    let mut steps = 0;
    while !s.search_step_while(64, &mut || true).done {
        steps += 1;
        assert!(steps < 1000);
    }
    assert_eq!(s.list.len(), 2400 / 7);
    drop(s);
}

/// A search that stops part-way (a bug) goes, and the indexes it was reading are left unread,
/// so the next search can't stop on them again: what they would find may be missing, until
/// their writer writes them again.
#[test]
fn a_search_that_fails_goes_and_leaves_its_index_alone() {
    let mut s = generated(2, 300, 0, true);
    s.set_background_search(true);
    s.start_search("zanzibar", SearchScope::All, false);
    s.search_step(2);
    let caught = crate::PANICS_CAUGHT.load(std::sync::atomic::Ordering::Relaxed);
    s.fail_next_search_step();
    assert!(s.search_step(8).done, "it's over");
    assert!(crate::PANICS_CAUGHT.load(std::sync::atomic::Ordering::Relaxed) > caught);
    assert_eq!(s.search_progress(), None, "the search went");
    s.start_search("zanzibar", SearchScope::All, false);
    settle(&mut s, 8);
    let p = s.search_progress().unwrap();
    assert_eq!((p.found, p.missing), (0, 600), "the index isn't read");
    // Written again: read again, and the search running finds what it has.
    let generation = index_now(&s, "a1");
    indexed(&mut s, "a1", generation + 1);
    settle(&mut s, 8);
    let p = s.search_progress().unwrap();
    assert_eq!((p.found, p.missing), (600 / 7, 0));
    drop(s);
}

/// The index passes under way, all told, for the status bar and the "may be missing" banner:
/// none once each is over; a date that isn't one is a problem the search says.
#[test]
fn indexing_under_way_and_what_a_search_couldnt_read() {
    use mailcraft_model::job::Work;
    let mut s = generated(1, 10, 0, false);
    assert_eq!(s.indexing_work(), None);
    let pass = |done, total, finished| Event::Indexed {
        account: "a1".into(),
        work: Work { done, total, ..Work::default() },
        generation: 1,
        failed: 0,
        finished,
    };
    s.apply(pass(3, 10, false));
    assert_eq!(s.indexing_work().map(|w| (w.done, w.total)), Some((3, 10)));
    s.apply(Event::Indexed {
        account: "gone".into(),
        work: Work { done: 1, total: 5, ..Work::default() },
        generation: 1,
        failed: 0,
        finished: false,
    });
    assert_eq!(s.indexing_work().map(|w| (w.done, w.total)), Some((3, 10)), "no account, no pass");
    s.apply(pass(10, 10, true));
    assert_eq!(s.indexing_work(), None);
    s.start_search("received:2024-13 notes after:soon", SearchScope::All, false);
    use mailcraft_model::search::Problem;
    assert_eq!(s.search_problems(), [Problem::Date("received:2024-13".into()), Problem::Date("after:soon".into())]);
    s.start_search("notes", SearchScope::All, false);
    assert!(s.search_problems().is_empty());
    drop(s);
}

/// A step tells what each part of it was (the window times them by kind), and a folder's list
/// isn't read while read marks of it wait to be written: the step goes on and reads it once they
/// are, rather than wait for them on the window's thread. The other folders' lists are read
/// meanwhile.
#[test]
fn a_step_tells_its_work_and_never_waits_for_marks() {
    let mut s = generated(4, 600, 0, true);
    s.set_background_search(true);
    s.run("folder.open", &json!({"account": "a1", "folder": "INBOX"})).unwrap();
    let store = s.store.clone().unwrap();
    let k = s.list[0].key.clone();
    let paused = store.pause_marks();
    s.apply_op("a1", Op::SetFlag { keys: vec![k], flag: Flag::Flagged, on: true });
    assert!(store.marks_waiting());
    // No folder's list in memory: a search reads them.
    s.cache.clear();
    let began = std::time::Instant::now();
    s.start_search("zanzibar", SearchScope::All, false);
    let mut kinds = HashSet::new();
    for _ in 0..50 {
        s.search_step_timed(64, &mut || true, &mut |w| {
            kinds.insert(w);
        });
    }
    assert!(began.elapsed() < std::time::Duration::from_secs(5), "waited {:?}", began.elapsed());
    assert!(!s.search_progress().unwrap().done, "the Inbox not read yet");
    let read: HashSet<&str> = s.cache.keys().map(|(_, f)| f.as_str()).collect();
    assert!(read.len() >= 3 && !read.contains("INBOX"), "the others read: {read:?}");
    drop(paused);
    let mut steps = 0;
    while !s
        .search_step_timed(64, &mut || true, &mut |w| {
            kinds.insert(w);
        })
        .done
    {
        steps += 1;
        assert!(steps < 10_000);
    }
    assert_eq!(s.list.len(), 2400 / 7);
    for w in [SearchWork::FollowIndexes, SearchWork::Show, SearchWork::ListPage, SearchWork::LookUp, SearchWork::Summaries, SearchWork::Finish] {
        assert!(kinds.contains(&w), "{w:?} in {kinds:?}");
    }
}
