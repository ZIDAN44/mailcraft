//! An Outlook 97–2002 (ANSI) data file added to: the account's mail, contacts, notes and tasks go
//! in beside what it had, their strings 8-bit, and the import reads them all back.

use super::*;
use outlook_pst::messaging::store::AnsiStore;
use outlook_pst::{AnsiPstFile, UnicodePstBatchMessage};

fn mail(store: &Store, folder: &str, uid: u32, subject: &str) -> mailcraft_model::Summary {
    let d = mime::Draft { subject: subject.into(), body: format!("{subject}, the body."), ..Default::default() };
    let raw = mime::build(&d, &EmailAddress::new("Riley", "riley@example.com"), 1_704_067_200, &format!("m{uid}@example.com")).unwrap();
    let key = MessageKey { account: "a1".into(), folder: folder.into(), validity: 1, uid };
    store.write_body(&key, &raw).unwrap();
    mime::summary_from_full(&raw, key, Flags::default(), 10, 0)
}

/// A message's subject as the file keeps it, by its 8-bit bytes and the code page the message
/// says.
fn subjects(path: &Path) -> Vec<(Vec<u8>, i32)> {
    use outlook_pst::ltp::prop_context::PropertyValue as V;
    let any: Rc<dyn PstStore> = AnsiStore::read(Rc::new(AnsiPstFile::open(path).unwrap())).unwrap();
    let top = any.open_folder(&any.properties().ipm_sub_tree_entry_id().unwrap()).unwrap();
    let inbox = top
        .hierarchy_table()
        .unwrap()
        .rows_matrix()
        .filter_map(|row| any.open_folder(&any.properties().make_entry_id(NodeId::from(u32::from(row.id()))).ok()?).ok())
        .find(|f| f.properties().display_name().is_ok_and(|n| n == "Inbox"))
        .unwrap();
    let mut out = Vec::new();
    for row in inbox.contents_table().unwrap().rows_matrix() {
        let entry = any.properties().make_entry_id(NodeId::from(u32::from(row.id()))).unwrap();
        let m = any.open_message(&entry, None).unwrap();
        let page = match m.properties().get(0x3FFD) {
            Some(V::Integer32(page)) => *page,
            _ => 0,
        };
        match m.properties().get(0x0037) {
            Some(V::String8(subject)) => out.push((subject.buffer().to_vec(), page)),
            other => panic!("not an 8-bit subject: {other:?}"),
        }
    }
    out.sort();
    out
}

#[test]
fn an_account_goes_into_an_ansi_data_file_and_back() {
    let dir = std::env::temp_dir().join(format!("mailcraft-pst-export-ansi-file-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("Old.pst");
    // The file's own mail, one message of which the account has too.
    drop(AnsiPstFile::create_empty(&file).unwrap());
    let theirs = |subject, message_id| UnicodePstMessage {
        subject,
        sender_name: "Ana",
        sender_email: "ana@example.com",
        recipients: &[],
        body: "Kept as it was.",
        html_body: None,
        message_id,
        delivery_time: 133_485_408_000_000_000,
        message_class: "IPM.Note",
        properties: &[],
    };
    let batch = [
        UnicodePstBatchMessage { message: theirs("Theirs", "<theirs@example.com>"), attachments: &[] },
        UnicodePstBatchMessage { message: theirs("Both", "<m1@example.com>"), attachments: &[] },
    ];
    drop(AnsiPstFile::append_many_in_folder_with_attachments(&file, &["Inbox"], &batch).unwrap());

    let store = Store::open(dir.join("cache")).unwrap();
    let inbox = FolderState {
        validity: 1,
        uid_next: 4,
        messages: vec![
            mail(&store, "INBOX", 1, "Both").into(),
            mail(&store, "INBOX", 2, "Grüße").into(),
            mail(&store, "INBOX", 3, "Ελληνικά").into(),
        ],
        ..Default::default()
    };
    store.save_folder("a1", "INBOX", &inbox).unwrap();
    let sent = FolderState { validity: 1, uid_next: 5, messages: vec![mail(&store, "Sent", 4, "Sent one").into()], ..Default::default() };
    store.save_folder("a1", "Sent", &sent).unwrap();
    let folder = |path: &str, role| mailcraft_model::Folder { account: "a1".into(), path: path.into(), role, selectable: true, ..Default::default() };
    store.save_folders("a1", &[folder("INBOX", FolderRole::Inbox), folder("Sent", FolderRole::Sent)]).unwrap();
    store.save_account_doc("a1", mailcraft_model::pim::PIM_DOC, &pim::account()).unwrap();

    let done = export(&store, "a1", "Riley's mail", &file, NONE, &mut |_| {}).unwrap();
    assert_eq!((done.messages, done.already), (3, 1));
    assert_eq!((done.contacts, done.notes, done.tasks), (4, 2, 4));
    // Still Outlook 97–2002's format, its strings 8-bit: Windows-1252 where they fit, else UTF-8.
    assert_eq!(Format::of(&file), Format::Ansi);
    let mut want =
        vec![(b"Both".to_vec(), 1252), (b"Theirs".to_vec(), 1252), (b"Gr\xFC\xDFe".to_vec(), 1252), ("Ελληνικά".as_bytes().to_vec(), 65001)];
    want.sort();
    assert_eq!(subjects(&file), want);
    assert_eq!(Format::Ansi.special_folder(&file, Special::SentItems).unwrap(), Some(vec!["Sent Items".to_string()]));
    // The `pst_append` fuzz target's ANSI seed (fuzz/README.md) is this file.
    if let Some(to) = std::env::var_os("MC_PST_ANSI_SEED") {
        std::fs::copy(&file, to).unwrap();
    }

    // Read back by the import, every subject as it was written.
    let back = Store::open(dir.join("back")).unwrap();
    let imported = crate::pst::import(&file, &back, "b1", &mut |_| {}).unwrap();
    assert_eq!((imported.messages, imported.contacts, imported.notes, imported.tasks), (5, 4, 2, 4), "{imported:?}");
    let folders = back.load_folders("b1").unwrap();
    let path = |role| folders.iter().find(|f| f.role == role).map(|f| f.path.clone()).unwrap();
    let mut subjects: Vec<String> = back.load_folder("b1", &path(FolderRole::Inbox)).unwrap().messages.iter().map(|m| m.subject.clone()).collect();
    subjects.sort();
    assert_eq!(subjects, ["Both", "Grüße", "Theirs", "Ελληνικά"]);
    let sent: Vec<String> = back.load_folder("b1", &path(FolderRole::Sent)).unwrap().messages.iter().map(|m| m.subject.clone()).collect();
    assert_eq!(sent, ["Sent one"]);

    // Added to again, nothing goes in twice.
    let again = export(&store, "a1", "Riley's mail", &file, NONE, &mut |_| {}).unwrap();
    assert_eq!((again.messages, again.already, again.pim_already), (0, 4, 10));
    let _ = std::fs::remove_dir_all(&dir);
}

/// The top folders of an ANSI file: each one's name as written and how many messages it holds.
fn top(path: &Path) -> Vec<(Vec<u8>, usize)> {
    use outlook_pst::ltp::prop_context::PropertyValue as V;
    let any: Rc<dyn PstStore> = AnsiStore::read(Rc::new(AnsiPstFile::open(path).unwrap())).unwrap();
    let top = any.open_folder(&any.properties().ipm_sub_tree_entry_id().unwrap()).unwrap();
    let mut out: Vec<(Vec<u8>, usize)> = top
        .hierarchy_table()
        .unwrap()
        .rows_matrix()
        .map(|row| {
            let f = any.open_folder(&any.properties().make_entry_id(NodeId::from(u32::from(row.id()))).unwrap()).unwrap();
            let name = match f.properties().get(0x3001) {
                Some(V::String8(name)) => name.buffer().to_vec(),
                other => panic!("not an 8-bit name: {other:?}"),
            };
            (name, f.contents_table().map_or(0, |t| t.rows_matrix().count()))
        })
        .collect();
    out.sort();
    out
}

/// Folders whose names Windows-1252 can't hold whole go into an ANSI file once, each export
/// adding to the same folder, and two that it would write alike stay apart.
#[test]
fn folders_windows_1252_cant_name_stay_one_each() {
    let dir = std::env::temp_dir().join(format!("mailcraft-pst-export-ansi-names-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("Old.pst");
    drop(AnsiPstFile::create_empty(&file).unwrap());
    let store = Store::open(dir.join("cache")).unwrap();
    let names = ["Письма", "Отчёты", "Receipts – 2024"];
    let folder = |path: &str| mailcraft_model::Folder {
        account: "a1".into(),
        path: path.into(),
        role: FolderRole::Other,
        selectable: true,
        ..Default::default()
    };
    store.save_folders("a1", &names.map(folder)).unwrap();
    let mut uid = 0;
    for round in 1..=3 {
        for name in names {
            let mut st = store.load_folder("a1", name).unwrap_or_default();
            uid += 1;
            st.messages.push(mail(&store, name, uid, &format!("{name} {round}")).into());
            st.validity = 1;
            st.uid_next = uid + 1;
            store.save_folder("a1", name, &st).unwrap();
        }
        let done = export(&store, "a1", "Riley's mail", &file, NONE, &mut |_| {}).unwrap();
        assert_eq!((done.messages, done.already), (3, 3 * (round - 1)));
        // Отчёты comes first and takes `??????`; Письма, written alike, has a number.
        let mut want = vec![
            (b"Deleted Items".to_vec(), 0),
            (b"Inbox".to_vec(), 0),
            (b"??????".to_vec(), round),
            (b"?????? (2)".to_vec(), round),
            (b"Receipts \x96 2024".to_vec(), round),
        ];
        want.sort();
        let mut found = top(&file);
        found.retain(|(name, _)| !matches!(name.as_slice(), b"Calendar" | b"Contacts" | b"Notes" | b"Tasks"));
        assert_eq!(found, want, "round {round}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
