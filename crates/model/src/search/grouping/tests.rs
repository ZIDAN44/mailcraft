use super::*;
use crate::{EmailAddress, Flags};

fn sample() -> Summary {
    Summary {
        subject: "Project kickoff: storyboards".into(),
        from: EmailAddress::new("Zoe Park", "zoe@studio.example"),
        to: vec![EmailAddress::new("Ann", "ann@studio.example")],
        has_attachments: true,
        flags: Flags { seen: false, flagged: true, ..Default::default() },
        ..Default::default()
    }
}

fn eval(s: &str) -> Tri {
    let m = sample();
    let st = SummaryText::of(&m);
    Query::parse(s).eval(&Doc { summary: &m, summary_text: &st, awaiting: false, files: None, text: &"" })
}

fn q(s: &str) -> bool {
    eval(s) == Tri::Yes
}

fn clause(s: &str) -> Clause {
    Query::parse(s).clauses.remove(0)
}

#[test]
fn or_matches_either_side_across_fields() {
    assert!(q("from:ann OR subject:kickoff"));
    assert!(q("subject:budget OR is:flagged"));
    assert!(!q("from:ann OR subject:budget"));
    // OR binds closer than the terms around it: a and (b or c).
    assert!(q("from:zoe subject:budget OR is:unread"));
    assert!(!q("from:ann subject:kickoff OR is:unread"));
    assert!(q("from:ann OR from:jun OR to:ann"), "a chain of ORs is one");
}

#[test]
fn groups_nest_and_negate() {
    assert!(q("(from:ann is:unread) OR (from:zoe has:attachment)"));
    assert!(!q("(from:ann is:unread) OR (from:zoe -has:attachment)"));
    assert!(q("-(from:ann OR subject:budget)"));
    assert!(!q("-(from:zoe subject:kickoff)"));
    assert!(q("-(from:zoe subject:budget)"), "not both");
    assert!(q("from:zoe (subject:budget OR (is:flagged (to:ann OR to:jun)))"));
    assert!(!q("from:zoe (subject:budget OR (is:read (to:ann OR to:jun)))"));
    // A negated term in an OR.
    assert!(q("-is:flagged OR from:zoe") && !q("-is:flagged OR from:ann"));
}

#[test]
fn what_doesnt_pair_up_reads_as_before() {
    // A group never closed ends with the query; a `)` with none open stays in its word.
    assert_eq!(Query::parse("(from:zoe is:unread").clauses, Query::parse("from:zoe is:unread").clauses);
    assert_eq!(clause("smile)").term, Term::Words(Field::Any, vec!["smile".into()]));
    assert_eq!(Query::parse("\"smile :)\"").clauses.len(), 1);
    // `OR` with nothing before it is the word, with nothing after it nothing; `or` is a word.
    assert_eq!(Query::parse("OR zoe").clauses.len(), 2);
    assert_eq!(Query::parse("zoe OR").clauses, Query::parse("zoe").clauses);
    assert_eq!(Query::parse("zoe or ann").clauses.len(), 3);
    // Empty groups are nothing, a group of one is that one.
    assert!(Query::parse("()").is_empty());
    assert_eq!(clause("(from:zoe)"), clause("from:zoe"));
    assert_eq!(clause("-(-from:zoe)"), clause("from:zoe"));
}

#[test]
fn shapes() {
    let any = |ts: Vec<Term>| Term::AnyOf(ts);
    let zoe = Term::Words(Field::From, vec!["zoe".into()]);
    let kick = Term::Words(Field::Subject, vec!["kickoff".into()]);
    assert_eq!(clause("from:zoe OR subject:kickoff").term, any(vec![zoe.clone(), kick.clone()]));
    assert_eq!(clause("-from:zoe OR subject:kickoff").term, any(vec![Term::All(vec![Clause { term: zoe.clone(), negated: true }]), kick.clone()]));
    let group = clause("-(from:zoe subject:kickoff)");
    assert!(group.negated);
    assert_eq!(group.term, Term::All(vec![Clause { term: zoe, negated: false }, Clause { term: kick, negated: false }]));
}

#[test]
fn depth_is_capped() {
    let deep = format!("{}zoe{}", "(".repeat(40), ")".repeat(40));
    let parsed = Query::parse(&deep);
    assert_eq!(parsed.clauses.len(), 1);
    fn depth(t: &Term) -> usize {
        match t {
            Term::All(cs) => 1 + cs.iter().map(|c| depth(&c.term)).max().unwrap_or(0),
            Term::AnyOf(ts) => 1 + ts.iter().map(depth).max().unwrap_or(0),
            _ => 0,
        }
    }
    assert!(depth(&parsed.clauses[0].term) <= MAX_DEPTH);
}

#[test]
fn dates_and_categories_inside_groups_are_reached() {
    let mut query = Query::parse("(received:2024 OR category:Work) is:unread");
    let mut seen = 0;
    query.each_term_mut(|t| {
        if matches!(t, Term::Date(_) | Term::Keyword(_)) {
            seen += 1;
        }
    });
    assert_eq!(seen, 2);
    // A category's name is put through for a server inside brackets too.
    let kw = |name: &str| (name == "Work").then(|| "$label2".to_string());
    assert_eq!(Query::with_keywords("(category:Work OR -category:Work)", kw), "(keyword:$label2 OR -keyword:$label2)");
    // A `)` with no group open is part of the word, as the search reads it.
    assert_eq!(Query::with_keywords("category:Work)", kw), "category:Work)");
}

#[test]
fn highlights_skip_what_a_group_leaves_out() {
    let sought = Query::parse("(subject:kickoff -subject:budget) OR body:scene").sought(&Field::Subject);
    assert_eq!(sought, vec![vec!["kickoff".to_string()]]);
}
