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
    /// couldn't be asked (the usual sign-in says why).
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
    /// Why a server that takes bearer tokens has no service that could be found.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
