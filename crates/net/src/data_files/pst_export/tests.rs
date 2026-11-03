use super::*;
use mailcraft_model::{EmailAddress, Flags};
use mailcraft_store::FolderState;
use outlook_pst::UnicodePstSpecialFolder as Special;
use outlook_pst::messaging::message::{Message as _, UnicodeMessage};

const NONE: &BTreeMap<String, i64> = &BTreeMap::new();

/// A data file's named property: its ID, by its property set's data1 (PSETID_Task's is
/// 0x00062003) and LID, as the writer's own reader reads the file's map.
fn named_id(path: &Path, set: u32, lid: u32) -> Option<u16> {
    use outlook_pst::messaging::named_prop::{NamedPropertyGuid, NamedPropertyId};
    let store = outlook_pst::messaging::store::UnicodeStore::read(Rc::new(outlook_pst::UnicodePstFile::open(path).ok()?)).ok()?;
    let map = outlook_pst::messaging::store::Store::named_property_map(store.as_ref()).ok()?;
    let guids = map.properties().stream_guid().ok()?;
    map.properties().stream_entry().ok()?.into_iter().find_map(|e| match (e.guid(), e.id()) {
        (NamedPropertyGuid::GuidIndex(i), NamedPropertyId::Number(n)) if n == lid && guids.get(usize::from(i))?.data1() == set => Some(e.prop_id()),
        _ => None,
    })
}

/// The properties `ids` of the message with `subject` in the folder at `names` of a data file.
fn props_of(path: &Path, names: &[&str], subject: &str, ids: &[u16]) -> Vec<Option<outlook_pst::ltp::prop_context::PropertyValue>> {
    let pst = outlook_pst::UnicodePstFile::open(path).unwrap();
    let store = UnicodeStore::read(Rc::new(pst)).unwrap();
    let any: Rc<dyn PstStore> = store.clone();
    let mut folder = any.open_folder(&any.properties().ipm_sub_tree_entry_id().unwrap()).unwrap();
    for name in names {
        let table = folder.hierarchy_table().unwrap().clone();
        folder = table
            .rows_matrix()
            .filter_map(|row| any.open_folder(&any.properties().make_entry_id(NodeId::from(u32::from(row.id()))).ok()?).ok())
            .find(|f| f.properties().display_name().is_ok_and(|n| n == *name))
            .unwrap();
    }
    for row in folder.contents_table().unwrap().rows_matrix() {
        let entry = any.properties().make_entry_id(NodeId::from(u32::from(row.id()))).unwrap();
        let m = UnicodeMessage::read(store.clone(), &entry, None).unwrap();
        if m.properties().get(0x0037).and_then(crate::pst::text_of).as_deref() == Some(subject) {
            return ids.iter().map(|id| m.properties().get(*id).cloned()).collect();
        }
    }
    panic!("no {subject} in {names:?}")
}

#[test]
fn an_account_round_trips_through_a_data_file() {
    use outlook_pst::ltp::prop_context::PropertyValue as V;
    let dir = std::env::temp_dir().join(format!("mailcraft-pst-export-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let store = Store::open(dir.join("cache")).unwrap();
    let mk = |folder: &str, uid: u32, subject: &str, with_file: bool, flags: Flags, edit: &dyn Fn(&mut mime::Draft)| {
        let mut d = mime::Draft {
            to: vec![EmailAddress::new("Ana", "ana@example.com")],
            cc: vec![EmailAddress::new("", "ben@example.com")],
            subject: subject.into(),
            body: "Hello there.".into(),
            html: Some("<p>Hello <b>there</b>.</p>".into()),
            attachments: if with_file {
                vec![mime::OutAttachment { name: "notes.txt".into(), mime: "text/plain".into(), data: b"remember".to_vec(), ..Default::default() }]
            } else {
                Vec::new()
            },
            ..Default::default()
        };
        edit(&mut d);
        let built = mime::build(&d, &EmailAddress::new("Riley", "riley@example.com"), 1_704_067_200, &format!("m{uid}@example.com")).unwrap();
        // A header of its own, which the file keeps (and a sent copy's Bcc, which build never writes).
        let mut raw = b"X-Custom: kept\r\nSensitivity: Private\r\n".to_vec();
        if !d.bcc.is_empty() {
            raw.extend_from_slice(b"Bcc: Cleo <cleo@example.com>\r\n");
        }
        raw.extend_from_slice(&built);
        let key = MessageKey { account: "a1".into(), folder: folder.into(), validity: 1, uid };
        store.write_body(&key, &raw).unwrap();
        mime::summary_from_full(&raw, key, flags, 10, 0)
    };
    let plain = |_: &mut mime::Draft| {};
    let folder = |path: &str, role| mailcraft_model::Folder {
        account: "a1".into(),
        path: path.into(),
        delimiter: Some('/'),
        role,
        selectable: true,
        ..Default::default()
    };
    let seen = Flags { seen: true, ..Default::default() };
    // Unread, flagged with a day it's due, in two categories (and a system keyword that isn't one),
    // marked important, received an hour after its Date:.
    let plan_flags = Flags { flagged: true, keywords: vec!["to=20do".into(), "$label1".into(), "$Junk".into()], ..Default::default() };
    let mut plan = mk("INBOX", 1, "Plan — café", true, plan_flags, &|d| d.importance = 1);
    plan.received = plan.date + 3600;
    let second = mk("INBOX", 2, "Second", false, Flags { answered: true, ..seen.clone() }, &|d| {
        d.in_reply_to = "m1@example.com".into();
        d.references = vec!["m0@example.com".into(), "m1@example.com".into()];
    });
    let inbox = FolderState { validity: 1, uid_next: 3, messages: vec![plan.into(), second.into()], ..Default::default() };
    let mut projects = FolderState {
        validity: 1,
        uid_next: 2,
        messages: vec![mk("INBOX/Projects", 1, "Nested", false, seen.clone(), &plain).into()],
        ..Default::default()
    };
    // Known by its header only: skipped.
    let mut header_only = mailcraft_model::Summary::clone(&projects.messages[0]);
    header_only.key.uid = 7;
    projects.messages.push(header_only.into());
    let one = |path: &str, uid, subject: &str, flags: Flags, edit: &dyn Fn(&mut mime::Draft)| FolderState {
        validity: 1,
        uid_next: uid + 1,
        messages: vec![mk(path, uid, subject, false, flags, edit).into()],
        ..Default::default()
    };
    let bcc = |d: &mut mime::Draft| d.bcc = vec![EmailAddress::new("Cleo", "cleo@example.com")];
    store.save_folder("a1", "INBOX", &inbox).unwrap();
    store.save_folder("a1", "INBOX/Projects", &projects).unwrap();
    store.save_folder("a1", "Sent", &one("Sent", 1, "Sent one", Flags { forwarded: true, ..seen.clone() }, &bcc)).unwrap();
    store.save_folder("a1", "Sent/2024", &one("Sent/2024", 1, "Older sent", seen.clone(), &plain)).unwrap();
    store.save_folder("a1", "Drafts", &one("Drafts", 1, "Draft one", Flags::default(), &plain)).unwrap();
    store.save_folder("a1", "Bin", &one("Bin", 1, "Thrown away", seen.clone(), &plain)).unwrap();
    store
        .save_folders(
            "a1",
            &[
                folder("INBOX", FolderRole::Inbox),
                folder("INBOX/Projects", FolderRole::Other),
                folder("Sent", FolderRole::Sent),
                folder("Sent/2024", FolderRole::Other),
                folder("Drafts", FolderRole::Drafts),
                folder("Bin", FolderRole::Trash),
                // No mail: still the file's Junk Email.
                folder("Spam", FolderRole::Junk),
            ],
        )
        .unwrap();
    let due = BTreeMap::from([("m1@example.com".to_string(), 20_000)]);

    let mut pim = mailcraft_model::pim::Pim::default();
    pim.events.push(mailcraft_model::pim::StoredEvent {
        event: mailcraft_model::ical::CalEvent {
            uid: "e1".into(),
            summary: "Screening".into(),
            start: 1_792_000_000,
            end: 1_792_003_600,
            ..Default::default()
        },
        ..Default::default()
    });
    pim.cards.push(mailcraft_model::pim::StoredCard {
        card: mailcraft_model::vcard::Card { uid: "c1".into(), name: "Ana Lima".into(), ..Default::default() },
        ..Default::default()
    });
    // A task with no due day goes in all the same.
    pim.tasks.push(mailcraft_model::pim::StoredTask {
        task: mailcraft_model::pim::Task { uid: "t1".into(), title: "Pack the lenses".into(), percent: 40, ..Default::default() },
    });
    store.save_account_doc("a1", mailcraft_model::pim::PIM_DOC, &pim).unwrap();
    let file = dir.join("Out.pst");
    // Something there that isn't a data file: left alone, and said.
    std::fs::write(&file, b"an older file").unwrap();
    assert!(export(&store, "a1", "Riley's mail", &file, NONE, &mut |_| {}).is_err_and(|e| e.contains("isn't an Outlook data file")));
    assert_eq!(std::fs::read(&file).unwrap(), b"an older file");
    std::fs::remove_file(&file).unwrap();
    let mut ticks = 0;
    let done = export(&store, "a1", "Riley's mail", &file, &due, &mut |_| ticks += 1).unwrap();
    assert_eq!(
        done,
        Exported { messages: 7, skipped: 1, already: 0, events: 1, occurrences: 0, tasks: 1, notes: 0, contacts: 1, pim_already: 0, pim_replaced: 0 }
    );
    // The calendar is inside it too: nothing beside it.
    assert!(!dir.join("Out Calendar.ics").exists() && !dir.join("Out Contacts.vcf").exists());
    assert_eq!(UnicodePstFile::special_folder(&file, Special::Calendar).unwrap(), Some(vec!["Calendar".to_string()]));
    assert!(ticks > 0);

    // The file records its special folders, as Outlook names them.
    for (special, name) in [
        (Special::SentItems, "Sent Items"),
        (Special::DeletedItems, "Deleted Items"),
        (Special::Drafts, "Drafts"),
        (Special::JunkEmail, "Junk Email"),
    ] {
        assert_eq!(UnicodePstFile::special_folder(&file, special).unwrap(), Some(vec![name.to_string()]), "{special:?}");
    }
    assert_eq!(UnicodePstFile::special_folder(&file, Special::Outbox).unwrap(), None);
    // What the import doesn't read back: the follow-up's text, icon and day, and that the
    // addresses are SMTP ones.
    let task_due = named_id(&file, 0x0006_2003, 0x8105).unwrap();
    let task_start = named_id(&file, 0x0006_2003, 0x8104).unwrap();
    let flag_request = named_id(&file, 0x0006_2008, 0x8530).unwrap();
    let day = (20_000 * 86_400 + 11_644_473_600) * 10_000_000;
    let got =
        props_of(&file, &["Inbox"], "Plan — café", &[task_due, task_start, flag_request, 0x1095, 0x0E2B, 0x0C1E, 0x0065, 0x0036, 0x0E06, 0x0039]);
    assert!(matches!(got[0], Some(V::Time(t)) if t == day), "{:?}", got[0]);
    assert!(matches!(got[1], Some(V::Time(t)) if t == day));
    assert_eq!(got[2].as_ref().and_then(crate::pst::text_of).as_deref(), Some("Follow up"));
    assert!(matches!(got[3], Some(V::Integer32(6))) && matches!(got[4], Some(V::Integer32(1))));
    assert_eq!(got[5].as_ref().and_then(crate::pst::text_of).as_deref(), Some("SMTP"));
    assert_eq!(got[6].as_ref().and_then(crate::pst::text_of).as_deref(), Some("riley@example.com"));
    assert!(matches!(got[7], Some(V::Integer32(2))));
    // Delivered an hour after it was sent.
    let (Some(V::Time(delivered)), Some(V::Time(sent))) = (&got[8], &got[9]) else { panic!("{got:?}") };
    assert_eq!(delivered - sent, 3600 * 10_000_000);
    // A draft is unsent; the others aren't.
    let draft = props_of(&file, &["Drafts"], "Draft one", &[0x0E07]);
    assert!(matches!(draft[0], Some(V::Integer32(f)) if f & 0x8 != 0 && f & 0x1 == 0));
    let older = props_of(&file, &["Sent Items", "2024"], "Older sent", &[0x0E07]);
    assert!(matches!(older[0], Some(V::Integer32(f)) if f & 0x8 == 0 && f & 0x1 != 0));

    // Read back with the importer.
    let back = Store::open(dir.join("back")).unwrap();
    let imported = crate::pst::import(&file, &back, "b1", &mut |_| {}).unwrap();
    assert_eq!((imported.messages, imported.contacts, imported.tasks, imported.events), (7, 1, 1, 1), "{imported:?}");
    let all = back.load_folders("b1").unwrap();
    for (role, path) in
        [(FolderRole::Sent, "Sent Items"), (FolderRole::Trash, "Deleted Items"), (FolderRole::Drafts, "Drafts"), (FolderRole::Junk, "Junk Email")]
    {
        assert!(all.iter().any(|f| f.role == role && f.path == path), "{role:?} {all:?}");
    }
    assert!(all.iter().any(|f| f.path == "Sent Items/2024" && f.total == 1), "{all:?}");
    let inbox = back.load_folder("b1", "INBOX").unwrap();
    assert_eq!(inbox.messages.len(), 2, "{all:?}");
    let plan = inbox.messages.iter().find(|m| m.subject == "Plan — café").unwrap();
    assert!(!plan.flags.seen && plan.flags.flagged && !plan.flags.draft, "{:?}", plan.flags);
    assert_eq!(plan.flags.keywords, ["to=20do", "$label1"]);
    assert_eq!(plan.importance, 1);
    assert_eq!(plan.received, plan.date + 3600);
    let raw = back.read_body(&MessageKey { account: "b1".into(), folder: "INBOX".into(), validity: 1, uid: plan.key.uid }).unwrap();
    let p = mime::parse(&raw).unwrap();
    assert_eq!(p.from.email, "riley@example.com");
    assert_eq!(p.cc[0].email, "ben@example.com");
    assert_eq!(p.message_id, "m1@example.com");
    assert_eq!(p.date, 1_704_067_200);
    assert!(p.attachments.iter().any(|a| a.name == "notes.txt"));
    // Its own header, kept whole.
    let head = String::from_utf8_lossy(&raw);
    assert!(head.contains("X-Custom: kept") && head.contains("Sensitivity: Private"), "{head}");
    let second = inbox.messages.iter().find(|m| m.subject == "Second").unwrap();
    assert!(second.flags.seen && second.flags.answered && !second.flags.flagged && second.flags.keywords.is_empty());
    assert_eq!(second.in_reply_to, "m1@example.com");
    assert_eq!(second.references, ["m0@example.com", "m1@example.com"]);
    let sent = &back.load_folder("b1", "Sent Items").unwrap().messages[0];
    assert!(sent.flags.seen && sent.flags.forwarded, "{:?}", sent.flags);
    let raw = back.read_body(&sent.key).unwrap();
    assert_eq!(mime::parse(&raw).unwrap().bcc[0].email, "cleo@example.com");
    let draft = &back.load_folder("b1", "Drafts").unwrap().messages[0];
    assert!(draft.flags.draft && !draft.flags.seen, "{:?}", draft.flags);
    assert_eq!(back.load_folder("b1", "Deleted Items").unwrap().messages[0].subject, "Thrown away");
    assert!(all.iter().any(|f| f.path.ends_with("Projects") && f.total == 1), "{all:?}");

    // Exported again into the same file (issue #18): added to, nothing twice; a new message goes
    // in beside the ones there, and the special folders stay the file's.
    let again = export(&store, "a1", "Riley's mail", &file, NONE, &mut |_| {}).unwrap();
    assert_eq!((again.messages, again.already, again.events, again.contacts, again.tasks, again.pim_already), (0, 7, 0, 0, 0, 3));
    let mut inbox = store.load_folder("a1", "INBOX").unwrap();
    inbox.messages.push(mk("INBOX", 3, "Third", false, Flags::default(), &plain).into());
    store.save_folder("a1", "INBOX", &inbox).unwrap();
    let more = export(&store, "a1", "Riley's mail", &file, NONE, &mut |_| {}).unwrap();
    assert_eq!((more.messages, more.already), (1, 7));
    let back = Store::open(dir.join("back2")).unwrap();
    let imported = crate::pst::import(&file, &back, "b2", &mut |_| {}).unwrap();
    // The event, the contact and the task once each.
    assert_eq!((imported.messages, imported.events, imported.contacts, imported.tasks), (8, 1, 1, 1), "{imported:?}");
    let subjects: Vec<String> = back.load_folder("b2", "INBOX").unwrap().messages.iter().map(|m| m.subject.clone()).collect();
    assert_eq!(subjects.iter().filter(|s| *s == "Second").count(), 1, "{subjects:?}");
    assert!(subjects.contains(&"Third".to_string()), "{subjects:?}");
    assert_eq!(back.load_folder("b2", "Sent Items").unwrap().messages.len(), 1);
    assert!(!back.load_folders("b2").unwrap().iter().any(|f| f.path.starts_with("Sent Items (")));
    assert!(!dir.join("Out.pst.part").exists());

    // A file that can't be replaced: the error says where the mail is, and it is there.
    let busy = dir.join("Busy.pst");
    std::fs::create_dir_all(busy.join("inside")).unwrap();
    let e = export(&store, "a1", "Riley's mail", &busy, NONE, &mut |_| {}).unwrap_err();
    assert!(e.contains("Busy.pst.part"), "{e}");
    let back = Store::open(dir.join("back3")).unwrap();
    assert_eq!(crate::pst::import(&dir.join("Busy.pst.part"), &back, "b3", &mut |_| {}).unwrap().messages, 8);

    // Nothing to write: an error, and the old file stays.
    let empty = dir.join("Empty.pst");
    assert!(export(&store, "nobody", "x", &empty, NONE, &mut |_| {}).is_err());
    assert!(!empty.exists());
    let _ = std::fs::remove_dir_all(&dir);
}

/// A file added to keeps its own special folders: the account's Sent mail goes into the folder
/// it records, whatever its name, and none of its own is recorded in its place.
#[test]
fn an_export_uses_the_special_folders_a_file_has() {
    let dir = std::env::temp_dir().join(format!("mailcraft-pst-export-special-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("Theirs.pst");
    let theirs = UnicodePstMessage {
        subject: "Theirs",
        sender_name: "Ana",
        sender_email: "ana@example.com",
        recipients: &[],
        body: "Kept.",
        html_body: None,
        message_id: "<theirs@example.com>",
        delivery_time: 133_485_408_000_000_000,
        message_class: "IPM.Note",
        properties: &[],
    };
    drop(UnicodePstFile::create_in_folder(&file, &["Gesendete Elemente"], &theirs).unwrap());
    drop(UnicodePstFile::set_special_folder(&file, Special::SentItems, &["Gesendete Elemente"]).unwrap());

    let store = Store::open(dir.join("cache")).unwrap();
    let d = mime::Draft { subject: "Ours".into(), body: "Sent.".into(), ..Default::default() };
    let raw = mime::build(&d, &EmailAddress::new("Riley", "riley@example.com"), 1_704_067_200, "ours@example.com").unwrap();
    let key = MessageKey { account: "a1".into(), folder: "Sent".into(), validity: 1, uid: 1 };
    store.write_body(&key, &raw).unwrap();
    let s = mime::summary_from_full(&raw, key, Flags { seen: true, ..Default::default() }, 10, 0);
    store.save_folder("a1", "Sent", &FolderState { validity: 1, uid_next: 2, messages: vec![s.into()], ..Default::default() }).unwrap();
    let sent = mailcraft_model::Folder { account: "a1".into(), path: "Sent".into(), role: FolderRole::Sent, selectable: true, ..Default::default() };
    store.save_folders("a1", &[sent]).unwrap();

    assert_eq!(export(&store, "a1", "Riley's mail", &file, NONE, &mut |_| {}).unwrap().messages, 1);
    assert_eq!(UnicodePstFile::special_folder(&file, Special::SentItems).unwrap(), Some(vec!["Gesendete Elemente".to_string()]));
    let back = Store::open(dir.join("back")).unwrap();
    assert_eq!(crate::pst::import(&file, &back, "b1", &mut |_| {}).unwrap().messages, 2);
    let folders = back.load_folders("b1").unwrap();
    let theirs = folders.iter().find(|f| f.path == "Gesendete Elemente").unwrap();
    assert_eq!((theirs.role, theirs.total), (FolderRole::Sent, 2), "{folders:?}");
    assert!(!folders.iter().any(|f| f.path == "Sent Items"), "{folders:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// MS-PST 5.1's permutation (`mpbbCrypt`'s first 256 bytes), which encodes a data block's bytes.
const PERMUTE: [u8; 256] = [
    0x41, 0x36, 0x13, 0x62, 0xA8, 0x21, 0x6E, 0xBB, 0xF4, 0x16, 0xCC, 0x04, 0x7F, 0x64, 0xE8, 0x5D, 0x1E, 0xF2, 0xCB, 0x2A, 0x74, 0xC5, 0x5E, 0x35,
    0xD2, 0x95, 0x47, 0x9E, 0x96, 0x2D, 0x9A, 0x88, 0x4C, 0x7D, 0x84, 0x3F, 0xDB, 0xAC, 0x31, 0xB6, 0x48, 0x5F, 0xF6, 0xC4, 0xD8, 0x39, 0x8B, 0xE7,
    0x23, 0x3B, 0x38, 0x8E, 0xC8, 0xC1, 0xDF, 0x25, 0xB1, 0x20, 0xA5, 0x46, 0x60, 0x4E, 0x9C, 0xFB, 0xAA, 0xD3, 0x56, 0x51, 0x45, 0x7C, 0x55, 0x00,
    0x07, 0xC9, 0x2B, 0x9D, 0x85, 0x9B, 0x09, 0xA0, 0x8F, 0xAD, 0xB3, 0x0F, 0x63, 0xAB, 0x89, 0x4B, 0xD7, 0xA7, 0x15, 0x5A, 0x71, 0x66, 0x42, 0xBF,
    0x26, 0x4A, 0x6B, 0x98, 0xFA, 0xEA, 0x77, 0x53, 0xB2, 0x70, 0x05, 0x2C, 0xFD, 0x59, 0x3A, 0x86, 0x7E, 0xCE, 0x06, 0xEB, 0x82, 0x78, 0x57, 0xC7,
    0x8D, 0x43, 0xAF, 0xB4, 0x1C, 0xD4, 0x5B, 0xCD, 0xE2, 0xE9, 0x27, 0x4F, 0xC3, 0x08, 0x72, 0x80, 0xCF, 0xB0, 0xEF, 0xF5, 0x28, 0x6D, 0xBE, 0x30,
    0x4D, 0x34, 0x92, 0xD5, 0x0E, 0x3C, 0x22, 0x32, 0xE5, 0xE4, 0xF9, 0x9F, 0xC2, 0xD1, 0x0A, 0x81, 0x12, 0xE1, 0xEE, 0x91, 0x83, 0x76, 0xE3, 0x97,
    0xE6, 0x61, 0x8A, 0x17, 0x79, 0xA4, 0xB7, 0xDC, 0x90, 0x7A, 0x5C, 0x8C, 0x02, 0xA6, 0xCA, 0x69, 0xDE, 0x50, 0x1A, 0x11, 0x93, 0xB9, 0x52, 0x87,
    0x58, 0xFC, 0xED, 0x1D, 0x37, 0x49, 0x1B, 0x6A, 0xE0, 0x29, 0x33, 0x99, 0xBD, 0x6C, 0xD9, 0x94, 0xF3, 0x40, 0x54, 0x6F, 0xF0, 0xC6, 0x73, 0xB8,
    0xD6, 0x3E, 0x65, 0x18, 0x44, 0x1F, 0xDD, 0x67, 0x10, 0xF1, 0x0C, 0x19, 0xEC, 0xAE, 0x03, 0xA1, 0x14, 0x7B, 0xA9, 0x0B, 0xFF, 0xF8, 0xA3, 0xC0,
    0xA2, 0x01, 0xF7, 0x2E, 0xBC, 0x24, 0x68, 0x75, 0x0D, 0xFE, 0xBA, 0x2F, 0xB5, 0xD0, 0xDA, 0x3D,
];

/// MS-PST's CRC: CRC-32's table, started at 0 and not inverted at the end.
fn crc(data: &[u8]) -> u32 {
    let mut c = 0u32;
    for &b in data {
        c ^= u32::from(b);
        for _ in 0..8 {
            c = if c & 1 == 1 { (c >> 1) ^ 0xEDB8_8320 } else { c >> 1 };
        }
    }
    c
}

/// Encode a data file the writer made the way Outlook encodes its own by default: every data
/// block's bytes permuted (internal blocks never are), found from the 16-byte trailers that end
/// blocks on 64-byte boundaries, with their CRCs, and the header's `bCryptMethod` (0x201) set
/// to permute, with the CRC over the header (0x20C, of the 516 bytes from 8).
fn permute_encode(path: &Path) {
    let mut f = std::fs::read(path).unwrap();
    let u32_at = |f: &[u8], at: usize| u32::from_le_bytes(f[at..at + 4].try_into().unwrap());
    for end in (64..=f.len()).step_by(64) {
        let size = usize::from(u16::from_le_bytes([f[end - 16], f[end - 15]]));
        let bid = u64::from_le_bytes(f[end - 8..end].try_into().unwrap());
        let Some(start) = end.checked_sub((size + 16).div_ceil(64) * 64) else { continue };
        if size == 0 || size > 8176 || bid & 2 != 0 || crc(&f[start..start + size]) != u32_at(&f, end - 12) {
            continue;
        }
        for b in &mut f[start..start + size] {
            *b = PERMUTE[usize::from(*b)];
        }
        let c = crc(&f[start..start + size]);
        f[end - 12..end - 8].copy_from_slice(&c.to_le_bytes());
    }
    f[0x201] = 1;
    let c = crc(&f[8..8 + 516]);
    f[0x20C..0x210].copy_from_slice(&c.to_le_bytes());
    std::fs::write(path, f).unwrap();
}

/// A data file Outlook made, as Outlook encodes them, is added to (the writer from MailCraft's
/// fork): what it had stays, a message it already has isn't written twice, it stays encoded,
/// and the import reads it all back.
#[test]
fn an_export_adds_to_an_encoded_data_file() {
    let dir = std::env::temp_dir().join(format!("mailcraft-pst-export-encoded-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("Outlook.pst");
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
    drop(UnicodePstFile::create_in_folder(&file, &["Inbox"], &theirs("Theirs", "<theirs@example.com>")).unwrap());
    drop(UnicodePstFile::append_in_folder(&file, &["Inbox"], &theirs("Both", "<m1@example.com>")).unwrap());
    permute_encode(&file);

    let store = Store::open(dir.join("cache")).unwrap();
    let mk = |uid: u32, subject: &str| {
        let d = mime::Draft { subject: subject.into(), body: "Hello there.".into(), ..Default::default() };
        let raw = mime::build(&d, &EmailAddress::new("Riley", "riley@example.com"), 1_704_067_200, &format!("m{uid}@example.com")).unwrap();
        let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid };
        store.write_body(&key, &raw).unwrap();
        mime::summary_from_full(&raw, key, Flags::default(), 10, 0)
    };
    let inbox = FolderState { validity: 1, uid_next: 3, messages: vec![mk(1, "Both").into(), mk(2, "Ours").into()], ..Default::default() };
    store.save_folder("a1", "INBOX", &inbox).unwrap();
    let folder =
        mailcraft_model::Folder { account: "a1".into(), path: "INBOX".into(), role: FolderRole::Inbox, selectable: true, ..Default::default() };
    store.save_folders("a1", &[folder]).unwrap();

    let done = export(&store, "a1", "Riley's mail", &file, NONE, &mut |_| {}).unwrap();
    assert_eq!((done.messages, done.already), (1, 1));
    assert_eq!(std::fs::read(&file).unwrap()[0x201], 1, "still permute-encoded");
    let back = Store::open(dir.join("back")).unwrap();
    assert_eq!(crate::pst::import(&file, &back, "b1", &mut |_| {}).unwrap().messages, 3);
    let mut subjects: Vec<String> = back.load_folder("b1", "INBOX").unwrap().messages.iter().map(|m| m.subject.clone()).collect();
    subjects.sort();
    assert_eq!(subjects, ["Both", "Ours", "Theirs"]);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A data file whose name-to-ID map can't take names (here: its bucket count is 0) is still
/// added to: a flagged message in a category goes in flagged, without its named properties.
#[test]
fn an_export_adds_to_a_file_whose_name_map_is_damaged() {
    use outlook_pst::ltp::prop_context::PropertyValue as V;
    let dir = std::env::temp_dir().join(format!("mailcraft-pst-export-name-map-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("Theirs.pst");
    let theirs = UnicodePstMessage {
        subject: "Theirs",
        sender_name: "Ana",
        sender_email: "ana@example.com",
        recipients: &[],
        body: "Kept.",
        html_body: None,
        message_id: "<theirs@example.com>",
        delivery_time: 133_485_408_000_000_000,
        message_class: "IPM.Note",
        properties: &[],
    };
    drop(UnicodePstFile::create_in_folder(&file, &["Inbox"], &theirs).unwrap());
    // The map's PidTagNameidBucketCount record (its ID 0x0001, PtypInteger32) set to 0, in the
    // one block that has it, its CRC made right.
    let mut f = std::fs::read(&file).unwrap();
    let mut patched = 0;
    for end in (64..=f.len()).step_by(64) {
        let size = usize::from(u16::from_le_bytes([f[end - 16], f[end - 15]]));
        let Some(start) = end.checked_sub((size + 16).div_ceil(64) * 64) else { continue };
        if size == 0 || size > 8176 || crc(&f[start..start + size]) != u32::from_le_bytes(f[end - 12..end - 8].try_into().unwrap()) {
            continue;
        }
        let Some(at) = f[start..start + size].windows(8).position(|w| w[..4] == [1, 0, 3, 0] && w[4..] != [0, 0, 0, 0]) else { continue };
        f[start + at + 4..start + at + 8].copy_from_slice(&0u32.to_le_bytes());
        let c = crc(&f[start..start + size]);
        f[end - 12..end - 8].copy_from_slice(&c.to_le_bytes());
        patched += 1;
    }
    assert_eq!(patched, 1);
    std::fs::write(&file, f).unwrap();

    let store = Store::open(dir.join("cache")).unwrap();
    let d = mime::Draft { subject: "Ours".into(), body: "Flagged.".into(), ..Default::default() };
    let mut messages = Vec::new();
    for uid in [1, 2] {
        let raw = mime::build(&d, &EmailAddress::new("Riley", "riley@example.com"), 1_704_067_200, &format!("ours{uid}@example.com")).unwrap();
        let key = MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid };
        store.write_body(&key, &raw).unwrap();
        let flags = Flags { flagged: true, keywords: vec!["Red".into()], ..Default::default() };
        messages.push(mime::summary_from_full(&raw, key, flags, 10, 0).into());
    }
    store.save_folder("a1", "INBOX", &FolderState { validity: 1, uid_next: 3, messages, ..Default::default() }).unwrap();
    let inbox =
        mailcraft_model::Folder { account: "a1".into(), path: "INBOX".into(), role: FolderRole::Inbox, selectable: true, ..Default::default() };
    store.save_folders("a1", &[inbox]).unwrap();

    assert_eq!(export(&store, "a1", "Riley's mail", &file, NONE, &mut |_| {}).unwrap().messages, 2);
    let [flag, status] = props_of(&file, &["Inbox"], "Ours", &[0x1090, 0x0037]).try_into().unwrap();
    assert!(matches!(flag, Some(V::Integer32(2))));
    assert!(status.is_some());
    let back = Store::open(dir.join("back")).unwrap();
    assert_eq!(crate::pst::import(&file, &back, "b1", &mut |_| {}).unwrap().messages, 3);
    let _ = std::fs::remove_dir_all(&dir);
}

/// An Outlook 97–2002 file that is only a header (damaged) says it can't be read, before
/// anything is written.
#[test]
fn a_damaged_ansi_data_file_says_it_cant_be_read() {
    let dir = std::env::temp_dir().join(format!("mailcraft-pst-export-ansi-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("Old.pst");
    let mut head = b"!BDN\0\0\0\0SM".to_vec();
    head.extend_from_slice(&14u16.to_le_bytes());
    head.resize(1024, 0);
    std::fs::write(&file, &head).unwrap();
    assert!(ids_in(&file).is_err_and(|e| e.contains("can't be read")));
    assert_eq!(std::fs::read(&file).unwrap(), head);
    let _ = std::fs::remove_dir_all(&dir);
}

mod ansi;
mod big;
mod calendar;
mod kinds;
mod pim;
mod replace;
