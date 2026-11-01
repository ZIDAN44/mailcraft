//! Engine tests: Outlook data files and the accounts that start from or keep one.

use super::*;

#[test]
fn pop_accounts_download_and_can_start_from_a_data_file() {
    let store = temp("pop-account");
    let mut s = Session::open(Some(store), mailcraft_model::dates::Clock::new(NOW, 0));
    // The POP server is guessed from the address; a copy stays on the server unless asked.
    let id =
        s.run("account.add", &json!({"email": "zoe@example.org", "password": "pw", "pop": true})).unwrap()["account"].as_str().unwrap().to_string();
    let a = s.account(&id).unwrap();
    let pop = a.config.pop.clone().unwrap();
    assert_eq!((pop.host.as_str(), pop.port, pop.security), ("mail.example.org", 995, mailcraft_model::Security::Tls));
    assert!(a.config.imap.host.is_empty() && !a.config.pop_delete && !a.is_local());
    assert_eq!((a.config.pop_keep_days, a.config.pop_delete_with_trash), (Some(14), false), "for 14 days, as Outlook's default");
    assert!(matches!(s.take_jobs().as_slice(), [Job::Start { account, .. }] if account.pop.is_some()));
    assert!(s.run("account.list", &json!({})).unwrap().to_string().contains("mail.example.org:995"));
    // Changed: the other server and removing mail once it's down.
    s.run("account.update", &json!({"account": id, "popHost": "pop.example.org", "popPort": 110, "popSecurity": "starttls", "popDelete": true}))
        .unwrap();
    let a = s.account(&id).unwrap();
    assert_eq!(a.config.pop.as_ref().map(|p| (p.host.as_str(), p.port)), Some(("pop.example.org", 110)));
    assert!(a.config.pop_delete);
    s.run("account.update", &json!({"account": id, "popDelete": false, "popKeepDays": 0, "popDeleteWithTrash": true})).unwrap();
    let a = s.account(&id).unwrap();
    assert_eq!((a.config.pop_delete, a.config.pop_keep_days, a.config.pop_delete_with_trash), (false, None, true), "0: kept for good");
    s.take_jobs();
    assert!(s.run("account.add", &json!({"email": "ana@example.org", "password": "pw", "pop": true, "popHost": " "})).is_err(), "a server is needed");

    // An existing data file comes in first; then the account connects.
    let id = s.run("account.add", &json!({"email": "ana@example.org", "password": "pw", "pop": true, "pst": "C:/Mail/old.pst"})).unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(matches!(s.take_jobs().as_slice(), [Job::ImportPst { path, .. }] if path == "C:/Mail/old.pst"), "held until the file is in");
    s.apply(Event::Imported { account: id.clone(), messages: 40, skipped: 0, done: true, error: None, report: None });
    assert!(matches!(s.take_jobs().as_slice(), [Job::Start { account, .. }] if account.id == id));
    // A file that can't be read: the account stays, and connects anyway.
    let id =
        s.run("account.add", &json!({"email": "bo@example.org", "password": "pw", "pop": true, "pst": "C:/Mail/locked.pst"})).unwrap()["account"]
            .as_str()
            .unwrap()
            .to_string();
    s.take_jobs();
    s.apply(Event::Imported {
        account: id.clone(),
        messages: 0,
        skipped: 0,
        done: true,
        error: Some("Outlook has this data file open".into()),
        report: None,
    });
    assert!(s.account(&id).is_some());
    assert!(s.notices.last().is_some_and(|n| n.error && n.text.contains("Outlook has this data file open")));
    assert!(matches!(s.take_jobs().as_slice(), [Job::Start { .. }]));
    let _ = std::fs::remove_dir_all(s.store.as_ref().unwrap().root());
}

#[test]
fn outlook_data_files_open_as_accounts_on_this_computer() {
    let mut s = sample_session("pst");
    assert!(s.run("account.importPst", &json!({"path": "  "})).is_err());
    let id =
        s.run("account.importPst", &json!({"path": r"C:\Users\me\Documents\Archive 2024.pst"})).unwrap()["account"].as_str().unwrap().to_string();
    let a = s.account(&id).unwrap();
    assert_eq!((a.config.name.as_str(), a.config.email.as_str()), ("Archive 2024", "Archive 2024.pst"));
    assert!(a.is_local());
    assert!(
        matches!(s.take_jobs().as_slice(), [Job::ImportPst { account, path, resume: false }] if *account == id && path.ends_with("Archive 2024.pst"))
    );

    // The import reports progress, then its folders, then that it's done: the Inbox opens.
    let progress = mailcraft_model::job::ImportProgress { imported: 200, ..Default::default() };
    s.apply(Event::ImportProgress { account: id.clone(), progress });
    assert!(s.notices.last().is_some_and(|n| !n.error && n.text.contains("200")));
    let inbox = mailcraft_model::Folder {
        account: id.clone(),
        path: "INBOX".into(),
        delimiter: Some('/'),
        role: FolderRole::Inbox,
        selectable: true,
        ..Default::default()
    };
    s.apply(Event::Folders { account: id.clone(), folders: vec![inbox] });
    // The file's contacts were written next to its mail: they show once it's done.
    let mut pim = mailcraft_model::pim::Pim::default();
    pim.cards.push(mailcraft_model::pim::StoredCard {
        book: mailcraft_model::pim::LOCAL.into(),
        card: mailcraft_model::vcard::Card { uid: "c1".into(), name: "Ana Lima".into(), ..Default::default() },
        ..Default::default()
    });
    s.store.clone().unwrap().save_account_doc(&id, mailcraft_model::pim::PIM_DOC, &pim).unwrap();
    s.apply(Event::Imported { account: id.clone(), messages: 250, skipped: 2, done: true, error: None, report: None });
    assert!(s.pim[&id].cards.iter().any(|c| c.card.name == "Ana Lima"));
    assert!(!s.pim[&id].calendars.is_empty(), "with a calendar to add to");
    assert!(s.notices.last().is_some_and(|n| n.text.contains("250 messages imported (2 could not be read)")));
    assert_eq!(s.view, View::Folder { account: id.clone(), folder: "INBOX".into() });
    assert_eq!(s.account(&id).unwrap().status, SyncStatus::Offline);

    // A file that isn't one: the account goes again.
    let bad = s.run("account.importPst", &json!({"path": "/tmp/notes.txt"})).unwrap()["account"].as_str().unwrap().to_string();
    s.apply(Event::Imported {
        account: bad.clone(),
        messages: 0,
        skipped: 0,
        done: true,
        error: Some("not an Outlook data file".into()),
        report: None,
    });
    assert!(s.account(&bad).is_none());
    assert!(s.notices.last().is_some_and(|n| n.error && n.text.contains("notes.txt could not be opened")));
    // Mail came in, then the last save failed: the account and its mail stay, and it's said.
    let late = s.run("account.importPst", &json!({"path": "C:/Mail/late.pst"})).unwrap()["account"].as_str().unwrap().to_string();
    let report = mailcraft_model::job::ImportReport { messages: 30, ..Default::default() };
    s.apply(Event::Imported {
        account: late.clone(),
        messages: 30,
        skipped: 0,
        done: true,
        error: Some("disk full".into()),
        report: Some(Box::new(report)),
    });
    assert!(s.account(&late).is_some(), "what came in is kept");
    assert!(s.notices.last().is_some_and(|n| n.error && n.text.contains("30 messages imported") && n.text.contains("disk full")));
    assert_eq!(s.account(&late).unwrap().status, SyncStatus::Offline);

    // Exporting: the current account, or the one asked for, to the file given.
    s.take_jobs();
    assert!(s.run("account.exportPst", &json!({"path": ""})).is_err());
    assert!(s.run("account.exportPst", &json!({"path": "out.pst", "account": "nope"})).is_err());
    s.run("account.exportPst", &json!({"path": "C:/out/Archive.pst", "account": id})).unwrap();
    assert!(
        matches!(s.take_jobs().as_slice(), [Job::ExportPst { account, path, name }] if *account == id && path == "C:/out/Archive.pst" && name == "Archive 2024")
    );
    s.apply(Event::Exported { account: id.clone(), messages: 100, skipped: 0, already: 0, done: false, error: None });
    assert!(s.notices.last().is_some_and(|n| n.text.contains("100 messages exported so far")));
    s.apply(Event::Exported { account: id.clone(), messages: 120, skipped: 3, already: 0, done: true, error: None });
    assert!(s.notices.last().is_some_and(|n| !n.error && n.text.contains("120 messages exported (3 not on this computer")));
    s.apply(Event::Exported { account: id.clone(), messages: 0, skipped: 0, already: 0, done: true, error: Some("disk full".into()) });
    assert!(s.notices.last().is_some_and(|n| n.error && n.text.contains("disk full")));
    done(s);
}

/// An import whose last save failed keeps what came in past the next start: the folder list the
/// end brought is saved again. When that fails too, the notice says to import again; and a file
/// of contacts only is kept like one of mail.
#[test]
fn a_failed_last_save_is_kept_across_a_restart() {
    use mailcraft_model::job::ImportReport;
    let store = Store::failing();
    let mut s = Session::open(Some(store.clone()), mailcraft_model::dates::Clock::new(NOW, 0));
    let import = |s: &mut Session, file: &str, report: ImportReport| {
        let id = s.run("account.importPst", &json!({"path": file})).unwrap()["account"].as_str().unwrap().to_string();
        let inbox =
            mailcraft_model::Folder { account: id.clone(), path: "INBOX".into(), role: FolderRole::Inbox, selectable: true, ..Default::default() };
        s.apply(Event::Folders { account: id.clone(), folders: vec![inbox] });
        let messages = usize::try_from(report.messages).unwrap();
        s.apply(Event::Imported {
            account: id.clone(),
            messages,
            skipped: 0,
            done: true,
            error: Some("disk full".into()),
            report: Some(Box::new(report)),
        });
        id
    };
    let late = import(&mut s, "C:/Mail/late.pst", ImportReport { messages: 30, ..Default::default() });
    assert!(s.notices.last().is_some_and(|n| n.error && n.text.contains("disk full") && !n.text.contains("again")));
    let cards = import(&mut s, "C:/Mail/cards.pst", ImportReport { contacts: 3, ..Default::default() });
    assert!(s.account(&cards).is_some(), "contacts came in: the account stays");
    store.fail(None, Some("folders.json"));
    let lost = import(&mut s, "C:/Mail/lost.pst", ImportReport { messages: 5, ..Default::default() });
    assert!(s.notices.last().is_some_and(|n| n.error && n.text.contains("Import the file again")));
    store.fail(None, None);

    let again = Session::open(Some(store), mailcraft_model::dates::Clock::new(NOW, 0));
    for id in [&late, &cards] {
        assert!(again.account(id).is_some_and(|a| a.folder(FolderRole::Inbox).is_some()), "{id} lists its folders after a restart");
    }
    assert!(again.account(&lost).is_some_and(|a| a.folders.is_empty()));
}

/// An account whose folders were kept under another account's name (a data file brought in from
/// it) opens them as its own: opening one said "no folder … in account a1".
#[test]
fn folders_kept_under_another_account_open_as_this_ones() {
    use mailcraft_model::dates::Clock;
    use mailcraft_model::{AccountConfig, Folder};
    let dir = std::env::temp_dir().join(format!("mailcraft-folder-owner-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    store.save_accounts(&[AccountConfig { id: "a2".into(), email: "kim@garnetbyte.example".into(), ..Default::default() }]).unwrap();
    let f = |path: &str| Folder { account: "a1".into(), path: path.into(), ..Default::default() };
    store.save_folders("a2", &[Folder { role: FolderRole::Inbox, ..f("INBOX") }, f("INBOX/gzamer")]).unwrap();
    let mut s = Session::open(Some(store), Clock::new(1_700_000_000, 0));
    assert!(s.accounts[0].folders.iter().all(|f| f.account == "a2"));
    s.run("folder.open", &json!({"account": "a2", "folder": "INBOX/gzamer"})).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// Removing an account keeps its data file where it is, as Outlook keeps a removed account's
/// .pst: the same address added again gets a file of its own beside it, and the kept one opens
/// again as a data file. A file that failed to open, or an account starting again, isn't kept.
#[test]
fn a_removed_account_keeps_its_data_file() {
    let dir = std::env::temp_dir().join(format!("mailcraft-engine-keep-file-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let files = dir.join("MailCraft Files");
    let store = Store::open_packed(dir.join("app"), &files).unwrap();
    let mut s = Session::open(Some(store), mailcraft_model::dates::Clock::new(NOW, 0));
    let id = s.run("account.add", &json!({"email": "zoe@example.org", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    let st = s.store.clone().unwrap();
    st.save_folders(&id, &[mailcraft_model::Folder { account: id.clone(), path: "INBOX".into(), selectable: true, ..Default::default() }]).unwrap();
    let kept = files.join("zoe@example.org.mailcraft");
    assert_eq!(st.data_file(&id).as_deref(), Some(kept.as_path()));

    let r = s.run("account.remove", &json!({"account": id})).unwrap();
    assert_eq!(r["dataFile"].as_str(), Some(kept.display().to_string().as_str()));
    assert!(kept.exists(), "the data file stays");
    assert!(s.accounts.is_empty() && st.data_file(&id).is_none());

    // Added again: a file of its own, the kept one untouched.
    let before = std::fs::read(&kept).unwrap();
    let id = s.run("account.add", &json!({"email": "zoe@example.org", "password": "pw"})).unwrap()["account"].as_str().unwrap().to_string();
    st.save_folders(&id, &[mailcraft_model::Folder { account: id.clone(), path: "INBOX".into(), selectable: true, ..Default::default() }]).unwrap();
    let second = files.join("zoe@example.org (2).mailcraft");
    assert_eq!(st.data_file(&id).as_deref(), Some(second.as_path()));
    assert_eq!(std::fs::read(&kept).unwrap(), before);

    // The kept file opens again (File › Open & Export) as a mailbox of its own.
    let opened = st.clone();
    let n = opened.import_data_file("a9", &kept).unwrap();
    assert_eq!(n, 0, "no mail in it, but it opens");
    assert_eq!(opened.load_folders("a9").unwrap().len(), 1);
    drop(s);
    let _ = std::fs::remove_dir_all(dir);
}
