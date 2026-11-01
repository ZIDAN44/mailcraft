use std::collections::BTreeSet;

use super::*;
use crate::dates::Clock;
use crate::dates::zone::Zone;
use crate::{EmailAddress, Flags};

fn sample() -> Summary {
    Summary {
        subject: "Project kickoff: storyboards".into(),
        from: EmailAddress::new("Zoe Park", "zoe@studio.example"),
        to: vec![EmailAddress::new("Ann", "ann@studio.example")],
        preview: "Bring the sketches for the opening scene".into(),
        has_attachments: true,
        flags: Flags { seen: false, flagged: true, ..Default::default() },
        ..Default::default()
    }
}

/// A query against a summary and a text (`""`: nothing besides the summary).
fn eval(query: &Query, m: &Summary, text: &dyn TextSource) -> Tri {
    let st = SummaryText::of(m);
    query.eval(&Doc { summary: m, summary_text: &st, awaiting: false, files: None, text })
}

fn q(s: &str, m: &Summary) -> bool {
    eval(&Query::parse(s), m, &"") == Tri::Yes
}

/// A text the index knows has every word looked for.
struct Found;

impl TextSource for Found {
    fn word(&self, _: TextField, _: &str) -> Tri {
        Tri::Yes
    }

    fn phrase(&self, _: TextField, _: &[String], _: &str) -> Tri {
        Tri::Unknown
    }
}

/// Whitespace between `field:` and its value is tolerated.
#[test]
fn whitespace_between_field_and_value() {
    let m = sample();
    assert_eq!(Query::parse("from: zoe").clauses, Query::parse("from:zoe").clauses);
    assert_eq!(Query::parse("FROM:   \"zoe park\"  -is:  read").clauses, Query::parse("from:\"zoe park\" -is:read").clauses);
    assert!(q("from: zoe", &m) && !q("from: ann", &m) && q("to: ann", &m) && q("-is: read", &m));
    // Unknown prefixes stay plain words, and so does a trailing `from:`.
    assert_eq!(Query::parse("note: zoe").clauses.len(), 2);
    assert_eq!(Query::parse("zoe from:").clauses.len(), 2);
    assert!(Query::parse("from:\t\n").clauses.len() == 1);
}

/// Sizes, and lists matching any of their names.
#[test]
fn sizes_and_lists() {
    let mut m = sample();
    m.size = 300 * 1024;
    assert!(q("larger:100KB", &m) && q("larger:100kb", &m) && !q("larger:1MB", &m) && q("smaller:0.5mb", &m) && !q("smaller:300K", &m));
    assert!(q("-larger:1MB", &m) && q("larger:307199", &m) && !q("larger:307200", &m));
    // Not sizes: plain words, never a crash.
    for junk in ["larger:", "larger:big", "larger:-5", "larger:1e999MB", "larger:NaN", "smaller:12XB", "larger:99999999999999999999GB"] {
        let _ = q(junk, &m);
    }
    assert_eq!(Query::parse("larger:99999999999999999999GB").clauses[0].term, Term::Size(true, 1024u64.pow(4)));
    assert!(q("from:ana,zoe", &m) && q("from:\"zoe park\",ann", &m) && !q("from:ana,jun", &m) && q("to:x,ann", &m));
    assert!(!q("-from:ana,zoe", &m) && q("from:zoe,", &m) && q("from:,zoe", &m));
    assert_eq!(Query::parse("from:,,").clauses.len(), 1, "commas alone: a phrase to look for");
    assert!(matches!(&Query::parse("from:a,b,c").clauses[0].term, Term::AnyOf(v) if v.len() == 3));
}

#[test]
fn parsing() {
    let p = Query::parse("from:zoe \"opening scene\" -is:read has:attachment");
    assert_eq!(p.clauses.len(), 4);
    assert_eq!(p.clauses[0].term, Term::Words(Field::From, vec!["zoe".into()]));
    assert_eq!(p.clauses[1].term, Term::Phrase(Field::Any, "opening scene".into()));
    assert!(p.clauses[2].negated);
    assert!(Query::parse("   ").is_empty());
    assert_eq!(Query::parse("subject:\"a b\"").clauses[0].term, Term::Phrase(Field::Subject, "a b".into()));
    assert_eq!(Query::parse("attachment:report,plan").clauses[0].term, {
        Term::AnyOf(vec![Term::Words(Field::Attachment, vec!["report".into()]), Term::Words(Field::Attachment, vec!["plan".into()])])
    });
    assert_eq!(Query::parse("attachments:\"q3 plan\"").clauses[0].term, Term::Phrase(Field::Attachment, "q3 plan".into()));
}

#[test]
fn matching() {
    let m = sample();
    assert!(q("proj", &m));
    assert!(q("from:zoe", &m));
    assert!(!q("from:ann", &m));
    assert!(q("to:ann", &m));
    assert!(q("subject:kickoff is:unread is:flagged", &m));
    assert!(q("hasattachments:yes", &m));
    assert!(!q("is:read", &m));
    assert!(q("\"opening scene\"", &m));
    assert!(!q("\"scene opening\"", &m));
    assert!(q("-from:ann", &m));
    assert!(q("zoe@studio.example", &m));
    assert!(q("body:sketches", &m));
    assert!(!q("is:important", &m) && !q("is:categorized", &m));
    let mut m2 = m.clone();
    m2.importance = 1;
    m2.flags.keywords = vec!["$Pinned".into(), "work".into()];
    assert!(q("is:important is:categorized", &m2));
    let label = Query { clauses: vec![Clause { term: Term::Categorized(vec!["$label1".into()]), negated: false }], problems: Vec::new() };
    assert_eq!(eval(&label, &m2, &""), Tri::Yes, "a keyword not in the list counts");
    m2.flags.keywords = vec!["$Pinned".into(), "$Forwarded".into(), "NonJunk".into(), "$MailFlagBit1".into()];
    assert_eq!(eval(&label, &m2, &""), Tri::No, "system keywords don't");
    m2.flags.keywords.push("$LABEL1".into());
    assert_eq!(eval(&label, &m2, &""), Tri::Yes);
    assert_eq!(eval(&Query::parse(""), &m, &""), Tri::Yes);
}

/// The body's text and the files' names answer what the summary doesn't, each where asked.
#[test]
fn the_text_answers_body_and_names() {
    let m = sample();
    let doc = IndexDoc {
        text: "The budget for the shoot".into(),
        names: vec!["Q3 Plan.pdf".into()],
        files: 1,
        truncated: false,
        parts_cut: false,
        has_text: true,
    };
    let t = |s: &str| eval(&Query::parse(s), &m, &doc);
    assert_eq!(t("budget"), Tri::Yes);
    assert_eq!(t("body:budget"), Tri::Yes);
    assert_eq!(t("attachment:budget"), Tri::No, "names only");
    assert_eq!(t("attachment:plan"), Tri::Yes);
    assert_eq!(t("attachment:\"q3 plan\""), Tri::Yes);
    assert_eq!(t("plan"), Tri::Yes, "words anywhere are looked for in the names too");
    assert_eq!(t("body:plan"), Tri::No);
    assert_eq!(t("\"for the shoot\""), Tri::Yes);
    assert_eq!(t("\"the shoot for\""), Tri::No);
    assert_eq!(t("-budget"), Tri::No);
    assert_eq!(t("-invoice"), Tri::Yes);
    assert_eq!(t("from:budget"), Tri::No, "the summary's fields don't read the text");
    assert_eq!(eval(&Query::parse("body:budget"), &m, &""), Tri::No, "without the text, the preview alone");
}

/// An address is found inside the preview's words, as in the rest of the summary, when the
/// text isn't there to look in.
#[test]
fn an_address_in_the_preview() {
    let mut m = sample();
    m.preview = "Write to <ana@studio.example> about it".into();
    assert!(q("ana@studio.example", &m));
    assert!(q("body:ana@studio.example", &m));
    assert!(q("\"ana@studio.example\"", &m));
    assert!(!q("jun@studio.example", &m));
}

/// `has:attachment` asks the text's files first (pictures shown in it aren't), then the summary.
#[test]
fn files_known_from_the_text_come_first() {
    let mut m = sample();
    m.has_attachments = true;
    let st = SummaryText::of(&m);
    let query = Query::parse("has:attachment");
    let doc = |files| Doc { summary: &m, summary_text: &st, awaiting: false, files, text: &"" };
    assert_eq!(query.eval(&doc(Some(false))), Tri::No, "only a picture the text shows");
    assert_eq!(query.eval(&doc(Some(true))), Tri::Yes);
    assert_eq!(query.eval(&doc(None)), Tri::Yes, "not read yet: the summary's mark");
}

/// Kleene's logic: a no settles an AND, a yes an OR, and NOT turns yes and no round.
#[test]
fn three_valued_logic() {
    use Tri::{No, Unknown, Yes};
    for (a, b, and, or) in [
        (Yes, Yes, Yes, Yes),
        (Yes, No, No, Yes),
        (Yes, Unknown, Unknown, Yes),
        (No, No, No, No),
        (No, Unknown, No, Unknown),
        (Unknown, Unknown, Unknown, Unknown),
    ] {
        assert_eq!((a.and(b), b.and(a), a.or(b), b.or(a)), (and, and, or, or), "{a:?} {b:?}");
    }
    assert_eq!((!Yes, !No, !Unknown), (No, Yes, Unknown));
}

/// What isn't in an index's postings is a no, unless the text it was made from was cut short or
/// had none: then a word looked for, or left out with `-`, can't be told, and only a summary
/// clause that fails settles it.
#[test]
fn a_truncated_or_textless_doc_is_unknown_when_a_word_is_missing() {
    let m = sample(); // unread
    let whole = IndexDoc { text: "storyboard notes".into(), has_text: true, ..Default::default() };
    let cut = IndexDoc { truncated: true, ..whole.clone() };
    let files_only = IndexDoc { text: String::new(), names: vec!["scan.pdf".into()], files: 1, truncated: false, parts_cut: false, has_text: false };
    let t = |s: &str, d: &IndexDoc| eval(&Query::parse(s), &m, d);
    assert_eq!((t("budget", &whole), t("-budget", &whole)), (Tri::No, Tri::Yes));
    assert_eq!((t("budget", &cut), t("-budget", &cut)), (Tri::Unknown, Tri::Unknown), "past the cut, it may be there");
    assert_eq!((t("budget", &files_only), t("-budget", &files_only)), (Tri::Unknown, Tri::Unknown));
    // AND: another clause's no settles it, a yes doesn't.
    assert_eq!(t("budget is:read", &cut), Tri::No);
    assert_eq!(t("-budget is:read", &files_only), Tri::No);
    assert_eq!(t("budget is:unread", &cut), Tri::Unknown);
    assert_eq!(t("storyboard budget", &cut), Tri::Unknown);
    assert_eq!(t("storyboard", &cut), Tri::Yes, "found is found");
    assert_eq!(t("-storyboard", &cut), Tri::No);
    // A word the summary has needs no text.
    assert_eq!(t("kickoff -zoe", &cut), Tri::No);
    assert_eq!(t("kickoff", &files_only), Tri::Yes);
    // Names: cut short, a name may be missing; no text says nothing about names.
    assert_eq!(t("attachment:report", &cut), Tri::Unknown);
    assert_eq!((t("attachment:scan", &files_only), t("attachment:report", &files_only)), (Tri::Yes, Tri::No));
    assert_eq!(indexed(false, TextField::Body, false, true), Tri::No);
}

#[test]
fn index_narrows_without_missing() {
    let a = sample();
    let mut b = sample();
    b.subject = "Invoice October".into();
    b.preview = "Payment due".into();
    let mut ix = Index::new();
    ix.add(0, &summary_text(&a));
    ix.add(1, &summary_text(&b));
    assert_eq!(ix.candidates(&Query::parse("invo")), BTreeSet::from([1]));
    assert_eq!(ix.candidates(&Query::parse("zoe proj")), BTreeSet::from([0]));
    assert_eq!(ix.candidates(&Query::parse("is:unread")).len(), 2);
    assert!(ix.candidates(&Query::parse("nothing-here")).is_empty());
    ix.add(1, "late body text");
    assert_eq!(ix.candidates(&Query::parse("late")), BTreeSet::from([1]));
}

/// The index keeps each word's documents sorted and once, whatever order they come in; plain
/// words it found are settled, the summary alone doesn't have them.
#[test]
fn the_index_settles_plain_words() {
    let mut ix = Index::new();
    ix.add(5, "storyboard review");
    ix.add(2, "storyboard storyboard");
    ix.add(9, "budget");
    ix.add(5, "storyboard again");
    assert_eq!(ix.prefix("story").into_iter().collect::<Vec<_>>(), [2, 5]);
    let query = Query::parse("storyboard review");
    assert_eq!(ix.candidates(&query).into_iter().collect::<Vec<_>>(), [5]);
    assert!(query.needs_text());
    let m = Summary { subject: "Weekly".into(), ..Default::default() };
    assert_eq!(eval(&query, &m, &Found), Tri::Yes, "the index found both words");
    assert_eq!(eval(&query, &m, &""), Tri::No, "without it, the words aren't in the summary");
    assert_eq!(eval(&Query::parse("\"storyboard review\""), &m, &Found), Tri::Unknown, "a phrase is checked in the text");
    for needs in ["\"storyboard review\"", "-budget storyboard", "body:storyboard", "zoe@studio.example", "attachment:plan"] {
        assert!(Query::parse(needs).needs_text(), "{needs}");
    }
    for not in ["from:zoe", "is:unread larger:1MB", "subject:weekly", "received:2024 to:ann"] {
        assert!(!Query::parse(not).needs_text(), "{not}");
    }
}

/// The words asked of the text, folded, with where and whether they must be missing.
#[test]
fn text_words_are_folded_and_placed() {
    let words = Query::parse("Résumé -body:draft attachment:\"Q3 plan\" from:ana").text_words();
    assert_eq!(
        words,
        [
            (TextField::Body, "resume".to_string(), false),
            (TextField::Names, "resume".to_string(), false),
            (TextField::Body, "draft".to_string(), true),
            (TextField::Names, "q3".to_string(), false),
            (TextField::Names, "plan".to_string(), false),
        ]
    );
    assert_eq!(Query::parse("Ürün \"Ünlü kişi\" -x").index_words(), ["urun", "unlu", "kisi"]);
}

/// An address is split into the words the index holds of it: with another word it finds
/// what it finds alone, as the summaries do.
#[test]
fn addresses_narrow_by_the_words_the_index_holds() {
    assert_eq!(Query::parse("from:zoe@studio.example").clauses[0].term, Term::Words(Field::From, vec!["zoe".into(), "studio.example".into()]));
    let m = sample();
    let mut ix = Index::new();
    ix.add(0, &summary_text(&m));
    ix.add(0, "the storyboard draft");
    ix.add(1, "budget from someone@else.example");
    for q in ["from:zoe@studio.example", "from:zoe@studio.example storyboard", "zoe@studio.example"] {
        assert_eq!(ix.candidates(&Query::parse(q)), BTreeSet::from([0]), "{q}");
    }
    assert!(ix.candidates(&Query::parse("from:zoe@studio.example budget")).is_empty());
    assert!(q("from:zoe@studio.example", &m));
    assert!(q("from:dio.example", &m) && q("to:udio.example", &m), "inside the address as written");
    assert!(!q("from:dio.exampel", &m));
    assert_eq!(eval(&Query::parse("someone@else.example"), &m, &"budget from <someone@else.example>"), Tri::Yes);
}

#[test]
fn categories_asked_by_keyword() {
    let k = |v: &str| (v == "Work" || v == "To do").then(|| if v == "Work" { "$label2".to_string() } else { "$label4".to_string() });
    assert_eq!(Query::with_keywords("category:Work  -tag:\"To do\" plan", k), "keyword:$label2 -keyword:$label4 plan");
    assert_eq!(Query::with_keywords("from:ana  category:Other", k), "from:ana  category:Other");
    // What gives no keyword atom is left as typed.
    assert_eq!(Query::with_keywords("category:x", |_| Some("two words".into())), "category:x");
    assert_eq!(Query::with_keywords("category: Work", k), "keyword:$label2");
    let q = Query::parse(&Query::with_keywords("-category:Work", k));
    assert_eq!(q.clauses.first().map(|c| (&c.term, c.negated)), Some((&Term::Keyword("$label2".into()), true)));
}

/// Folding: accents go both ways, letters are spelled out, other scripts stay as they are.
#[test]
fn folding_reads_accents_and_letters() {
    let f = |w: &str| fold_word(w).into_owned();
    assert_eq!((f("é"), f("É"), f("e")), ("e".into(), "e".into(), "e".into()));
    assert_eq!((f("Straße"), f("STRASSE")), ("strasse".into(), "strasse".into()));
    assert_eq!((f("Æsir"), f("Øresund"), f("łódź"), f("Œuvre")), ("aesir".into(), "oresund".into(), "lodz".into(), "oeuvre".into()));
    assert_eq!((f("ıstanbul"), f("İstanbul"), f("ISTANBUL")), ("istanbul".into(), "istanbul".into(), "istanbul".into()));
    assert_eq!((f("Άθήνα"), f("ΑΘΗΝΑ"), f("λόγος"), f("ΛΟΓΟΣ")), ("αθηνα".into(), "αθηνα".into(), "λογοσ".into(), "λογοσ".into()));
    assert_eq!((f("ёлка"), f("йод")), ("елка".into(), "йод".into()), "ё folds, й stays");
    assert_eq!(f("Ｒｅｐｏｒｔ２０２４"), "report2024", "fullwidth letters and digits");
    assert_eq!(f("Re\u{301}sume\u{301}"), "resume", "accents written apart (NFD)");
    // Bengali and CJK pass through unchanged.
    assert_eq!((f("বাংলা"), f("東京")), ("বাংলা".into(), "東京".into()));
    assert!(matches!(fold_word("plain"), std::borrow::Cow::Borrowed(_)), "lower-case ASCII isn't copied");
    // Through the query: both ways round.
    let mut m = sample();
    m.subject = "Résumé für Zoë".into();
    assert!(q("resume", &m) && q("RÉSUMÉ", &m) && q("fur zoe", &m) && q("subject:\"resume fur\"", &m));
    m.subject = "resume".into();
    assert!(q("résumé", &m), "an accent typed finds the word written without");
    assert_eq!(tokens("a\u{301}bc ").collect::<Vec<_>>(), ["abc"]);
    let mut out = String::new();
    fold_text("Grüße, Ana! 東京", &mut out);
    assert_eq!(out, "grusse ana 東 京");
    assert_eq!(tokens(&"x".repeat(41)).count(), 0, "base64 debris isn't a word");
}

fn berlin(now: i64) -> Clock {
    Clock::new(now, 3600).in_zone(Zone::named("Europe/Berlin"))
}

fn range(q: &str, clock: &Clock) -> Option<(i64, i64)> {
    let mut query = Query::parse(q);
    query.resolve(clock);
    match query.clauses.first().map(|c| &c.term) {
        Some(Term::Date(spec)) => spec.range,
        _ => None,
    }
}

/// UTC seconds of a local Berlin wall-clock time.
fn local(y: i64, m: u32, d: u32, clock: &Clock) -> i64 {
    clock.midnight(crate::dates::days_from_civil(y, m, d))
}

/// `received:2024` is the year in the user's zone: from its first local midnight to the next
/// year's.
#[test]
fn received_is_a_period_in_the_local_zone() {
    let c = berlin(1_718_000_000); // June 2024
    assert_eq!(range("received:2024", &c), Some((1_704_063_600, 1_735_686_000)), "2024-01-01 00:00 and 2025-01-01 00:00, CET");
    assert_eq!(range("received:2024-03", &c), Some((local(2024, 3, 1, &c), local(2024, 4, 1, &c))));
    assert_eq!(range("sent:2024/03/15", &c), Some((local(2024, 3, 15, &c), local(2024, 3, 16, &c))));
    assert_eq!(range("date:20240315", &c), range("received:2024-03-15", &c));
    // Summer time starts on 31 March 2024: that day is 23 hours long.
    let (start, end) = range("received:2024-03-31", &c).unwrap_or_default();
    assert_eq!(end - start, 23 * 3600);
    let (start, end) = range("received:2024-10-27", &c).unwrap_or_default();
    assert_eq!(end - start, 25 * 3600, "and it ends on 27 October");
    assert_eq!(start, 1_729_980_000, "midnight there is still summer time (UTC+2)");
    // Ranges, comparisons, after and before.
    assert_eq!(range("received:2024-01-01..2024-03-31", &c), Some((local(2024, 1, 1, &c), local(2024, 4, 1, &c))));
    assert_eq!(range("received:2024-03..", &c), Some((local(2024, 3, 1, &c), i64::MAX)));
    assert_eq!(range("received:..2024", &c), Some((i64::MIN, local(2025, 1, 1, &c))));
    assert_eq!(range("received:>=2024-03-01", &c), Some((local(2024, 3, 1, &c), i64::MAX)));
    assert_eq!(range("received:>2024-03", &c), Some((local(2024, 4, 1, &c), i64::MAX)));
    assert_eq!(range("received:<2024-03-01", &c), Some((i64::MIN, local(2024, 3, 1, &c))));
    assert_eq!(range("received:<=2024-03", &c), Some((i64::MIN, local(2024, 4, 1, &c))));
    assert_eq!(range("after:2024-03-01", &c), Some((local(2024, 3, 1, &c), i64::MAX)));
    assert_eq!(range("before:2024-03-01", &c), Some((i64::MIN, local(2024, 3, 1, &c))));
    // Matching: inside, at the ends, turned round.
    let mut m = sample();
    let check = |s: &str, m: &Summary| {
        let mut query = Query::parse(s);
        query.resolve(&c);
        eval(&query, m, &"")
    };
    m.date = local(2024, 1, 1, &c);
    assert_eq!((check("received:2024", &m), check("received:2023", &m), check("-received:2024", &m)), (Tri::Yes, Tri::No, Tri::No));
    m.date = local(2025, 1, 1, &c) - 1;
    assert_eq!((check("received:2024", &m), check("before:2025", &m), check("after:2025", &m)), (Tri::Yes, Tri::Yes, Tri::No));
    assert_eq!(eval(&Query::parse("received:2024"), &m, &""), Tri::Unknown, "not placed in a zone yet: can't tell");
    // Sent late in 2023, it came in 2024: `received:` (and after/before) go by when it came,
    // `sent:` and `date:` by its Date:.
    m.date = local(2024, 1, 1, &c) - 3600;
    m.received = local(2024, 1, 1, &c) + 3600;
    assert_eq!((check("received:2024", &m), check("after:2024", &m), check("before:2024", &m)), (Tri::Yes, Tri::Yes, Tri::No));
    assert_eq!((check("sent:2024", &m), check("sent:2023", &m), check("date:2023", &m)), (Tri::No, Tri::Yes, Tri::Yes));
    assert!(!Query::parse("received:2024").needs_text());
}

/// `last week` is Monday to Monday, whether today is a Monday or a Sunday.
#[test]
fn weeks_months_and_days_from_today() {
    let monday = berlin(1_718_013_600); // Monday 2024-06-10 12:00 CEST
    let sunday = berlin(1_718_532_000); // Sunday 2024-06-16 12:00 CEST
    for c in [monday, sunday] {
        assert_eq!(range("received:\"last week\"", &c), Some((local(2024, 6, 3, &c), local(2024, 6, 10, &c))));
        assert_eq!(range("received:lastweek", &c), range("received:last-week", &c));
        assert_eq!(range("received:thisweek", &c), Some((local(2024, 6, 10, &c), local(2024, 6, 17, &c))));
        assert_eq!(range("received:lastmonth", &c), Some((local(2024, 5, 1, &c), local(2024, 6, 1, &c))));
        assert_eq!(range("received:thisyear", &c), range("received:2024", &c));
        assert_eq!(range("received:lastyear", &c), range("received:2023", &c));
    }
    assert_eq!(range("received:today", &monday), Some((local(2024, 6, 10, &monday), local(2024, 6, 11, &monday))));
    assert_eq!(range("received: yesterday", &sunday), Some((local(2024, 6, 15, &sunday), local(2024, 6, 16, &sunday))));
    let january = berlin(1_704_200_000); // 2024-01-02
    assert_eq!(range("received:lastmonth", &january), range("received:2023-12", &january));
}

/// A date that isn't one is left out and said, never searched as words.
#[test]
fn junk_dates_are_problems() {
    for junk in [
        "received:2024-13",
        "received:2024-02-30",
        "received:99999999999",
        "received:..",
        "received:>",
        "received:1969",
        "received:yesterdays",
        "after:2024-00-10",
        "before:>=2024",
        "received:2024-01-01..2024-99",
        "received:24",
    ] {
        let query = Query::parse(junk);
        assert!(query.clauses.is_empty(), "{junk}: {:?}", query.clauses);
        assert_eq!(query.problems, [Problem::Date(junk.to_string())], "{junk}");
    }
    let query = Query::parse("budget -received:2024-13 from:ana");
    assert_eq!(
        (query.clauses.len(), query.problems, Query::parse("received:2024").problems),
        (2, vec![Problem::Date("received:2024-13".into())], vec![])
    );
    // A value not meant as a date is still the words typed (the problem said all the same):
    // these weren't fields before, and prose after them was searched whole.
    for (typed, words) in [
        ("budget before: launch", vec!["budget", "before", "launch"]),
        ("meeting date: friday", vec!["meeting", "date", "friday"]),
        ("sent: from the phone", vec!["sent", "from", "the", "phone"]),
        ("after:party", vec!["after", "party"]),
        ("received:soon", vec!["received", "soon"]),
    ] {
        let query = Query::parse(typed);
        let found: Vec<String> = query
            .clauses
            .iter()
            .flat_map(|c| match &c.term {
                Term::Words(Field::Any, ws) => ws.clone(),
                other => panic!("{typed}: {other:?}"),
            })
            .collect();
        assert_eq!((found, query.problems.len()), (words.iter().map(|w| w.to_string()).collect(), 1), "{typed}");
    }
    let mut m = sample();
    m.subject = "After party photos".into();
    assert!(q("after:party", &m));
    // Leap days are days.
    assert!(Query::parse("received:2024-02-29").problems.is_empty() && !Query::parse("received:2023-02-29").problems.is_empty());
    assert!(dates::is_day("2024-03-15") && dates::is_day("2024/3/5") && !dates::is_day("2024-03") && !dates::is_day("today"));
    // Clocks at the ends of time place them without a crash.
    for now in [i64::MIN, -1, 0, i64::MAX] {
        for q in ["received:today", "received:lastmonth", "received:lastyear", "received:2024", "before:thisweek"] {
            let _ = range(q, &Clock::new(now, 14 * 3600));
            let _ = range(q, &berlin(now));
        }
    }
}

/// A word that folds to nothing (a combining mark alone, which Unicode counts a word) asks the
/// index for nothing: as an empty prefix it offered every message, and matched them all.
#[test]
fn a_word_folding_to_nothing_is_no_word() {
    let mut index = Index::new();
    index.add(1, "alpha");
    index.add(2, "beta");
    for typed in ["\u{345}", "\u{301}\u{345}", "body:\u{345}", "from:\u{345},\u{301}"] {
        let query = Query::parse(typed);
        assert!(query.index_words().iter().all(|w| !w.is_empty()), "{typed}");
        let m = Summary { subject: "x".into(), ..Default::default() };
        assert_ne!(eval(&query, &m, &"anything"), Tri::Yes, "{typed}");
    }
    // Beside other words it's no word at all: they alone narrow and decide.
    let query = Query::parse("beta \u{345}");
    assert_eq!((query.index_words(), index.candidates(&query)), (vec!["beta".to_string()], BTreeSet::from([2])));
    let m = Summary { subject: "Beta\u{345} notes".into(), ..Default::default() };
    assert_eq!(eval(&Query::parse("beta\u{345}"), &m, &""), Tri::Yes);
}
