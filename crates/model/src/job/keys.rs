//! Keys held where MailCraft can't read them, and what their authorities say of them.

use serde::{Deserialize, Serialize};

/// A certificate with its private key held where MailCraft can't read it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeldKey {
    /// What it's called there (the card's label, the keychain's name).
    pub label: String,
    /// The certificate (DER).
    pub cert: Vec<u8>,
    /// Where the key is, for the network layer to find it again (opaque here).
    pub at: String,
}

/// Whether a certificate's authority withdrew it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum Revocation {
    /// Not withdrawn; the answer holds until `until` (Unix seconds), when it says.
    Good { until: Option<i64> },
    /// Withdrawn at `at` (Unix seconds).
    Revoked { at: i64 },
    /// No answer to go by (none given, or none to trust), and why.
    Unknown { why: String },
}
