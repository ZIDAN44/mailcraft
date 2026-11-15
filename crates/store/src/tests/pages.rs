//! Store tests: a big folder's list kept in pages by number, only the pages that changed written
//! again, and a list read and written as it was kept before.

use super::*;

/// A big folder's list is kept in pages by number: read back newest first as it was; a flag
/// change writes one page, new mail the last one; shrunk, the pages go; and a list kept whole
/// (from before) still reads.
#[test]
fn a_big_folder_list_is_kept_in_pages() {
    for packed in [false, true] {
        let dir = scratch(&format!("pages-{packed}"));
        let s = if packed { Store::open_packed(dir.join("app"), dir.join("Files")).unwrap() } else { Store::open(&dir).unwrap() };
        let n = PAGE_MESSAGES as u32 * 2 + 500;
        let msg = |uid: u32| Summary {
            key: MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 3, uid },
            subject: format!("Report {uid}"),
            date: 1_700_000_000 + i64::from(uid % 977) * 60,
            ..Default::default()
        };
        let mut st = FolderState { validity: 3, uid_next: n + 1, messages: (1..=n).map(msg).map(Into::into).collect(), ..Default::default() };
        sort_newest_first(&mut st.messages);
        assert_eq!(s.save_folder_counted("a1", "INBOX", &st, None).unwrap(), 4, "three pages and the list");
        let back = s.load_folder("a1", "INBOX").unwrap();
        assert_eq!(back, st, "{packed}: read back newest first, as it was");
        // Read a page at a time (a search reading it over frames): the same list.
        let mut load = s.folder_load("a1", "INBOX").unwrap();
        let mut steps = 0;
        while !load.step(&s).unwrap() {
            steps += 1;
        }
        assert_eq!((steps, load.pages_read()), (2, 4), "a page a step, the head counted");
        assert!(load.done() && load.step(&s).unwrap(), "done stays done");
        assert_eq!(load.finish(), st);
        // A flag: one page.
        if let Some(m) = st.messages.iter_mut().find(|m| m.key.uid == 10).map(std::sync::Arc::make_mut) {
            m.flags.seen = true;
        }
        assert_eq!(s.save_folder_counted("a1", "INBOX", &st, None).unwrap(), 2, "one page and the list");
        assert!(s.load_folder("a1", "INBOX").unwrap().find(10).unwrap().flags.seen);
        // Told which changed: kept beside the pages, which aren't even looked at.
        if let Some(m) = st.messages.iter_mut().find(|m| m.key.uid == 8000).map(std::sync::Arc::make_mut) {
            m.flags.flagged = true;
        }
        let changed: std::collections::HashSet<u32> = [8000].into_iter().collect();
        assert_eq!(s.save_folder_counted("a1", "INBOX", &st, Some(&changed)).unwrap(), 1, "the marks alone");
        assert_eq!(s.load_folder("a1", "INBOX").unwrap(), st);
        // New mail: the last page, and the page the mark was kept beside (the marks emptied).
        st.messages.insert(0, msg(n + 1).into());
        st.uid_next = n + 2;
        assert_eq!(s.save_folder_counted("a1", "INBOX", &st, None).unwrap(), 4);
        assert!(s.load_folder("a1", "INBOX").unwrap().find(8000).unwrap().flags.flagged);
        assert_eq!(s.load_folder("a1", "INBOX").unwrap().messages.len(), n as usize + 1);
        // Told only of a flag, but a message came too: every page looked at, so it's kept.
        st.messages.insert(0, msg(n + 2).into());
        st.uid_next = n + 3;
        assert_eq!(s.save_folder_counted("a1", "INBOX", &st, Some(&changed)).unwrap(), 2);
        assert_eq!(s.load_folder("a1", "INBOX").unwrap().messages.len(), n as usize + 2);
        // Nothing changed: the list alone.
        assert_eq!(s.save_folder_counted("a1", "INBOX", &st, None).unwrap(), 1);
        // Shrunk to a small folder: kept whole again, the pages gone.
        st.messages.truncate(10);
        s.save_folder("a1", "INBOX", &st).unwrap();
        assert_eq!(s.load_folder("a1", "INBOX").unwrap(), st);
        assert!(!s.files.is_file(&s.folder_dir("a1", "INBOX").join("state-0.json")));
    }
}

/// Summaries put right about their files (what an index pass found): only the pages holding
/// one that changed are written again, with the list's head; a list of another validity, or
/// messages already right, write nothing; a list kept whole is put right in place.
#[test]
fn a_files_flip_writes_only_the_pages_it_touches() {
    let s = Store::memory();
    let n = PAGE_MESSAGES as u32 * 2 + 500;
    let mut st =
        FolderState { validity: 3, uid_next: n + 1, messages: (1..=n).map(|uid| msg(uid, true)).map(Into::into).collect(), ..Default::default() };
    for m in st.messages.iter_mut().filter(|m| m.key.uid % 2 == 0).map(std::sync::Arc::make_mut) {
        m.has_attachments = true;
    }
    sort_newest_first(&mut st.messages);
    s.save_folder("a1", "INBOX", &st).unwrap();
    s.track_changes();
    // 10 had files only by its pictures; 4097 (the second page) carries one after all.
    let found = search::FilesFound { validity: 3, with: vec![4097], without: vec![10] };
    assert_eq!(s.set_has_files("a1", "INBOX", &found).unwrap(), 2);
    let mut written: Vec<String> = s.take_changes().into_iter().map(|(p, _)| p.rsplit('/').next().unwrap_or_default().to_string()).collect();
    written.sort();
    assert_eq!(written, ["state-0.json", "state-1.json", "state.json"], "the third page isn't written");
    let back = s.load_folder("a1", "INBOX").unwrap();
    assert!(!back.find(10).unwrap().has_attachments && back.find(4097).unwrap().has_attachments);
    assert!(back.find(12).unwrap().has_attachments && !back.find(11).unwrap().has_attachments, "the others as they were");
    assert_eq!(back.messages.len(), st.messages.len());
    // Already right, or another validity: nothing written.
    assert_eq!(s.set_has_files("a1", "INBOX", &found).unwrap(), 0);
    assert_eq!(s.set_has_files("a1", "INBOX", &search::FilesFound { validity: 4, without: vec![12], ..Default::default() }).unwrap(), 0);
    assert!(s.take_changes().is_empty());
    assert!(s.load_folder("a1", "INBOX").unwrap().find(12).unwrap().has_attachments);
    // A small list, kept whole.
    let small = FolderState { validity: 3, uid_next: 3, messages: vec![msg(2, false).into(), msg(1, false).into()], ..Default::default() };
    s.save_folder("a1", "Small", &small).unwrap();
    assert_eq!(s.set_has_files("a1", "Small", &search::FilesFound { validity: 3, with: vec![1], ..Default::default() }).unwrap(), 1);
    let back = s.load_folder("a1", "Small").unwrap();
    assert!(back.find(1).unwrap().has_attachments && !back.find(2).unwrap().has_attachments);
}

/// The summaries a folder's list holds are shared in memory (`SharedSummary`), not on disk: a
/// `state.json` written while the list held them plain reads back the same, and is written back
/// byte for byte as the plain list was.
#[test]
fn a_list_of_shared_summaries_is_kept_as_it_was_before() {
    // As a list of plain summaries wrote it (fields left out read as their defaults).
    let before = br#"{"validity":3,"uid_next":10,"highest_modseq":77,"messages":[{"key":{"account":"a1","folder":"INBOX","validity":3,"uid":9},"subject":"Hello","flags":{"seen":true}},{"key":{"account":"a1","folder":"INBOX","validity":3,"uid":4},"subject":"Older","size":120}],"window":0}"#;
    let s = temp_store("shared-format");
    let dir = s.folder_dir("a1", "INBOX");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("state.json"), before).unwrap();
    let st = s.load_folder("a1", "INBOX").unwrap();
    assert_eq!((st.validity, st.uid_next, st.highest_modseq, st.messages.len()), (3, 10, 77, 2));
    let plain: Vec<Summary> = st.messages.iter().map(|m| Summary::clone(m)).collect();
    assert_eq!((plain[0].subject.as_str(), plain[0].flags.seen, plain[1].key.uid, plain[1].size), ("Hello", true, 4, 120));
    // Written back: what the plain list made of the same summaries.
    #[derive(Serialize)]
    struct Before {
        validity: u32,
        uid_next: u32,
        highest_modseq: u64,
        messages: Vec<Summary>,
        window: usize,
    }
    s.save_folder("a1", "INBOX", &st).unwrap();
    let written = String::from_utf8(fs::read(dir.join("state.json")).unwrap()).unwrap();
    let old = Before { validity: 3, uid_next: 10, highest_modseq: 77, messages: plain.clone(), window: 0 };
    assert_eq!(written, serde_json::to_string(&old).unwrap());
    assert_eq!(serde_json::to_vec(&st.messages).unwrap(), serde_json::to_vec(&plain).unwrap());
}
