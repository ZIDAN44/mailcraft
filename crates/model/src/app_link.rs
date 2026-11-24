//! `mailcraft:` links: MailCraft's own, which its notifications carry (issue #273). Clicked, the
//! system starts MailCraft with the link (`mailcraft.exe --open "mailcraft:show?key=…"`), which
//! hands it to the MailCraft running: it comes forward, at the message the link names.
//!
//! Any program or web page can start such a link too, so one does no more than that: it shows a
//! message already here, or nothing. It's read capped, and a key that doesn't read is no key.

use crate::MessageKey;
use crate::mailto::{percent_decode, percent_encode};

/// The scheme, as the installer registers it.
pub const SCHEME: &str = "mailcraft";
/// What a notification's link asks: MailCraft shown.
const SHOW: &str = "mailcraft:show";
/// Longest link read: a key is an account, two numbers and a folder's path.
pub const MAX_LINK: usize = 4096;

/// The link that shows MailCraft, at `key` when there's one.
pub fn show_link(key: Option<&MessageKey>) -> String {
    match key {
        Some(k) => format!("{SHOW}?key={}", percent_encode(&k.to_id())),
        None => SHOW.to_string(),
    }
}

/// Whether `s` is a `mailcraft:` link (any letter case), leading blanks ignored.
pub fn is_app_link(s: &str) -> bool {
    let s = s.trim_start();
    s.get(..SCHEME.len() + 1).is_some_and(|p| p.eq_ignore_ascii_case("mailcraft:"))
}

/// What a link asks for: MailCraft shown (`Some(None)`), at a message (`Some(Some(key))`);
/// `None` for anything else, or one too long. A key that doesn't read is no key: MailCraft is
/// shown all the same.
pub fn parse(link: &str) -> Option<Option<MessageKey>> {
    let link = link.trim();
    if link.len() > MAX_LINK || !is_app_link(link) {
        return None;
    }
    let rest = link.get(SHOW.len()..).filter(|_| link.get(..SHOW.len()).is_some_and(|p| p.eq_ignore_ascii_case(SHOW)))?;
    let query = match rest.strip_prefix('?') {
        Some(q) => q,
        None if rest.is_empty() || rest == "/" => return Some(None),
        None => return None,
    };
    let key = query.split('&').find_map(|field| field.strip_prefix("key=")).and_then(|k| MessageKey::from_id(&percent_decode(k)));
    Some(key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn key(folder: &str) -> MessageKey {
        MessageKey { account: "acc-1".into(), folder: folder.into(), validity: 7, uid: 42 }
    }

    #[test]
    fn a_link_names_the_message_it_shows() {
        let k = key("Projects/Short film & more ?#%");
        let link = show_link(Some(&k));
        assert!(link.starts_with("mailcraft:show?key=acc-1%3A7%3A42%3AProjects%2F"), "{link}");
        assert_eq!(parse(&link), Some(Some(k)));
        assert_eq!(parse(&show_link(None)), Some(None));
        assert_eq!(parse("  MailCraft:SHOW?key=acc-1%3A7%3A42%3AINBOX "), Some(Some(key("INBOX"))));
        assert_eq!(parse("mailcraft:show/"), Some(None));
    }

    #[test]
    fn anything_else_shows_nothing_or_no_message() {
        for not_ours in ["mailto:a@b.example", "https://x.example", "mailcraft:", "mailcraft:open?key=a%3A1%3A2%3AX", "mailcraft:showme", "", "mail"]
        {
            assert_eq!(parse(not_ours), None, "{not_ours}");
        }
        assert_eq!(parse(&format!("mailcraft:show?key={}", "a".repeat(MAX_LINK))), None, "too long");
        // MailCraft is shown; no message for a key that doesn't read.
        for junk in
            ["mailcraft:show?key=", "mailcraft:show?key=%3A%3A%3A", "mailcraft:show?key=a%3Ax%3A1%3AF", "mailcraft:show?other=1", "mailcraft:show?"]
        {
            assert_eq!(parse(junk), Some(None), "{junk}");
        }
        assert!(!is_app_link("mailcraf"));
        assert!(is_app_link(" MAILCRAFT:show"));
    }

    proptest! {
        #[test]
        fn every_key_goes_there_and_back(account in "[^:]{1,20}", folder in "\\PC{1,40}", validity in any::<u32>(), uid in any::<u32>()) {
            let k = MessageKey { account, folder, validity, uid };
            prop_assert_eq!(parse(&show_link(Some(&k))), Some(Some(k)));
        }

        #[test]
        fn junk_never_panics(s in "\\PC{0,200}") {
            let _ = parse(&format!("mailcraft:show?{s}"));
            let _ = parse(&s);
        }
    }
}
