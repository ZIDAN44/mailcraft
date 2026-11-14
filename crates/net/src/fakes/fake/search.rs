//! The fake's `SEARCH` (RFC 3501 §6.4.4): a command's keys, and whether a message matches them.

use super::*;

/// A `SEARCH` command's keys and strings (quotes taken off).
pub(super) fn search_tokens(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let (mut quoted, mut escaped) = (false, false);
    for c in s.chars() {
        match c {
            _ if escaped => {
                cur.push(c);
                escaped = false;
            }
            '\\' if quoted => escaped = true,
            '"' => quoted = !quoted,
            ' ' if !quoted => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            _ => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Whether a message (`seq`, its message number) matches every key (RFC 3501 §6.4.4; strings as
/// substrings, any case; a sequence set is the message numbers').
pub(super) fn search_matches(m: &Msg, seq: u32, keys: &[String], max: u32, exists: u32) -> bool {
    let raw = String::from_utf8_lossy(&m.raw).to_lowercase();
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((raw.as_str(), ""));
    let header = |name: &str| head.lines().filter(|l| l.starts_with(&format!("{name}:"))).collect::<Vec<_>>().join("\n");
    let mut i = 0;
    let mut negate = false;
    while let Some(k) = keys.get(i) {
        let arg = keys.get(i + 1).map(|a| a.to_lowercase()).unwrap_or_default();
        let (ok, used) = match k.to_ascii_uppercase().as_str() {
            "ALL" => (true, 1),
            "NOT" => {
                negate = !negate;
                i += 1;
                continue;
            }
            "CHARSET" => {
                i += 2;
                continue;
            }
            "UNSEEN" => (!m.flags.iter().any(|f| f == "\\Seen"), 1),
            "SEEN" => (m.flags.iter().any(|f| f == "\\Seen"), 1),
            "FLAGGED" => (m.flags.iter().any(|f| f == "\\Flagged"), 1),
            "UNFLAGGED" => (!m.flags.iter().any(|f| f == "\\Flagged"), 1),
            "UID" => (in_set(m.uid, &arg, max), 2),
            "TEXT" => (raw.contains(&arg), 2),
            "BODY" => (body.contains(&arg), 2),
            "SUBJECT" => (header("subject").contains(&arg), 2),
            "FROM" => (header("from").contains(&arg), 2),
            "TO" => (header("to").contains(&arg), 2),
            "HEADER" => (header(&arg).contains(&keys.get(i + 2).map(|v| v.to_lowercase()).unwrap_or_default()), 3),
            "KEYWORD" => (m.flags.iter().any(|f| f.eq_ignore_ascii_case(&arg)), 2),
            set if set.starts_with(|c: char| c.is_ascii_digit() || c == '*') => (in_set(seq, set, exists), 1),
            _ => (false, 1),
        };
        if ok == negate {
            return false;
        }
        negate = false;
        i += used;
    }
    true
}

pub(super) fn in_set(uid: u32, set: &str, max: u32) -> bool {
    set.split(',').any(|part| {
        let val = |x: &str| if x == "*" { max } else { x.parse().unwrap_or(0) };
        match part.split_once(':') {
            Some((a, b)) => {
                let (a, b) = (val(a), val(b));
                (a.min(b)..=a.max(b)).contains(&uid)
            }
            None => val(part) == uid,
        }
    })
}
