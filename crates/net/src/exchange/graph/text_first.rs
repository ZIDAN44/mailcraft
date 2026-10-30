//! A message's text first (issue #24), from Microsoft Graph: its body, headers and the list of
//! its files without their bytes, in one request, to read while its whole source comes. Graph
//! gives no message sizes, so every message with files comes this way.

use mailcraft_model::EmailAddress;

use super::*;

/// What's asked for: the headers to show, the body, and the files' names, types and sizes.
const FIELDS: &str = "subject,body,from,toRecipients,ccRecipients,replyTo,sentDateTime,internetMessageId,importance,hasAttachments";

fn address(v: &Value) -> Option<EmailAddress> {
    let a = v.get("emailAddress")?;
    let email = a.get("address").and_then(Value::as_str).filter(|e| e.contains('@'))?;
    Some(EmailAddress::new(a.get("name").and_then(Value::as_str).unwrap_or_default(), email))
}

fn addresses(v: &Value, field: &str) -> Vec<EmailAddress> {
    v.get(field).and_then(Value::as_array).map(|l| l.iter().filter_map(address).collect()).unwrap_or_default()
}

impl Client {
    /// Message `id`'s text first, as MIME: `None` when it has no files (then the whole of it is
    /// as quick).
    pub fn download_text(&self, id: &str) -> Result<Option<Vec<u8>>> {
        let v = self.get(&format!("/me/messages/{}?$select={FIELDS}&$expand=attachments($select=name,contentType,size,isInline)", seg(id)))?;
        Ok(text_first(&v))
    }
}

/// A message's text first, as MIME; `None` without a body or files.
pub(super) fn text_first(v: &Value) -> Option<Vec<u8>> {
    let body = v.get("body")?;
    let files: Vec<mime::PendingFile> = v
        .get("attachments")
        .and_then(Value::as_array)?
        .iter()
        .take(500)
        .map(|a| {
            let item = a.get("@odata.type").and_then(Value::as_str).is_some_and(|t| t.ends_with("itemAttachment"));
            mime::PendingFile {
                name: a.get("name").and_then(Value::as_str).unwrap_or_default().to_string(),
                mime: if item {
                    "message/rfc822".into()
                } else {
                    a.get("contentType").and_then(Value::as_str).unwrap_or("application/octet-stream").to_string()
                },
                size: a.get("size").and_then(Value::as_u64).unwrap_or(0),
                content_id: None,
                inline: false,
            }
        })
        .collect();
    if files.is_empty() {
        return None;
    }
    let content = body.get("content").and_then(Value::as_str).unwrap_or_default().to_string();
    let html = body.get("contentType").and_then(Value::as_str).is_none_or(|t| t.eq_ignore_ascii_case("html"));
    let d = mime::Draft {
        to: addresses(v, "toRecipients"),
        cc: addresses(v, "ccRecipients"),
        reply_to: addresses(v, "replyTo"),
        subject: v.get("subject").and_then(Value::as_str).unwrap_or_default().to_string(),
        body: if html { String::new() } else { content.clone() },
        html: html.then_some(content),
        importance: match v.get("importance").and_then(Value::as_str) {
            Some("high") => 1,
            Some("low") => -1,
            _ => 0,
        },
        ..Default::default()
    };
    let from = v.get("from").and_then(address).unwrap_or_default();
    let date = v.get("sentDateTime").and_then(Value::as_str).and_then(jmap::utc_date).unwrap_or(0);
    let id = v.get("internetMessageId").and_then(Value::as_str).unwrap_or_default();
    mime::text_first(&d, &from, date, id, &files).ok()
}

/// A message's text first, its files listed but not downloaded (issue #24), when it has files
/// and isn't here already: `None` otherwise (then [`fetch_body`] brings it whole).
pub fn fetch_text(c: &Client, st: &State, store: &Store, key: &MessageKey) -> Result<Option<Vec<u8>>> {
    if store.has_body(key) || key.validity != VALIDITY {
        return Ok(None);
    }
    let fs = store.load_folder(&key.account, &key.folder)?;
    let files = fs.find(key.uid).is_some_and(|m| m.has_attachments);
    match st.id(key.uid).filter(|_| files) {
        Some(id) => c.download_text(id),
        None => Ok(None),
    }
}
