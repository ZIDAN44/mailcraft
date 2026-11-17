//! Safe `mailto:` links for contact addresses read from server data.

use std::fmt::Write as _;

/// Encode one address as a `mailto:` URL without letting data become URL headers or fragments.
pub(crate) fn link(email: &str) -> Option<String> {
    let email = email.trim();
    if email.is_empty() || email.len() > 254 || email.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return None;
    }
    let mut url = String::from("mailto:");
    for byte in email.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'@' | b'.' | b'_' | b'+' | b'-' | b'~') {
            url.push(char::from(byte));
        } else {
            write!(&mut url, "%{byte:02X}").ok()?;
        }
    }
    Some(url)
}
