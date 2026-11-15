//! The fake's organisation directory (issue #9): `/users` and `/me/people`, searched.

use super::*;

impl Data {
    /// A search of the directory: `/users` (which wants `ConsistencyLevel: eventual` with a
    /// search), or the relevant people (the directory's only when asked for).
    pub(super) fn directory_answer(&self, parts: &[&str], query: &str) -> Answer {
        let header = |k: &str| self.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(k)).map(|(_, v)| v.clone()).unwrap_or_default();
        let search = crate::oauth::unpct(&param(query, "$search").unwrap_or_default());
        // The words searched for: what's inside the quotes, `displayName:`/`mail:` dropped.
        let words: Vec<String> = search
            .split('"')
            .skip(1)
            .step_by(2)
            .map(|w| w.rsplit(':').next().unwrap_or("").trim().to_lowercase())
            .filter(|w| !w.is_empty())
            .collect();
        let hit = |n: &str, a: &str| {
            let (n, a) = (n.to_lowercase(), a.to_lowercase());
            words.iter().any(|w| a.starts_with(w.as_str()) || n.split(' ').any(|p| p.starts_with(w.as_str())))
        };
        let top: usize = param(query, "$top").and_then(|t| t.parse().ok()).unwrap_or(10);
        let found: Vec<&(String, String)> = self.directory.iter().filter(|(n, a)| hit(n, a)).take(top).collect();
        if parts == ["users"] {
            if self.users_forbidden {
                return forbidden();
            }
            if !search.is_empty() && header("ConsistencyLevel") != "eventual" {
                return error(
                    400,
                    "Request_UnsupportedQuery",
                    "Request with $search query parameter only works through MSGraph with a special request header: 'ConsistencyLevel: eventual'",
                );
            }
            let value: Vec<Value> = found.iter().map(|(n, a)| json!({"displayName": n, "mail": a, "userPrincipalName": a})).collect();
            return ok(json!({"@odata.count": value.len(), "value": value}));
        }
        // The relevant people: the directory's only when asked for.
        let with_directory = header("X-PeopleQuery-QuerySources").contains("Directory");
        let value: Vec<Value> = found
            .iter()
            .filter(|_| with_directory)
            .map(|(n, a)| json!({"displayName": n, "scoredEmailAddresses": [{"address": a, "relevanceScore": 8}], "personType": {"class": "Person", "subclass": "OrganizationUser"}}))
            .collect();
        ok(json!({"value": value}))
    }
}
