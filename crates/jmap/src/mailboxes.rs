//! Mailboxes as the server describes them, and the folders MailCraft keeps for them: their
//! paths of names, and two mailboxes of one path kept apart.

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
    let paths: Vec<String> = boxes
        .iter()
        .map(|b| {
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
            parts.join("/")
        })
        .collect();
    // Of mailboxes with one path, the one with the lowest id keeps it (whatever order the server
    // lists them in, the same one; ids that are numbers count up, so most often the oldest).
    let mut owner: HashMap<&str, &str> = HashMap::new();
    for (b, path) in boxes.iter().zip(&paths) {
        let id = owner.entry(path.as_str()).or_insert(b.id.as_str());
        if (b.id.len(), b.id.as_str()) < (id.len(), *id) {
            *id = b.id.as_str();
        }
    }
    let owns = |b: &Mailbox, path: &str| !path.is_empty() && owner.get(path) == Some(&b.id.as_str());
    // Two mailboxes with one path (or an unnamed one) stay apart: the others get their id after
    // it. That path can be taken too (a mailbox named so, or an id that reads the same once cut
    // to what a path holds), so each is checked against those given, a number after it until
    // it's free; they're given in the order of their ids, so in the same way whatever order the
    // server lists them in.
    let mut taken: HashSet<String> = boxes.iter().zip(&paths).filter(|(b, p)| owns(b, p)).map(|(_, p)| p.clone()).collect();
    let mut others: Vec<usize> = (0..boxes.len()).filter(|i| !boxes.get(*i).zip(paths.get(*i)).is_some_and(|(b, p)| owns(b, p))).collect();
    others.sort_by_key(|i| boxes.get(*i).map(|b| (b.id.len(), b.id.as_str())));
    let mut apart: HashMap<usize, String> = HashMap::with_capacity(others.len());
    for i in others {
        let (Some(b), Some(path)) = (boxes.get(i), paths.get(i)) else { continue };
        let first = format!("{path}~{}", b.id.chars().filter(char::is_ascii_alphanumeric).take(16).collect::<String>());
        // Free within as many tries as there are mailboxes to have taken them.
        let free =
            std::iter::once(first.clone()).chain((2..boxes.len().saturating_add(3)).map(|n| format!("{first}~{n}"))).find(|p| !taken.contains(p));
        let given = free.unwrap_or(first);
        taken.insert(given.clone());
        apart.insert(i, given);
    }
    let mut out: Vec<(Folder, String)> = Vec::with_capacity(boxes.len());
    for (i, (b, path)) in boxes.iter().zip(&paths).enumerate() {
        let path = apart.remove(&i).unwrap_or_else(|| path.clone());
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
