//! Contacts, contact groups, notes and tasks through a data file: exported as Outlook's items in
//! its Contacts, Notes and Tasks folders, and read back by MailCraft's import field by field.

use super::*;
use mailcraft_model::pim::{Note, Pim, StoredCard, StoredNote, StoredTask, Task, TaskPriority, TaskStatus};
use mailcraft_model::vcard::{Card, PostalAddress, Typed};

fn typed(value: &str, kind: &str) -> Typed {
    Typed { value: value.into(), kind: kind.into() }
}

fn address(kind: &str, street: &str, city: &str) -> PostalAddress {
    PostalAddress {
        kind: kind.into(),
        street: street.into(),
        city: city.into(),
        region: "Norte".into(),
        postcode: "4000".into(),
        country: "Portugal".into(),
    }
}

fn scratch(label: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("mailcraft-pst-export-pim-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A card with everything Outlook has a field for, and more than it has (a fourth address, a
/// phone of a kind it hasn't, a third work phone, two more postal addresses, an address's kind,
/// other properties); one with a birthday without a year; two groups, one whose note lists its
/// members as the import writes it; two notes; four tasks.
fn account() -> Pim {
    let png = "iVBORw0KGgo=";
    let ana = Card {
        uid: "c1".into(),
        name: "Ana Lima".into(),
        given: "Ana".into(),
        family: "Lima".into(),
        emails: vec![
            typed("ana@studio.example", "work"),
            typed("ana@home.example", ""),
            typed("al@posteo.example", ""),
            typed("ana.l@old.example", ""),
        ],
        phones: vec![
            typed("+351 1", "work"),
            typed("+351 2", "work"),
            typed("+351 3", "home"),
            typed("+351 4", "cell"),
            typed("+351 5", "fax"),
            typed("+351 6", "pager"),
            typed("+351 7", ""),
            typed("+351 8", "video"),
            typed("+351 9", "work"),
        ],
        org: "Studio Lantern".into(),
        title: "Director".into(),
        note: "Met at the fair.\nTwice.".into(),
        addresses: vec![
            address("work", "1 Quay\nUnit 4", "Porto"),
            address("home", "2 Hill", "Braga"),
            address("", "3 Shore", "Faro"),
            address("postal", "PO Box 9", "Porto"),
            address("work", "5 Dock", "Lisboa"),
        ],
        birthday: "1990-04-01".into(),
        extra: vec![
            "URL:https://ana.example/".into(),
            "NICKNAME:Bee".into(),
            "URL;TYPE=work:https://lantern.example/".into(),
            format!("PHOTO:data:image/png;base64,{png}"),
            "X-MAILCRAFT-ATTACH;X-FILENAME=\"cv; 2026.pdf\":data:application/pdf;base64,Y3Y=".into(),
        ],
    };
    let ben =
        Card { uid: "c2".into(), name: "Ben".into(), emails: vec![typed("ben@example.com", "")], birthday: "--12-24".into(), ..Default::default() };
    let crew = Card {
        uid: "g1".into(),
        name: "Crew".into(),
        note: "Ana Lima <ana@studio.example>\nben@example.com\n\nThe film crew.".into(),
        extra: vec!["X-ROLE:crew".into(), "KIND:group".into(), "MEMBER:mailto:ana@studio.example".into(), "MEMBER:mailto:ben@example.com".into()],
        ..Default::default()
    };
    let pair = Card {
        uid: "g2".into(),
        name: "Pair".into(),
        note: "Just us.".into(),
        extra: vec!["KIND:group".into(), "MEMBER:mailto:cleo@example.com".into()],
        ..Default::default()
    };
    let day = |d: u32| mailcraft_model::dates::days_from_civil(2026, 11, d) * 86_400;
    let tasks = vec![
        Task {
            uid: "t1".into(),
            title: "Book the stage".into(),
            notes: "Hall B\nor C".into(),
            start: Some(day(3)),
            due: Some(day(10)),
            status: TaskStatus::InProgress,
            percent: 40,
            priority: TaskPriority::High,
            categories: vec!["Film".into(), "Läuft".into()],
            reminder: Some(day(9) + 9 * 3600),
            rrule: Some("FREQ=WEEKLY;INTERVAL=1;BYDAY=TU".into()),
            modified: 1_790_000_000,
            extra: vec!["X-CUSTOM:1".into(), crate::pst_pim::event_file("plan.pdf", "application/pdf", b"%PDF-1.4 plan")],
            ..Default::default()
        },
        Task {
            uid: "t2".into(),
            title: "Return the lenses".into(),
            due: Some(day(1)),
            completed: Some(1_790_100_000),
            status: TaskStatus::Completed,
            percent: 100,
            priority: TaskPriority::Low,
            modified: 1_790_100_000,
            ..Default::default()
        },
        Task { uid: "t3".into(), title: "Hourly check".into(), rrule: Some("FREQ=HOURLY".into()), modified: 1_790_000_000, ..Default::default() },
        Task {
            uid: "t4".into(),
            title: "Weekly call".into(),
            due: Some(day(4)),
            status: TaskStatus::Waiting,
            rrule: Some("FREQ=WEEKLY".into()),
            modified: 1_790_000_000,
            ..Default::default()
        },
    ];
    let notes = vec![
        Note { uid: "n1".into(), text: "Shopping\nLenses\nTape".into(), color: 1, modified: 1_790_000_000 },
        Note { uid: "n2".into(), text: "Call back".into(), color: 3, modified: 1_790_000_100 },
    ];
    Pim {
        cards: [ana, ben, crew, pair].into_iter().map(|card| StoredCard { book: "local".into(), card, ..Default::default() }).collect(),
        notes: notes.into_iter().map(|note| StoredNote { note, ..Default::default() }).collect(),
        tasks: tasks.into_iter().map(|task| StoredTask { task }).collect(),
        ..Default::default()
    }
}

/// `items` without their uids (the import makes its own), by `key`.
fn by<T: Clone, K: Ord>(items: &[T], uid: impl Fn(&mut T), key: impl Fn(&T) -> K) -> Vec<T> {
    let mut out: Vec<T> = items.to_vec();
    out.iter_mut().for_each(uid);
    out.sort_by_key(key);
    out
}

#[test]
fn contacts_groups_notes_and_tasks_round_trip_inside_a_data_file() {
    use outlook_pst::ltp::prop_context::PropertyValue as V;
    let dir = scratch("round-trip");
    let store = Store::open(dir.join("cache")).unwrap();
    let pim = account();
    store.save_account_doc("a1", mailcraft_model::pim::PIM_DOC, &pim).unwrap();
    // No mail at all: the file is made for them.
    let file = dir.join("People.pst");
    let done = export(&store, "a1", "People", &file, NONE, &mut |_| {}).unwrap();
    assert_eq!(done, Exported { contacts: 4, notes: 2, tasks: 4, ..Default::default() });
    assert!(!dir.join("People Contacts.vcf").exists() && !dir.join("People Calendar.ics").exists());

    // Outlook's folders, recorded as the file's own.
    for (special, class) in [(Special::Contacts, "IPF.Contact"), (Special::Notes, "IPF.StickyNote"), (Special::Tasks, "IPF.Task")] {
        let names = UnicodePstFile::special_folder(&file, special).unwrap().unwrap();
        assert_eq!(names, [special.default_name()]);
        let store = UnicodeStore::read(Rc::new(outlook_pst::UnicodePstFile::open(&file).unwrap())).unwrap();
        let any: Rc<dyn PstStore> = store.clone();
        let top = any.open_folder(&any.properties().ipm_sub_tree_entry_id().unwrap()).unwrap();
        let table = top.hierarchy_table().unwrap().clone();
        let folder = table
            .rows_matrix()
            .filter_map(|row| any.open_folder(&any.properties().make_entry_id(NodeId::from(u32::from(row.id()))).ok()?).ok())
            .find(|f| f.properties().display_name().is_ok_and(|n| n == names[0]))
            .unwrap();
        assert_eq!(folder.properties().get(0x3613).and_then(crate::pst::text_of).as_deref(), Some(class));
    }
    // As Outlook keeps them: the classes, a contact's three addresses and picture, a group's
    // checksum, a task's fraction done, a note's colour and size.
    let class = |names: &[&str], subject: &str| props_of(&file, names, subject, &[0x001A]).remove(0).as_ref().and_then(crate::pst::text_of);
    assert_eq!(class(&["Contacts"], "Ana Lima").as_deref(), Some("IPM.Contact"));
    assert_eq!(class(&["Contacts"], "Crew").as_deref(), Some("IPM.DistList"));
    assert_eq!(class(&["Notes"], "Shopping").as_deref(), Some("IPM.StickyNote"));
    assert_eq!(class(&["Tasks"], "Book the stage").as_deref(), Some("IPM.Task"));
    let address = |lid| named_id(&file, 0x0006_2004, lid).unwrap();
    let got =
        props_of(&file, &["Contacts"], "Ana Lima", &[address(0x8083), address(0x8093), address(0x80A3), address(0x8015), address(0x8005), 0x3A1C]);
    let texts: Vec<Option<String>> = got.iter().map(|v| v.as_ref().and_then(crate::pst::text_of)).collect();
    assert_eq!(texts[..3], [Some("ana@studio.example".into()), Some("ana@home.example".into()), Some("al@posteo.example".into())]);
    assert!(matches!(got[3], Some(V::Boolean(true))));
    assert_eq!(texts[4].as_deref(), Some("Lima, Ana"));
    assert_eq!(texts[5].as_deref(), Some("+351 4"));
    let got = props_of(&file, &["Contacts"], "Crew", &[address(0x8053), address(0x804C)]);
    assert_eq!(got[0].as_ref().and_then(crate::pst::text_of).as_deref(), Some("Crew"));
    assert!(matches!(got[1], Some(V::Integer32(_))));
    let percent = named_id(&file, 0x0006_2003, 0x8102).unwrap();
    assert!(matches!(props_of(&file, &["Tasks"], "Book the stage", &[percent])[0], Some(V::Floating64(f)) if f == 0.4));
    let note = |lid| named_id(&file, 0x0006_200E, lid).unwrap();
    let got = props_of(&file, &["Notes"], "Shopping", &[note(0x8B00), note(0x8B02), note(0x8B03)]);
    assert!(matches!(got[..], [Some(V::Integer32(1)), Some(V::Integer32(200)), Some(V::Integer32(166))]), "{got:?}");

    // Read back by the import: every field.
    let back = Store::open(dir.join("back")).unwrap();
    let imported = crate::pst::import(&file, &back, "b1", &mut |_| {}).unwrap();
    assert_eq!((imported.messages, imported.contacts, imported.notes, imported.tasks), (0, 4, 2, 4), "{imported:?}");
    assert_eq!((imported.skipped, imported.skips.len()), (0, 0), "{imported:?}");
    let got: Pim = back.load_account_doc("b1", mailcraft_model::pim::PIM_DOC).unwrap();
    let cards = |p: &Pim| p.cards.iter().map(|c| c.card.clone()).collect::<Vec<_>>();
    let mut want = by(&cards(&pim), |c| c.uid.clear(), |c| c.name.clone());
    // A group whose note doesn't list its members gets the list, as one from Outlook does.
    if let Some(pair) = want.iter_mut().find(|c| c.name == "Pair") {
        pair.note = "cleo@example.com\n\nJust us.".into();
    }
    let have = by(&cards(&got), |c| c.uid.clear(), |c| c.name.clone());
    assert_eq!(have.len(), want.len());
    for (h, w) in have.iter().zip(&want) {
        assert_eq!(h, w, "{}", w.name);
    }
    let notes = |p: &Pim| p.notes.iter().map(|n| n.note.clone()).collect::<Vec<_>>();
    assert_eq!(by(&notes(&got), |n| n.uid.clear(), |n| n.text.clone()), by(&notes(&pim), |n| n.uid.clear(), |n| n.text.clone()));
    let tasks = |p: &Pim| p.tasks.iter().map(|t| t.task.clone()).collect::<Vec<_>>();
    let (have, want) = (by(&tasks(&got), |t| t.uid.clear(), |t| t.title.clone()), by(&tasks(&pim), |t| t.uid.clear(), |t| t.title.clone()));
    for (h, w) in have.iter().zip(&want) {
        assert_eq!(h, w, "{}", w.title);
    }
    assert_eq!(have.len(), want.len());
    let _ = std::fs::remove_dir_all(&dir);
}

/// A file added to keeps its own contacts, and its own Contacts folder, whatever its name, takes
/// the account's; a mail folder of the account named as Outlook names one of its folders keeps
/// it, and the notes go beside it. Added to again, nothing goes in twice.
#[test]
fn a_file_added_to_keeps_its_own_contacts() {
    let dir = scratch("added-to");
    let file = dir.join("Outlook.pst");
    let first = UnicodePstMessage {
        subject: "Hello",
        sender_name: "Zoe",
        sender_email: "zoe@example.com",
        recipients: &[],
        body: "Hi.",
        html_body: None,
        message_id: "<hello@example.com>",
        delivery_time: 133_000_000_000_000_000,
        message_class: "IPM.Note",
        properties: &[],
    };
    drop(UnicodePstFile::create(&file, &first).unwrap());
    drop(UnicodePstFile::set_special_folder(&file, Special::Contacts, &["Kontakte"]).unwrap());
    let given = [
        UnicodePstProperty::tagged(0x3001, pst_rw::UnicodePstValue::Unicode("Zoe Park")),
        UnicodePstProperty::tagged(0x3A06, pst_rw::UnicodePstValue::Unicode("Zoe")),
    ];
    let zoe = UnicodePstMessage { subject: "Zoe Park", message_class: "IPM.Contact", message_id: "", body: "", properties: &given, ..first };
    drop(UnicodePstFile::append_in_folder(&file, &["Kontakte"], &zoe).unwrap());

    let store = Store::open(dir.join("cache")).unwrap();
    let mut pim = account();
    pim.cards.truncate(1);
    pim.tasks.clear();
    pim.notes.truncate(1);
    store.save_account_doc("a1", mailcraft_model::pim::PIM_DOC, &pim).unwrap();
    // A mail folder named Notes, with a message in it.
    let raw = b"From: Ana <ana@studio.example>\r\nSubject: Field notes\r\nMessage-ID: <field@example.com>\r\nDate: Mon, 2 Nov 2026 10:00:00 +0000\r\n\r\nBody.\r\n";
    let key = MessageKey { account: "a1".into(), folder: "Notes".into(), validity: 1, uid: 1 };
    store.write_body(&key, raw).unwrap();
    let summary = mime::summary_from_full(raw, key, mailcraft_model::Flags::default(), 10, 0);
    store
        .save_folder("a1", "Notes", &mailcraft_store::FolderState { validity: 1, uid_next: 2, messages: vec![summary.into()], ..Default::default() })
        .unwrap();
    let notes_folder = mailcraft_model::Folder { account: "a1".into(), path: "Notes".into(), selectable: true, ..Default::default() };
    store.save_folders("a1", &[notes_folder]).unwrap();

    for round in 0..2 {
        let done = export(&store, "a1", "Mine", &file, NONE, &mut |_| {}).unwrap();
        assert_eq!((done.messages, done.contacts, done.notes), (1 - round, 1, 1), "{done:?}");
        assert_eq!(UnicodePstFile::special_folder(&file, Special::Contacts).unwrap(), Some(vec!["Kontakte".to_string()]));
        assert_eq!(UnicodePstFile::special_folder(&file, Special::Notes).unwrap(), Some(vec!["Notes (2)".to_string()]));
        let back = Store::open(dir.join(format!("back{round}"))).unwrap();
        let imported = crate::pst::import(&file, &back, "b1", &mut |_| {}).unwrap();
        assert_eq!((imported.messages, imported.contacts, imported.notes), (2, 2, 1), "{imported:?}");
        let got: mailcraft_model::pim::Pim = back.load_account_doc("b1", mailcraft_model::pim::PIM_DOC).unwrap();
        let mut names: Vec<String> = got.cards.iter().map(|c| c.card.name.clone()).collect();
        names.sort();
        assert_eq!(names, ["Ana Lima", "Zoe Park"]);
        let folders = back.load_folders("b1").unwrap();
        assert!(folders.iter().any(|f| f.path == "Notes" && f.total == 1), "{folders:?}");
        assert!(!folders.iter().any(|f| f.path == "Contacts"), "{folders:?}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
