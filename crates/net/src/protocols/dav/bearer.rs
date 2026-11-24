//! Signing in to a calendar and contacts server with an OAuth access token (Google's CalDAV and
//! CardDAV, issues #262 and #263). The token opens the whole Google account, so it goes only to
//! the hosts the session was made for, over https, never to wherever a redirect or a server's
//! href points; and only Google refusing the token itself (a permission it lacks, or the API
//! turned off in the project) is the sign-in's, not a 403 for one calendar or item.

use crate::http::{self, Url};

/// Longest reason of Google's said.
const MAX_REASON: usize = 500;

/// A Bearer sign-in: the `Authorization` value, and the hosts it may go to.
pub(super) struct Bearer {
    header: String,
    hosts: Vec<String>,
    /// What Google said in refusing the token a permission this session: told apart from the
    /// token itself refused (401), which is the whole sign-in's.
    refused: std::sync::Mutex<Vec<String>>,
}

impl Bearer {
    pub(super) fn new(token: &str, hosts: &[&str]) -> Bearer {
        Bearer {
            header: format!("Bearer {token}"),
            hosts: hosts.iter().map(|h| h.to_ascii_lowercase()).collect(),
            refused: std::sync::Mutex::default(),
        }
    }

    /// Google refused the token a permission, saying `why`.
    pub(super) fn note_refused(&self, why: &str) {
        let mut said = self.refused.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if !said.iter().any(|s| s == why) && said.len() < 8 {
            said.push(why.to_string());
        }
    }

    /// Whether `why` is what Google said in refusing the token a permission this session.
    pub(super) fn was_refused(&self, why: &str) -> bool {
        self.refused.lock().unwrap_or_else(std::sync::PoisonError::into_inner).iter().any(|s| s == why)
    }

    /// The `Authorization` value for a request to `url`: `None` (sent without it) to a host the
    /// session wasn't made for, or in the clear to anywhere but this computer (a test server).
    pub(super) fn for_url(&self, url: &str) -> Option<&str> {
        let u = Url::parse(url).ok()?;
        let host = u.host.trim_end_matches('.').to_ascii_lowercase();
        let safe = u.https || http::is_loopback(&host);
        (safe && self.hosts.contains(&host)).then_some(self.header.as_str())
    }
}

/// Why a 403 refused the token itself, from Google's answer: a permission the sign-in didn't
/// grant (`insufficient_scope`), or the API not turned on in the project (`accessNotConfigured`).
/// `None` for any other 403: that one calendar or item, not the sign-in.
pub(super) fn refusal(headers: &[(String, String)], body: &[u8]) -> Option<String> {
    let challenge = headers.iter().filter(|(k, _)| k.eq_ignore_ascii_case("WWW-Authenticate")).find(|(_, v)| v.contains("insufficient_scope"));
    if let Some((_, v)) = challenge {
        let scope = v.split_once("scope=\"").and_then(|(_, after)| after.split_once('"')).map(|(s, _)| s);
        return Some(match scope {
            Some(s) => format!("sign in again to allow MailCraft this permission: {}", capped(s)),
            None => "sign in again to allow MailCraft this permission".into(),
        });
    }
    let body = String::from_utf8_lossy(body.get(..body.len().min(64 << 10)).unwrap_or_default());
    if !body.contains("accessNotConfigured") && !body.contains("SERVICE_DISABLED") {
        return None;
    }
    let reason = body.split_once("<internalReason>").and_then(|(_, after)| after.split_once("</internalReason>")).map(|(r, _)| r.trim());
    Some(match reason {
        Some(r) if !r.is_empty() => capped(r).to_string(),
        _ => "the server's API isn't turned on for this Google client".into(),
    })
}

/// `s`, cut at [`MAX_REASON`] bytes on a character's boundary.
fn capped(s: &str) -> &str {
    let mut end = s.len().min(MAX_REASON);
    while !s.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    s.get(..end).unwrap_or_default()
}
