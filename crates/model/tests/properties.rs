//! Round trips that must hold for any input, not only the examples the unit tests pick:
//! what MailCraft writes it reads back the same.

use mailcraft_model::dates::Clock;
use mailcraft_model::job::{Eta, ImportProgress, Work, eta};
use mailcraft_model::search::{Advanced, Doc, MAX_TOKEN_CHARS, Query, SummaryText, fold_word, tokens};
use mailcraft_model::vcard::{self, Card, PostalAddress, Typed};
use mailcraft_model::{EmailAddress, Summary, ical, lookalike, thread, utf7};
use proptest::prelude::*;

/// Words the search box reads as plain words (no `field:`, no leading `-` or quote).
fn words() -> impl Strategy<Value = String> {
    prop::collection::vec("[a-zA-Z0-9à-ÿ]{1,8}", 0..4).prop_map(|w| w.join(" "))
}

/// A day as the Advanced form's date fields hold it, or none.
fn day() -> impl Strategy<Value = String> {
    prop_oneof![Just(String::new()), (1970u32..=9999, 1u32..=12, 1u32..=28).prop_map(|(y, m, d)| format!("{y:04}-{m:02}-{d:02}"))]
}

fn advanced() -> impl Strategy<Value = Advanced> {
    (words(), words(), words(), words(), words(), any::<[bool; 4]>(), day(), day()).prop_map(|(w, from, to, subject, body, b, after, before)| {
        Advanced {
            words: w,
            from,
            to,
            subject,
            body,
            attachments: b[0],
            unread: b[1],
            flagged: b[2],
            important: b[3],
            received_after: after,
            received_before: before,
            rest: String::new(),
        }
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

/// A message with an id, references and a conversation index drawn from small sets, so they
/// meet (and loop, and repeat).
fn threaded() -> impl Strategy<Value = Summary> {
    let id = || "[a-e]?".prop_map(|s| if s.is_empty() { s } else { format!("<{s}@x>") });
    let index = (0u8..3, proptest::collection::vec(0u8..3, 0..4), any::<bool>()).prop_map(|(guid, blocks, odd)| {
        let mut v = vec![1, 2, 3, 4, 5, 6];
        v.extend([guid; 16]);
        blocks.iter().for_each(|&b| v.extend([b; 5]));
        if odd {
            v.push(0);
        }
        v
    });
    (id(), proptest::collection::vec(id(), 0..4), id(), "(Re: )?[xy]?", 0i64..10, prop_oneof![Just(0i64), 0i64..20], index).prop_map(
        |(message_id, references, in_reply_to, subject, date, received, index)| {
            let mut m = Summary { message_id, references, in_reply_to, subject, date, received, ..Default::default() };
            m.set_thread_index(&index);
            m
        },
    )
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

    /// Folding a word twice changes nothing more than once, whatever its letters.
    #[test]
    fn folding_is_idempotent(w in r"\PC{0,30}|[a-zA-Zà-ɏͰ-ϿḀ-ỿἀ-῿！-～Ѐ-ӿ\u{300}-\u{36f}]{0,20}") {
        let once = fold_word(&w).into_owned();
        prop_assert_eq!(fold_word(&once).into_owned(), once.clone());
        for t in tokens(&w) {
            prop_assert!(!t.is_empty() && t.chars().count() <= MAX_TOKEN_CHARS);
            prop_assert_eq!(fold_word(&t).into_owned(), t.to_string());
        }
    }

    /// Whatever is typed, reading it, placing its dates on any clock and checking it against
    /// any summary never panics.
    #[test]
    fn any_query_reads_and_checks(q in r#"[a-z:"<>=./ 0-9-]{0,40}|\PC{0,60}"#, now in any::<i64>(), offset in any::<i32>(), subject in r"\PC{0,30}") {
        let mut query = Query::parse(&q);
        query.resolve(&Clock::new(now, offset));
        let who = mailcraft_model::EmailAddress::new(subject.clone(), "ana@studio.example");
        let m = mailcraft_model::Summary { preview: subject.clone(), from: who.clone(), to: vec![who.clone()], cc: vec![who], subject, date: now, ..Default::default() };
        let st = SummaryText::of(&m);
        let whole = query.eval(&Doc { summary: &m, summary_text: &st, awaiting: false, files: None, text: &q.as_str() });
        // Only the parts the query looks in, folded: the same answer.
        let some = SummaryText::for_query(&m, &query);
        prop_assert_eq!(query.eval(&Doc { summary: &m, summary_text: &some, awaiting: false, files: None, text: &q.as_str() }), whole);
        let _ = (query.text_words(), query.index_words(), query.needs_text());
        prop_assert!(query.index_words().iter().all(|w| !w.is_empty()));
    }

    /// Accents typed alone, a field's value or not, ask the index for no empty word (which would
    /// offer every message) and match no summary for sure.
    #[test]
    fn marks_alone_are_no_words(marks in r"[\u{300}-\u{36f}\u{345}]{1,6}", field in "|from:|to:|subject:|body:|attachment:", subject in "[a-zA-Zà-ÿ ]{0,30}") {
        let query = Query::parse(&format!("{field}{marks}"));
        prop_assert!(query.index_words().iter().all(|w| !w.is_empty()));
        prop_assert!(query.text_words().iter().all(|(_, w, _)| !w.is_empty()));
        let m = mailcraft_model::Summary { subject, ..Default::default() };
        let st = SummaryText::of(&m);
        prop_assert_ne!(query.eval(&Doc { summary: &m, summary_text: &st, awaiting: false, files: None, text: &"anything at all" }), mailcraft_model::search::Tri::Yes);
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

    /// Whatever an import reports (times that go on, work that never goes back, totals up to
    /// the limit and past it), the time left is unknown or 1 s to the most ever said, and the
    /// share done never goes back nor reaches the end.
    #[test]
    fn an_import_estimate_stays_in_range(
        steps in prop::collection::vec((0u64..5_000, any::<u64>(), 0u64..3_000), 1..60),
        total in prop_oneof![Just(0u64), Just(u64::MAX), any::<u64>(), 1u64..1_000_000],
        bytes_total in prop_oneof![Just(0u64), Just(u64::MAX), any::<u64>()],
    ) {
        let mut eta = Eta::default();
        let mut p = ImportProgress { work: Work { total, bytes_total, ..Default::default() }, ..Default::default() };
        let (mut now, mut last) = (0u64, None);
        for (dt, more, ui_dt) in steps {
            p.run_ms = p.run_ms.saturating_add(dt);
            p.work.done = p.work.done.saturating_add(more % 1_000);
            p.work.bytes_done = p.work.bytes_done.saturating_add(more);
            now = now.saturating_add(ui_dt);
            eta.update(&p, now);
            for at in [now, now.saturating_add(10_000), now.saturating_add(40_000)] {
                prop_assert!(eta.remaining_s(at).is_none_or(|s| (1..=eta::MAX_S).contains(&s)));
            }
            let permille = p.permille();
            prop_assert!(permille.is_none_or(|m| m <= 999));
            prop_assert!(last <= permille, "{last:?} then {permille:?}");
            last = permille;
        }
    }

    /// Every message lands in exactly one conversation, whatever its ids, conversation index
    /// and dates; conversations come newest first, each oldest first.
    #[test]
    fn every_message_is_threaded_once(msgs in proptest::collection::vec(threaded(), 0..40)) {
        let t = thread::threads(&msgs);
        let mut all: Vec<usize> = t.iter().flat_map(|t| t.messages.iter().copied()).collect();
        all.sort_unstable();
        prop_assert_eq!(all, (0..msgs.len()).collect::<Vec<_>>());
        for c in &t {
            let whens: Vec<i64> = c.messages.iter().map(|&i| msgs[i].when()).collect();
            prop_assert!(whens.windows(2).all(|w| w[0] <= w[1]));
            prop_assert_eq!(whens.iter().copied().max(), Some(c.latest));
        }
        prop_assert!(t.windows(2).all(|w| w[0].latest >= w[1].latest));
    }

    /// A reply whose ids name only a message not listed (the original kept under another id)
    /// joins the message its conversation index answers, whatever else is in the folder.
    #[test]
    fn a_reply_to_an_unlisted_id_joins_its_index_parent(mut msgs in proptest::collection::vec(threaded(), 0..40), refs in any::<bool>()) {
        let root = [vec![1, 2, 3, 4, 5, 6], vec![9; 16]].concat();
        let mut original = Summary { message_id: "<original@y>".into(), subject: "Plan".into(), ..Default::default() };
        original.set_thread_index(&root);
        let mut reply = Summary { message_id: "<reply@y>".into(), subject: "Other words".into(), date: 30, ..Default::default() };
        if refs {
            reply.references = vec!["<gone@y>".into(), "<absent@y>".into()];
        } else {
            reply.in_reply_to = "<absent@y>".into();
        }
        reply.set_thread_index(&[root, vec![7; 5]].concat());
        let (o, r) = (msgs.len(), msgs.len() + 1);
        msgs.extend([original, reply]);
        let t = thread::threads(&msgs);
        prop_assert!(t.iter().any(|c| c.messages.contains(&o) && c.messages.contains(&r)));
    }
}
