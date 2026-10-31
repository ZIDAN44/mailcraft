//! Engine tests: a store that fails to read or write loses no mail (a list that couldn't be read
//! was saved as empty, a source that couldn't be kept left a message without one).

use super::*;

use mailcraft_store::FolderState;

const SHORT_FILM: &str = "Projects/Short film";

/// The sample in a cache whose files can be made to fail.
fn failing_session() -> Session {
    let store = Store::failing();
    assert!(sample::seed_if_empty(&store, NOW).unwrap());
    let mut s = Session::open(Some(store), mailcraft_model::dates::Clock::new(NOW, 0));
    assert!(s.take_jobs().is_empty());
    s
}

fn store(s: &Session) -> &Store {
    s.store.as_ref().unwrap()
}

fn folder(s: &Session, path: &str) -> FolderState {
    store(s).load_folder(sample::SAMPLE_ACCOUNT, path).unwrap()
}

fn subjects(st: &FolderState) -> Vec<String> {
    let mut v: Vec<String> = st.messages.iter().map(|m| m.subject.clone()).collect();
    v.sort();
    v
}

/// Every message of a folder has its source.
fn all_kept(s: &Session, st: &FolderState) -> bool {
    st.messages.iter().all(|m| store(s).read_body(&m.key).is_some())
}

#[test]
fn a_local_move_that_cant_read_or_write_leaves_the_mail_where_it_was() {
    let mut s = failing_session();
    let inbox = folder(&s, "INBOX");
    let film = folder(&s, SHORT_FILM);
    assert!(!film.messages.is_empty() && all_kept(&s, &inbox) && all_kept(&s, &film));
    // A list, a source that can't be read; a source that can't be written.
    for (reads, writes) in [(Some("state.json"), None), (Some(".eml"), None), (None, Some(".eml"))] {
        let k = key_of(&s, "Character turnarounds");
        s.run("list.select", &k).unwrap();
        store(&s).fail(reads, writes);
        let _ = s.run("mail.move", &json!({"folder": SHORT_FILM}));
        store(&s).fail(None, None);
        assert!(s.notices.last().is_some_and(|n| n.error), "{reads:?} {writes:?}: said");
        assert_eq!(subjects(&folder(&s, "INBOX")), subjects(&inbox), "{reads:?} {writes:?}: still in the Inbox");
        assert_eq!(subjects(&folder(&s, SHORT_FILM)), subjects(&film), "{reads:?} {writes:?}: the destination keeps its mail");
        assert!(all_kept(&s, &folder(&s, "INBOX")) && all_kept(&s, &folder(&s, SHORT_FILM)), "{reads:?} {writes:?}");
        s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
        assert!(s.list.iter().any(|m| m.subject == "Character turnarounds"), "{reads:?} {writes:?}: shown where it is");
    }
    // Working again, it moves.
    let k = key_of(&s, "Character turnarounds");
    s.run("list.select", &k).unwrap();
    s.run("mail.move", &json!({"folder": SHORT_FILM})).unwrap();
    assert_eq!(folder(&s, SHORT_FILM).messages.len(), film.messages.len() + 1);
    assert!(all_kept(&s, &folder(&s, SHORT_FILM)));
}

#[test]
fn filing_locally_that_cant_read_or_write_keeps_the_draft_and_the_message() {
    let mut s = failing_session();
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "ana@soundlab.example", "subject": "Mix notes", "body": "First take\n"})).unwrap();
    s.run("compose.saveDraft", &json!({})).unwrap();
    let drafts = folder(&s, "Drafts");
    let first = s.compose.as_ref().unwrap().draft.clone().unwrap();
    assert!(drafts.find(first.uid).is_some() && all_kept(&s, &drafts));
    let sent = folder(&s, "Sent");
    s.run("compose.update", &json!({"body": "Second take\n"})).unwrap();
    for (reads, writes) in [(Some("state.json"), None), (None, Some(".eml"))] {
        store(&s).fail(reads, writes);
        let saved = s.run("compose.saveDraft", &json!({}));
        let kept = s.keep_drafts();
        let sending = s.run("compose.send", &json!({}));
        store(&s).fail(None, None);
        assert!(saved.is_err() && kept == 0 && sending.is_err(), "{reads:?} {writes:?}: {saved:?} {kept} {sending:?}");
        // The earlier draft is still there, whole, and the message is still being written.
        assert_eq!(subjects(&folder(&s, "Drafts")), subjects(&drafts), "{reads:?} {writes:?}");
        assert!(all_kept(&s, &folder(&s, "Drafts")), "{reads:?} {writes:?}");
        assert_eq!(subjects(&folder(&s, "Sent")), subjects(&sent), "{reads:?} {writes:?}: nothing half-sent");
        let c = s.compose.as_ref().unwrap();
        assert_eq!((c.draft.as_ref(), c.body.as_str()), (Some(&first), "Second take\n"), "{reads:?} {writes:?}");
    }
    // Working again, it's saved and sent.
    s.run("compose.saveDraft", &json!({})).unwrap();
    assert!(folder(&s, "Drafts").find(first.uid).is_none(), "the earlier draft goes once the new one is kept");
    s.run("compose.send", &json!({})).unwrap();
    assert!(s.compose.is_none());
    assert_eq!(folder(&s, "Sent").messages.len(), sent.messages.len() + 1);
    assert!(folder(&s, "Drafts").messages.iter().all(|m| m.subject != "Mix notes"));
}

#[test]
fn calendars_and_contacts_that_cant_be_read_arent_written_over() {
    let cache = Store::failing();
    assert!(sample::seed_if_empty(&cache, NOW).unwrap());
    let before = cache.load_account_doc::<serde_json::Value>(sample::SAMPLE_ACCOUNT, mailcraft_model::pim::PIM_DOC).unwrap();
    cache.fail(Some(mailcraft_model::pim::PIM_DOC), None);
    let mut s = Session::open(Some(cache), mailcraft_model::dates::Clock::new(NOW, 0));
    store(&s).fail(None, None);
    assert!(s.notices.iter().any(|n| n.error), "said");
    s.run("contact.create", &json!({"name": "Mira Chen", "emails": "mira@colour.example"})).unwrap();
    let after = store(&s).load_account_doc::<serde_json::Value>(sample::SAMPLE_ACCOUNT, mailcraft_model::pim::PIM_DOC).unwrap();
    assert_eq!(after, before, "what couldn't be read isn't saved over");
}
