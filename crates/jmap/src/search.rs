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
        // JMAP has no filter for the files' names: the text, which holds them (and the body).
        Field::Attachment => "text",
    };
    let keyword = |k: &str, has: bool| if has { json!({"hasKeyword": k}) } else { json!({"notKeyword": k}) };
    // `-attachment:x` as NOT text x would also leave out mail that only mentions x.
    for c in q.clauses.iter().filter(|c| !(c.negated && c.term.asks_names())).take(16) {
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
            // Any keyword atom (RFC 8621 §4.1.1 allows what IMAP does): Thunderbird's `to=20do`
            // too (issue #28).
            Term::Keyword(k) if mailcraft_model::is_keyword(k) => conds.push(keyword(k, !c.negated)),
            // Complete and not flagged again (asked negated, it's checked here).
            Term::Complete if !c.negated => {
                conds.extend([keyword(mailcraft_model::FOLLOW_UP_COMPLETE, true), keyword("$flagged", false)]);
            }
            _ => {}
        }
    }
    (!conds.is_empty()).then(|| json!({"operator": "AND", "conditions": conds}))
}

impl<H: Http> Client<H> {
    /// The newest `limit` ids in a mailbox that `filter` matches.
    pub(super) async fn search(&self, mailbox: &str, filter: &Value, limit: usize) -> Result<Vec<String>> {
        let filter = json!({"operator": "AND", "conditions": [{"inMailbox": mailbox}, filter]});
        let r = self
            .mail(vec![(
                "Email/query",
                json!({"accountId": self.account, "filter": filter, "sort": [{"property": "receivedAt", "isAscending": false}], "limit": limit.min(FOUND_MAX)}),
            )])
            .await?;
        Ok(list(r.first(), "ids").iter().filter_map(Value::as_str).map(str::to_string).take(limit).collect())
    }

    /// Of these messages, those still in a mailbox, with their keywords.
    pub(super) async fn still_in(&self, mailbox: &str, ids: &[String]) -> Result<Vec<(String, Flags)>> {
        let mut out = Vec::new();
        for chunk in ids.chunks(self.max_get.clamp(1, 200)) {
            let r = self
                .mail(vec![("Email/get", json!({"accountId": self.account, "ids": chunk, "properties": ["id", "keywords", "mailboxIds"]}))])
                .await?;
            out.extend(
                list(r.first(), "list")
                    .iter()
                    .filter(|e| e.get("mailboxIds").and_then(|m| m.get(mailbox)) == Some(&Value::Bool(true)))
                    .filter_map(|e| Some((e.get("id")?.as_str()?.to_string(), keywords(e.get("keywords"))))),
            );
        }
        Ok(out)
    }
}

/// The messages of `folder` the server finds for `filter` ([`search_filter`]), the newest `cap`
/// of them: those not kept yet come into its list, and stay there past the window while the
/// server has them there (issue #4). Their keys, and how many came in. A folder never synced is
/// left to its sync.
pub async fn find_on_server<H: Http>(
    c: &Client<H>,
    st: &mut State,
    store: &Store,
    account: &str,
    folder: &str,
    filter: &Value,
    cap: usize,
) -> Result<(Vec<MessageKey>, usize)> {
    let mailbox = st.mailbox(folder)?.to_string();
    let mut fs = store.load_folder(account, folder)?;
    if fs.validity != VALIDITY {
        return Ok((Vec::new(), 0));
    }
    let ids = c.search(&mailbox, filter, cap.min(FOUND_MAX)).await?;
    let numbers: Vec<u32> = ids.iter().map(|id| st.number(id)).collect();
    let missing: Vec<String> = ids.iter().zip(&numbers).filter(|(_, n)| fs.find(**n).is_none()).map(|(id, _)| id.clone()).collect();
    let mut new = 0;
    for e in c.headers(&missing).await? {
        let Some(id) = e.get("id").and_then(Value::as_str) else { continue };
        let key = MessageKey { account: account.to_string(), folder: folder.to_string(), validity: VALIDITY, uid: st.number(id) };
        if fs.find(key.uid).is_none() {
            fs.messages.push(email_summary(&e, key).into());
            new += 1;
        }
    }
    st.add_found(folder, &numbers);
    if new > 0 {
        sort_newest_first(&mut fs.messages);
        fs.uid_next = fs.messages.iter().map(|m| m.key.uid).max().unwrap_or(0).saturating_add(1);
        store.save_folder(account, folder, &fs)?;
    }
    Ok((numbers.iter().filter_map(|n| fs.find(*n).map(|m| m.key.clone())).collect(), new))
}
