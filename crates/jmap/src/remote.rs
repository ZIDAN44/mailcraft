//! What changes on the server: flags, moves and folders, mail stored and sent there; and message
//! sources downloaded from it.

use super::*;

/// Download one message's source (and cache it), with its summary as its folder now has it, filled
/// from it ([`keep_source`]); one cached already comes as it is, with no summary.
pub async fn fetch_body<H: Http>(c: &Client<H>, st: &State, store: &Store, key: &MessageKey) -> Result<(Vec<u8>, Option<Summary>)> {
    if let Some(raw) = store.read_body(key) {
        return Ok((raw, None));
    }
    let gone = || Error::No("the message is no longer on the server".into());
    let id = st.id(key.uid).filter(|_| key.validity == VALIDITY).ok_or_else(gone)?.to_string();
    let blob = c.blobs(std::slice::from_ref(&id)).await?.remove(&id).ok_or_else(gone)?;
    let raw = c.download(&blob).await?;
    let filled = keep_source(store, key, &raw)?;
    Ok((raw, filled))
}

/// Apply one change on the server.
pub async fn apply_remote<H: Http>(c: &Client<H>, st: &State, op: &Op) -> Result<()> {
    let ids = |keys: &[MessageKey]| -> Vec<(String, String)> {
        keys.iter().filter(|k| k.validity == VALIDITY).filter_map(|k| Some((st.id(k.uid)?.to_string(), k.folder.clone()))).collect()
    };
    let patch_all = |keys: &[MessageKey], field: &str, on: bool| -> Map<String, Value> {
        let value = if on { Value::Bool(true) } else { Value::Null };
        ids(keys).into_iter().map(|(id, _)| (id, json!({ field: value.clone() }))).collect()
    };
    match op {
        Op::SetFlag { keys, flag, on } => match flag_keyword(*flag) {
            Some(k) => c.set_emails(patch_all(keys, &format!("keywords/{k}"), *on), Vec::new()).await,
            None => Ok(()),
        },
        Op::SetKeyword { keys, keyword, on } if mailcraft_model::is_keyword(keyword) => {
            c.set_emails(patch_all(keys, &format!("keywords/{}", keyword.to_ascii_lowercase()), *on), Vec::new()).await
        }
        Op::SetKeyword { .. } => Ok(()),
        Op::Move { keys, to } => {
            let to_box = st.mailbox(to)?.to_string();
            let mut update = Map::new();
            for (id, from) in ids(keys) {
                if from == *to {
                    continue;
                }
                let mut p = Map::new();
                if let Ok(b) = st.mailbox(&from) {
                    p.insert(format!("mailboxIds/{b}"), Value::Null);
                }
                p.insert(format!("mailboxIds/{to_box}"), Value::Bool(true));
                update.insert(id, Value::Object(p));
            }
            c.set_emails(update, Vec::new()).await
        }
        Op::Expunge { keys } => c.set_emails(Map::new(), ids(keys).into_iter().map(|(id, _)| id).collect()).await,
        Op::CreateFolder { path } => {
            let (parent, name) = split_path(st, path)?;
            c.set_mailboxes(json!({"create": {"f": {"name": name, "parentId": parent}}})).await.map(|_| ())
        }
        Op::RenameFolder { from, to } => {
            let id = st.mailbox(from)?.to_string();
            let (parent, name) = split_path(st, to)?;
            c.set_mailboxes(json!({"update": {id: {"name": name, "parentId": parent}}})).await.map(|_| ())
        }
        Op::DeleteFolder { path } => {
            let id = st.mailbox(path)?.to_string();
            c.set_mailboxes(json!({"destroy": [id], "onDestroyRemoveEmails": true})).await.map(|_| ())
        }
        Op::EmptyFolder { path } => c.empty(st.mailbox(path)?).await,
    }
}

/// The parent mailbox id (`None` at the top) and the leaf name of a folder path.
pub(super) fn split_path(st: &State, path: &str) -> Result<(Option<String>, String)> {
    match path.rsplit_once('/') {
        Some((parent, leaf)) => Ok((Some(st.mailbox(parent)?.to_string()), utf7::decode(leaf))),
        None => Ok((None, utf7::decode(path))),
    }
}

/// Store a message in a folder (a draft, or the copy of a sent message).
pub async fn append<H: Http>(c: &Client<H>, st: &State, folder: &str, raw: &[u8], keywords: &[&str]) -> Result<String> {
    c.import(raw, st.mailbox(folder)?, keywords).await
}

/// Send a message: stored in the Sent folder, then submitted for the server to deliver (so the
/// copy in Sent needs no second upload).
pub async fn send<H: Http>(c: &Client<H>, st: &State, sent: Option<&str>, raw: &[u8], from: &str, recipients: &[String]) -> Result<()> {
    let folder = sent.filter(|f| st.mailboxes.contains_key(*f)).ok_or_else(|| Error::Send("the account has no Sent folder to send from".into()))?;
    let id = append(c, st, folder, raw, &["$seen"]).await?;
    match c.submit(&id, from, recipients).await {
        Ok(()) => Ok(()),
        Err(e) => {
            // Not sent: don't leave a copy that looks sent.
            let _ = c.set_emails(Map::new(), vec![id]).await;
            Err(e)
        }
    }
}
