//! The fake's mail methods: `Mailbox/get|set` and `Email/query|get|set|import`.

use super::*;

impl Data {
    /// `Mailbox/get`: those asked for, or all; failing while it's set to.
    pub(super) fn mailbox_get(&mut self, name: &str, want: Option<Vec<String>>) -> (String, Value) {
        if self.fail_mailbox_get > 0 {
            self.fail_mailbox_get -= 1;
            return ("error".into(), json!({"type": "serverUnavailable"}));
        }
        let list: Vec<Value> =
            self.mailboxes.iter().filter(|m| want.as_ref().is_none_or(|w| w.contains(&m.id))).map(|m| self.mailbox_json(m)).collect();
        (name.into(), json!({"accountId": ACCOUNT, "state": "s", "list": list, "notFound": []}))
    }

    /// `Mailbox/set`: mailboxes made, renamed or moved, and destroyed with the mail only in them.
    pub(super) fn mailbox_set(&mut self, name: &str, args: &Value) -> (String, Value) {
        let mut created = Map::new();
        for (k, v) in args.get("create").and_then(Value::as_object).cloned().unwrap_or_default() {
            let id = self.mailbox(v["name"].as_str().unwrap_or("?"), v["parentId"].as_str(), None);
            created.insert(k, json!({"id": id}));
        }
        let mut not_updated = Map::new();
        for (id, v) in args.get("update").and_then(Value::as_object).cloned().unwrap_or_default() {
            match self.mailboxes.iter_mut().find(|m| m.id == id) {
                Some(m) => {
                    m.name = v["name"].as_str().unwrap_or(&m.name).to_string();
                    m.parent = v["parentId"].as_str().map(str::to_string);
                }
                None => {
                    not_updated.insert(id, json!({"type": "notFound"}));
                }
            }
        }
        for id in args.get("destroy").and_then(Value::as_array).cloned().unwrap_or_default() {
            let id = id.as_str().unwrap_or_default().to_string();
            self.mailboxes.retain(|m| m.id != id);
            for e in &mut self.emails {
                e.mailboxes.remove(&id);
            }
            self.emails.retain(|e| !e.mailboxes.is_empty());
        }
        (name.into(), json!({"accountId": ACCOUNT, "created": created, "notUpdated": not_updated}))
    }

    /// `Email/query`: the ids of what meets the filter, newest first, a page of them.
    pub(super) fn email_query(&self, name: &str, args: &Value) -> (String, Value) {
        let mut found: Vec<&Em> = self.emails.iter().filter(|e| self.matches(e, &args["filter"])).collect();
        found.sort_by(|a, b| b.received.cmp(&a.received));
        let limit = args["limit"].as_u64().unwrap_or(1000) as usize;
        let position = args["position"].as_u64().unwrap_or(0) as usize;
        let ids: Vec<&str> = found.iter().skip(position).take(limit).map(|e| e.id.as_str()).collect();
        let mut answer = json!({"accountId": ACCOUNT, "queryState": "q", "ids": ids, "position": position});
        if args["calculateTotal"] == Value::Bool(true) {
            answer["total"] = json!(found.len());
        }
        (name.into(), answer)
    }

    pub(super) fn email_get(&self, name: &str, want: Option<Vec<String>>) -> (String, Value) {
        let want = want.unwrap_or_default();
        // As a server refuses more than its maxObjectsInGet (RFC 8620 §5.1).
        if want.len() > MAX_GET {
            return ("error".into(), json!({"type": "requestTooLarge"}));
        }
        let list: Vec<Value> = want.iter().filter_map(|id| self.email(id)).map(|e| self.email_json(e)).collect();
        (name.into(), json!({"accountId": ACCOUNT, "state": "s", "list": list}))
    }

    /// `Email/set`: keywords and mailboxes patched on or off, emails destroyed.
    pub(super) fn email_set(&mut self, name: &str, args: &Value) -> (String, Value) {
        let mut not_updated = Map::new();
        for (id, patch) in args.get("update").and_then(Value::as_object).cloned().unwrap_or_default() {
            let Some(e) = self.emails.iter_mut().find(|e| e.id == id) else {
                not_updated.insert(id, json!({"type": "notFound"}));
                continue;
            };
            for (path, v) in patch.as_object().cloned().unwrap_or_default() {
                let (field, key) = path.split_once('/').unwrap_or((&path, ""));
                let set = if field == "keywords" { &mut e.keywords } else { &mut e.mailboxes };
                if v == Value::Bool(true) {
                    set.insert(key.to_string());
                } else {
                    set.remove(key);
                }
            }
        }
        let destroy: Vec<String> = args
            .get("destroy")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
            .unwrap_or_default();
        self.emails.retain(|e| !destroy.contains(&e.id));
        (name.into(), json!({"accountId": ACCOUNT, "notUpdated": not_updated, "destroyed": destroy}))
    }

    /// `Email/import`: emails made from blobs uploaded.
    pub(super) fn email_import(&mut self, name: &str, args: &Value) -> (String, Value) {
        let mut created = Map::new();
        for (k, v) in args.get("emails").and_then(Value::as_object).cloned().unwrap_or_default() {
            let blob = v["blobId"].as_str().unwrap_or_default().to_string();
            let id = self.id("E");
            let keys = |f: &str| v[f].as_object().map(|m| m.keys().cloned().collect()).unwrap_or_default();
            self.emails.push(Em {
                id: id.clone(),
                blob: blob.clone(),
                mailboxes: keys("mailboxIds"),
                keywords: keys("keywords"),
                received: "2030-01-01T00:00:00Z".into(),
            });
            created.insert(k, json!({"id": id, "blobId": blob}));
        }
        (name.into(), json!({"accountId": ACCOUNT, "created": created}))
    }
}
