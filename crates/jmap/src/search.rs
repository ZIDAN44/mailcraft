//! The search box's query as a JMAP `Email/query` filter.

use super::*;

/// The search box's query as a JMAP `Email/query` filter (RFC 8621 §4.4.1), for mail past what's
/// kept (issue #4): words anywhere are `text`, `from:` `from` and so on, `is:unread` no `$seen`,
/// a leading `-` `NOT`. What JMAP can't ask (importance, categories) is left to the list; `None`
/// when nothing is left to ask.
pub fn search_filter(query: &str) -> Option<Value> {
    use mailcraft_model::search::{Field, Query, Term};
    let q = Query::parse(query);
    let mut conds = Vec::new();
    let field = |f: &Field| match f {
        Field::Any => "text",
        Field::From => "from",
        Field::To => "to",
        Field::Subject => "subject",
        Field::Body => "body",
    };
    let keyword = |k: &str, has: bool| if has { json!({"hasKeyword": k}) } else { json!({"notKeyword": k}) };
    for c in q.clauses.iter().take(16) {
        let not = |v: Value| if c.negated { json!({"operator": "NOT", "conditions": [v]}) } else { v };
        match &c.term {
            Term::Words(f, words) => {
                for w in words.iter().filter(|w| !w.is_empty()).take(8) {
                    conds.push(not(json!({ field(f): w.chars().take(200).collect::<String>() })));
                }
            }
            Term::Phrase(f, p) if !p.is_empty() => conds.push(not(json!({ field(f): p.chars().take(200).collect::<String>() }))),
            Term::Unread(on) => conds.push(keyword("$seen", *on == c.negated)),
            Term::Flagged(on) => conds.push(keyword("$flagged", *on != c.negated)),
            Term::HasAttachment(on) => conds.push(json!({"hasAttachment": *on != c.negated})),
            // Bigger than: minSize is "at least", so one byte more.
            Term::Size(true, b) => conds.push(not(json!({"minSize": b.saturating_add(1)}))),
            Term::Size(false, b) => conds.push(not(json!({"maxSize": b}))),
            Term::AnyOf(any) => {
                let one: Vec<Value> = any
                    .iter()
                    .filter_map(|t| match t {
                        Term::Words(f, w) if !w.is_empty() => Some(json!({ field(f): w.join(" ").chars().take(200).collect::<String>() })),
                        Term::Phrase(f, p) if !p.is_empty() => Some(json!({ field(f): p.chars().take(200).collect::<String>() })),
                        _ => None,
                    })
                    .take(16)
                    .collect();
                if one.len() == any.len() && !one.is_empty() {
                    conds.push(not(json!({"operator": "OR", "conditions": one})));
                }
            }
            Term::Keyword(k)
                if !k.is_empty() && k.len() < 100 && k.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '$' | '_' | '-' | '.')) =>
            {
                conds.push(keyword(k, !c.negated));
            }
            _ => {}
        }
    }
    (!conds.is_empty()).then(|| json!({"operator": "AND", "conditions": conds}))
}
