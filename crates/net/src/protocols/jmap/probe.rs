//! How a JMAP server is signed in to, asked before its account is added: what its session
//! answers a request without a sign-in (`WWW-Authenticate`, RFC 9110 §11.6.1), and for one that
//! takes bearer tokens (RFC 6750), the single sign-on service its address names.

use mailcraft_jmap::challenge;
use mailcraft_model::JmapProbe;

use crate::http::{self, Request};
use crate::oauth::oidc;

/// The most of a session's answer without a sign-in that's read.
const MAX_REFUSAL: usize = 64 << 10;

/// How the JMAP server at `server` (a session address, or the server's name) is signed in to:
/// what it answers a request without a sign-in, and where it takes bearer tokens, the single
/// sign-on service its address names (as [`oidc::describe`] finds it, from wherever the session
/// address sent the request). Only advice for the add dialog: what can't be asked is a password,
/// whose usual sign-in then says what's wrong, with why it couldn't be asked (it may be passing:
/// a server that's down, a network that isn't up yet). A request to the server: only ever on a
/// thread of its own.
pub fn probe(server: &str) -> JmapProbe {
    let password = JmapProbe { password: true, ..JmapProbe::default() };
    let session = mailcraft_jmap::session_url(server);
    let (resp, at) = match http::send_where(&Request::get(&session, MAX_REFUSAL).header("Accept", "application/json")) {
        Ok(r) => r,
        Err(e) => return JmapProbe { error: Some(e.to_string()), ..password },
    };
    if resp.status >= 500 {
        return JmapProbe { error: Some(format!("the server answered HTTP {}", resp.status)), ..password };
    }
    if resp.status != 401 {
        return password;
    }
    let challenges = challenge::in_headers(&resp.headers);
    let Some(bearer) = challenge::offered(&challenges, "bearer") else { return password };
    let mut found = JmapProbe {
        password: challenge::offered(&challenges, "basic").is_some(),
        bearer: true,
        scope: oidc::scopes(bearer.param("scope").unwrap_or_default()).join(" "),
        ..JmapProbe::default()
    };
    match oidc::describe(&oidc::description_url(&at.origin())) {
        Ok(d) => {
            found.client_id = oidc::given_client(&d.issuer);
            found.issuer = Some(d.issuer);
        }
        Err(e) => found.error = Some(e.to_string()),
    }
    found
}

#[cfg(test)]
mod tests;
