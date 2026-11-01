//! What the user types in the search box (and saved searches read back): the query language,
//! its dates placed on any clock, the query checked against a message and its text, folding,
//! the Advanced search form, the server-side forms of a search, and IMAP's modified UTF-7. The
//! same text taken as a sender's name and address, checked for passing as someone else (a
//! look-alike domain, a known person's name on another address) and for coming from outside.
#![no_main]

use libfuzzer_sys::fuzz_target;
use mailcraft_model::dates::Clock;
use mailcraft_model::search::{Advanced, Doc, IndexDoc, Query, SummaryText, fold_word, tokens};
use mailcraft_model::{lookalike, utf7};

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let mut q = Query::parse(&text);
    let _ = q.index_words();
    let _ = q.text_words();
    let _ = q.needs_text();
    // A clock read from the input too, so dates are placed at the ends of time.
    let now = data.get(..8).and_then(|b| b.try_into().ok()).map_or(0, i64::from_le_bytes);
    q.resolve(&Clock::new(now, 3600));
    let m = mailcraft_model::Summary { subject: text.to_string(), date: now, ..Default::default() };
    let st = SummaryText::of(&m);
    let body = IndexDoc { text: text.to_string(), names: vec![text.to_string()], files: 1, truncated: data.len() % 2 == 0, has_text: true };
    let _ = q.eval(&Doc { summary: &m, summary_text: &st, awaiting: false, files: None, text: &body });
    let _ = q.eval(&Doc { summary: &m, summary_text: &st, awaiting: true, files: Some(true), text: &&*text });
    let _ = tokens(&fold_word(&text)).count();
    let a = Advanced::from_query(&text);
    let _ = Advanced::from_query(&a.to_query());
    let _ = mailcraft_jmap::search_filter(&text);
    let _ = utf7::encode(&utf7::decode(&text));
    let (name, email) = text.split_once('\n').unwrap_or((&text, &text));
    let known = ["paypal.com".to_string(), "studio.example".to_string(), lookalike::domain_of(email)].into();
    let _ = lookalike::check_sender(&mailcraft_model::EmailAddress::new(name, email), &known);
    let _ = lookalike::skeleton(email);
    let mut people = lookalike::KnownPeople::new();
    people.add(&mailcraft_model::EmailAddress::new("Jane Director", "jane@studio.example"));
    people.add(&mailcraft_model::EmailAddress::new(email, name));
    let check = lookalike::SenderCheck::new(known, people, [name.to_string()]);
    let from = mailcraft_model::EmailAddress::new(name, email);
    let _ = check.check(&from, false);
    let _ = check.check(&from, true);
    let _ = lookalike::name_key(name);
    let _ = lookalike::is_free_mail(email);
    let org = lookalike::clean_domain(name).into_iter().collect();
    let _ = lookalike::is_external(email, &org);
});
