//! An account's signatures by name (issue #11), as Outlook's Signatures and Stationery keeps
//! them: "Full" for new messages, "Short" (or none) for replies and forwards, any of them
//! switched to while writing.

use serde::{Deserialize, Serialize};

use crate::AccountConfig;

/// One signature: its text, its look as HTML (empty when it's plain text: bold, colours, sizes,
/// links and pictures, as pasted from Outlook or set in the signature editor), or HTML code
/// sent as written (a business card in columns; `html` is then its text as the editor shows it).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signature {
    pub name: String,
    #[serde(default)]
    pub text: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub html: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub code: String,
}

/// The most signatures an account keeps.
pub const MAX_SIGNATURES: usize = 50;
/// The longest name a signature has, in characters.
pub const MAX_SIGNATURE_NAME: usize = 64;
/// The name the signature an account had before it could have several gets.
pub const FIRST_SIGNATURE: &str = "Signature";

/// A signature's name as given, trimmed; `None` when it's empty, too long or has a line break
/// or other control character in it.
pub fn signature_name(s: &str) -> Option<String> {
    let s = s.trim();
    let ok = !s.is_empty() && s.chars().count() <= MAX_SIGNATURE_NAME && !s.chars().any(char::is_control);
    ok.then(|| s.to_string())
}

impl Signature {
    /// Whether there's nothing in it.
    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty() && self.html.trim().is_empty() && self.code.trim().is_empty()
    }
}

impl AccountConfig {
    /// A signature by name (names are told apart without regard to case, as Outlook's are).
    pub fn signature_named(&self, name: &str) -> Option<&Signature> {
        self.signatures.iter().find(|s| s.name.eq_ignore_ascii_case(name.trim()))
    }

    /// The signature that goes into a new message, or (`reply`) into a reply or forward.
    pub fn default_signature(&self, reply: bool) -> Option<&Signature> {
        let name = if reply { &self.signature_reply } else { &self.signature_new };
        name.as_deref().and_then(|n| self.signature_named(n))
    }

    /// The one signature of older settings into `signatures`, for new messages and replies
    /// alike as it was; nothing changes once that's done, or for an account that had none.
    pub fn migrate_signature(&mut self) {
        let old = Signature {
            name: FIRST_SIGNATURE.to_string(),
            text: std::mem::take(&mut self.signature),
            html: std::mem::take(&mut self.signature_html),
            code: std::mem::take(&mut self.signature_code),
        };
        if old.is_empty() || !self.signatures.is_empty() {
            return;
        }
        self.signature_new = Some(old.name.clone());
        self.signature_reply = Some(old.name.clone());
        self.signatures.push(old);
    }

    /// Keep signature `sig`: in place of `was` (renamed, the defaults following it), or of one
    /// of its name, else as a new one. An account's first signature goes into new messages and
    /// replies alike. Says why when it can't be kept.
    pub fn keep_signature(&mut self, was: Option<&str>, sig: Signature) -> Result<(), String> {
        let name = signature_name(&sig.name).ok_or("a signature needs a name, on one line, of up to 64 characters")?;
        let at = was.and_then(|w| self.signatures.iter().position(|s| s.name.eq_ignore_ascii_case(w.trim())));
        let same = self.signatures.iter().position(|s| s.name.eq_ignore_ascii_case(&name));
        if same.is_some_and(|i| at.is_some_and(|a| a != i)) {
            return Err(format!("there's a signature called {name} already"));
        }
        let sig = Signature { name: name.clone(), ..sig };
        match at.or(same) {
            Some(i) => {
                let old = self.signatures.get(i).map(|s| s.name.clone()).unwrap_or_default();
                for d in [&mut self.signature_new, &mut self.signature_reply] {
                    if d.as_deref().is_some_and(|n| n.eq_ignore_ascii_case(&old)) {
                        *d = Some(name.clone());
                    }
                }
                if let Some(slot) = self.signatures.get_mut(i) {
                    *slot = sig;
                }
            }
            None => {
                if self.signatures.len() >= MAX_SIGNATURES {
                    return Err(format!("an account keeps up to {MAX_SIGNATURES} signatures"));
                }
                if self.signatures.is_empty() {
                    self.signature_new = Some(name.clone());
                    self.signature_reply = Some(name);
                }
                self.signatures.push(sig);
            }
        }
        Ok(())
    }

    /// Remove a signature; where it was a default, none goes in. False when there's no such one.
    pub fn remove_signature(&mut self, name: &str) -> bool {
        let before = self.signatures.len();
        self.signatures.retain(|s| !s.name.eq_ignore_ascii_case(name.trim()));
        for d in [&mut self.signature_new, &mut self.signature_reply] {
            if d.as_deref().is_some_and(|n| n.eq_ignore_ascii_case(name.trim())) {
                *d = None;
            }
        }
        self.signatures.len() != before
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sig(name: &str, text: &str) -> Signature {
        Signature { name: name.into(), text: text.into(), ..Default::default() }
    }

    /// Older settings' one signature becomes "Signature", for new messages and replies, once.
    #[test]
    fn the_one_signature_of_older_settings_is_kept() {
        let json = r#"{"id":"a1","name":"Ann","email":"ann@x.example","username":"ann","imap":{"host":"h","port":993},
            "smtp":{"host":"h","port":465},"signature":"Ann\nSales","signature_html":"<b>Ann</b>","signature_code":"<table></table>"}"#;
        let mut a: AccountConfig = serde_json::from_str(json).unwrap();
        a.migrate_signature();
        assert_eq!(
            a.signatures,
            vec![Signature { name: "Signature".into(), text: "Ann\nSales".into(), html: "<b>Ann</b>".into(), code: "<table></table>".into() }]
        );
        assert_eq!((a.signature_new.as_deref(), a.signature_reply.as_deref()), (Some("Signature"), Some("Signature")));
        assert!(a.signature.is_empty() && a.signature_html.is_empty() && a.signature_code.is_empty());
        let saved = serde_json::to_string(&a).unwrap();
        assert!(!saved.contains("signature_html") && !saved.contains("\"signature\":"), "{saved}");
        let mut back: AccountConfig = serde_json::from_str(&saved).unwrap();
        back.migrate_signature();
        assert_eq!(back, a);
        // None at all: none made up.
        let mut none = AccountConfig::guess("bo@x.example");
        none.migrate_signature();
        assert!(none.signatures.is_empty() && none.signature_new.is_none());
    }

    #[test]
    fn signatures_are_named_renamed_and_removed_with_their_defaults() {
        let mut a = AccountConfig::guess("ann@x.example");
        a.keep_signature(None, sig(" Full ", "Ann Lee\nSales")).unwrap();
        assert_eq!((a.signature_new.as_deref(), a.signature_reply.as_deref()), (Some("Full"), Some("Full")), "the first is for both");
        a.keep_signature(None, sig("Short", "Ann")).unwrap();
        a.signature_reply = Some("Short".into());
        assert_eq!(a.default_signature(false).map(|s| s.text.as_str()), Some("Ann Lee\nSales"));
        assert_eq!(a.default_signature(true).map(|s| s.text.as_str()), Some("Ann"));
        // Saved again under its name (any case): replaced.
        a.keep_signature(None, sig("short", "A.")).unwrap();
        assert_eq!(a.signatures.len(), 2);
        assert_eq!(a.default_signature(true).map(|s| s.text.as_str()), Some("A."));
        // Renamed: the defaults follow; onto another's name: refused.
        a.keep_signature(Some("Short"), sig("Brief", "A.")).unwrap();
        assert_eq!(a.signature_reply.as_deref(), Some("Brief"));
        assert!(a.keep_signature(Some("Brief"), sig("FULL", "x")).is_err());
        for bad in ["", "  ", "a\nb", &"x".repeat(65)] {
            assert!(a.keep_signature(None, sig(bad, "x")).is_err(), "{bad:?}");
        }
        assert!(a.remove_signature("full"));
        assert!(!a.remove_signature("full"));
        assert_eq!((a.signature_new.as_deref(), a.signature_reply.as_deref()), (None, Some("Brief")));
        assert!(a.default_signature(false).is_none());
        // A default naming one that isn't there (settings edited by hand): none.
        a.signature_new = Some("Gone".into());
        assert!(a.default_signature(false).is_none());
        for n in 0..MAX_SIGNATURES {
            let _ = a.keep_signature(None, sig(&format!("s{n}"), "x"));
        }
        assert_eq!(a.signatures.len(), MAX_SIGNATURES);
        assert!(a.keep_signature(None, sig("one more", "x")).is_err());
    }
}
