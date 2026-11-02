//! Engine tests: a read mark or category changed on a server account's mail, said back by the
//! pool once the cache on disk has it (`FolderChanged`), keeps the folder in memory and the list;
//! so does a message downloaded, its summary filled from it (`Filled`).

use mailcraft_model::{Folder, MessageKey, Summary};
use mailcraft_store::FolderState;

use super::*;

/// A session over a server account (an IMAP one, as added) whose Inbox of `n` unread messages is
/// on disk and shown.
fn server_inbox(n: u32) -> (Session, String) {
    let mut s = Session::open(Some(Store::memory()), mailcraft_model::dates::Clock::new(NOW, 0));
    let id = s.run("account.add", &json!({"email": "zoe@example.org", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    assert!(!s.account(&id).unwrap().is_local());
    let messages = (1..=n)
        .map(|uid| Summary {
            key: MessageKey { account: id.clone(), folder: "INBOX".into(), validity: 1, uid },
            date: NOW - i64::from(uid) * 37,
            message_id: format!("<m{uid}@x>"),
            subject: format!("Plan {}", uid / 3),
            ..Default::default()
        })
        .map(Into::into)
        .collect();
    let store = s.store.clone().unwrap();
    store.save_folder(&id, "INBOX", &FolderState { validity: 1, uid_next: n + 1, messages, ..Default::default() }).unwrap();
    let inbox = Folder { account: id.clone(), path: "INBOX".into(), role: FolderRole::Inbox, selectable: true, ..Default::default() };
    s.apply(Event::Folders { account: id.clone(), folders: vec![inbox] });
    s.open_view(View::Folder { account: id.clone(), folder: "INBOX".into() });
    // Read from disk (the account, added, showed it empty).
    inbox_changed(&mut s, &id);
    s.take_jobs();
    (s, id)
}

/// The changes the session sent, made in the cache on disk as the pool makes them; not yet said.
fn made_on_disk(s: &mut Session) -> Vec<Op> {
    let store = s.store.clone().unwrap();
    s.take_jobs()
        .into_iter()
        .filter_map(|j| match j {
            Job::Apply { account, op } => {
                store.apply_op(&account, &op).unwrap();
                Some(op)
            }
            _ => None,
        })
        .collect()
}

fn inbox_changed(s: &mut Session, id: &str) {
    s.apply(Event::FolderChanged { account: id.to_string(), folder: "INBOX".into() });
}

/// Down after Down on a big server folder: each read mark's echo keeps the folder in memory and
/// the list (each read all of the folder from disk again and built the list again).
#[test]
fn a_read_marks_echo_keeps_the_folder_and_the_list() {
    let (mut s, id) = server_inbox(10_941);
    let keys: Vec<MessageKey> = s.row_keys().into_iter().take(50).collect();
    let (built, threadings) = (s.nav_structure(), s.nav.threadings());
    let started = std::time::Instant::now();
    for k in &keys {
        s.open_message(k);
        assert_eq!(made_on_disk(&mut s).len(), 1);
        inbox_changed(&mut s, &id);
    }
    eprintln!("50 opened, marked read and said back in {:?}", started.elapsed());
    assert!(keys.iter().all(|k| s.summary(k).is_some_and(|m| m.flags.seen)), "read");
    assert_eq!(s.nav_structure(), built, "no rows built");
    assert_eq!(s.nav.threadings(), threadings, "no conversations worked out");
    assert_eq!(s.echoes.waiting(&id), 0);
    // What memory holds is what disk holds.
    let shown = s.list.clone();
    s.cache.remove(&(id.clone(), "INBOX".to_string()));
    s.refresh_list();
    assert_eq!(s.list, shown);
}

/// What a Down step costs the window on a big server folder (`--release -- --ignored
/// --nocapture`): the step, its read mark and the mark's echo; the pool's write to disk is
/// timed apart, as the window made it before, and so is the echo read as any other change (what a
/// body downloaded on the way cost before it was said as its summary filled, `Filled`), and the
/// summary filled taken in.
#[test]
#[ignore]
fn down_steps_on_a_big_server_folder() {
    use std::time::{Duration, Instant};
    let (mut s, id) = server_inbox(10_941);
    s.defer_bodies(true);
    let store = s.store.clone().unwrap();
    let first = s.row_keys()[0].to_id();
    s.run("list.open", &json!({"key": first})).unwrap();
    made_on_disk(&mut s);
    inbox_changed(&mut s, &id);
    let (mut window, mut pool, mut reread, mut filled) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for _ in 0..100 {
        let t = Instant::now();
        s.run("list.next", &json!({})).unwrap();
        drop(s.take_body_work());
        let jobs = s.take_jobs();
        let stepped = t.elapsed();
        let t = Instant::now();
        for j in jobs {
            if let Job::Apply { account, op } = j {
                store.apply_op(&account, &op).unwrap();
                store.add_op(&account, op).unwrap();
            }
        }
        pool.push(t.elapsed());
        let t = Instant::now();
        inbox_changed(&mut s, &id);
        window.push(stepped + t.elapsed());
        // The same change said by a sync: the folder read from disk and the list built again.
        let t = Instant::now();
        inbox_changed(&mut s, &id);
        reread.push(t.elapsed());
        // Its body downloaded: its summary filled, said to the window.
        let Some(open) = s.reading.as_ref().map(|r| r.key.clone()) else { continue };
        let Some(m) = downloaded(&s, &open) else { continue };
        let t = Instant::now();
        s.apply(Event::Filled { summary: Box::new(m) });
        filled.push(t.elapsed());
    }
    let median = |mut v: Vec<Duration>| {
        v.sort();
        (v[v.len() / 2], v[v.len() - 1])
    };
    let ((w, wmax), (p, pmax), (r, rmax), (f, fmax)) = (median(window), median(pool), median(reread), median(filled));
    println!("Down step, the window: median {w:?}, max {wmax:?}");
    println!("the read mark on disk (the pool's thread): median {p:?}, max {pmax:?}");
    println!("the folder read again and the list built (an echo, or a body downloaded, before): median {r:?}, max {rmax:?}");
    println!("a body downloaded, its summary filled taken in: median {f:?}, max {fmax:?}");
}

/// `key`'s summary as the pool says it once its source is downloaded: filled from it (a preview,
/// a file) and saved in the folder on disk, as `keep_source` does.
fn downloaded(s: &Session, key: &MessageKey) -> Option<Summary> {
    let store = s.store.clone()?;
    let mut st = store.load_folder(&key.account, &key.folder).ok()?;
    let m = st.find_mut(key.uid)?;
    m.preview = format!("First take of the mix, number {}", key.uid);
    m.has_attachments = true;
    let filled = m.clone();
    store.save_folder(&key.account, &key.folder, &st).ok()?;
    Some(filled)
}

/// A message downloaded on a big server folder: its summary filled from it comes to the window
/// (`Filled`), which puts it in the folder it holds and in the list's entry, keeping the read
/// mark it made meanwhile; no rows built, no conversations worked out, nothing read from disk.
/// What memory holds is then what disk holds. It read all of the folder again and built the list
/// (24 ms on 10,941 messages).
#[test]
fn a_message_downloaded_fills_its_entry_without_reading_the_folder() {
    let (mut s, id) = server_inbox(10_941);
    let key = s.row_keys()[3].clone();
    let filled = downloaded(&s, &key).unwrap();
    s.open_message(&key);
    assert_eq!(made_on_disk(&mut s).len(), 1);
    inbox_changed(&mut s, &id);
    let (built, threadings) = (s.nav_structure(), s.nav.threadings());
    // A change on disk the window doesn't know of: read again, the folder would show it.
    let store = s.store.clone().unwrap();
    let other = s.row_keys()[7].clone();
    let mut st = store.load_folder(&id, "INBOX").unwrap();
    let was = st.find(other.uid).unwrap().subject.clone();
    st.find_mut(other.uid).unwrap().subject = "Changed on disk only".into();
    store.save_folder(&id, "INBOX", &st).unwrap();
    s.apply(Event::Filled { summary: Box::new(filled) });
    assert_eq!(s.summary(&other).unwrap().subject, was, "the folder not read again");
    st.find_mut(other.uid).unwrap().subject = was;
    st.find_mut(key.uid).unwrap().flags.seen = true;
    store.save_folder(&id, "INBOX", &st).unwrap();
    let m = s.summary(&key).unwrap();
    assert!(m.preview.starts_with("First take") && m.has_attachments && m.flags.seen, "{m:?}");
    assert!(s.list.iter().any(|x| x.key == key && x.has_attachments && x.flags.seen), "the list's entry too");
    assert_eq!(s.nav_structure(), built, "no rows built");
    assert_eq!(s.nav.threadings(), threadings, "no conversations worked out");
    let shown = s.list.clone();
    s.cache.remove(&(id.clone(), "INBOX".to_string()));
    s.refresh_list();
    assert_eq!(s.list, shown, "memory as disk has it");
}

/// A message downloaded again while no search shows: its folder's texts, kept from a search
/// before, take its new preview, and the next search goes by that one. They kept the old text (a
/// summary changed in place keeps its uid at its place): a word the preview no longer had was
/// still found in it, one only the new preview had wasn't, until the folder was read again.
#[test]
fn a_message_filled_between_searches_is_searched_as_filled() {
    let (mut s, _id) = server_inbox(40);
    let key = s.row_keys()[5].clone();
    let found = |s: &mut Session, q: &str| s.run("search.run", &json!({"query": q})).unwrap()["results"].as_u64().unwrap();
    let mut filled = downloaded(&s, &key).unwrap();
    s.apply(Event::Filled { summary: Box::new(filled.clone()) });
    assert_eq!(found(&mut s, "mix"), 1);
    s.run("search.clear", &json!({})).unwrap();
    assert!(s.search.is_none());
    filled.preview = "Second pass of the stems".into();
    s.apply(Event::Filled { summary: Box::new(filled) });
    assert_eq!(found(&mut s, "mix"), 0, "the old preview not searched");
    s.run("search.clear", &json!({})).unwrap();
    assert_eq!(found(&mut s, "stems"), 1, "the new one");
    assert_eq!(found(&mut s, "plan"), 40);
}

/// A message a filter hid (Has attachments) whose summary filled now shows a file: the list is
/// built again, the message in it. It was looked for in the list first, and not being there, left
/// out until something else built the list.
#[test]
fn a_message_filled_with_a_file_joins_the_attachments_filter() {
    let (mut s, _) = server_inbox(30);
    let key = s.row_keys()[4].clone();
    s.run("list.filter", &json!({"value": "attachments"})).unwrap();
    assert!(s.list.is_empty());
    let filled = downloaded(&s, &key).unwrap();
    s.apply(Event::Filled { summary: Box::new(filled) });
    assert_eq!(s.list.iter().map(|m| m.key.clone()).collect::<Vec<_>>(), vec![key]);
}

/// A summary filled that joins its message to a conversation (its references come with its
/// source): the list is built again with conversations on, so the message shows in its thread.
#[test]
fn a_message_filled_into_a_conversation_builds_the_list() {
    let (mut s, id) = server_inbox(30);
    s.run("view.conversations", &json!({"on": true})).unwrap();
    let keys = s.row_keys();
    let (parent, child) = (keys[0].clone(), keys[5].clone());
    let parent_id = s.summary(&parent).unwrap().message_id.clone();
    let store = s.store.clone().unwrap();
    let mut st = store.load_folder(&id, "INBOX").unwrap();
    let m = st.find_mut(child.uid).unwrap();
    m.subject = s.summary(&parent).unwrap().subject.clone();
    m.in_reply_to = parent_id.clone();
    m.references = vec![parent_id];
    let filled = m.clone();
    store.save_folder(&id, "INBOX", &st).unwrap();
    let rows = s.nav.rows.len();
    s.apply(Event::Filled { summary: Box::new(filled) });
    assert_eq!(s.nav.rows.len(), rows - 1, "one conversation where there were two rows");
}

/// A sync changing the folder on disk while read marks wait for their echo: one more change is
/// said than waited for, and the last is read from disk, the new mail with it.
#[test]
fn a_change_from_the_server_among_echoes_is_read() {
    let (mut s, id) = server_inbox(30);
    let keys: Vec<MessageKey> = s.row_keys().into_iter().take(2).collect();
    s.open_message(&keys[0]);
    s.open_message(&keys[1]);
    // New mail synced, and said, before the pool makes the read marks.
    let store = s.store.clone().unwrap();
    let mut st = store.load_folder(&id, "INBOX").unwrap();
    let new = MessageKey { account: id.clone(), folder: "INBOX".into(), validity: 1, uid: 31 };
    st.messages.insert(0, Summary { key: new.clone(), date: NOW, subject: "Arrived".into(), ..Default::default() }.into());
    store.save_folder(&id, "INBOX", &st).unwrap();
    inbox_changed(&mut s, &id);
    assert_eq!(made_on_disk(&mut s).len(), 2);
    inbox_changed(&mut s, &id);
    assert!(s.index_of(&new).is_none(), "one echo still to come");
    inbox_changed(&mut s, &id);
    assert!(s.index_of(&new).is_some(), "read from disk");
    assert!(keys.iter().all(|k| s.summary(k).is_some_and(|m| m.flags.seen)));
}

/// The folder read from disk again before the pool made the change (let go of meanwhile): its
/// echo puts the change in it, as the list has it, without reading it again.
#[test]
fn an_echo_puts_the_change_in_a_folder_read_before_it() {
    let (mut s, id) = server_inbox(300);
    let k = s.row_keys()[3].clone();
    s.apply_op(&id, Op::SetFlag { keys: vec![k.clone()], flag: Flag::Flagged, on: true });
    assert!(s.summary(&k).unwrap().flags.flagged);
    // Read from disk again, where it isn't flagged yet.
    s.cache.remove(&(id.clone(), "INBOX".to_string()));
    s.refresh_list();
    assert!(!s.summary(&k).unwrap().flags.flagged);
    let (built, revision) = (s.nav_structure(), s.revision);
    made_on_disk(&mut s);
    inbox_changed(&mut s, &id);
    assert!(s.summary(&k).unwrap().flags.flagged, "the change, in memory");
    assert_eq!(s.nav_structure(), built, "patched, not built again");
    assert_ne!(s.revision, revision);
}

/// Two changes waiting and the folder read from disk again before the pool made either: each
/// echo puts its own change in (only the last one did, and the first was lost from memory).
#[test]
fn every_waiting_change_gets_into_a_folder_read_before_them() {
    let (mut s, id) = server_inbox(300);
    let rows = s.row_keys();
    let (a, b) = (rows[3].clone(), rows[5].clone());
    s.apply_op(&id, Op::SetFlag { keys: vec![a.clone()], flag: Flag::Flagged, on: true });
    s.apply_op(&id, Op::SetFlag { keys: vec![b.clone()], flag: Flag::Flagged, on: true });
    s.cache.remove(&(id.clone(), "INBOX".to_string()));
    s.refresh_list();
    let built = s.nav_structure();
    assert_eq!(made_on_disk(&mut s).len(), 2);
    inbox_changed(&mut s, &id);
    assert!(s.summary(&a).unwrap().flags.flagged, "the first, as its echo comes");
    inbox_changed(&mut s, &id);
    assert!(s.summary(&a).unwrap().flags.flagged && s.summary(&b).unwrap().flags.flagged);
    assert_eq!(s.nav_structure(), built, "patched, not built again");
    assert_eq!(s.echoes.waiting(&id), 0);
}

/// A message flagged, the folder read again, then unflagged: the first echo doesn't flag it
/// again, as the change after it says how it ends.
#[test]
fn a_later_waiting_change_says_how_it_ends() {
    let (mut s, id) = server_inbox(30);
    let k = s.row_keys()[2].clone();
    s.apply_op(&id, Op::SetFlag { keys: vec![k.clone()], flag: Flag::Flagged, on: true });
    s.cache.remove(&(id.clone(), "INBOX".to_string()));
    s.refresh_list();
    s.apply_op(&id, Op::SetFlag { keys: vec![k.clone()], flag: Flag::Flagged, on: false });
    made_on_disk(&mut s);
    inbox_changed(&mut s, &id);
    assert!(!s.summary(&k).unwrap().flags.flagged, "not flagged again on the way");
    inbox_changed(&mut s, &id);
    assert!(!s.summary(&k).unwrap().flags.flagged);
    let store = s.store.clone().unwrap();
    assert!(!store.load_folder(&id, "INBOX").unwrap().messages.iter().any(|m| m.key == k && m.flags.flagged), "as disk has it");
}

/// A change the cache on disk couldn't take comes with a notice: the account's echoes are let go
/// of, and what it says changed is read from disk (the read mark there isn't). So is an echo
/// waited for too long (a front end that never says).
#[test]
fn a_notice_or_a_long_wait_lets_go_of_echoes() {
    let (mut s, id) = server_inbox(30);
    let keys: Vec<MessageKey> = s.row_keys().into_iter().take(2).collect();
    s.open_message(&keys[0]);
    s.take_jobs();
    assert_eq!(s.echoes.waiting(&id), 1);
    s.apply(Event::Notice { account: id.clone(), message: "the list couldn't be saved".into() });
    assert_eq!(s.echoes.waiting(&id), 0);
    inbox_changed(&mut s, &id);
    assert!(!s.summary(&keys[0]).unwrap().flags.seen, "as disk has it");

    s.open_message(&keys[1]);
    s.take_jobs();
    s.clock.now += 61;
    inbox_changed(&mut s, &id);
    assert!(!s.summary(&keys[1]).unwrap().flags.seen, "as disk has it");
    assert_eq!(s.echoes.waiting(&id), 0);
}
