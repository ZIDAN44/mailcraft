//! The fake's account methods: `Quota/get`, `VacationResponse/get|set`, `Identity/get|set` and
//! `EmailSubmission/set`.

use super::*;

impl Data {
    pub(super) fn quota_get(&self, name: &str) -> (String, Value) {
        (
            name.into(),
            json!({"accountId": ACCOUNT, "list": [
                {"id": "q1", "resourceType": "count", "used": self.emails.len(), "hardLimit": 1000, "scope": "account"},
                {"id": "q2", "resourceType": "octets", "used": self.blobs.values().map(Vec::len).sum::<usize>(), "hardLimit": 1_000_000, "scope": "account"},
            ]}),
        )
    }

    pub(super) fn vacation_get(&self, name: &str) -> (String, Value) {
        let mut v = if self.vacation.is_object() { self.vacation.clone() } else { json!({"isEnabled": false}) };
        v["id"] = json!("singleton");
        (name.into(), json!({"accountId": ACCOUNT, "list": [v]}))
    }

    /// `VacationResponse/set`: refused when its text says `refuse`.
    pub(super) fn vacation_set(&mut self, name: &str, args: &Value) -> (String, Value) {
        let patch = args["update"]["singleton"].clone();
        if patch["textBody"].as_str().is_some_and(|t| t.contains("refuse")) {
            return (
                name.into(),
                json!({"accountId": ACCOUNT, "notUpdated": {"singleton": {"type": "invalidProperties", "description": "not allowed"}}}),
            );
        }
        self.vacation = patch;
        (name.into(), json!({"accountId": ACCOUNT, "updated": {"singleton": null}}))
    }

    /// `Identity/get`: the account's own (unless it starts with none), and those made.
    pub(super) fn identity_get(&self, name: &str) -> (String, Value) {
        let mut list: Vec<Value> = if self.no_identity { Vec::new() } else { vec![json!({"id": "I1", "email": USER, "name": "Me"})] };
        list.extend(self.made_identities.iter().enumerate().map(|(i, e)| json!({"id": format!("M{i}"), "email": e, "name": ""})));
        (name.into(), json!({"accountId": ACCOUNT, "list": list}))
    }

    pub(super) fn identity_set(&mut self, name: &str, args: &Value) -> (String, Value) {
        let mut created = Map::new();
        for (k, v) in args.get("create").and_then(Value::as_object).cloned().unwrap_or_default() {
            let email = v["email"].as_str().unwrap_or_default().to_string();
            created.insert(k, json!({"id": format!("M{}", self.made_identities.len())}));
            self.made_identities.push(email);
        }
        (name.into(), json!({"accountId": ACCOUNT, "created": created}))
    }

    /// `EmailSubmission/set`: each sending kept, one to a recipient that isn't there refused.
    pub(super) fn submission_set(&mut self, name: &str, args: &Value) -> (String, Value) {
        let mut created = Map::new();
        let mut not_created = Map::new();
        for (k, v) in args.get("create").and_then(Value::as_object).cloned().unwrap_or_default() {
            let rcpt: Vec<String> = v["envelope"]["rcptTo"]
                .as_array()
                .map(|a| a.iter().filter_map(|r| r["email"].as_str().map(str::to_string)).collect())
                .unwrap_or_default();
            if rcpt.iter().any(|r| r.starts_with("nobody@")) {
                not_created.insert(k, json!({"type": "forbiddenToSend", "description": "no such recipient"}));
                continue;
            }
            let from = v["envelope"]["mailFrom"]["email"].as_str().unwrap_or_default().to_string();
            self.submissions.push((v["emailId"].as_str().unwrap_or_default().to_string(), from, rcpt));
            created.insert(k, json!({"id": "S1"}));
        }
        (name.into(), json!({"accountId": ACCOUNT, "created": created, "notCreated": not_created}))
    }
}
