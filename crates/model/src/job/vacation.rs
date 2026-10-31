//! Automatic replies, as the server keeps them.

use serde::{Deserialize, Serialize};

/// Automatic replies (out of office): sent once per sender while on, between the dates when
/// set. Kept on the server, so they work while this computer is off.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Vacation {
    pub enabled: bool,
    #[serde(default)]
    pub subject: String,
    #[serde(default)]
    pub text: String,
    /// First and last day (`YYYY-MM-DD`), when limited.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<String>,
    /// Exchange: the reply to people outside the organisation, when it isn't the same as `text`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external: Option<String>,
    /// Exchange: who outside the organisation gets a reply: `all` (the default, empty), `known`
    /// (the user's contacts only) or `none`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub outside: String,
}

impl Vacation {
    /// The reply to people outside the organisation.
    pub fn external_text(&self) -> &str {
        self.external.as_deref().unwrap_or(&self.text)
    }
}

/// A `YYYY-MM-DD` date that is a real day.
pub fn is_day(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 10 || b.get(4) != Some(&b'-') || b.get(7) != Some(&b'-') {
        return false;
    }
    let num = |a: usize, z: usize| s.get(a..z).filter(|x| x.bytes().all(|c| c.is_ascii_digit())).and_then(|x| x.parse::<u32>().ok());
    match (num(0, 4), num(5, 7), num(8, 10)) {
        (Some(y), Some(m), Some(d)) => {
            let days = match m {
                1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
                4 | 6 | 9 | 11 => 30,
                2 if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 => 29,
                2 => 28,
                _ => 0,
            };
            y >= 1970 && (1..=days).contains(&d)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days() {
        for ok in ["2026-10-15", "2024-02-29", "2000-02-29"] {
            assert!(is_day(ok), "{ok}");
        }
        for bad in ["2026-02-29", "1900-02-29", "2026-13-01", "2026-1-01", "26-10-15", "2026-10-15T00", "", "2026-10-00", "abcd-ef-gh"] {
            assert!(!is_day(bad), "{bad}");
        }
    }
}
