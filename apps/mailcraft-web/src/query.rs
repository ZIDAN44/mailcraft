//! The page address's parameters (`?sample&open=…`), each read whole.
#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

/// The parameters of `query` (`?a&b=c`): each name with its raw (still encoded) value, if any.
fn params(query: &str) -> impl Iterator<Item = (&str, Option<&str>)> {
    query.trim_start_matches('?').split('&').filter(|kv| !kv.is_empty()).map(|kv| match kv.split_once('=') {
        Some((k, v)) => (k, Some(v)),
        None => (kv, None),
    })
}

/// Whether `query` has `name` as a parameter of its own, bare or with a value. Another's value
/// may hold the same letters (a pop-out's `open=` names the message's folder, `INBOX/samples`
/// say): matched as a part of the text, that opened the sample mailbox.
pub fn has(query: &str, name: &str) -> bool {
    params(query).any(|(k, _)| k == name)
}

/// The raw (still encoded) value of `name` in `query`.
pub fn value<'a>(query: &'a str, name: &str) -> Option<&'a str> {
    params(query).find_map(|(k, v)| if k == name { v } else { None })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_parameter_is_matched_whole() {
        // A pop-out for a message in INBOX/samples, its key encoded as the page writes it.
        let popped = "?open=a1%3A1%3A1%3AINBOX%2Fsamples%3A7";
        assert!(!has(popped, "sample") && !has(popped, "webgl"));
        assert_eq!(value(popped, "open"), Some("a1%3A1%3A1%3AINBOX%2Fsamples%3A7"));
        // The sample mailbox's own pop-out keeps it the sample.
        let sample = "?sample&open=sample%3A1%3A1%3AINBOX%3A3";
        assert!(has(sample, "sample") && !has(sample, "open=") && !has(sample, "samples"));
        assert_eq!(value(sample, "sample"), None);
        assert!(has("?webgl", "webgl") && has("?sample=1", "sample"));
        assert!(!has("", "sample") && !has("?", "sample") && value("?open", "open").is_none());
        assert_eq!(value("?open=", "open"), Some(""));
    }
}
