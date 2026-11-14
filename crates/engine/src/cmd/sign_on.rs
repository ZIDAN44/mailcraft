//! Signing in through a single sign-on (OpenID Connect), for a JMAP server that takes no
//! passwords: how a server is signed in to (`account.probeJmap`), asked before its account is
//! added, and the service an account signs in with (`"oauth": "oidc"` with `issuer`,
//! `clientId` and `scope`, in `account.add` and `account.update`).

use mailcraft_model::{JmapProbe, OIDC, OidcSettings};

use super::*;

/// The most servers whose answers are kept: the add dialog asks about a few.
const MAX_PROBES: usize = 64;
const MAX_ADDRESS: usize = OidcSettings::MAX_ADDRESS;
const MAX_CLIENT_ID: usize = OidcSettings::MAX_CLIENT_ID;

pub(super) fn commands() -> Vec<CommandSpec> {
    vec![CommandSpec::new("account.probeJmap", "How the server is signed in to", "File › Accounts", probe_jmap).params(
        r#"{"url": string (a JMAP session address, or the server's name), "again"?: bool (ask it again)} (asks once; call again for the answer: {"pending": true}, then {"password": bool (it takes a user name and password), "bearer": bool (it takes a sign-in through the browser), "issuer"?: string (the single sign-on service that signs its users in), "scope"?: string (what it asks for), "clientId"?: string (the one MailCraft was given for that service), "error"?: string (why it, or its service, couldn't be asked: "again" asks again)})"#,
    )]
}

fn probe_jmap(s: &mut Session, v: &Value) -> CmdResult {
    let url = p::req_str(v, "url")?.trim();
    if url.is_empty() || url.chars().count() > MAX_ADDRESS || url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(CmdError::Params("`url` is a JMAP session address, or the server's name".into()));
    }
    if p::bool(v, "again").unwrap_or(false) {
        s.jmap_probes.remove(url);
    }
    let mut answer = match s.jmap_probes.get(url) {
        Some(Some(probe)) => serde_json::to_value(probe).map_err(|e| CmdError::Failed(e.to_string()))?,
        Some(None) => json!({"pending": true}),
        None => {
            if s.jmap_probes.len() >= MAX_PROBES {
                s.jmap_probes.clear();
            }
            s.jmap_probes.insert(url.to_string(), None);
            s.push_job(mailcraft_model::Job::ProbeJmap { url: url.to_string() });
            json!({"pending": true})
        }
    };
    if let Some(o) = answer.as_object_mut() {
        o.insert("url".into(), json!(url));
    }
    Ok(answer)
}

/// An issuer as an account keeps it: https (plain http only to this computer's loopback
/// address, a test service), no user, query or fragment, nothing invisible, at most
/// [`MAX_ADDRESS`] characters; its trailing `/` dropped.
pub(super) fn issuer_param(typed: &str) -> Result<String, CmdError> {
    let bad = || CmdError::Params("`issuer` is the sign-in service's address (https://sso.example.com)".into());
    let issuer = typed.trim().trim_end_matches('/');
    if issuer.chars().count() > MAX_ADDRESS || issuer.chars().any(|c| c.is_whitespace() || c.is_control()) || issuer.contains(['?', '#']) {
        return Err(bad());
    }
    let (https, rest) = match (issuer.strip_prefix("https://"), issuer.strip_prefix("http://")) {
        (Some(rest), _) => (true, rest),
        (None, Some(rest)) => (false, rest),
        (None, None) => return Err(bad()),
    };
    let authority = rest.split('/').next().unwrap_or_default();
    if authority.is_empty() || authority.contains('@') {
        return Err(bad());
    }
    // Its host: a bracketed IPv6 address, or what comes before a port.
    let host = match authority.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or_default(),
        None => authority.rsplit_once(':').map_or(authority, |(h, _)| h),
    };
    if !https && !mailcraft_model::loopback(host) {
        return Err(CmdError::Params("`issuer` isn't https: only a test service at this computer's loopback address can be plain http".into()));
    }
    Ok(issuer.to_string())
}

/// A client id as its administrator gave it: trimmed, at most [`MAX_CLIENT_ID`] characters,
/// nothing invisible. Empty: the one MailCraft was given for the issuer.
fn client_id_param(typed: &str) -> Result<String, CmdError> {
    let c = typed.trim();
    if c.chars().count() > MAX_CLIENT_ID || c.chars().any(char::is_control) {
        return Err(CmdError::Params(format!("`clientId` is at most {MAX_CLIENT_ID} characters, none of them invisible")));
    }
    Ok(c.to_string())
}

/// The scopes to ask for, with spaces between them (RFC 6749 §3.3). Empty: what the server asks
/// for, else `openid profile email`.
fn scope_param(typed: &str) -> Result<String, CmdError> {
    let scopes: Vec<&str> = typed.split_ascii_whitespace().collect();
    let scope = |s: &&str| s.len() <= 256 && s.bytes().all(|b| b.is_ascii_graphic() && b != b'"' && b != b'\\');
    if scopes.len() > OidcSettings::MAX_SCOPES || !scopes.iter().all(scope) {
        return Err(CmdError::Params("`scope` is a few scopes with spaces between them (openid profile email)".into()));
    }
    Ok(scopes.join(" "))
}

/// The single sign-on of an account that signs in to the JMAP server at `jmap` through one:
/// what `v` says (`issuer`, `clientId`, `scope`), else what the account had (`current`), else
/// what asking the server found (`account.probeJmap`), which comes first for a server the
/// account was `moved` to: its service may be another. Another issuer starts from nothing: the
/// client id and scopes were the other service's. The client id MailCraft was given for the
/// issuer isn't kept with the account: it's looked up as it signs in, so a new one given with
/// MailCraft is the one it signs in under.
pub(super) fn oidc_settings(s: &Session, v: &Value, jmap: &str, current: Option<&OidcSettings>, moved: bool) -> Result<OidcSettings, CmdError> {
    let found = s.jmap_probes.get(jmap.trim()).and_then(Option::as_ref).filter(|f| f.bearer);
    let given = p::str(v, "issuer").map(issuer_param).transpose()?;
    let mut o = current.cloned().unwrap_or_default();
    let named = found.and_then(JmapProbe::service).and_then(|i| issuer_param(i).ok());
    let issuer = match (given, named) {
        (Some(i), _) => i,
        (None, Some(n)) if moved || o.issuer.is_empty() => n,
        (None, _) => o.issuer.clone(),
    };
    if issuer.is_empty() {
        return Err(CmdError::Params("`issuer` is required: the sign-in service's address (account.probeJmap finds it)".into()));
    }
    if issuer != o.issuer {
        o = OidcSettings { issuer, ..OidcSettings::default() };
    }
    // What the server was found to ask for is its own service's.
    let found = found.filter(|f| f.service().map(|i| i.trim().trim_end_matches('/')) == Some(o.issuer.as_str()));
    if let Some(c) = p::str(v, "clientId") {
        o.client_id = client_id_param(c)?;
    }
    match p::str(v, "scope") {
        Some(sc) => o.scope = scope_param(sc)?,
        None if o.scope.is_empty() => o.scope = found.map(|f| f.scope.clone()).unwrap_or_default(),
        None => {}
    }
    Ok(o)
}

/// `account.update`'s sign-in, for `c` with its new settings. `oauth`: "oidc" for a JMAP account
/// to sign in through its single sign-on, with `issuer`, `clientId` and `scope` as `account.add`
/// takes them (which change it, for one that does already, as moving to another server does);
/// null to sign in with a password again, given as `password` and returned to be kept. An
/// account that signs in with Google or Microsoft stays as it is.
pub(super) fn sign_in_param(s: &Session, v: &Value, c: &mut AccountConfig) -> Result<Option<String>, CmdError> {
    let other = |o: &str| CmdError::Params(format!("this account signs in with {o}"));
    // Moved to another JMAP server, whose single sign-on may be another.
    let moved = s.account(&c.id).is_some_and(|a| a.config.jmap != c.jmap);
    match v.get("oauth") {
        None if !c.signs_on() => Ok(None),
        None => {
            let Some(jmap) = c.jmap.clone() else {
                return Err(CmdError::Params("an account that signs in through a single sign-on keeps its JMAP server".into()));
            };
            if moved || ["issuer", "clientId", "scope"].iter().any(|k| v.get(*k).is_some()) {
                c.oidc = Some(oidc_settings(s, v, &jmap, c.oidc.as_ref(), moved)?);
            }
            Ok(None)
        }
        Some(Value::String(o)) if o == OIDC => {
            if let Some(o) = c.oauth.as_deref().filter(|o| *o != OIDC) {
                return Err(other(o));
            }
            let Some(jmap) = c.jmap.clone() else {
                return Err(CmdError::Params("only a JMAP account signs in through a single sign-on".into()));
            };
            c.oidc = Some(oidc_settings(s, v, &jmap, c.oidc.as_ref(), moved)?);
            c.oauth = Some(OIDC.into());
            Ok(None)
        }
        Some(Value::Null) => match c.oauth.as_deref() {
            None => Ok(None),
            Some(OIDC) => {
                let password = p::str(v, "password").filter(|p| !p.is_empty());
                let password = password.ok_or_else(|| CmdError::Params("`password` is required to sign in with one".into()))?;
                c.oauth = None;
                Ok(Some(password.to_string()))
            }
            Some(o) => Err(other(o)),
        },
        Some(_) => Err(CmdError::Params("`oauth` is \"oidc\" or null".into())),
    }
}
