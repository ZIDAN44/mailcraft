//! The fake's mail: folders, messages, sending and automatic replies, and JSON batches.

use super::*;

impl Data {
    /// `POST /$batch`: each request answered in turn (its own `Prefer` heeded), past the
    /// concurrency cap a 429.
    pub(super) fn batch(&mut self, input: &Value, base: &str) -> Answer {
        let mut out = Vec::new();
        for (i, r) in input["requests"].as_array().cloned().unwrap_or_default().into_iter().enumerate() {
            if self.at_once.is_some_and(|n| i >= n) {
                let mut refused = serde_json::Map::new();
                refused.insert("id".into(), r["id"].clone());
                refused.insert("status".into(), json!(429));
                refused.insert("headers".into(), json!({ "retry-after": 1 }));
                refused.insert("body".into(), json!({"error": {"code": "TooManyRequests"}}));
                out.push(Value::Object(refused));
                continue;
            }
            // Graph reads Prefer from each batch entry's own headers.
            let prefer = r.pointer("/headers/Prefer").and_then(Value::as_str).unwrap_or_default();
            self.asked_immutable = asks_immutable(prefer);
            let b = serde_json::to_vec(&r["body"]).unwrap_or_default();
            let (status, rb, _) = self.route(r["method"].as_str().unwrap_or("GET"), r["url"].as_str().unwrap_or("/"), &b, base);
            let rbody: Value = serde_json::from_slice(&rb).unwrap_or(Value::Null);
            out.push(json!({"id": r["id"], "status": status, "body": rbody}));
        }
        ok(json!({ "responses": out }))
    }

    /// `/me/mailFolders…`: folders listed (paged), made, renamed, moved and deleted, and the
    /// messages in one.
    pub(super) fn on_mail_folders(&mut self, method: &str, parts: &[&str], input: &Value, path: &str, query: &str, base: &str) -> Answer {
        match (method, parts) {
            ("GET", ["me", "mailFolders"]) => {
                self.page(self.folders.iter().filter(|f| f.parent == ROOT).map(|f| self.folder_json(f)).collect(), path, query, base)
            }
            ("POST", ["me", "mailFolders"]) => {
                let id = self.folder(input["displayName"].as_str().unwrap_or("?"), None, None);
                ok(json!({ "id": id }))
            }
            ("GET", ["me", "mailFolders", id]) => match self.resolve(id) {
                Some(id) => {
                    let f = self.folders.iter().find(|f| f.id == id).cloned();
                    f.map_or_else(not_found, |f| ok(self.folder_json(&f)))
                }
                None => not_found(),
            },
            ("PATCH", ["me", "mailFolders", id]) => match self.folders.iter_mut().find(|f| f.id == *id) {
                Some(f) => {
                    f.name = input["displayName"].as_str().unwrap_or(&f.name).to_string();
                    ok(json!({ "id": id }))
                }
                None => not_found(),
            },
            ("DELETE", ["me", "mailFolders", id]) => {
                let id = id.to_string();
                self.folders.retain(|f| f.id != id);
                self.messages.retain(|m| m.folder != id);
                (204, Vec::new(), "text/plain")
            }
            ("GET", ["me", "mailFolders", id, "childFolders"]) => {
                let list = self.folders.iter().filter(|f| f.parent == *id).map(|f| self.folder_json(f)).collect();
                self.page(list, path, query, base)
            }
            ("POST", ["me", "mailFolders", id, "childFolders"]) => {
                let id = self.folder(input["displayName"].as_str().unwrap_or("?"), Some(id), None);
                ok(json!({ "id": id }))
            }
            ("POST", ["me", "mailFolders", id, "move"]) => {
                let dest = input["destinationId"].as_str().unwrap_or_default().to_string();
                let dest = if dest == "msgfolderroot" { ROOT.to_string() } else { dest };
                match self.folders.iter_mut().find(|f| f.id == *id) {
                    Some(f) => {
                        f.parent = dest;
                        ok(json!({ "id": id }))
                    }
                    None => not_found(),
                }
            }
            ("GET", ["me", "mailFolders", id, "messages"]) => self.messages_in(id, path, query, base),
            _ => not_found(),
        }
    }

    /// A folder's messages, newest first, `$top` a page (`@odata.nextLink`, with `$skip`, for
    /// the rest): those a search's words find, or meeting read and flag conditions.
    fn messages_in(&mut self, id: &str, path: &str, query: &str, base: &str) -> Answer {
        let Some(id) = self.resolve(id) else { return not_found() };
        let mut list: Vec<&Msg> = self.messages.iter().filter(|m| m.folder == id).collect();
        if let Some(o) = param(query, "$orderby")
            && o != "receivedDateTime%20desc"
        {
            return self.refuse(format!("messages asked for in another order than the newest first: {o}"));
        }
        // A search: its words (KQL, the whole in quotes), or read and flag conditions.
        if let Some(s) = param(query, "$search") {
            let s = crate::oauth::unpct(&s);
            let inner = s.strip_prefix('"').and_then(|s| s.strip_suffix('"')).unwrap_or(&s).replace("\\\"", "\"");
            list.retain(|m| words_match(&inner, &m.raw));
        }
        if let Some(f) = param(query, "$filter") {
            let f = crate::oauth::unpct(&f);
            for cond in f.split(" and ") {
                match cond {
                    "isRead eq false" => list.retain(|m| !m.read),
                    "isRead eq true" => list.retain(|m| m.read),
                    "flag/flagStatus eq 'flagged'" => list.retain(|m| m.flagged),
                    "flag/flagStatus ne 'flagged'" => list.retain(|m| !m.flagged),
                    "hasAttachments eq true" => list.clear(),
                    _ => {}
                }
            }
        }
        list.sort_by(|a, b| b.received.cmp(&a.received));
        // `$top` a page, `@odata.nextLink` (with `$skip`) for the rest.
        let top: usize = param(query, "$top").and_then(|t| t.parse().ok()).unwrap_or(10);
        let skip: usize = param(query, "$skip").and_then(|s| s.parse().ok()).unwrap_or(0);
        let more = list.len() > skip.saturating_add(top);
        let list: Vec<Value> = list.into_iter().skip(skip).take(top).map(|m| self.message_json(m)).collect();
        let mut v = json!({ "value": list });
        if more {
            let rest: String = query.split('&').filter(|kv| !kv.starts_with("$skip=")).collect::<Vec<_>>().join("&");
            v["@odata.nextLink"] = json!(format!("{base}{path}?{rest}&$skip={}", skip + top));
        }
        ok(v)
    }

    /// `/me/messages…`: a draft made from MIME; a message read (with its files' list), changed,
    /// moved, deleted, or its source.
    pub(super) fn on_messages(&mut self, method: &str, parts: &[&str], input: &Value, body: &[u8], query: &str) -> Answer {
        match (method, parts) {
            ("POST", ["me", "messages"]) => {
                let raw = unbase64(body);
                let drafts = self.resolve("drafts").unwrap_or_default();
                let id = self.deliver(&drafts, &raw, true, "2030-01-01T00:00:00Z");
                if let Some(m) = self.messages.iter_mut().find(|m| m.id == id) {
                    m.draft = true;
                }
                ok(json!({ "id": id, "parentFolderId": drafts }))
            }
            // With its body and files' list (a message's text first, issue #24).
            ("GET", ["me", "messages", id]) if query.contains("$expand=attachments") => match self.message(id) {
                Some(m) => {
                    let mut v = self.message_json(m);
                    let p = mailcraft_mime::parse(&m.raw);
                    v["body"] = json!({"contentType": "text", "content": p.as_ref().map(|p| p.text.clone())});
                    v["attachments"] = json!(p
                        .map(|p| p
                            .attachments
                            .iter()
                            .map(|a| json!({"@odata.type": "#microsoft.graph.fileAttachment", "name": a.name, "contentType": a.mime, "size": a.size, "isInline": a.inline, "contentId": a.content_id}))
                            .collect::<Vec<_>>())
                        .unwrap_or_default());
                    ok(v)
                }
                None => not_found(),
            },
            ("GET", ["me", "messages", id]) => self.message(id).map_or_else(not_found, |m| ok(self.message_json(m))),
            ("PATCH", ["me", "messages", id]) => match self.messages.iter_mut().find(|m| m.id == plain_id(id)) {
                Some(m) => {
                    if let Some(r) = input["isRead"].as_bool() {
                        m.read = r;
                    }
                    if let Some(f) = input["flag"]["flagStatus"].as_str() {
                        m.flagged = f == "flagged";
                    }
                    ok(json!({ "id": id }))
                }
                None => not_found(),
            },
            ("DELETE", ["me", "messages", id]) => {
                let id = plain_id(id).to_string();
                self.messages.retain(|m| m.id != id);
                (204, Vec::new(), "text/plain")
            }
            ("POST", ["me", "messages", id, "move"]) => {
                let dest = input["destinationId"].as_str().and_then(|d| self.resolve(d));
                match (dest, self.messages.iter_mut().find(|m| m.id == plain_id(id))) {
                    (Some(d), Some(m)) => {
                        m.folder = d;
                        ok(json!({ "id": id }))
                    }
                    _ => not_found(),
                }
            }
            ("GET", ["me", "messages", id, "$value"]) if self.unreadable.iter().any(|u| u == plain_id(id)) => {
                error(500, "ErrorInternalServerError", "can't read it")
            }
            ("GET", ["me", "messages", id, "$value"]) => match self.message(id) {
                Some(m) => (200, m.raw.clone(), "text/plain"),
                None => not_found(),
            },
            _ => not_found(),
        }
    }

    /// `POST /me/sendMail`: refused for a recipient that isn't there, else sent and filed in
    /// Sent Items (a moment later, when set to).
    pub(super) fn send_mail(&mut self, body: &[u8]) -> Answer {
        let raw = unbase64(body);
        if String::from_utf8_lossy(&raw).contains("nobody@") {
            return error(400, "ErrorInvalidRecipients", "no such recipient");
        }
        if self.file_late {
            self.late.push((std::time::Instant::now() + std::time::Duration::from_secs(1), raw.clone()));
        } else {
            let sent = self.resolve("sentitems").unwrap_or_default();
            self.deliver(&sent, &raw, true, "2030-01-01T00:00:00Z");
        }
        self.sent.push(raw);
        (202, Vec::new(), "text/plain")
    }

    /// `/me/mailboxSettings…`: the automatic replies, read and set.
    pub(super) fn on_mailbox_settings(&mut self, method: &str, parts: &[&str], input: &Value) -> Answer {
        match (method, parts) {
            ("GET", ["me", "mailboxSettings", "automaticRepliesSetting"]) => ok(self.replies.clone()),
            ("PATCH", ["me", "mailboxSettings"]) => {
                let r = input["automaticRepliesSetting"].clone();
                if r["internalReplyMessage"].as_str() == Some("refuse") {
                    return error(400, "", "refused");
                }
                self.replies = r;
                ok(json!({}))
            }
            _ => not_found(),
        }
    }
}
