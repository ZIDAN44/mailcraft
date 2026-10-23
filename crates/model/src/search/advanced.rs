//! Advanced search: the search box's query as a form (words, From, To, Subject, the body, and
//! what a message is), and back. What the form has no field for stays as it was written.

use super::*;

/// The fields of the search box's advanced search (issue #4, as Outlook's).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Advanced {
    /// Words anywhere.
    pub words: String,
    pub from: String,
    /// To or Cc.
    pub to: String,
    pub subject: String,
    pub body: String,
    pub attachments: bool,
    pub unread: bool,
    pub flagged: bool,
    pub important: bool,
    /// The rest of the query (`larger:`, `category:`, words left out with `-`, a second
    /// `from:`…), kept as written.
    pub rest: String,
}

impl Advanced {
    /// The form filled from a query.
    pub fn from_query(query: &str) -> Advanced {
        let mut a = Advanced::default();
        let (mut words, mut rest) = (Vec::new(), Vec::new());
        for tok in split_tokens(query).into_iter().take(64) {
            let field = |slot: &mut String, v: &str| {
                if slot.is_empty() && !v.is_empty() {
                    *slot = v.strip_prefix('"').map(|x| x.strip_suffix('"').unwrap_or(x)).unwrap_or(v).trim().to_string();
                    true
                } else {
                    false
                }
            };
            let taken = match tok.split_once(':') {
                Some((k, v)) if !tok.starts_with(['-', '"']) => match (k.to_ascii_lowercase().as_str(), v.to_ascii_lowercase().as_str()) {
                    ("from", _) => field(&mut a.from, v),
                    ("to", _) => field(&mut a.to, v),
                    ("subject", _) => field(&mut a.subject, v),
                    ("body", _) => field(&mut a.body, v),
                    ("has", "attachment" | "attachments") | ("hasattachment" | "hasattachments", "yes" | "true") => {
                        a.attachments = true;
                        true
                    }
                    ("is", "unread") => {
                        a.unread = true;
                        true
                    }
                    ("is", "flagged") => {
                        a.flagged = true;
                        true
                    }
                    ("is", "important") => {
                        a.important = true;
                        true
                    }
                    _ => false,
                },
                Some(_) => false,
                // A word with an unclosed quote stays in the rest (which goes last): written
                // before the fields, it would quote them into itself.
                None if !tok.starts_with('-') && tok.matches('"').count() % 2 == 0 => {
                    words.push(tok.clone());
                    true
                }
                None => false,
            };
            if !taken {
                rest.push(tok);
            }
        }
        a.words = words.join(" ");
        a.rest = rest.join(" ");
        a
    }

    /// The query the form asks: `budget from:ana subject:"q3 plan" has:attachment`. The rest
    /// goes last, so the form's fields are what's read back into them.
    pub fn to_query(&self) -> String {
        let mut out: Vec<String> = Vec::new();
        if !self.words.trim().is_empty() {
            out.push(self.words.trim().to_string());
        }
        for (key, v) in [("from", &self.from), ("to", &self.to), ("subject", &self.subject), ("body", &self.body)] {
            // Quotes can't be inside a quoted value: they're dropped.
            let v: String = v.chars().filter(|c| *c != '"' && !c.is_control()).collect();
            let v = v.trim();
            if v.is_empty() {
                continue;
            }
            // Several words are looked for together, as written; a list (`ana,jun`) as it is.
            if v.contains(char::is_whitespace) {
                out.push(format!("{key}:\"{v}\""));
            } else {
                out.push(format!("{key}:{v}"));
            }
        }
        for (on, term) in
            [(self.attachments, "has:attachment"), (self.unread, "is:unread"), (self.flagged, "is:flagged"), (self.important, "is:important")]
        {
            if on {
                out.push(term.to_string());
            }
        }
        if !self.rest.trim().is_empty() {
            out.push(self.rest.trim().to_string());
        }
        out.join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A query becomes the form and back, what the form can't show kept as it was.
    #[test]
    fn a_query_fills_the_form_and_back() {
        let a = Advanced::from_query(r#"budget review from:ana subject:"q3 plan" -draft larger:1MB has:attachment is:unread from:jun"#);
        assert_eq!(a.words, "budget review");
        assert_eq!(a.from, "ana");
        assert_eq!(a.subject, "q3 plan");
        assert!(a.attachments && a.unread && !a.flagged);
        assert_eq!(a.rest, "-draft larger:1MB from:jun", "a second From, words left out and sizes stay as written");
        let q = a.to_query();
        assert_eq!(q, r#"budget review from:ana subject:"q3 plan" has:attachment is:unread -draft larger:1MB from:jun"#);
        assert_eq!(Advanced::from_query(&q), a, "read back the same");
        // The same search either way, the terms in another order.
        let terms = |q: &str| {
            let mut v: Vec<String> = Query::parse(q).clauses.iter().map(|c| format!("{c:?}")).collect();
            v.sort();
            v
        };
        assert_eq!(terms(&q), terms(r#"budget review from:ana subject:"q3 plan" -draft larger:1MB has:attachment is:unread from:jun"#));
    }

    /// Typed into the form: several words become one phrase, a list stays a list, quotes go.
    #[test]
    fn the_form_asks_what_was_typed() {
        let a = Advanced {
            from: "Ana Park".into(),
            to: "ana,jun".into(),
            body: "say \"hi\"".into(),
            flagged: true,
            important: true,
            ..Default::default()
        };
        let q = a.to_query();
        assert_eq!(q, r#"from:"Ana Park" to:ana,jun body:"say hi" is:flagged is:important"#);
        let parsed = Query::parse(&q);
        assert_eq!(parsed.clauses.first().map(|c| &c.term), Some(&Term::Phrase(Field::From, "ana park".into())));
        assert_eq!(Advanced::default().to_query(), "");
        assert_eq!(Advanced::from_query(""), Advanced::default());
    }

    /// An unclosed quote stays where it was (found by tests/properties.rs): moved ahead of the
    /// rest, it swallowed it and the form changed on every trip.
    #[test]
    fn an_unclosed_quote_stays_in_the_rest() {
        let a = Advanced::from_query(r#"": """ ""#);
        assert_eq!(a.words, "");
        assert_eq!(a.rest, r#"": """ ""#);
        assert_eq!(Advanced::from_query(&a.to_query()), a);
        let b = Advanced::from_query(r#"plan "q3 from:ana"#);
        assert_eq!((b.words.as_str(), b.rest.as_str()), ("plan", r#""q3 from:ana"#));
    }
}
