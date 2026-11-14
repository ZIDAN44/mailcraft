//! Signing in through the browser with any OpenID Connect service: a JMAP server behind a single
//! sign-on takes bearer tokens only (`WWW-Authenticate: Bearer`, RFC 6750), which its users get
//! from the service in the browser, as [`authorize`](super::authorize) does for Google and
//! Microsoft. Nothing here is any one service's.
//!
//! The service is found from the mail server: its `/.well-known/openid-configuration` sends on
//! to the service's (OIDC Discovery 1.0), which names where to sign in and where tokens come
//! from. The client id is the one the service's administrator registered MailCraft under: the
//! account's, or else the one MailCraft was given for the issuer in `MAILCRAFT_OIDC_CLIENTS`
//! (`https://sso.example.com=mailcraft;https://kc.example.org/realms/mail=mc-desktop`), read at
//! run time and baked in at build time, run time's first.

use mailcraft_model::{OidcSettings, loopback};
use serde_json::Value;

use super::Provider;
use crate::http::{Request, Url};
use crate::{NetError, Result};

/// Where a service describes itself, under its issuer (OIDC Discovery §4).
pub const WELL_KNOWN: &str = "/.well-known/openid-configuration";

/// What's asked for when the server doesn't say: who signed in (OIDC Core §5.4).
pub const DEFAULT_SCOPE: &str = "openid profile email";

/// The most of a description that's read: they're a few kilobytes.
const MAX_DESCRIPTION: usize = 256 << 10;
/// The longest address or issuer taken from one.
pub const MAX_FIELD: usize = 2048;
/// The most scopes kept from a description.
pub const MAX_SUPPORTED: usize = 256;
/// The most scopes asked for.
pub const MAX_SCOPES: usize = OidcSettings::MAX_SCOPES;

/// What a sign-in service says of itself (OIDC Discovery §3), as far as signing in needs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Description {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    /// The scopes it says it takes (`offline_access` among them, or not).
    pub scopes_supported: Vec<String>,
}

/// Https, or plain http to a test service at this computer's loopback address.
fn secure(url: &Url) -> bool {
    url.https || loopback(&url.host)
}

/// Where `issuer` describes itself.
pub fn description_url(issuer: &str) -> String {
    format!("{}{WELL_KNOWN}", issuer.trim().trim_end_matches('/'))
}

/// An issuer as compared: its origin (the host in lower case, no default port) and its path
/// without a trailing `/`. An issuer has no query or fragment (OIDC Discovery §2).
fn issuer_key(s: &str) -> Option<(String, String)> {
    if s.contains(['?', '#']) {
        return None;
    }
    let u = Url::parse(s).ok()?;
    Some((u.origin(), u.path.trim_end_matches('/').to_string()))
}

/// Whether `a` and `b` are the same issuer ([`issuer_key`]); what isn't one is no issuer's.
fn same_issuer(a: &str, b: &str) -> bool {
    issuer_key(a).is_some_and(|k| issuer_key(b) == Some(k))
}

/// The service at `at` speaks for another, `says`.
fn another_service(at: &str, says: &str) -> NetError {
    NetError::Protocol(format!("the sign-in service at {} says it's {}", clip(at), clip(says)))
}

/// The sign-in service's `what` isn't https.
fn not_https(what: &str) -> NetError {
    NetError::Tls(format!("the sign-in service's {what} isn't https"))
}

/// At most 120 characters of something a server said, for a message.
fn clip(s: &str) -> String {
    s.chars().take(120).collect()
}

/// A scope (RFC 6749 §3.3: visible ASCII but `"` and `\`), short.
fn scope_token(s: &str) -> bool {
    !s.is_empty() && s.len() <= 256 && s.bytes().all(|b| b.is_ascii_graphic() && b != b'"' && b != b'\\')
}

/// A service's description at `url`, the redirects followed, as [`description_from`] reads it.
/// Asked for over https only: plain http to this computer's loopback address alone, for a test
/// service.
pub fn describe(url: &str) -> Result<Description> {
    if !secure(&Url::parse(url)?) {
        return Err(not_https("address"));
    }
    let (resp, at) = crate::http::send_where(&Request::get(url, MAX_DESCRIPTION).header("Accept", "application/json"))?;
    if !resp.ok() {
        return Err(NetError::Protocol(format!("the sign-in service's description answered HTTP {}", resp.status)));
    }
    description_from(&resp.body, &at)
}

/// A service's description, `body`, as it came from `at`. Hostile input: capped, from https
/// only (plain http from this computer's loopback address alone, for a test service, whose
/// endpoints must be there too), and the issuer it names must be the one at `at` (OIDC
/// Discovery §4.3): a description can't speak for another service.
pub fn description_from(body: &[u8], at: &Url) -> Result<Description> {
    if !secure(at) {
        return Err(not_https("address"));
    }
    let v: Value = serde_json::from_slice(body).map_err(|_| NetError::Protocol("the sign-in service's description isn't JSON".into()))?;
    let text = |k: &str| {
        v.get(k)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty() && s.len() <= MAX_FIELD && !s.chars().any(|c| c.is_whitespace() || c.is_control()))
    };
    let missing = |k: &str| NetError::Protocol(format!("the sign-in service's description has no `{k}`"));
    let issuer = text("issuer").ok_or_else(|| missing("issuer"))?;
    let here = at.path.strip_suffix(WELL_KNOWN).map(|p| format!("{}{p}", at.origin()));
    if !here.as_deref().is_some_and(|h| same_issuer(h, issuer)) {
        return Err(another_service(&here.unwrap_or_else(|| format!("{}{}", at.origin(), at.path)), issuer));
    }
    let endpoint = |k: &str| -> Result<String> {
        let e = text(k).filter(|e| !e.contains('#')).ok_or_else(|| missing(k))?;
        let u = Url::parse(e)?;
        // Plain http only where the description itself came from this computer.
        if !(u.https || (!at.https && loopback(&u.host))) {
            return Err(not_https(k));
        }
        Ok(e.to_string())
    };
    // PKCE with S256 is all MailCraft does (RFC 7636 §4.2): never `plain`.
    let methods = v.get("code_challenge_methods_supported").and_then(Value::as_array);
    if methods.is_some_and(|m| !m.iter().any(|m| m.as_str() == Some("S256"))) {
        return Err(NetError::Protocol("the sign-in service doesn't take PKCE with S256".into()));
    }
    let scopes_supported = v
        .get("scopes_supported")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).filter(|s| scope_token(s)).take(MAX_SUPPORTED).map(str::to_string).collect())
        .unwrap_or_default();
    Ok(Description {
        issuer: issuer.to_string(),
        authorization_endpoint: endpoint("authorization_endpoint")?,
        token_endpoint: endpoint("token_endpoint")?,
        scopes_supported,
    })
}

/// The scopes in `s` (space-separated), each once, without what isn't one; at most
/// [`MAX_SCOPES`].
pub fn scopes(s: &str) -> Vec<&str> {
    let mut scopes: Vec<&str> = Vec::new();
    for s in s.split_ascii_whitespace().filter(|s| scope_token(s)) {
        if !scopes.contains(&s) && scopes.len() < MAX_SCOPES {
            scopes.push(s);
        }
    }
    scopes
}

/// The scopes asked for: `wanted` (what the server's `WWW-Authenticate` named, kept with the
/// account), else [`DEFAULT_SCOPE`]; with `offline_access` where the service takes it, so the
/// sign-in lasts (OIDC Core §11). What isn't a scope is left out.
pub fn scope_for(wanted: &str, supported: &[String]) -> String {
    let mut scopes = scopes(wanted);
    if scopes.is_empty() {
        scopes = DEFAULT_SCOPE.split(' ').collect();
    }
    if supported.iter().any(|s| s == "offline_access") && !scopes.contains(&"offline_access") {
        scopes.push("offline_access");
    }
    scopes.join(" ")
}

/// The client ids MailCraft was given, by issuer: `MAILCRAFT_OIDC_CLIENTS` at run time, then as
/// it was at build time.
pub fn clients() -> Vec<(String, String)> {
    clients_from(&std::env::var("MAILCRAFT_OIDC_CLIENTS").unwrap_or_default(), option_env!("MAILCRAFT_OIDC_CLIENTS").unwrap_or_default())
}

/// The pairs given at run time, then those given at build time: the first for an issuer is the
/// one.
fn clients_from(run: &str, built: &str) -> Vec<(String, String)> {
    let mut all = parse_clients(run);
    all.extend(parse_clients(built));
    all
}

/// `issuer=client_id` pairs with `;` between them, spaces around each part trimmed and an
/// issuer's trailing `/` dropped; what isn't one is left out.
pub fn parse_clients(s: &str) -> Vec<(String, String)> {
    s.split(';')
        .filter_map(|pair| {
            let (issuer, client) = pair.split_once('=')?;
            let (issuer, client) = (issuer.trim().trim_end_matches('/'), client.trim());
            (!issuer.is_empty() && client_ok(client)).then(|| (issuer.to_string(), client.to_string()))
        })
        .collect()
}

/// A client id as one can be sent: not empty, as long as an account takes, nothing invisible.
fn client_ok(c: &str) -> bool {
    !c.is_empty() && c.chars().count() <= OidcSettings::MAX_CLIENT_ID && !c.chars().any(char::is_control)
}

/// The client id MailCraft was given for `issuer`, compared as written but for a trailing `/`.
pub fn given_client(issuer: &str) -> Option<String> {
    given_in(&clients(), issuer)
}

fn given_in(clients: &[(String, String)], issuer: &str) -> Option<String> {
    let issuer = issuer.trim().trim_end_matches('/');
    clients.iter().find(|(i, _)| i == issuer).map(|(_, c)| c.clone())
}

/// The client id an account signs in under: its own, else the one MailCraft was given for its
/// issuer.
pub fn client_id(oidc: &OidcSettings) -> Option<String> {
    Some(oidc.client_id.trim()).filter(|c| client_ok(c)).map(str::to_string).or_else(|| given_client(&oidc.issuer))
}

/// The provider an account signs in with: the service its settings name, as that describes
/// itself, under its client id, asking for its scopes. Never another service its address sends
/// the request on to: the account's tokens are its own service's. A request to the service: only
/// ever on a thread of its own.
pub fn provider(oidc: &OidcSettings) -> Result<Provider> {
    let client_id = client_id(oidc).ok_or_else(|| NetError::Auth(mailcraft_model::NO_CLIENT_ID.into()))?;
    let d = describe(&description_url(&oidc.issuer))?;
    if !same_issuer(oidc.issuer.trim(), &d.issuer) {
        return Err(another_service(oidc.issuer.trim(), &d.issuer));
    }
    Ok(provider_from(&d, client_id, &oidc.scope))
}

/// The provider for a service's description.
pub fn provider_from(d: &Description, client_id: String, scope: &str) -> Provider {
    Provider {
        id: mailcraft_model::OIDC.into(),
        // What its refusals are said with ("sso.example.com: invalid_grant").
        name: Url::parse(&d.issuer).map_or_else(|_| clip(&d.issuer), |u| u.host),
        auth_url: d.authorization_endpoint.clone(),
        token_url: d.token_endpoint.clone(),
        scope: scope_for(scope, &d.scopes_supported),
        client_id,
        client_secret: None,
    }
}

#[cfg(test)]
pub(crate) mod tests;
