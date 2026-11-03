//! Pinning into a data file and back: Outlook's files have no pin, so a pinned message carries
//! MailCraft's own property (`pst_pim::PINNED`), in the Unicode format and the ANSI one, and the
//! import pins it again; a message without it comes in unpinned.

use super::*;
use outlook_pst::AnsiPstFile;
use outlook_pst::ltp::prop_context::PropertyValue as V;
use outlook_pst::messaging::named_prop::{NamedPropertyGuid, NamedPropertyId};
use outlook_pst::messaging::store::AnsiStore;

/// The account's Inbox: "Pinned" pinned (and in a category), "Loose" not.
fn account(dir: &Path) -> Store {
    let store = Store::open(dir.join("cache")).unwrap();
    let mail = |uid: u32, subject: &str, flags: Flags| {
        let d = mime::Draft { subject: subject.into(), body: "Hello.".into(), ..Default::default() };
        let raw = mime::build(&d, &EmailAddress::new("Riley", "riley@example.com"), 1_704_067_200, &format!("m{uid}@example.com")).unwrap();
        let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid };
        store.write_body(&key, &raw).unwrap();
        mime::summary_from_full(&raw, key, flags, 10, 0)
    };
    let mut pinned = Flags { seen: true, keywords: vec!["to=20do".into()], ..Default::default() };
    pinned.set_keyword(mailcraft_model::PINNED, true);
    let inbox = FolderState {
        validity: 1,
        uid_next: 3,
        messages: vec![mail(1, "Pinned", pinned).into(), mail(2, "Loose", Flags::default()).into()],
        ..Default::default()
    };
    store.save_folder("a1", "INBOX", &inbox).unwrap();
    let inbox =
        mailcraft_model::Folder { account: "a1".into(), path: "INBOX".into(), role: FolderRole::Inbox, selectable: true, ..Default::default() };
    store.save_folders("a1", &[inbox]).unwrap();
    store
}

/// The pin of each message in the file's Inbox, by subject: its property's value, None without
/// one.
fn pins(any: Rc<dyn PstStore>) -> Vec<(String, Option<V>)> {
    let map = any.named_property_map().unwrap();
    let strings = match map.properties().get(0x0004) {
        Some(V::Binary(b)) => b.buffer().to_vec(),
        _ => Vec::new(),
    };
    // A string's name: its length in bytes, then UTF-16 (MS-PST 2.4.7.4).
    let name_at = |at: usize| {
        let len = u32::from_le_bytes(strings.get(at..at + 4)?.try_into().ok()?) as usize;
        let units: Vec<u16> = strings.get(at + 4..at + 4 + len)?.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
        String::from_utf16(&units).ok()
    };
    let id = map.properties().stream_entry().unwrap().into_iter().find_map(|e| match (e.guid(), e.id()) {
        (NamedPropertyGuid::PublicStrings, NamedPropertyId::StringOffset(at)) if name_at(at as usize).as_deref() == Some(crate::pst_pim::PINNED) => {
            Some(e.prop_id())
        }
        _ => None,
    });
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
        let subject = m.properties().get(0x0037).and_then(crate::pst::text_of).unwrap();
        out.push((subject, id.and_then(|id| m.properties().get(id).cloned())));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Imported into a store of its own: each Inbox message's subject, whether it's pinned, and
/// its keywords.
fn imported(file: &Path, dir: &Path, label: &str) -> Vec<(String, bool, Vec<String>)> {
    let back = Store::open(dir.join(label)).unwrap();
    crate::pst::import(file, &back, label, &mut |_| {}).unwrap();
    let folders = back.load_folders(label).unwrap();
    let inbox = folders.iter().find(|f| f.role == FolderRole::Inbox).unwrap();
    let mut out: Vec<_> = back
        .load_folder(label, &inbox.path)
        .unwrap()
        .messages
        .iter()
        .map(|m| (m.subject.clone(), m.flags.pinned(), m.flags.keywords.clone()))
        .collect();
    out.sort();
    out
}

#[test]
fn pinning_goes_into_a_unicode_data_file_and_back() {
    let dir = std::env::temp_dir().join(format!("mailcraft-pst-export-pinned-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let store = account(&dir);
    let file = dir.join("Out.pst");
    assert_eq!(export(&store, "a1", "Riley's mail", &file, NONE, &mut |_| {}).unwrap().messages, 2);
    let any: Rc<dyn PstStore> = UnicodeStore::read(Rc::new(UnicodePstFile::open(&file).unwrap())).unwrap();
    let got = pins(any);
    assert!(matches!(got.as_slice(), [(loose, None), (pinned, Some(V::Boolean(true)))] if loose == "Loose" && pinned == "Pinned"), "{got:?}");
    assert_eq!(
        imported(&file, &dir, "b1"),
        [("Loose".into(), false, vec![]), ("Pinned".into(), true, vec!["$Pinned".to_string(), "to=20do".to_string()])]
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn pinning_goes_into_an_ansi_data_file_and_back() {
    let dir = std::env::temp_dir().join(format!("mailcraft-pst-export-pinned-ansi-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let store = account(&dir);
    let file = dir.join("Old.pst");
    drop(AnsiPstFile::create_empty(&file).unwrap());
    assert_eq!(export(&store, "a1", "Riley's mail", &file, NONE, &mut |_| {}).unwrap().messages, 2);
    assert_eq!(Format::of(&file), Format::Ansi);
    let any: Rc<dyn PstStore> = AnsiStore::read(Rc::new(AnsiPstFile::open(&file).unwrap())).unwrap();
    let got = pins(any);
    assert!(matches!(got.as_slice(), [(loose, None), (pinned, Some(V::Boolean(true)))] if loose == "Loose" && pinned == "Pinned"), "{got:?}");
    assert_eq!(
        imported(&file, &dir, "b1"),
        [("Loose".into(), false, vec![]), ("Pinned".into(), true, vec!["$Pinned".to_string(), "to=20do".to_string()])]
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A file written without MailCraft's pin (Outlook's, say) comes in unpinned, and so does one
/// whose pin says false.
#[test]
fn a_message_without_the_pin_comes_in_unpinned() {
    use outlook_pst::messaging::named_prop::PS_PUBLIC_STRINGS;
    use outlook_pst::{UnicodePstName, UnicodePstValue};
    let dir = std::env::temp_dir().join(format!("mailcraft-pst-export-unpinned-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("Theirs.pst");
    let message = |subject, message_id, properties| UnicodePstMessage {
        subject,
        sender_name: "Ana",
        sender_email: "ana@example.com",
        recipients: &[],
        body: "Kept.",
        html_body: None,
        message_id,
        delivery_time: 133_485_408_000_000_000,
        message_class: "IPM.Note",
        properties,
    };
    let unpinned = [UnicodePstProperty::named(PS_PUBLIC_STRINGS, UnicodePstName::String(crate::pst_pim::PINNED), UnicodePstValue::Boolean(false))];
    let batch = [
        UnicodePstBatchMessage { message: message("Plain", "<plain@example.com>", &[]), attachments: &[] },
        UnicodePstBatchMessage { message: message("Unpinned", "<unpinned@example.com>", &unpinned), attachments: &[] },
    ];
    drop(UnicodePstFile::create_empty(&file).unwrap());
    drop(UnicodePstFile::append_many_in_folder_with_attachments(&file, &["Inbox"], &batch).unwrap());
    assert_eq!(imported(&file, &dir, "b1"), [("Plain".into(), false, vec![]), ("Unpinned".into(), false, vec![])]);
    let _ = std::fs::remove_dir_all(&dir);
}
