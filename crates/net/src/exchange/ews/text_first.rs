//! A message's text first (issue #24): its body, headers and the list of its files, without
//! their bytes, in one quick `GetItem`, so it can be read while its whole source comes (that
//! source holds every attachment: 20 MB took 20 seconds before a line showed).

use mailcraft_model::EmailAddress;

use super::*;

/// Messages smaller than this come whole at once: their text first would save little.
pub const TEXT_FIRST_OVER: u32 = 1 << 20;

/// What's asked for: the body and the files' list, and the headers to show.
const FIELDS: [&str; 12] = [
    "item:Body",
    "item:Attachments",
    "item:Subject",
    "message:From",
    "message:ToRecipients",
    "message:CcRecipients",
    "message:ReplyTo",
    "item:DateTimeSent",
    "message:InternetMessageId",
    "message:InReplyTo",
    "item:Importance",
    "item:HasAttachments",
];

/// A mailbox element as an address.
fn mailbox(m: &El) -> Option<EmailAddress> {
    let email = m.text_of("EmailAddress").filter(|e| e.contains('@'))?;
    Some(EmailAddress::new(m.text_of("Name").unwrap_or_default(), email))
}

fn mailboxes(it: &El, name: &str) -> Vec<EmailAddress> {
    it.child(name).map(|l| l.children("Mailbox").filter_map(mailbox).collect()).unwrap_or_default()
}

impl Client {
    /// Message `id`'s text first, as MIME (`mailcraft_mime::text_first`): `None` when the server
    /// gave no body, or the message has no files (then the whole message is as quick).
    pub fn download_text(&self, id: &str) -> Result<Option<Vec<u8>>> {
        let fields: String = FIELDS.iter().map(|f| format!("<t:FieldURI FieldURI=\"{f}\"/>")).collect();
        // No `BodyType` and no headers asked for: some servers (grommunio) send the whole source
        // with either.
        let m = self.call_one(&format!(
            "<m:GetItem><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties>{fields}</t:AdditionalProperties></m:ItemShape>\
             <m:ItemIds><t:ItemId Id=\"{}\"/></m:ItemIds></m:GetItem>",
            esc(id)
        ))?;
        Ok(text_first(&m))
    }
}

/// An item's text first, as MIME; `None` without a body or files.
pub(super) fn text_first(m: &El) -> Option<Vec<u8>> {
    let it = m.find("Message").or_else(|| m.find("Items").and_then(|i| i.kids.first()))?;
    let body = it.child("Body")?;
    let mut files = Vec::new();
    if let Some(list) = it.child("Attachments") {
        for a in &list.kids {
            let name = a.text_of("Name").unwrap_or_default().to_string();
            let mime = match a.name.as_str() {
                // An attached message (an `ItemAttachment`) is a message.
                "ItemAttachment" => "message/rfc822".to_string(),
                _ => a.text_of("ContentType").unwrap_or("application/octet-stream").to_string(),
            };
            let size = a.text_of("Size").and_then(|s| s.trim().parse::<u64>().ok()).unwrap_or(0);
            let content_id = a.text_of("ContentId").map(str::to_string).filter(|c| !c.trim().is_empty());
            let inline = a.text_of("IsInline").is_some_and(|v| v.eq_ignore_ascii_case("true")) && content_id.is_some();
            files.push(mailcraft_mime::PendingFile { name, mime, size, content_id, inline });
        }
    }
    if files.is_empty() {
        return None;
    }
    let html = body.attr("BodyType").is_none_or(|t| t.eq_ignore_ascii_case("HTML"));
    let d = mailcraft_mime::Draft {
        to: mailboxes(it, "ToRecipients"),
        cc: mailboxes(it, "CcRecipients"),
        reply_to: mailboxes(it, "ReplyTo"),
        subject: it.text_of("Subject").unwrap_or_default().to_string(),
        body: if html { String::new() } else { body.text.clone() },
        html: html.then(|| body.text.clone()),
        in_reply_to: it.text_of("InReplyTo").unwrap_or_default().to_string(),
        importance: match it.text_of("Importance") {
            Some("High") => 1,
            Some("Low") => -1,
            _ => 0,
        },
        ..Default::default()
    };
    let from = it.child("From").and_then(|f| f.child("Mailbox")).and_then(mailbox).unwrap_or_default();
    let date = it.text_of("DateTimeSent").and_then(jmap::utc_date).unwrap_or(0);
    let id = it.text_of("InternetMessageId").unwrap_or_default();
    mailcraft_mime::text_first(&d, &from, date, id, &files).ok()
}
