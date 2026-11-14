//! Mailboxes as the server describes them, and the folders they become.

use super::*;

/// A mailbox as the server describes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mailbox {
    pub id: String,
    pub name: String,
    pub parent: Option<String>,
    pub role: FolderRole,
    pub total: u32,
    pub unread: u32,
}

impl Mailbox {
    pub(super) fn from_json(v: &Value) -> Option<Mailbox> {
        Some(Mailbox {
            id: v.get("id")?.as_str()?.to_string(),
            name: v.get("name").and_then(Value::as_str).unwrap_or_default().chars().take(255).collect(),
            parent: v.get("parentId").and_then(Value::as_str).map(str::to_string),
            role: v.get("role").and_then(Value::as_str).and_then(FolderRole::from_code).unwrap_or(FolderRole::Other),
            total: count(v, "totalEmails"),
            unread: count(v, "unreadEmails"),
        })
    }
}

/// Folders for mailboxes, each with its mailbox id. Paths are the names from the top joined by
/// `/` (in modified UTF-7, as IMAP names are kept); the Inbox is `INBOX`.
pub fn folders(account: &str, boxes: &[Mailbox]) -> Vec<(Folder, String)> {
    let by_id: HashMap<&str, &Mailbox> = boxes.iter().map(|b| (b.id.as_str(), b)).collect();
    let mut out: Vec<(Folder, String)> = Vec::new();
    for b in boxes {
        let mut parts: Vec<String> = Vec::new();
        let mut cur = Some(b);
        while let Some(m) = cur {
            if parts.len() >= MAX_DEPTH {
                break;
            }
            parts.push(if m.role == FolderRole::Inbox { INBOX.to_string() } else { utf7::encode(&m.name.replace('/', "∕")) });
            cur = m.parent.as_deref().and_then(|p| by_id.get(p).copied()).filter(|p| p.id != b.id);
        }
        parts.reverse();
        let mut path = parts.join("/");
        // Two mailboxes with one path (or an unnamed one) stay apart.
        if path.is_empty() || out.iter().any(|(f, _)| f.path == path) {
            path = format!("{path}~{}", b.id.chars().filter(char::is_ascii_alphanumeric).take(16).collect::<String>());
        }
        let folder = Folder {
            account: account.to_string(),
            path,
            delimiter: Some('/'),
            role: b.role,
            selectable: true,
            unread: b.unread,
            total: b.total,
            ..Default::default()
        };
        out.push((folder, b.id.clone()));
    }
    out
}

/// Special folders keep the names their mailbox gives them (Exchange names them in the
/// language the mailbox was made in: "Postvak IN", "Verzonden items"), shown as Outlook shows
/// them rather than in MailCraft's own words.
pub fn name_special(found: &mut [(Folder, String)], boxes: &[Mailbox]) {
    let names: HashMap<&str, &str> = boxes.iter().map(|b| (b.id.as_str(), b.name.trim())).collect();
    for (f, id) in found {
        if f.role != FolderRole::Other
            && let Some(name) = names.get(id.as_str()).filter(|n| !n.is_empty())
        {
            f.name = Some((*name).to_string());
        }
    }
}

/// Refresh the folder list (and the mailbox ids behind it).
pub async fn list_folders<H: Http>(c: &Client<H>, st: &mut State, account: &str) -> Result<Vec<Folder>> {
    let found = folders(account, &c.mailboxes().await?);
    st.mailboxes = found.iter().map(|(f, id)| (f.path.clone(), id.clone())).collect();
    Ok(found.into_iter().map(|(f, _)| f).collect())
}
