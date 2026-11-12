//! Commands: an account's signatures by name, which goes into new messages and which into
//! replies and forwards (Outlook's Signatures and Stationery), and the Signature menu of a
//! message being written (issue #11).

use mailcraft_model::Signature;
use mailcraft_model::signature::FIRST_SIGNATURE;

use super::*;

pub(super) fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("account.setSignature", "Signature", "File › Options › Mail › Signatures", set_signature).params(
            r#"{"account": string, "signature": string, "name"?: string (default: the one for new messages, else the first; a new name adds one; empty text and code remove it), "was"?: string (its name before, renamed), "html"?: string (its look: bold, colours, links; the text then comes from it), "code"?: string (HTML code sent as written, a business card in columns; the text and look then come from it)}"#,
        ),
        CommandSpec::new("account.removeSignature", "Delete signature", "File › Options › Mail › Signatures", remove_signature)
            .params(r#"{"account": string, "name": string}"#),
        CommandSpec::new("account.signatureDefaults", "Choose default signature", "File › Options › Mail › Signatures", signature_defaults)
            .params(r#"{"account": string, "new"?: string|null (the one for new messages; null: none), "reply"?: string|null (for replies and forwards)}"#),
        CommandSpec::new("account.signatures", "Signatures", "File › Options › Mail › Signatures", list_signatures).params(r#"{"account": string}"#),
        CommandSpec::new("compose.insertSignature", "Insert signature", "Message › Signature", insert_signature)
            .params(r#"{"name"?: string (default: the one in the message, else the account's for this kind of message, else its first), "at"?: number (the character it goes at, when there's none in the message to replace; default where a signature goes: above the quote of a reply or forward, else the end)}"#)
            .when(has_compose),
    ]
}

/// Longest signature HTML taken: its formatting, and its pictures written in.
const SIGNATURE_HTML_MAX: usize = 200_000 + mailcraft_mime::SIGNATURE_PICTURES * (mailcraft_mime::SIGNATURE_PICTURE_MAX / 3 * 4 + 64);
/// Longest signature text taken.
const SIGNATURE_TEXT_MAX: usize = 10_000;

fn config_of(s: &Session, v: &Value) -> Result<AccountConfig, CmdError> {
    let id = p::req_str(v, "account")?;
    s.account(id).map(|a| a.config.clone()).ok_or_else(|| CmdError::Params(format!("no account `{id}`")))
}

/// A signature from the editor: its text, its look as HTML (kept as the editor would send it,
/// with its pictures written in, issue #11, its text the signature's), or HTML code sent as
/// written, the editor showing its text.
fn signature_param(v: &Value, name: String) -> Result<Signature, CmdError> {
    let mut sig = Signature { name, text: p::req_str(v, "signature")?.chars().take(SIGNATURE_TEXT_MAX).collect(), ..Default::default() };
    sig.code = p::str(v, "code").map(mailcraft_mime::signature_code).unwrap_or_default();
    let html = if sig.code.is_empty() { p::str(v, "html").map(str::to_string) } else { Some(sig.code.clone()) };
    if let Some(html) = html.filter(|h| !h.trim().is_empty()) {
        let html: String = html.chars().take(SIGNATURE_HTML_MAX).collect();
        let (text, styles, pictures) = mailcraft_mime::signature_html(&html);
        if !text.trim().is_empty() && !styles.is_plain() {
            sig.html = mailcraft_mime::embed_pictures(&mailcraft_mime::rich::to_html(&text, &styles), &pictures);
        }
        if !text.trim().is_empty() && (!styles.is_plain() || !sig.code.is_empty()) {
            // Without its pictures, nor the lines they had to themselves (a card's photo).
            sig.text = text.replace(mailcraft_mime::rich::PICTURE, "").trim_start_matches(['\n', '\r']).chars().take(SIGNATURE_TEXT_MAX).collect();
        }
    }
    Ok(sig)
}

pub(super) fn set_signature(s: &mut Session, v: &Value) -> CmdResult {
    let mut config = config_of(s, v)?;
    let was = p::str(v, "was");
    let name = p::str(v, "name")
        .or(was)
        .or(config.signature_new.as_deref())
        .or(config.signatures.first().map(|s| s.name.as_str()))
        .unwrap_or(FIRST_SIGNATURE)
        .to_string();
    let sig = signature_param(v, name)?;
    // Nothing in it: it goes (as an empty signature always meant none).
    if sig.is_empty() {
        config.remove_signature(was.unwrap_or(&sig.name));
        s.update_account(config, None)?;
        return Ok(json!({"removed": sig.name}));
    }
    let name = sig.name.trim().to_string();
    config.keep_signature(was, sig).map_err(CmdError::Params)?;
    s.update_account(config, None)?;
    Ok(json!({"saved": name}))
}

pub(super) fn remove_signature(s: &mut Session, v: &Value) -> CmdResult {
    let mut config = config_of(s, v)?;
    let name = p::req_str(v, "name")?;
    if !config.remove_signature(name) {
        return Err(CmdError::Params(format!("no signature called {name}")));
    }
    s.update_account(config, None)?;
    Ok(json!({"removed": name}))
}

pub(super) fn signature_defaults(s: &mut Session, v: &Value) -> CmdResult {
    let mut config = config_of(s, v)?;
    for (key, slot) in [("new", &mut config.signature_new), ("reply", &mut config.signature_reply)] {
        match v.get(key) {
            None => {}
            Some(Value::Null) => *slot = None,
            Some(Value::String(n)) if n.trim().is_empty() => *slot = None,
            Some(Value::String(n)) => {
                let known = config.signatures.iter().find(|s| s.name.eq_ignore_ascii_case(n.trim())).map(|s| s.name.clone());
                *slot = Some(known.ok_or_else(|| CmdError::Params(format!("no signature called {n}")))?);
            }
            Some(_) => return Err(CmdError::Params(format!("`{key}` is a signature's name, or null for none"))),
        }
    }
    let out = json!({"new": config.signature_new, "reply": config.signature_reply});
    s.update_account(config, None)?;
    Ok(out)
}

pub(super) fn list_signatures(s: &mut Session, v: &Value) -> CmdResult {
    let config = config_of(s, v)?;
    let list: Vec<Value> = config
        .signatures
        .iter()
        .map(|g| json!({"name": g.name, "text": g.text, "formatted": !g.html.is_empty(), "code": !g.code.is_empty()}))
        .collect();
    Ok(json!({"signatures": list, "new": config.signature_new, "reply": config.signature_reply}))
}

/// A signature into the message being written (Outlook's Insert › Signature, or one picked from
/// its list): in place of the one there, else at character `at` (else where a signature goes) on a line of its
/// own after the usual `-- ` line.
pub(super) fn insert_signature(s: &mut Session, v: &Value) -> CmdResult {
    let (account, current, reply, from) = s
        .compose
        .as_ref()
        .map(|c| (c.account.clone(), c.signature_name.clone(), c.mode.is_some(), c.from.clone()))
        .ok_or_else(|| CmdError::Disabled("no message is being written".into()))?;
    let sig = match p::str(v, "name") {
        Some(name) => Some(s.signature_of(&account, name).ok_or_else(|| CmdError::Params(format!("no signature called {name}")))?),
        None => s
            .signature_of(&account, &current)
            .or_else(|| s.signature_for(&account, &from, reply))
            .or_else(|| s.account(&account).and_then(|a| a.config.signatures.first().cloned())),
    };
    let sig = sig
        .filter(|g| !crate::Session::signature_text(Some(g)).is_empty())
        .ok_or_else(|| CmdError::Params("this account has no signature yet: make one with Signatures…".into()))?;
    // No place given: where a signature goes (above a reply's quote, not under it).
    let at = p::u64(v, "at").and_then(|a| usize::try_from(a).ok());
    let range = s.put_signature(Some(&sig), at)?;
    Ok(json!({"inserted": range.start, "end": range.end, "name": sig.name}))
}
