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

use mailcraft_model::{JmapProbe, OidcSettings};
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
const MAX_FIELD: usize = 2048;
/// The most scopes kept from a description.
const MAX_SUPPORTED: usize = 256;
/// The most scopes asked for.
const MAX_SCOPES: usize = 32;
/// The longest client id.
pub const MAX_CLIENT_ID: usize = 256;

/// What a sign-in service says of itself (OIDC Discovery §3), as far as signing in needs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Description {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    /// The scopes it says it takes (`offline_access` among them, or not).
    pub scopes_supported: Vec<String>,
}

/// Whether `host` is this computer's loopback address, written as one (a name could be
/// anything's).
pub(crate) fn loopback(host: &str) -> bool {
    host.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

/// Https, or plain http to a test service at this computer's loopback address.
fn secure(url: &Url) -> bool {
    url.https || loopback(&url.host)
}

/// Where the sign-in service of the mail server at `server` (a JMAP session address) describes
/// itself: under the server's own origin, which sends on to the service's.
pub fn description_url_for_server(server: &str) -> Result<String> {
    Ok(format!("{}{WELL_KNOWN}", Url::parse(server)?.origin()))
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

/// At most 120 characters of something a server said, for a message.
fn clip(s: &str) -> String {
    s.chars().take(120).collect()
}

/// A scope (RFC 6749 §3.3: visible ASCII but `"` and `\`), short.
fn scope_token(s: &str) -> bool {
    !s.is_empty() && s.len() <= 256 && s.bytes().all(|b| b.is_ascii_graphic() && b != b'"' && b != b'\\')
}

/// A service's description at `url`, the redirects followed. Hostile input: capped, over https
/// only (plain http to this computer's loopback address alone, for a test service, whose
/// endpoints must be there too), and the issuer it names must be the one at the address it came
/// from (OIDC Discovery §4.3): a description can't speak for another service.
pub fn describe(url: &str) -> Result<Description> {
    let not_https = |what: &str| NetError::Tls(format!("the sign-in service's {what} isn't https"));
    if !secure(&Url::parse(url)?) {
        return Err(not_https("address"));
    }
    let (resp, at) = crate::http::send_where(&Request::get(url, MAX_DESCRIPTION).header("Accept", "application/json"))?;
    if !secure(&at) {
        return Err(not_https("address"));
    }
    if !resp.ok() {
        return Err(NetError::Protocol(format!("the sign-in service's description answered HTTP {}", resp.status)));
    }
    let v: Value = serde_json::from_slice(&resp.body).map_err(|_| NetError::Protocol("the sign-in service's description isn't JSON".into()))?;
    let text = |k: &str| {
        v.get(k)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty() && s.len() <= MAX_FIELD && !s.chars().any(|c| c.is_whitespace() || c.is_control()))
    };
    let missing = |k: &str| NetError::Protocol(format!("the sign-in service's description has no `{k}`"));
    let issuer = text("issuer").ok_or_else(|| missing("issuer"))?;
    let here = at.path.strip_suffix(WELL_KNOWN).map(|p| format!("{}{p}", at.origin()));
    let theirs = issuer_key(issuer);
    if theirs.is_none() || here.as_deref().and_then(issuer_key) != theirs {
        let at = here.unwrap_or_else(|| format!("{}{}", at.origin(), at.path));
        return Err(NetError::Protocol(format!("the sign-in service at {} says it's {}", clip(&at), clip(issuer))));
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

/// A client id as one can be sent: not empty, short, nothing invisible.
fn client_ok(c: &str) -> bool {
    !c.is_empty() && c.len() <= MAX_CLIENT_ID && !c.chars().any(char::is_control)
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
/// itself, under its client id, asking for its scopes. A request to the service: only ever on a
/// thread of its own.
pub fn provider(oidc: &OidcSettings) -> Result<Provider> {
    let client_id = client_id(oidc).ok_or_else(|| NetError::Auth(mailcraft_model::NO_CLIENT_ID.into()))?;
    let d = describe(&description_url(&oidc.issuer))?;
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

/// The most of a session's answer without a sign-in that's read.
const MAX_REFUSAL: usize = 64 << 10;

/// How the JMAP server at `server` (a session address, or the server's name) is signed in to:
/// what it answers a request without a sign-in, and where it takes bearer tokens, the single
/// sign-on service its address names (as [`describe`] finds it, from wherever the session
/// address sent the request). Only advice for the add dialog: what can't be asked is a password,
/// whose usual sign-in then says what's wrong. A request to the server: only ever on a thread of
/// its own.
pub fn probe(server: &str) -> JmapProbe {
    let password = JmapProbe { password: true, ..JmapProbe::default() };
    let session = mailcraft_jmap::session_url(server);
    let Ok((resp, at)) = crate::http::send_where(&Request::get(&session, MAX_REFUSAL).header("Accept", "application/json")) else { return password };
    if resp.status != 401 {
        return password;
    }
    let challenges = mailcraft_jmap::Response { status: resp.status, headers: resp.headers, body: Vec::new() }.challenges();
    let offers = |scheme: &str| challenges.iter().find(|c| c.scheme == scheme);
    let Some(bearer) = offers("bearer") else { return password };
    let mut found = JmapProbe {
        password: offers("basic").is_some(),
        bearer: true,
        scope: scopes(bearer.param("scope").unwrap_or_default()).join(" "),
        ..JmapProbe::default()
    };
    match describe(&format!("{}{WELL_KNOWN}", at.origin())) {
        Ok(d) => {
            found.client_id = given_client(&d.issuer);
            found.issuer = Some(d.issuer);
        }
        Err(e) => found.error = Some(e.to_string()),
    }
    found
}

#[cfg(test)]
mod tests;
