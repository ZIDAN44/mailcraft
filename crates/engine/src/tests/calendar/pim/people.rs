//! The address book: people made, changed, chosen and shared, the contact someone is saved as,
//! and the address book and notes over a lot of them.

use super::*;

/// The contact an address above a message is saved as: by the whole address, case and spaces
/// aside; of two with it, the first by name, as the People pane lists them; a search that only
/// contains it (Joann for Ann) isn't one.
#[test]
fn a_contact_is_found_by_its_whole_address() {
    let mut s = sample_session("contact-by-address");
    for (name, email) in [("Zed Ann", "ann@x.example"), ("Bea Ann", "Ann@X.example"), ("Joann", "joann@x.example")] {
        s.run("contact.create", &json!({"name": name, "emails": email})).unwrap();
    }
    let name = |s: &Session, e: &str| s.contact_by_address(e).map(|c| c.display());
    assert_eq!(name(&s, " ANN@x.example "), Some("Bea Ann".to_string()));
    assert_eq!(name(&s, "joann@x.example"), Some("Joann".to_string()));
    assert_eq!(name(&s, "nn@x.example"), None);
    assert_eq!(name(&s, "  "), None);
    // As the search of People finds them.
    let found: Vec<String> = s.people("ann@x").iter().map(|p| p.card.display()).collect();
    assert_eq!(found, ["Bea Ann", "Joann", "Zed Ann"]);
}

/// What a frame asks of the address book and the notes, timed: the people above a message (50
/// names, each looked up by address as the reading pane does), the People pane (everyone, A to
/// Z) and the Notes pane searched (its count, then its notes). 5,000 contacts and 1,000 notes of
/// about 2 KB. Run under the heavy lock:
/// `cargo test --release -p mailcraft-engine --lib measure_people_frame -- --ignored --nocapture`.
#[test]
#[ignore = "a measurement: run in release"]
fn measure_people_frame() {
    use mailcraft_model::pim::{StoredCard, StoredNote};
    use mailcraft_model::vcard::{Card, Typed};
    let mut s = sample_session("measure-people");
    let p = s.pim_mut("sample");
    let book = p.books.first().map(|b| b.id.clone()).unwrap_or_default();
    for i in 0..5000 {
        let card = Card {
            uid: format!("c{i}"),
            name: format!("Person {i}"),
            org: "Studio".into(),
            emails: vec![Typed { value: format!("person{i}@example.org"), ..Default::default() }],
            phones: vec![Typed { value: format!("+1 555 {i:04}"), ..Default::default() }],
            ..Default::default()
        };
        p.cards.push(StoredCard { book: book.clone(), card, ..Default::default() });
    }
    let text = "Notes from the review, kept to see a search's cost. ".repeat(40);
    for i in 0..1000 {
        let note = mailcraft_model::pim::Note { uid: format!("n{i}"), text: format!("Note {i}\n{text}"), ..Default::default() };
        p.notes.push(StoredNote { note, ..Default::default() });
    }
    let header: Vec<String> = (0..50).map(|i| format!("Person{}@Example.org", i * 97)).collect();
    let frame = |s: &Session| {
        let known = header.iter().filter(|e| s.contact_by_address(e).is_some()).count();
        known + s.people("").len() + s.notes("review").len() + s.notes("review").len()
    };
    let n = frame(&s);
    let t = std::time::Instant::now();
    for _ in 0..20 {
        assert_eq!(frame(&s), n);
    }
    println!("MEASURE people and notes per frame ({n}): {:.2} ms", t.elapsed().as_secs_f64() * 1000.0 / 20.0);
}

#[test]
fn people_in_the_address_book() {
    let mut s = sample_session("people");
    let all = s.run("contact.list", &json!({})).unwrap();
    assert!(all.as_array().unwrap().len() >= 5);
    let hits = s.run("contact.list", &json!({"query": "lantern"})).unwrap();
    assert!(hits.as_array().unwrap().iter().all(|p| p["org"] == "Studio Lantern"));
    let uid = s
        .run("contact.create", &json!({"name": "Mira Chen", "emails": "mira@colour.example, mira.chen@home.example", "org": "Colour House"}))
        .unwrap()["uid"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(s.run("contact.create", &json!({"name": "Bad", "emails": ["not an address"]})).is_err());
    assert!(s.run("contact.create", &json!({})).is_err(), "a name or address is needed");
    // Saved people are suggested before people only seen in mail.
    let top: Vec<String> = s.suggest_contacts("mi", 2).into_iter().map(|c| c.address.email).collect();
    assert!(top.contains(&"mira@colour.example".to_string()), "{top:?}");
    s.run("contact.update", &json!({"uid": uid, "phones": ["+1 555 0101"], "title": "Colourist"})).unwrap();
    let p = s.find_person(&uid).unwrap();
    assert_eq!((p.card.phones[0].value.as_str(), p.card.title.as_str(), p.card.emails.len()), ("+1 555 0101", "Colourist", 2));
    // People are lent, not copied: a card can carry megabytes of picture and files, and People
    // lists everyone on every frame.
    let stored = s.pim.values().flat_map(|x| &x.cards).find(|c| c.card.uid == uid).map(|c| &c.card).unwrap();
    assert!(std::ptr::eq(p.card, stored) && s.people("Mira").iter().any(|x| std::ptr::eq(x.card, stored)));
    s.run("contact.delete", &json!({"uid": uid})).unwrap();
    assert!(s.find_person(&uid).is_none());
    // The sender of a message, once.
    s.run("folder.open", &json!({"account": "sample", "folder": "Junk"})).unwrap();
    let k = s.list.iter().find(|m| m.from.email == "win@prizes.example").map(|m| m.key.clone()).expect("the sample's prize mail");
    s.run("list.open", &json!({"key": k.to_id()})).unwrap();
    let first = s.run("contact.fromMessage", &json!({})).unwrap();
    let second = s.run("contact.fromMessage", &json!({})).unwrap();
    assert_eq!(first["uid"], second["uid"]);
    assert_eq!(second["existing"], true);
}

/// Issue #1: a contact's numbers keep what they are (business, home, mobile…), and its
/// addresses and birthday are fields of their own, as in Outlook.
#[test]
fn contacts_keep_typed_numbers_addresses_and_birthdays() {
    let mut s = sample_session("contact-fields");
    let uid = s
        .run(
            "contact.create",
            &json!({
                "name": "Jos Peeters",
                "emails": ["jos@company.example", "jos@home.example"],
                "phones": [{"value": "+32 2 123 45 67", "kind": "work"}, {"value": "+32 490 12 34 56", "kind": "cell"}, {"value": "+32 9 765 43 21", "kind": "home"}],
                "addresses": [{"kind": "work", "street": "An street 1", "postcode": "1000", "city": "Brussel", "country": "Belgium"}, {"kind": "home", "city": "Gent"}, {"street": " "}],
                "birthday": "1980-02-29",
            }),
        )
        .unwrap()["uid"]
        .as_str()
        .unwrap()
        .to_string();
    let c = s.find_person(&uid).unwrap().card;
    let kinds: Vec<(&str, &str)> = c.phones.iter().map(|p| (p.kind.as_str(), p.value.as_str())).collect();
    assert_eq!(kinds, [("work", "+32 2 123 45 67"), ("cell", "+32 490 12 34 56"), ("home", "+32 9 765 43 21")]);
    assert_eq!(c.addresses.len(), 2, "an empty one isn't kept");
    assert_eq!(c.addresses[0].one_line(), "An street 1, 1000 Brussel, Belgium");
    assert_eq!(c.birthday, "1980-02-29");
    assert_eq!(c.emails.len(), 2);
    // Changing one thing leaves the rest; a birthday that isn't a date is refused.
    s.run("contact.update", &json!({"uid": uid, "title": "Owner"})).unwrap();
    assert_eq!(s.find_person(&uid).unwrap().card.phones.len(), 3);
    assert!(s.run("contact.update", &json!({"uid": uid, "birthday": "next tuesday"})).is_err());
    s.run("contact.update", &json!({"uid": uid, "birthday": ""})).unwrap();
    assert!(s.find_person(&uid).unwrap().card.birthday.is_empty());
    // Strings still work for phones (kind unknown).
    s.run("contact.update", &json!({"uid": uid, "phones": "+1 555 0100, +1 555 0101"})).unwrap();
    assert_eq!(s.find_person(&uid).unwrap().card.phones.len(), 2);
}

/// Issue #1: Select Names opens with the address book chosen (Outlook's "Show this address list
/// first"), and one address book can be shown alone.
#[test]
fn the_address_book_opens_with_the_one_chosen() {
    let mut s = sample_session("default-book");
    let all = s.run("contact.addressBook", &json!({})).unwrap();
    let people = s.run("contact.addressBook", &json!({"source": "people"})).unwrap();
    assert!(all.as_array().unwrap().len() >= people.as_array().unwrap().len());
    let books = s.address_books();
    let (book, _, _) = books.first().cloned().expect("the sample has an address book");
    assert!(s.run("contact.defaultBook", &json!({"source": "book:nobody:x"})).is_err());
    s.run("contact.defaultBook", &json!({"source": book})).unwrap();
    assert_eq!(s.prefs.address_book, book);
    let mine = s.run("contact.addressBook", &json!({})).unwrap();
    assert_eq!(mine, s.run("contact.addressBook", &json!({"source": book})).unwrap(), "it opens with the one chosen");
    assert!(!mine.as_array().unwrap().is_empty());
    let (account, id) = book.strip_prefix("book:").and_then(|b| b.split_once(':')).unwrap();
    let in_book = s.people("").into_iter().filter(|p| p.account == account && p.book == id).flat_map(|p| &p.card.emails).count();
    assert_eq!(mine.as_array().unwrap().len(), in_book);
    let back: crate::session::Prefs = serde_json::from_value(serde_json::to_value(&s.prefs).unwrap()).unwrap();
    assert_eq!(back.address_book, book, "kept with the settings");
}

/// Issue #1: an organisation's shared contacts (Exchange public folders) can be read, not
/// changed or deleted.
#[test]
fn shared_contacts_are_read_only() {
    use mailcraft_model::pim::{BookInfo, DavSync, StoredCard};
    let mut s = sample_session("shared-contacts");
    let card = mailcraft_model::vcard::Card { uid: "desk".into(), name: "Front desk".into(), ..Default::default() };
    let mut books = s.pim.get("sample").unwrap().books.clone();
    books.push(BookInfo { id: "public:pc".into(), name: "Company contacts".into(), read_only: true, ctag: String::new() });
    let sync = DavSync {
        books: Some(books),
        cards: vec![StoredCard { book: "public:pc".into(), href: "public:desk".into(), card, ..Default::default() }],
        synced_books: vec!["public:pc".into()],
        ..Default::default()
    };
    s.apply(Event::DavSynced { account: "sample".into(), sync: Box::new(sync) });
    assert!(s.find_person("desk").is_some(), "shown with the others");
    assert!(s.person_read_only("desk"));
    assert!(s.run("contact.update", &json!({"uid": "desk", "name": "Reception"})).is_err());
    assert!(s.run("contact.delete", &json!({"uid": "desk"})).is_err());
    assert!(s.find_person("desk").is_some());
    // New contacts go to a book that can take them.
    let made = s.run("contact.create", &json!({"name": "New one"})).unwrap();
    assert!(!s.person_read_only(made["uid"].as_str().unwrap()));
}
