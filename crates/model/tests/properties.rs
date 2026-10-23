//! Round trips that must hold for any input, not only the examples the unit tests pick:
//! what MailCraft writes it reads back the same.

use mailcraft_model::search::{Advanced, Query};
use mailcraft_model::vcard::{self, Card, PostalAddress, Typed};
use mailcraft_model::{ical, utf7};
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

    /// iCalendar text values: escaped and unescaped, the same text.
    #[test]
    fn ical_text_round_trips(text in "[^\r]{0,60}") {
        prop_assert_eq!(ical::unescape(&ical::escape(&text)), text);
    }
}
