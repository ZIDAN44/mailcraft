use super::*;
use proptest::prelude::*;

/// The decoder the hidden text was read with before [`decode_refs`], as it was.
fn old_words(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(at) = rest.find('&') {
        out.push_str(rest.get(..at).unwrap_or(""));
        let after = rest.get(at + 1..).unwrap_or("");
        let Some(len) = after.as_bytes().iter().take(12).position(|b| *b == b';') else {
            out.push('&');
            rest = after;
            continue;
        };
        let name = after.get(..len).unwrap_or("");
        match name {
            "nbsp" | "ensp" | "emsp" | "thinsp" => out.push(' '),
            "lt" => out.push('<'),
            "gt" => out.push('>'),
            "quot" => out.push('"'),
            "apos" => out.push('\''),
            "amp" => out.push('&'),
            n if n.starts_with('#') => {
                let code = match n.get(1..).and_then(|d| d.strip_prefix(['x', 'X']).map(|h| (h, 16)).or(Some((d, 10)))) {
                    Some((d, radix)) => u32::from_str_radix(d, radix).ok(),
                    None => None,
                };
                match code.and_then(char::from_u32) {
                    Some(c) if c.is_alphanumeric() || c.is_ascii_punctuation() => out.push(c),
                    _ => out.push(' '),
                }
            }
            n if n.chars().all(|c| c.is_ascii_alphanumeric()) && !n.is_empty() => out.push(' '),
            _ => {
                out.push('&');
                rest = after;
                continue;
            }
        }
        rest = after.get(len + 1..).unwrap_or("");
    }
    out.push_str(rest);
    out
}

/// The decoder Outlook's attributes were read with before [`decode_refs`], as it was but for the
/// one change made on purpose: a name of 11 bytes is read (`<= 11`, it was `<= 10`), as the
/// hidden text's decoder read it (`&#0000000038;` is a `&` to a browser).
fn old_value(v: &str, longest: usize) -> String {
    let mut out = String::with_capacity(v.len());
    let mut rest = v;
    while let Some(at) = rest.find('&') {
        out.push_str(rest.get(..at).unwrap_or(""));
        let after = rest.get(at + 1..).unwrap_or("");
        let named = after.find(';').filter(|n| *n <= longest).and_then(|n| Some((after.get(..n)?, n + 1)));
        let c = named.and_then(|(name, len)| {
            let c = match name {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                _ => {
                    let n = name.strip_prefix('#')?;
                    let code = match n.strip_prefix(['x', 'X']) {
                        Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                        None => n.parse::<u32>().ok()?,
                    };
                    Some(char::from_u32(code).filter(|c| *c != '\0').unwrap_or('\u{fffd}'))
                }
            }?;
            Some((c, len))
        });
        match c {
            Some((c, len)) => {
                out.push(c);
                rest = after.get(len..).unwrap_or("");
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// The XML decoders before [`decode_xml_refs`] (Exchange's attribute values in the network layer,
/// Office files here), as they were but for what changed on purpose: they read names of up to 10
/// and 9 bytes (`longest`), and `&#x…;` but not `&#X…;` (`upper`), which XML's text was read with.
fn old_xml(v: &str, longest: usize, upper: bool) -> String {
    let mut out = String::with_capacity(v.len());
    let mut rest = v;
    while let Some(i) = rest.find('&') {
        out.push_str(rest.get(..i).unwrap_or(""));
        let after = rest.get(i + 1..).unwrap_or("");
        let Some(end) = after.find(';').filter(|e| *e <= longest) else {
            out.push('&');
            rest = after;
            continue;
        };
        let name = after.get(..end).unwrap_or("");
        let c = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            n => n
                .strip_prefix("#x")
                .or_else(|| n.strip_prefix("#X").filter(|_| upper))
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| n.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match c {
            Some(c) => {
                out.push(c);
                rest = after.get(end + 1..).unwrap_or("");
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Text made of what references are made of, so that most `&` start one.
fn refs_text() -> impl Strategy<Value = String> {
    let piece = prop_oneof![
        Just("&".to_string()),
        Just(";".to_string()),
        Just("#".to_string()),
        Just("x".to_string()),
        "[0-9]{1,8}",
        "[a-fA-F0-9]{1,6}",
        prop_oneof![Just("amp"), Just("lt"), Just("gt"), Just("quot"), Just("apos"), Just("nbsp"), Just("shy"), Just("thinsp")]
            .prop_map(String::from),
        "[a-z \u{e9}\u{20ac}+-]{1,4}",
    ];
    proptest::collection::vec(piece, 0..24).prop_map(|p| p.concat())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(4000))]

    /// One decoder for both, each pass reading what it read before.
    #[test]
    fn each_pass_reads_as_it_did(s in refs_text()) {
        prop_assert_eq!(decode_refs(&s, Refs::Words).into_owned(), old_words(&s));
        prop_assert_eq!(decode_refs(&s, Refs::Value).into_owned(), old_value(&s, 11));
    }

    /// XML reads as the network layer's and the Office reader's decoders did.
    #[test]
    fn xml_reads_as_it_did(s in refs_text()) {
        prop_assert_eq!(decode_xml_refs(&s).into_owned(), old_xml(&s, 11, true));
    }
}

/// What changed for XML: `&#X26;` is read (XML's text read it, its values didn't), and so is a
/// numbered reference of up to 11 bytes; one that is no character stays as written, as before.
#[test]
fn xml_reads_every_numbered_reference() {
    let v = "a&#X26;b&#00000038;c&#0000000038;d&#xD800;e&nbsp;f";
    assert_eq!(old_xml(v, 10, false), "a&#X26;b&c&#0000000038;d&#xD800;e&nbsp;f");
    assert_eq!(decode_xml_refs(v), "a&b&c&d&#xD800;e&nbsp;f");
    assert_eq!(xml_ref_char("#X26"), Some('&'));
    assert_eq!((xml_ref_char("apos"), xml_ref_char("nbsp"), xml_ref_char("#xFFFFFFFF")), (Some('\''), None, None));
}

/// The one change: a numbered reference of 11 bytes is read in an attribute too.
#[test]
fn a_long_numbered_reference_is_read_in_a_value() {
    let v = "a&#0000000038;b";
    assert_eq!(old_value(v, 10), v);
    assert_eq!(decode_refs(v, Refs::Value), "a&b");
    assert_eq!(decode_refs(v, Refs::Words), "a&b");
}

#[test]
fn values_and_words_read_their_own_way() {
    let s = "a&amp;b&#38;c&#x26;d&nbsp;e&#8203;f&bogus;g&#;h& i&#0;";
    assert_eq!(decode_refs(s, Refs::Value), "a&b&c&d&nbsp;e\u{200b}f&bogus;g&#;h& i\u{fffd}");
    assert_eq!(decode_refs(s, Refs::Words), "a&b&c&d e f g h& i ");
    assert!(matches!(decode_refs("no references", Refs::Value), Cow::Borrowed(_)));
    // A `;` far off isn't looked for to the end, and ends no reference.
    let long = format!("&{};", "a".repeat(20));
    assert_eq!(decode_refs(&long, Refs::Words), long);
}

/// Every pass reads a picture's and a link's address as the text converter does: the list of
/// pictures to load had `&#38;` as written, and the picture the text shows never came; a link's
/// `&amp;#38;` was read twice, as `&`.
#[test]
fn every_pass_reads_an_address_alike() {
    let src = "https://pictures.example/p.png?a=1&b=2";
    let html = r#"<p>Hi</p><img alt="Logo" src="https://pictures.example/p.png?a=1&#38;b=2"><a href="https://links.example/?q=x&amp;#38;y">go</a><img src="https://t.example/o.gif?a=1&#x26;b=2" width="1" height="1">"#;
    let shown: Vec<String> = crate::html_to_rich(html).into_iter().flatten().filter_map(|s| s.image).collect();
    assert_eq!(shown, [src]);
    assert_eq!(crate::html_images(html), [src, "https://t.example/o.gif?a=1&b=2"]);
    assert_eq!(crate::html_links(html), ["https://links.example/?q=x&#38;y"]);
    assert_eq!(crate::trackers::trackers(html).pictures, ["https://t.example/o.gif?a=1&b=2"]);
    let quoted = crate::quote_html(html, &[(src.to_string(), "logo@x".to_string())], &["https://t.example/o.gif?a=1&b=2".to_string()]);
    assert!(quoted.contains("src=\"cid:logo@x\"") && !quoted.contains("t.example"), "{quoted}");
}

proptest! {
    /// The one escaper writes what each of its three copies wrote (the layout's replaced `"`
    /// second, the others last: the same, as no replacement makes another's character).
    #[test]
    fn escaped_as_each_copy_was(s in "[&<>\"'a é;#]{0,24}") {
        let markup = s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;");
        let layout = s.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;").replace('>', "&gt;");
        prop_assert_eq!(escaped(&s), markup);
        prop_assert_eq!(escaped(&s), layout);
    }
}
