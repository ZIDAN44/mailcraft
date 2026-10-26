//! Exchange Online's directory (issue #9): the organisation's people, searched as names are typed,
//! with a token of its own ([`crate::oauth::DIRECTORY_SCOPE`]).
//!
//! The directory itself first (`/users`, `User.ReadBasic.All`: everyone, someone who joined
//! today included); where that's refused (a guest, a tenant that hides its users), the people
//! relevant to the user with the directory's included (`/me/people`, `People.Read`).

use super::*;

/// Most people asked for in one search.
const DIRECTORY_TOP: usize = 25;
/// Longest search sent.
const MAX_QUERY: usize = 120;

/// Whether a failure means the token lacks the permission (the user hasn't allowed it, or the
/// tenant took it away), so asking again in the browser is what helps.
pub fn needs_consent(e: &NetError) -> bool {
    match e {
        NetError::Auth(_) => true,
        NetError::No(m) => m.starts_with("HTTP 403"),
        _ => false,
    }
}

/// Everyone in the directory at `base` whose name or address starts with `query`, by the
/// directory's token: (name, address).
pub fn search_directory(base: &str, token: &str, query: &str) -> Result<Vec<(String, String)>> {
    // The search goes inside quotes (OData's $search): quotes and backslashes in it are dropped,
    // and controls never sent.
    let q: String = query.trim().chars().filter(|c| !c.is_control() && !matches!(c, '"' | '\\')).take(MAX_QUERY).collect();
    let q = q.trim();
    if q.is_empty() {
        return Ok(Vec::new());
    }
    let base = base.trim().trim_end_matches('/');
    let url = http::Url::parse(base)?;
    if !url.https && !matches!(url.host.as_str(), "127.0.0.1" | "localhost") {
        return Err(NetError::Tls("the Graph address isn't https".into()));
    }
    let auth = format!("Bearer {token}");
    let search = http::pct(&format!("\"displayName:{q}\" OR \"mail:{q}\""));
    let users =
        Request::get(&format!("{base}/users?$search={search}&$count=true&$top={DIRECTORY_TOP}&$select=displayName,mail,userPrincipalName"), MAX_JSON)
            .header("Authorization", &auth)
            .header("Accept", "application/json")
            // Microsoft's directory takes $search only with this header (and $count).
            .header("ConsistencyLevel", "eventual");
    match answer(&users) {
        Ok(v) => return Ok(people_of(&v, |p| p.get("mail").and_then(Value::as_str).map(str::to_string))),
        // Not allowed to list the directory: the relevant people, the directory's with them.
        Err(NetError::No(m)) if m.starts_with("HTTP 403") => log::info!("directory: {m}; asking the relevant people"),
        Err(e) => return Err(e),
    }
    let people = Request::get(
        &format!("{base}/me/people?$search={}&$top={DIRECTORY_TOP}&$select=displayName,scoredEmailAddresses", http::pct(&format!("\"{q}\""))),
        MAX_JSON,
    )
    .header("Authorization", &auth)
    .header("Accept", "application/json")
    .header("X-PeopleQuery-QuerySources", "Mailbox,Directory");
    let v = answer(&people)?;
    Ok(people_of(&v, |p| {
        p.get("scoredEmailAddresses")?.as_array()?.iter().find_map(|a| a.get("address").and_then(Value::as_str)).map(str::to_string)
    }))
}

fn answer(r: &Request) -> Result<Value> {
    let resp = send_through_limits(r)?;
    check(&resp)?;
    json_body(&resp.body)
}

/// The people in an answer's `value`, each with an address that is one, once, at most
/// [`DIRECTORY_TOP`] of them.
fn people_of(v: &Value, address: impl Fn(&Value) -> Option<String>) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for p in v.get("value").and_then(Value::as_array).into_iter().flatten() {
        let Some(a) = address(p).map(|a| a.trim().to_string()).filter(|a| a.contains('@') && a.len() <= 320 && !a.chars().any(char::is_control))
        else {
            continue;
        };
        if out.iter().any(|(_, o)| o.eq_ignore_ascii_case(&a)) {
            continue;
        }
        let name: String = p.get("displayName").and_then(Value::as_str).unwrap_or("").trim().chars().filter(|c| !c.is_control()).take(256).collect();
        out.push((name, a));
        if out.len() >= DIRECTORY_TOP {
            break;
        }
    }
    out
}
