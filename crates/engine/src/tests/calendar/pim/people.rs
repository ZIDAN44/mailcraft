//! The address book and notes over a lot of them, and the contact someone is saved as.

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
#[ignore]
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
