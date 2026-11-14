use proptest::prelude::*;

use super::*;

fn schemes(h: &str) -> Vec<String> {
    parse(h).into_iter().map(|c| c.scheme).collect()
}

#[test]
fn what_servers_send() {
    // A server that takes only bearer tokens (an OIDC sign-in), and names a scheme of its own.
    let h = r#"Bearer realm="mail", error="invalid_token", scope="openid profile email", Ticket realm="https://jmap.example.com""#;
    let c = parse(h);
    assert_eq!(schemes(h), ["bearer", "ticket"]);
    assert_eq!((c[0].param("scope"), c[0].param("ERROR")), (Some("openid profile email"), Some("invalid_token")));
    assert_eq!(c[1].param("realm"), Some("https://jmap.example.com"));
    // RFC 9110 §11.6.1's example: two challenges, an escaped quote, a parameter that's a token.
    let h = r#"Newauth realm="apps", type=1, title="Login to \"apps\"", Basic realm="simple""#;
    let c = parse(h);
    assert_eq!(schemes(h), ["newauth", "basic"]);
    assert_eq!((c[0].param("type"), c[0].param("title")), (Some("1"), Some(r#"Login to "apps""#)));
    assert_eq!(c[1].param("realm"), Some("simple"));
    // A token68, empty list elements, a bare scheme in capitals, spaces around `=`.
    assert_eq!(
        parse("Negotiate YII+/a==, , BASIC"),
        vec![
            Challenge { scheme: "negotiate".into(), token68: Some("YII+/a==".into()), params: Vec::new() },
            Challenge { scheme: "basic".into(), ..Challenge::default() },
        ]
    );
    let c = parse(r#"Basic Realm = "x" , charset="UTF-8""#);
    assert_eq!(c[0].params, [("realm".to_string(), "x".to_string()), ("charset".into(), "UTF-8".into())]);
    assert_eq!(schemes("Bearer"), ["bearer"]);
    assert!(parse("").is_empty() && parse(" ,, ").is_empty());
}

#[test]
fn what_cant_be_read_ends_the_list() {
    // A quote that never ends, a control character, junk after a value, no scheme at all.
    assert_eq!(schemes(r#"Basic realm="x", Bearer realm="open"#), ["basic", "bearer"]);
    assert_eq!(parse("Bearer realm=\"a\u{1}b\", Basic").len(), 1);
    assert_eq!(schemes(r#"Basic realm="x" junk, Bearer"#), ["basic"]);
    assert!(parse(r#""Basic""#).is_empty());
    assert_eq!(schemes("Basic, @, Bearer"), ["basic"]);
    // The caps: so many challenges, so many parameters, so much of the header.
    assert_eq!(parse(&vec!["Basic"; 100].join(", ")).len(), MAX_CHALLENGES);
    let params: Vec<String> = (0..100).map(|i| format!("p{i}=v")).collect();
    assert_eq!(parse(&format!("Bearer {}", params.join(", ")))[0].params.len(), MAX_PARAMS);
    // Cut within the cap, on a character's boundary: the quote never ends there.
    let long = format!("Bearer realm=\"x{}\", Basic", "é".repeat(MAX_HEADER));
    assert_eq!(parse(&long), vec![Challenge { scheme: "bearer".into(), ..Challenge::default() }]);
}

proptest! {
    /// Whatever arrives, it's read within the caps, without a panic.
    #[test]
    fn any_header_is_read_within_the_caps(text in "\\PC{0,300}", bytes in proptest::collection::vec(any::<u8>(), 0..300)) {
        for h in [text, String::from_utf8_lossy(&bytes).into_owned()] {
            let c = parse(&h);
            prop_assert!(c.len() <= MAX_CHALLENGES);
            prop_assert!(c.iter().all(|c| !c.scheme.is_empty() && c.params.len() <= MAX_PARAMS));
        }
    }

    /// Challenges written out are read back as they were.
    #[test]
    fn challenges_round_trip(
        list in proptest::collection::vec(
            ("[A-Za-z][A-Za-z0-9-]{0,10}", proptest::collection::vec(("[a-z][a-z0-9_]{0,8}", "[ -~]{0,20}"), 0..5)),
            1..6,
        )
    ) {
        let written: Vec<String> = list
            .iter()
            .map(|(scheme, params)| {
                let quoted: Vec<String> =
                    params.iter().map(|(n, v)| format!("{n}=\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\""))).collect();
                if quoted.is_empty() { scheme.clone() } else { format!("{scheme} {}", quoted.join(", ")) }
            })
            .collect();
        let want: Vec<Challenge> = list
            .iter()
            .map(|(scheme, params)| Challenge { scheme: scheme.to_ascii_lowercase(), token68: None, params: params.clone() })
            .collect();
        prop_assert_eq!(parse(&written.join(", ")), want);
    }
}
