//! Organisation directory search over RFC 9670 JMAP Sharing (`urn:ietf:params:jmap:principals`).

use super::*;

/// Maximum number of principals fetched per directory search.
pub const DIRECTORY_MAX: usize = 100;
/// Longest search query sent to the server.
const MAX_QUERY: usize = 256;

impl<H: Http> Client<H> {
    /// Search the organisation's directory (`Principal/query` + `Principal/get`); returns an
    /// empty list when the JMAP server does not advertise `urn:ietf:params:jmap:principals`.
    pub async fn search_directory(&self, query: &str) -> Result<Vec<(String, String)>> {
        let Some(account) = &self.principals else {
            return Ok(Vec::new());
        };
        let q: String = query.trim().chars().filter(|c| !c.is_control()).take(MAX_QUERY).collect();
        if q.is_empty() {
            return Ok(Vec::new());
        }
        let limit = self.max_get.min(DIRECTORY_MAX);
        let r = self
            .call(
                &[CORE, PRINCIPALS],
                vec![
                    ("Principal/query", json!({"accountId": account, "filter": {"text": q}, "limit": limit})),
                    (
                        "Principal/get",
                        json!({
                            "accountId": account,
                            "#ids": {"resultOf": "c0", "name": "Principal/query", "path": "/ids"},
                            "properties": ["name", "email"],
                        }),
                    ),
                ],
            )
            .await?;
        let mut out: Vec<(String, String)> = Vec::new();
        for p in list(r.get(1), "list") {
            let Some(email) = p.get("email").and_then(Value::as_str).map(str::trim).filter(|e| mailcraft_model::addr::is_valid_email(e)) else {
                continue;
            };
            let name = p.get("name").and_then(Value::as_str).map(str::trim).unwrap_or_default();
            if !out.iter().any(|(_, seen)| seen.eq_ignore_ascii_case(email)) {
                out.push((name.to_string(), email.to_string()));
            }
        }
        Ok(out)
    }
}
