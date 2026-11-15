//! A signed-in JMAP session: the session fetched and checked, the batches of method calls
//! sent to it, and the calls MailCraft makes (mailboxes, messages, sending, the account's
//! identities, quota and automatic replies).

use super::*;

/// A signed-in JMAP session.
pub struct Client<H> {
    pub(super) http: H,
    pub(super) auth: String,
    pub(super) api: String,
    pub(super) download: String,
    pub(super) upload: String,
    /// The push template (RFC 8620 §7.3), when the server offers one.
    pub(super) events: Option<String>,
    pub(super) account: String,
    pub(super) submission: bool,
    pub(super) quota: bool,
    pub(super) vacation: bool,
    pub(super) max_get: usize,
}

fn str_at<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    v.get(key).and_then(Value::as_str).ok_or_else(|| Error::Protocol(format!("the JMAP session has no `{key}`")))
}

fn json_body(resp: &Response) -> Result<Value> {
    serde_json::from_slice(&resp.body).map_err(|e| Error::Protocol(format!("JMAP: {e}")))
}

/// Turn an HTTP failure into the error the caller knows how to handle.
fn check(resp: &Response) -> Result<()> {
    match resp.status {
        200..=299 => Ok(()),
        401 | 403 => Err(Error::Auth("the server refused the user name or password".into())),
        // Busy or down: reconnect with back-off.
        429 | 500..=599 => Err(Error::Closed(format!("HTTP {}", resp.status))),
        s => {
            // RFC 7807 problem details, when there are any.
            let detail = serde_json::from_slice::<Value>(&resp.body)
                .ok()
                .and_then(|v| v.get("detail").or_else(|| v.get("type")).and_then(Value::as_str).map(str::to_string))
                .unwrap_or_default();
            Err(Error::Bad(format!("HTTP {s} {detail}").trim().to_string()))
        }
    }
}

/// `{accountId}`-style template variables (RFC 6570 level 1), percent-encoded.
pub(super) fn expand(template: &str, vars: &[(&str, &str)]) -> String {
    let mut out = template.to_string();
    for (k, v) in vars {
        out = out.replace(&format!("{{{k}}}"), &pct(v));
    }
    out
}

impl<H: Http> Client<H> {
    /// Fetch the session and check it offers mail. `bearer` signs in with an OAuth access token.
    pub async fn connect(http: H, url: &str, user: &str, secret: &str, bearer: bool) -> Result<Client<H>> {
        let auth = if bearer {
            format!("Bearer {secret}")
        } else {
            format!("Basic {}", mailcraft_model::base64::encode(format!("{user}:{secret}").as_bytes()))
        };
        let session = session_url(url);
        let resp = http.send(Request::get(&session, 1 << 20).header("Authorization", &auth).header("Accept", "application/json")).await?;
        check(&resp)?;
        let v = json_body(&resp)?;
        let caps = v.get("capabilities").cloned().unwrap_or(Value::Null);
        if caps.get(MAIL).is_none() {
            return Err(Error::Protocol("this JMAP server doesn't offer mail".into()));
        }
        let account = v
            .get("primaryAccounts")
            .and_then(|p| p.get(MAIL))
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Protocol("no mail account in the JMAP session".into()))?;
        let base = Url::parse(&session)?;
        let resolve = |key: &str| -> Result<String> {
            let u = base.join(str_at(&v, key)?)?;
            // Never from HTTPS down to plain HTTP (the password goes with every request).
            if base.https && !u.https {
                return Err(Error::Tls(format!("the JMAP {key} isn't https")));
            }
            Ok(u.full())
        };
        let max_get = caps.get(CORE).and_then(|c| c.get("maxObjectsInGet")).and_then(Value::as_u64).unwrap_or(500);
        Ok(Client {
            api: resolve("apiUrl")?,
            // Templates keep their braces: resolve only the part before them.
            download: template_url(&base, str_at(&v, "downloadUrl")?)?,
            upload: template_url(&base, str_at(&v, "uploadUrl")?)?,
            events: v.get("eventSourceUrl").and_then(Value::as_str).and_then(|t| template_url(&base, t).ok()),
            account: account.to_string(),
            submission: caps.get(SUBMISSION).is_some(),
            quota: caps.get(QUOTA).is_some(),
            vacation: caps.get(VACATION).is_some(),
            max_get: usize::try_from(max_get).unwrap_or(500).clamp(1, 1000),
            auth,
            http,
        })
    }

    /// The `Authorization` header value every request carries.
    pub fn authorization(&self) -> &str {
        &self.auth
    }

    /// Carry a new OAuth access token from now on (the old one ran out).
    pub fn set_bearer(&mut self, token: &str) {
        self.auth = format!("Bearer {token}");
    }

    /// The push address (RFC 8620 §7.3) for every type of change, when the server pushes.
    pub fn events_url(&self) -> Option<String> {
        self.events.as_ref().map(|t| expand(t, &[("types", "*"), ("closeafter", "no"), ("ping", "30")]))
    }

    /// Run a batch of method calls; the results in order. A method error fails the batch.
    async fn call(&self, using: &[&str], calls: Vec<(&str, Value)>) -> Result<Vec<Value>> {
        let method_calls: Vec<Value> = calls.iter().enumerate().map(|(i, (name, args))| json!([name, args, format!("c{i}")])).collect();
        let body = json!({ "using": using, "methodCalls": method_calls });
        let bytes = serde_json::to_vec(&body).map_err(|e| Error::Protocol(e.to_string()))?;
        let resp = self
            .http
            .send(
                Request::post(&self.api, "application/json", bytes, MAX_JSON)
                    .header("Authorization", &self.auth)
                    .header("Accept", "application/json"),
            )
            .await?;
        check(&resp)?;
        let v = json_body(&resp)?;
        let responses = v.get("methodResponses").and_then(Value::as_array).ok_or_else(|| Error::Protocol("JMAP: no methodResponses".into()))?;
        let mut out = Vec::with_capacity(calls.len());
        for i in 0..calls.len() {
            let id = format!("c{i}");
            let r = responses
                .iter()
                .filter_map(Value::as_array)
                .find(|r| r.get(2).and_then(Value::as_str) == Some(id.as_str()))
                .ok_or_else(|| Error::Protocol(format!("JMAP: no answer to {}", calls.get(i).map_or("", |c| c.0))))?;
            let name = r.first().and_then(Value::as_str).unwrap_or_default();
            let args = r.get(1).cloned().unwrap_or(Value::Null);
            if name == "error" {
                let kind = args.get("type").and_then(Value::as_str).unwrap_or("error");
                let desc = args.get("description").and_then(Value::as_str).unwrap_or_default();
                return Err(Error::No(format!("{kind} {desc}").trim().to_string()));
            }
            out.push(args);
        }
        Ok(out)
    }

    async fn mail(&self, calls: Vec<(&str, Value)>) -> Result<Vec<Value>> {
        self.call(&[CORE, MAIL], calls).await
    }

    pub async fn mailboxes(&self) -> Result<Vec<Mailbox>> {
        let r = self.mail(vec![("Mailbox/get", json!({"accountId": self.account, "ids": null}))]).await?;
        Ok(list(r.first(), "list").iter().filter_map(Mailbox::from_json).collect())
    }

    /// The newest `limit` ids in a mailbox with their keywords, and its (total, unread) counts.
    /// A page of the server's `maxObjectsInGet` at a time (by position): asked at once, a folder
    /// kept no more than that however many it was to keep.
    pub(super) async fn newest(&self, mailbox: &str, limit: usize) -> Result<(Vec<(String, Flags)>, Option<(u32, u32)>)> {
        let limit = limit.clamp(1, 100_000);
        let (mut ids, mut counts, mut seen, mut total) = (Vec::new(), None, 0usize, None);
        for page in 0..1000 {
            if seen >= limit {
                break;
            }
            let want = (limit - seen).min(self.max_get);
            let mut calls = vec![
                (
                    "Email/query",
                    json!({"accountId": self.account, "filter": {"inMailbox": mailbox}, "sort": [{"property": "receivedAt", "isAscending": false}],
                           "position": seen, "limit": want, "calculateTotal": page == 0}),
                ),
                (
                    "Email/get",
                    json!({"accountId": self.account, "#ids": {"resultOf": "c0", "name": "Email/query", "path": "/ids"}, "properties": ["id", "keywords"]}),
                ),
            ];
            if page == 0 {
                calls.push(("Mailbox/get", json!({"accountId": self.account, "ids": [mailbox], "properties": ["totalEmails", "unreadEmails"]})));
            }
            let r = self.mail(calls).await?;
            let order: Vec<&str> = list(r.first(), "ids").iter().filter_map(Value::as_str).collect();
            let got: HashMap<&str, Flags> =
                list(r.get(1), "list").iter().filter_map(|e| Some((e.get("id")?.as_str()?, keywords(e.get("keywords"))))).collect();
            ids.extend(order.iter().filter_map(|id| got.get(id).map(|f| (id.to_string(), f.clone()))));
            if page == 0 {
                counts = list(r.get(2), "list").first().map(|m| (count(m, "totalEmails"), count(m, "unreadEmails")));
                total = r.first().and_then(|q| q.get("total")).and_then(Value::as_u64).and_then(|t| usize::try_from(t).ok());
            }
            seen += order.len();
            // The end: nothing more, all there is, or (with no total) a short page.
            if order.is_empty() || total.map_or(order.len() < want, |t| seen >= t) {
                break;
            }
        }
        Ok((ids, counts))
    }

    /// How many messages to ask for in one `Email/get`.
    fn get_chunk(&self) -> usize {
        self.max_get.clamp(1, GET_CHUNK)
    }

    /// Headers and details of messages, for the list.
    pub(super) async fn headers(&self, ids: &[String]) -> Result<Vec<Value>> {
        let mut out = Vec::new();
        for chunk in ids.chunks(self.get_chunk()) {
            let r = self
                .mail(vec![(
                    "Email/get",
                    json!({"accountId": self.account, "ids": chunk, "properties": ["id", "keywords", "size", "receivedAt", "preview", "hasAttachment", "headers"]}),
                )])
                .await?;
            out.extend(list(r.first(), "list").iter().cloned());
        }
        Ok(out)
    }

    /// The blob ids of messages' sources.
    pub(super) async fn blobs(&self, ids: &[String]) -> Result<HashMap<String, String>> {
        let r = self.mail(vec![("Email/get", json!({"accountId": self.account, "ids": ids, "properties": ["id", "blobId"]}))]).await?;
        Ok(list(r.first(), "list")
            .iter()
            .filter_map(|e| Some((e.get("id")?.as_str()?.to_string(), e.get("blobId")?.as_str()?.to_string())))
            .collect())
    }

    pub async fn download(&self, blob: &str) -> Result<Vec<u8>> {
        let url = expand(&self.download, &[("accountId", &self.account), ("blobId", blob), ("type", "message/rfc822"), ("name", "message.eml")]);
        let resp = self.http.send(Request::get(&url, MAX_BLOB).header("Authorization", &self.auth)).await?;
        check(&resp)?;
        Ok(resp.body)
    }

    pub async fn upload(&self, raw: &[u8]) -> Result<String> {
        let url = expand(&self.upload, &[("accountId", &self.account)]);
        let resp = self.http.send(Request::post(&url, "message/rfc822", raw.to_vec(), 1 << 20).header("Authorization", &self.auth)).await?;
        check(&resp)?;
        let v = json_body(&resp)?;
        Ok(str_at(&v, "blobId")?.to_string())
    }

    /// Add a message to a mailbox from its source.
    pub async fn import(&self, raw: &[u8], mailbox: &str, keywords: &[&str]) -> Result<String> {
        let blob = self.upload(raw).await?;
        let kw: Map<String, Value> = keywords.iter().map(|k| (k.to_string(), Value::Bool(true))).collect();
        let r = self
            .mail(vec![(
                "Email/import",
                json!({"accountId": self.account, "emails": {"m": {"blobId": blob, "mailboxIds": {mailbox: true}, "keywords": kw}}}),
            )])
            .await?;
        set_error(r.first(), "notCreated")?;
        r.first()
            .and_then(|v| v.get("created"))
            .and_then(|c| c.get("m"))
            .and_then(|m| m.get("id"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| Error::Protocol("JMAP: the import made no message".into()))
    }

    /// Update and destroy messages (`Email/set`).
    pub(super) async fn set_emails(&self, update: Map<String, Value>, destroy: Vec<String>) -> Result<()> {
        if update.is_empty() && destroy.is_empty() {
            return Ok(());
        }
        let r = self.mail(vec![("Email/set", json!({"accountId": self.account, "update": update, "destroy": destroy}))]).await?;
        set_error(r.first(), "notUpdated")?;
        set_error(r.first(), "notDestroyed")
    }

    pub(super) async fn set_mailboxes(&self, args: Value) -> Result<Value> {
        let mut a = args;
        if let Some(o) = a.as_object_mut() {
            o.insert("accountId".into(), Value::String(self.account.clone()));
        }
        let r = self.mail(vec![("Mailbox/set", a)]).await?;
        for k in ["notCreated", "notUpdated", "notDestroyed"] {
            set_error(r.first(), k)?;
        }
        Ok(r.into_iter().next().unwrap_or(Value::Null))
    }

    /// Send a message already stored on the server, from the identity for `from`.
    pub(super) async fn submit(&self, email: &str, from: &str, recipients: &[String]) -> Result<()> {
        if !self.submission {
            return Err(Error::Send("this JMAP server doesn't send mail".into()));
        }
        let using = [CORE, MAIL, SUBMISSION];
        let r = self.call(&using, vec![("Identity/get", json!({"accountId": self.account, "ids": null}))]).await?;
        let identities = list(r.first(), "list");
        let found = identities
            .iter()
            .find(|i| i.get("email").and_then(Value::as_str).is_some_and(|e| e.eq_ignore_ascii_case(from)))
            .or_else(|| identities.first())
            .and_then(|i| i.get("id"))
            .and_then(Value::as_str)
            .map(str::to_string);
        // None yet (Stalwart makes none for a new account): one made for the address, as RFC 8621
        // §6 lets a client; a server that won't says why.
        let identity = match found {
            Some(id) => id,
            None => {
                let r = self.call(&using, vec![("Identity/set", json!({"accountId": self.account, "create": {"i": {"email": from}}}))]).await?;
                set_error(r.first(), "notCreated")
                    .map_err(|e| Error::Send(format!("the JMAP account has no identity to send from, and none could be made: {e}")))?;
                r.first()
                    .and_then(|v| v.get("created")?.get("i")?.get("id")?.as_str())
                    .map(str::to_string)
                    .ok_or_else(|| Error::Send("the JMAP account has no identity to send from".into()))?
            }
        };
        let identity = identity.as_str();
        let rcpt: Vec<Value> = recipients.iter().map(|r| json!({"email": r})).collect();
        let r = self
            .call(
                &using,
                vec![(
                    "EmailSubmission/set",
                    json!({"accountId": self.account, "create": {"s": {"identityId": identity, "emailId": email, "envelope": {"mailFrom": {"email": from}, "rcptTo": rcpt}}}}),
                )],
            )
            .await?;
        set_error(r.first(), "notCreated").map_err(|e| Error::Send(e.to_string()))
    }

    /// The addresses the account's mail is sent from, as the server keeps them (RFC 8621 §6:
    /// its identities), each with its name and where replies to it go; none for a server
    /// that doesn't send mail. At most a hundred, each held to what an address is.
    pub async fn identities(&self) -> Result<Vec<mailcraft_model::Identity>> {
        if !self.submission {
            return Ok(Vec::new());
        }
        let r = self.call(&[CORE, MAIL, SUBMISSION], vec![("Identity/get", json!({"accountId": self.account, "ids": null}))]).await?;
        let text = |v: Option<&Value>, most: usize| v.and_then(Value::as_str).unwrap_or_default().trim().chars().take(most).collect::<String>();
        let found = list(r.first(), "list").iter().take(100).map(|i| {
            let reply_to = i.get("replyTo").and_then(Value::as_array).and_then(|l| l.first()).and_then(|a| a.get("email"));
            mailcraft_model::Identity {
                email: text(i.get("email"), 254),
                name: text(i.get("name"), 200),
                reply_to: text(reply_to, 254),
                ..Default::default()
            }
        });
        Ok(found.filter(|i| mailcraft_model::addr::is_valid_email(&i.email)).collect())
    }

    /// Space used and the limit in bytes for the whole account (RFC 9425); `None` without quotas.
    pub async fn quota(&self) -> Result<Option<(u64, u64)>> {
        if !self.quota {
            return Ok(None);
        }
        let r = self.call(&[CORE, QUOTA], vec![("Quota/get", json!({"accountId": self.account, "ids": null}))]).await?;
        let octets = list(r.first(), "list")
            .iter()
            .filter(|q| q.get("resourceType").and_then(Value::as_str) == Some("octets"))
            .filter(|q| q.get("scope").and_then(Value::as_str).is_none_or(|s| s == "account"))
            .find_map(|q| Some((q.get("used")?.as_u64()?, q.get("hardLimit").and_then(Value::as_u64).unwrap_or(0))));
        Ok(octets)
    }

    /// The automatic replies (RFC 8621 §8); an error when the server offers none.
    pub async fn vacation(&self) -> Result<mailcraft_model::job::Vacation> {
        if !self.vacation {
            return Err(Error::No("this server doesn't offer automatic replies".into()));
        }
        let r = self.call(&[CORE, VACATION], vec![("VacationResponse/get", json!({"accountId": self.account, "ids": ["singleton"]}))]).await?;
        let v = list(r.first(), "list").first().cloned().unwrap_or(Value::Null);
        let day =
            |k: &str| v.get(k).and_then(Value::as_str).and_then(|d| d.get(..10)).filter(|d| mailcraft_model::job::is_day(d)).map(str::to_string);
        Ok(mailcraft_model::job::Vacation {
            enabled: v.get("isEnabled").and_then(Value::as_bool).unwrap_or(false),
            subject: v.get("subject").and_then(Value::as_str).unwrap_or_default().to_string(),
            text: v.get("textBody").and_then(Value::as_str).unwrap_or_default().to_string(),
            from: day("fromDate"),
            until: day("toDate"),
            ..Default::default()
        })
    }

    pub async fn set_vacation(&self, v: &mailcraft_model::job::Vacation) -> Result<()> {
        if !self.vacation {
            return Err(Error::No("this server doesn't offer automatic replies".into()));
        }
        // Dates are whole days: from the start of the first to the end of the last.
        let from = v.from.as_ref().map(|d| format!("{d}T00:00:00Z"));
        let to = v.until.as_ref().map(|d| format!("{d}T23:59:59Z"));
        let patch = json!({
            "isEnabled": v.enabled,
            "subject": if v.subject.trim().is_empty() { Value::Null } else { Value::String(v.subject.clone()) },
            "textBody": v.text,
            "fromDate": from,
            "toDate": to,
        });
        let r =
            self.call(&[CORE, VACATION], vec![("VacationResponse/set", json!({"accountId": self.account, "update": {"singleton": patch}}))]).await?;
        set_error(r.first(), "notUpdated")
    }

    /// Destroy every message in a mailbox, in batches.
    pub(super) async fn empty(&self, mailbox: &str) -> Result<()> {
        for _ in 0..1000 {
            let r = self
                .mail(vec![("Email/query", json!({"accountId": self.account, "filter": {"inMailbox": mailbox}, "limit": self.max_get.min(500)}))])
                .await?;
            let ids: Vec<String> = list(r.first(), "ids").iter().filter_map(|v| v.as_str().map(str::to_string)).collect();
            if ids.is_empty() {
                return Ok(());
            }
            self.set_emails(Map::new(), ids).await?;
        }
        Err(Error::No("the folder didn't empty".into()))
    }
}

/// A download or upload template resolved against the session URL, braces intact.
pub(super) fn template_url(base: &Url, t: &str) -> Result<String> {
    let cut = t.find('{').unwrap_or(t.len());
    let (head, rest) = t.split_at(cut);
    let u = base.join(head)?;
    if base.https && !u.https {
        return Err(Error::Tls("a JMAP address isn't https".into()));
    }
    Ok(format!("{}{rest}", u.full()))
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
        for chunk in ids.chunks(self.get_chunk()) {
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
