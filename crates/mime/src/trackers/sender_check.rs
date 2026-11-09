//! Whether the mail server that took a message confirmed who sent it, from the
//! `Authentication-Results` header it wrote (RFC 8601): the sender's domain passing DMARC, or
//! DKIM or SPF for that same domain. Only the top header counts, the one the last server
//! added: one further down may be the sender's own. This says whether the domain in the From
//! line is the one that sent the message, and nothing about whether the mail is any good.

use serde::{Deserialize, Serialize};

/// What the mail server found out about a message's sender.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SenderCheck {
    /// It says nothing that settles it (no such header, or none for the sender's domain).
    #[default]
    Unknown,
    /// The domain in the From line sent it.
    Confirmed,
    /// The domain in the From line says it didn't.
    Failed,
}

/// The most of a header that's read, and the checks in it.
const MAX_HEADER: usize = 16 * 1024;
const MAX_CHECKS: usize = 32;

/// `header` (the top `Authentication-Results`) read for a message from an address at `from`.
pub(crate) fn sender_check(header: &str, from: &str) -> SenderCheck {
    let from = from.trim().trim_end_matches('.').to_ascii_lowercase();
    if from.is_empty() {
        return SenderCheck::Unknown;
    }
    let end = (0..=header.len().min(MAX_HEADER)).rev().find(|n| header.is_char_boundary(*n)).unwrap_or(0);
    let text = without_comments(header.get(..end).unwrap_or(""));
    let (mut confirmed, mut dmarc_failed, mut other_failed) = (false, false, false);
    for part in text.split(';').take(MAX_CHECKS) {
        let mut words = part.split_whitespace();
        // The server's name comes first, in a part with no `=`, and may be missing.
        let Some((method, result)) = words.by_ref().find_map(|w| w.split_once('=')) else { continue };
        let (method, result) = (method.to_ascii_lowercase(), result.to_ascii_lowercase());
        let value = |names: &[&str]| {
            part.split_whitespace().find_map(|w| {
                let (k, v) = w.split_once('=')?;
                names.iter().any(|n| k.eq_ignore_ascii_case(n)).then(|| v.trim_matches('"').to_ascii_lowercase())
            })
        };
        let aligned = |names: &[&str]| value(names).is_some_and(|v| same_domain(domain_of(&v), &from));
        let passed = matches!(result.as_str(), "pass" | "bestguesspass");
        match method.as_str() {
            "dmarc" if passed && aligned(&["header.from"]) => confirmed = true,
            "dmarc" if result == "fail" => dmarc_failed = true,
            "dkim" if passed && aligned(&["header.d", "header.i"]) => confirmed = true,
            "spf" if passed && aligned(&["smtp.mailfrom", "smtp.helo"]) => confirmed = true,
            "dkim" | "spf" if result == "fail" => other_failed = true,
            _ => {}
        }
    }
    if dmarc_failed {
        SenderCheck::Failed
    } else if confirmed {
        SenderCheck::Confirmed
    } else if other_failed {
        SenderCheck::Failed
    } else {
        SenderCheck::Unknown
    }
}

/// `header` without its comments (`(sender IP is 192.0.2.1)`), which may hold anything.
fn without_comments(header: &str) -> String {
    let mut out = String::with_capacity(header.len());
    let (mut depth, mut quoted, mut escaped) = (0usize, false, false);
    for c in header.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' if depth > 0 || quoted => escaped = true,
            '"' if depth == 0 => {
                quoted = !quoted;
                out.push(c);
            }
            '(' if !quoted => depth = depth.saturating_add(1),
            ')' if !quoted && depth > 0 => depth -= 1,
            c if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

/// The domain of `@domain`, `name@domain` or `domain`.
fn domain_of(v: &str) -> &str {
    v.rsplit('@').next().unwrap_or(v).trim_end_matches('.')
}

/// Whether two domains are one, or one is under the other, as DMARC's relaxed alignment has it
/// (`mail.shop.example` for `shop.example`); never a whole suffix such as `example` or `co.uk`.
fn same_domain(a: &str, b: &str) -> bool {
    let under = |long: &str, short: &str| short.split('.').count() >= 2 && long.strip_suffix(short).is_some_and(|rest| rest.ends_with('.'));
    !a.is_empty() && (a == b || under(a, b) || under(b, a))
}

#[cfg(test)]
mod tests {
    use super::*;
    use SenderCheck::{Confirmed, Failed, Unknown};

    /// As the big mail services write it.
    #[test]
    fn what_mail_services_write_is_read() {
        let gmail = "mx.google.com;\r\n dkim=pass header.i=@shop.example header.s=s1 header.b=abc;\r\n spf=pass (google.com: domain of bounce@mail.shop.example designates 192.0.2.1 as permitted sender) smtp.mailfrom=bounce@mail.shop.example;\r\n dmarc=pass (p=REJECT sp=REJECT dis=NONE) header.from=shop.example";
        assert_eq!(sender_check(gmail, "shop.example"), Confirmed);
        let microsoft = "spf=pass (sender IP is 192.0.2.1) smtp.mailfrom=shop.example; dkim=pass (signature was verified) header.d=shop.example; dmarc=pass action=none header.from=shop.example;compauth=pass reason=100";
        assert_eq!(sender_check(microsoft, "shop.example"), Confirmed);
        assert_eq!(sender_check("mx.example; dmarc=bestguesspass action=none header.from=shop.example", "shop.example"), Confirmed);
    }

    /// Passing for some other domain proves nothing about the one in the From line.
    #[test]
    fn only_the_senders_own_domain_counts() {
        for h in [
            "mx.example; dkim=pass header.d=spammer.example; spf=pass smtp.mailfrom=spammer.example",
            "mx.example; dmarc=pass header.from=spammer.example",
            "mx.example; dkim=pass header.d=example",
            "mx.example; dkim=pass header.d=notshop.example",
            "mx.example; dkim=pass",
            "mx.example; dkim=none; spf=none; dmarc=none",
            "mx.example 1; none",
            "",
        ] {
            assert_eq!(sender_check(h, "shop.example"), Unknown, "{h}");
        }
        assert_eq!(sender_check("mx.example; dkim=pass header.d=mail.shop.example", "shop.example"), Confirmed, "a part of it");
        assert_eq!(sender_check("mx.example; dkim=pass header.d=shop.example", "news.shop.example"), Confirmed);
        assert_eq!(sender_check("mx.example; dkim=pass header.d=example", "shop.example"), Unknown, "a suffix isn't a sender");
        assert_eq!(sender_check("mx.example; dkim=pass header.d=shop.example", ""), Unknown);
    }

    /// A failure says so; DMARC, the sender's own policy, has the last word.
    #[test]
    fn failing_is_failing() {
        assert_eq!(sender_check("mx.example; dmarc=fail header.from=shop.example", "shop.example"), Failed);
        assert_eq!(sender_check("mx.example; dkim=pass header.d=shop.example; dmarc=fail header.from=shop.example", "shop.example"), Failed);
        assert_eq!(sender_check("mx.example; spf=fail smtp.mailfrom=shop.example", "shop.example"), Failed);
        assert_eq!(
            sender_check("mx.example; spf=fail smtp.mailfrom=shop.example; dkim=pass header.d=shop.example", "shop.example"),
            Confirmed,
            "a forward breaks SPF"
        );
    }

    /// Words in a comment are no results, and hostile input ends quietly.
    #[test]
    fn comments_and_hostile_input() {
        assert_eq!(sender_check("mx.example; x=y (dmarc=pass header.from=shop.example)", "shop.example"), Unknown);
        assert_eq!(sender_check("mx.example; dmarc=fail (dmarc=pass; header.from=shop.example) header.from=shop.example", "shop.example"), Failed);
        for h in ["(((((", ")))", "=;=;=", "\"", "\\", "é=é;@", &"a=b;".repeat(100_000), &"(".repeat(100_000)] {
            let _ = sender_check(h, "shop.example");
        }
        let late = format!("{}; dmarc=pass header.from=shop.example", "x".repeat(MAX_HEADER));
        assert_eq!(sender_check(&late, "shop.example"), Unknown, "past what's read");
    }
}
