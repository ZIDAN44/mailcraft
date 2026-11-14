//! How a JMAP server is signed in to, asked before an account is added
//! ([`Job::ProbeJmap`](super::Job::ProbeJmap)): the add dialog offers a sign-in through the
//! browser where the server takes no passwords.

use serde::{Deserialize, Serialize};

/// What a JMAP server's session said to a request without a sign-in (its `WWW-Authenticate`,
/// RFC 9110 §11.6.1), and the single sign-on service its address names.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct JmapProbe {
    /// It takes a user name and password: it asks for them (Basic), says nothing of how, or
    /// couldn't be asked (`error` says why, and so does the usual sign-in).
    pub password: bool,
    /// It takes bearer tokens (RFC 6750): a sign-in through the browser.
    pub bearer: bool,
    /// The single sign-on service (OpenID Connect) that signs its users in, when found.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issuer: Option<String>,
    /// The scopes its `WWW-Authenticate` asks for, space-separated.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub scope: String,
    /// The client id MailCraft was given for that service (`MAILCRAFT_OIDC_CLIENTS`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    /// Why the server couldn't be asked (it wasn't reached, or failed), or why one that takes
    /// bearer tokens has no service that could be found: either may be passing, so it's worth
    /// asking again.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl JmapProbe {
    /// The single sign-on service it signs in through, when it takes bearer tokens and its
    /// service was found.
    pub fn service(&self) -> Option<&str> {
        self.issuer.as_deref().filter(|_| self.bearer)
    }

    /// Whether it takes no password at all: only a sign-in through the browser.
    pub fn takes_no_password(&self) -> bool {
        self.bearer && !self.password
    }

    /// Whether its service was found, but MailCraft has no client id for it: one is typed.
    pub fn needs_client_id(&self) -> bool {
        self.service().is_some() && self.client_id.is_none()
    }

    /// Whether an account can sign in through its service under the client id `typed` (empty:
    /// the one MailCraft was given).
    pub fn signs_in_with(&self, typed: &str) -> bool {
        self.service().is_some() && (self.client_id.is_some() || !typed.trim().is_empty())
    }
}
