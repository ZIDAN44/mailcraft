//! Exported again into the same data file: a calendar item, contact, note or task changed since
//! goes in in place of its earlier copy, one unchanged stays as it was, in a Unicode file and in
//! an Outlook 97–2002 (ANSI) one; and the import reads each folder back with one copy of each.

use super::*;
use mailcraft_model::ical::CalEvent;
use mailcraft_model::pim::{Note, Pim, StoredCard, StoredEvent, StoredNote, StoredTask, Task};
use mailcraft_model::vcard::Card;
use mailcraft_pst::write;

fn scratch(label: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("mailcraft-pst-export-replace-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Two of each: an event, a contact, a note and a task.
fn account() -> Pim {
    let event = |uid: &str, summary: &str, start: i64| StoredEvent {
        calendar: "local".into(),
        event: CalEvent { uid: uid.into(), summary: summary.into(), start, end: start + 3600, ..Default::default() },
        ..Default::default()
    };
    let card = |uid: &str, name: &str| StoredCard {
        book: "local".into(),
        card: Card { uid: uid.into(), name: name.into(), ..Default::default() },
        ..Default::default()
    };
    let note = |uid: &str, text: &str| StoredNote {
        note: Note { uid: uid.into(), text: text.into(), color: 3, modified: 1_790_000_000, files: Vec::new() },
        ..Default::default()
    };
    let task =
        |uid: &str, title: &str| StoredTask { task: Task { uid: uid.into(), title: title.into(), modified: 1_790_000_000, ..Default::default() } };
    Pim {
        events: vec![event("e1@example.com", "Standup", 1_792_000_800), event("e2@example.com", "Review", 1_792_100_800)],
        cards: vec![card("c1", "Ana Lima"), card("c2", "Ben Ruiz")],
        notes: vec![note("n1", "Shopping"), note("n2", "Ideas")],
        tasks: vec![task("t1", "Book the stage"), task("t2", "Return the lenses")],
        ..Default::default()
    }
}

/// Each folder's items, by subject with their node IDs, sorted.
fn items(path: &Path) -> BTreeMap<String, Vec<(String, u32)>> {
    let any = store_of(path).unwrap();
    let top = any.open_folder(any.ipm_subtree().unwrap()).unwrap();
    let mut out = BTreeMap::new();
    for row in top.hierarchy_table().unwrap().rows_matrix() {
        let folder = any.open_folder(row.id()).unwrap();
        let name = folder.properties().display_name().unwrap();
        if !["Calendar", "Contacts", "Notes", "Tasks"].contains(&name.as_str()) {
            continue;
        }
        let mut list = Vec::new();
        for row in folder.contents_table().unwrap().rows_matrix() {
            let entry = row.id();
            let m = Message::read(any.clone(), entry).unwrap();
            list.push((m.properties().get(0x0037).and_then(crate::pst::text_of).unwrap_or_default(), row.id()));
        }
        list.sort();
        out.insert(name, list);
    }
    out
}

fn subjects(items: &BTreeMap<String, Vec<(String, u32)>>, folder: &str) -> Vec<String> {
    items.get(folder).map(|list| list.iter().map(|(s, _)| s.clone()).collect()).unwrap_or_default()
}

fn node(items: &BTreeMap<String, Vec<(String, u32)>>, folder: &str, subject: &str) -> u32 {
    items.get(folder).and_then(|list| list.iter().find(|(s, _)| s == subject)).map(|(_, n)| *n).unwrap()
}

/// Export the account into `file`, change one of each and export again; then once more with
/// nothing changed.
fn exported_again_in_place(label: &str, ansi: bool) {
    let dir = scratch(label);
    let file = dir.join("Out.pst");
    if ansi {
        write::create_empty(&file, mailcraft_pst::Format::Ansi).unwrap();
    }
    let store = Store::open(dir.join("cache")).unwrap();
    let mut pim = account();
    store.save_account_doc("a1", mailcraft_model::pim::PIM_DOC, &pim).unwrap();
    let done = export(&store, "a1", "Mine", &file, NONE, &mut |_| {}).unwrap();
    assert_eq!((done.events, done.contacts, done.notes, done.tasks, done.pim_already, done.pim_replaced), (2, 2, 2, 2, 0, 0), "{done:?}");
    let before = items(&file);
    assert_eq!(subjects(&before, "Calendar"), ["Review", "Standup"]);

    pim.events[0].event.summary = "Standup, moved".into();
    pim.cards[0].card.name = "Ana Lima-Costa".into();
    pim.notes[0].note.text = "Shopping list".into();
    pim.tasks[0].task.title = "Book the big stage".into();
    store.save_account_doc("a1", mailcraft_model::pim::PIM_DOC, &pim).unwrap();
    let done = export(&store, "a1", "Mine", &file, NONE, &mut |_| {}).unwrap();
    assert_eq!((done.events, done.contacts, done.notes, done.tasks, done.pim_already, done.pim_replaced), (1, 1, 1, 1, 4, 4), "{done:?}");
    assert_eq!(format_of(&file), if ansi { Format::Ansi } else { Format::Unicode });

    // One copy of each, the changed ones as they are now; the unchanged ones untouched.
    let after = items(&file);
    assert_eq!(subjects(&after, "Calendar"), ["Review", "Standup, moved"]);
    assert_eq!(subjects(&after, "Contacts"), ["Ana Lima-Costa", "Ben Ruiz"]);
    assert_eq!(subjects(&after, "Notes"), ["Ideas", "Shopping list"]);
    assert_eq!(subjects(&after, "Tasks"), ["Book the big stage", "Return the lenses"]);
    for (folder, kept) in [("Calendar", "Review"), ("Contacts", "Ben Ruiz"), ("Notes", "Ideas"), ("Tasks", "Return the lenses")] {
        assert_eq!(node(&after, folder, kept), node(&before, folder, kept), "{folder}");
    }

    // Read back by the import: each item once, as it is now.
    let back = Store::open(dir.join("back")).unwrap();
    let imported = crate::pst::import(&file, &back, "b1", &mut |_| {}).unwrap();
    assert_eq!((imported.events, imported.contacts, imported.notes, imported.tasks), (2, 2, 2, 2), "{imported:?}");
    let got: Pim = back.load_account_doc("b1", mailcraft_model::pim::PIM_DOC).unwrap();
    let sorted = |mut v: Vec<String>| {
        v.sort();
        v
    };
    assert_eq!(sorted(got.events.iter().map(|e| e.event.summary.clone()).collect()), ["Review", "Standup, moved"]);
    assert_eq!(sorted(got.cards.iter().map(|c| c.card.name.clone()).collect()), ["Ana Lima-Costa", "Ben Ruiz"]);
    assert_eq!(sorted(got.notes.iter().map(|n| n.note.text.clone()).collect()), ["Ideas", "Shopping list"]);
    assert_eq!(sorted(got.tasks.iter().map(|t| t.task.title.clone()).collect()), ["Book the big stage", "Return the lenses"]);

    // Nothing changed since: nothing written, nothing taken out.
    let done = export(&store, "a1", "Mine", &file, NONE, &mut |_| {}).unwrap();
    assert_eq!((done.events, done.contacts, done.notes, done.tasks, done.pim_already, done.pim_replaced), (0, 0, 0, 0, 8, 0), "{done:?}");
    assert_eq!(items(&file), after);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn changed_items_replace_their_earlier_copies() {
    exported_again_in_place("unicode", false);
}

#[test]
fn changed_items_replace_their_earlier_copies_in_an_ansi_file() {
    exported_again_in_place("ansi", true);
}

/// Two accounts exported into one file, each with a meeting of the same uid (the organizer's and
/// the guest's, so each written as its account has it) and a card of the same uid with other
/// fields: exported again, each keeps its own copies, and a change in one takes out only its own.
#[test]
fn two_accounts_in_one_file_keep_their_own_copies() {
    let dir = scratch("two");
    let file = dir.join("Out.pst");
    let store = Store::open(dir.join("cache")).unwrap();
    let person = |email: &str, name: &str, partstat: &str| mailcraft_model::ical::Attendee {
        email: email.into(),
        name: name.into(),
        partstat: partstat.into(),
        rsvp: partstat == "NEEDS-ACTION",
    };
    let meeting = |partstat: &str| StoredEvent {
        calendar: "local".into(),
        event: CalEvent {
            uid: "review@studio.example".into(),
            summary: "Review".into(),
            start: 1_792_000_800,
            end: 1_792_004_400,
            organizer: Some(person("riley@studio.example", "Riley", "")),
            attendees: vec![person("ana@studio.example", "Ana", partstat)],
            ..Default::default()
        },
        ..Default::default()
    };
    let card = |name: &str| StoredCard {
        book: "local".into(),
        card: Card { uid: "shared-card".into(), name: name.into(), ..Default::default() },
        ..Default::default()
    };
    let config = |id: &str, email: &str| mailcraft_model::AccountConfig { id: id.into(), email: email.into(), ..Default::default() };
    store.save_accounts(&[config("a1", "riley@studio.example"), config("a2", "ana@studio.example")]).unwrap();
    let mut one = Pim { events: vec![meeting("NEEDS-ACTION")], cards: vec![card("Sam Ortiz")], ..Default::default() };
    let two = Pim { events: vec![meeting("ACCEPTED")], cards: vec![card("Sam Ortiz-Vega")], ..Default::default() };
    store.save_account_doc("a1", mailcraft_model::pim::PIM_DOC, &one).unwrap();
    store.save_account_doc("a2", mailcraft_model::pim::PIM_DOC, &two).unwrap();
    let counts = |account: &str| {
        let d = export(&store, account, "Both", &file, NONE, &mut |_| {}).unwrap();
        (d.events, d.contacts, d.pim_already, d.pim_replaced)
    };
    assert_eq!(counts("a1"), (1, 1, 0, 0));
    assert_eq!(counts("a2"), (1, 1, 0, 0));
    let both = items(&file);
    assert_eq!(subjects(&both, "Calendar"), ["Review", "Review"]);
    assert_eq!(subjects(&both, "Contacts").len(), 2);

    // Again, each in turn: nothing written, nothing of the other's taken out.
    assert_eq!(counts("a1"), (0, 0, 2, 0));
    assert_eq!(counts("a2"), (0, 0, 2, 0));
    assert_eq!(items(&file), both);

    // The first account's meeting changes: its copy is replaced, the guest's stays.
    one.events[0].event.summary = "Review, moved".into();
    store.save_account_doc("a1", mailcraft_model::pim::PIM_DOC, &one).unwrap();
    assert_eq!(counts("a1"), (1, 0, 1, 1));
    assert_eq!(counts("a2"), (0, 0, 2, 0));
    let after = items(&file);
    assert_eq!(subjects(&after, "Calendar"), ["Review", "Review, moved"]);
    assert_eq!(after.get("Contacts"), both.get("Contacts"));
    let _ = std::fs::remove_dir_all(&dir);
}
