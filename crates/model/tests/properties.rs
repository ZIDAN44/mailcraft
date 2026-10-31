//! Round trips that must hold for any input, not only the examples the unit tests pick:
//! what MailCraft writes it reads back the same.

use mailcraft_model::search::{Advanced, Query};
use mailcraft_model::vcard::{self, Card, PostalAddress, Typed};
use mailcraft_model::{EmailAddress, ical, lookalike, utf7};
use proptest::prelude::*;

/// Words the search box reads as plain words (no `field:`, no leading `-` or quote).
fn words() -> impl Strategy<Value = String> {
    prop::collection::vec("[a-zA-Z0-9à-ÿ]{1,8}", 0..4).prop_map(|w| w.join(" "))
}

fn advanced() -> impl Strategy<Value = Advanced> {
    (words(), words(), words(), words(), words(), any::<[bool; 4]>()).prop_map(|(w, from, to, subject, body, b)| Advanced {
        words: w,
        from,
        to,
        subject,
        body,
        attachments: b[0],
        unread: b[1],
        flagged: b[2],
        important: b[3],
        rest: String::new(),
    })
}

/// Text a person types into a card's field: any characters, lines included.
fn field() -> impl Strategy<Value = String> {
    "[^\r\u{0}]{0,40}".prop_map(|s| s.trim().to_string())
}

fn typed(value: &'static str) -> impl Strategy<Value = Typed> {
    (value, prop::sample::select(vec!["", "work", "home", "cell"])).prop_map(|(value, kind)| Typed { value, kind: kind.to_string() })
}

fn card() -> impl Strategy<Value = Card> {
    (
        "[a-z0-9-]{1,20}",
        "[^\r\n\u{0}]{0,30}".prop_filter("a name", |s| !s.trim().is_empty()).prop_map(|s| s.trim().to_string()),
        (field(), field(), field(), field(), field()),
        prop::collection::vec(typed(r"[a-z0-9.+-]{1,10}@[a-z]{1,8}\.[a-z]{2,3}"), 0..3),
        prop::collection::vec(typed(r"\+?[0-9 ]{3,15}"), 0..3),
        prop::collection::vec((field(), field(), field(), field(), field()), 0..2),
        prop::sample::select(vec!["", "1990-04-01", "--12-31"]),
    )
        .prop_map(|(uid, name, (given, family, org, title, note), emails, phones, addresses, birthday)| Card {
            uid,
            name,
            given: given.replace('\n', " "),
            family: family.replace('\n', " "),
            emails: emails.into_iter().map(|t| Typed { value: t.value.trim().to_string(), kind: t.kind }).collect(),
            phones: phones.into_iter().map(|t| Typed { value: t.value.trim().to_string(), kind: t.kind }).collect(),
            org: org.replace('\n', " "),
            title: title.replace('\n', " "),
            note,
            addresses: addresses
                .into_iter()
                .map(|(street, city, region, postcode, country)| PostalAddress {
                    kind: "home".into(),
                    street,
                    city: city.replace('\n', " "),
                    region: region.replace('\n', " "),
                    postcode: postcode.replace('\n', " "),
                    country: country.replace('\n', " "),
                })
                .filter(|a| !a.is_empty())
                .collect(),
            birthday: birthday.to_string(),
            extra: Vec::new(),
        })
}

proptest! {
    /// The Advanced search form becomes a query and comes back the same.
    #[test]
    fn advanced_search_form_round_trips(a in advanced()) {
        prop_assert_eq!(Advanced::from_query(&a.to_query()), a);
    }

    /// Whatever was typed in the search box, the form it fills writes a query that fills the
    /// same form again, and asks the same search.
    #[test]
    fn advanced_search_settles_after_one_trip(q in r#"[a-z:" -]{0,40}|\PC{0,40}"#) {
        let once = Advanced::from_query(&q);
        let twice = Advanced::from_query(&once.to_query());
        prop_assert_eq!(&Advanced::from_query(&twice.to_query()), &twice);
        let _ = Query::parse(&once.to_query());
    }

    /// A contact written as a vCard reads back as the same contact.
    #[test]
    fn vcard_round_trips(c in card()) {
        let text = vcard::write(&c);
        let back = vcard::parse(&text);
        prop_assert_eq!(back.len(), 1, "{}", text);
        prop_assert_eq!(&back[0], &c, "{}", text);
    }

    /// Any folder name survives IMAP's modified UTF-7, and the wire form is ASCII.
    #[test]
    fn utf7_round_trips(name in r"\PC{0,40}") {
        let wire = utf7::encode(&name);
        prop_assert!(wire.is_ascii());
        prop_assert_eq!(utf7::decode(&wire), name);
    }

    /// An online meeting added to any event and taken out again leaves its place and notes as
    /// they were (issue #25), and the event written and read back in between still has it.
    #[test]
    fn an_online_meeting_comes_out_as_it_went_in(notes in "[^\r\u{0}]{0,80}", place in "[^\r\n\u{0}]{0,30}", code in "[0-9-]{0,12}") {
        let room = ical::MeetingRoom::new("Room", "https://meet.example.edu/rooms/abc-def-ghi", &code, "").unwrap();
        let before = ical::CalEvent { uid: "u".into(), description: notes.clone(), location: place.clone(), start: 1_800_000_000, end: 1_800_003_600, ..Default::default() };
        let mut e = before.clone();
        e.add_online_meeting(&room, &ical::MeetingLabels::default());
        let mut back = ical::parse(&ical::write(std::slice::from_ref(&e), None, 1_800_000_000), 0).events.remove(0);
        prop_assert_eq!(back.online_meeting(), Some(room.url.clone()));
        back.remove_online_meeting();
        prop_assert_eq!(back.description.as_str(), notes.trim_end());
        prop_assert_eq!(back.location.trim(), place.trim());
        prop_assert!(back.conferences().is_empty());
    }

    /// iCalendar text values: escaped and unescaped, the same text.
    #[test]
    fn ical_text_round_trips(text in "[^\r]{0,60}") {
        prop_assert_eq!(ical::unescape(&ical::escape(&text)), text);
    }

    /// A person's name compares the same whichever order its words come in, "Last, First"
    /// included (issue #29), and any name and address is checked without fault.
    #[test]
    fn known_names_compare_in_any_order(first in "[a-zA-Zà-ÿ]{2,12}", last in "[a-zA-Zà-ÿ]{2,12}", junk in r"\PC{0,80}") {
        let key = lookalike::name_key(&format!("{first} {last}"));
        prop_assert_eq!(&key, &lookalike::name_key(&format!("{last}, {first}")));
        prop_assert_eq!(&key, &lookalike::name_key(&format!("  {} {}  ", last.to_uppercase(), first)));
        let mut people = lookalike::KnownPeople::new();
        people.add(&EmailAddress::new(format!("{first} {last}"), "them@work.example"));
        let from = EmailAddress::new(format!("{last}, {first}"), "them.too@gmail.com");
        prop_assert_eq!(people.check(&from, false).is_some(), key.is_some());
        let _ = people.check(&EmailAddress::new(junk.clone(), junk.clone()), false);
        let _ = lookalike::name_key(&junk);
    }
}
