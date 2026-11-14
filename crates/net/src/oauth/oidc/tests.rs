use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;

use super::*;

/// A web server of the test's own on this computer: each request answered by
/// `answer(its address, the request's first line)`.
fn service(answer: impl Fn(&str, &str) -> Vec<u8> + Send + 'static) -> String {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://127.0.0.1:{}", l.local_addr().unwrap().port());
    let at = base.clone();
    std::thread::spawn(move || {
        for conn in l.incoming() {
            let Ok(mut s) = conn else { continue };
            let mut reader = BufReader::new(s.try_clone().unwrap());
            let mut first = String::new();
            let _ = reader.read_line(&mut first);
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
            }
            let _ = s.write_all(&answer(&at, &first));
        }
    });
    base
}

fn json(body: &str) -> Vec<u8> {
    format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).into_bytes()
}

/// A description whose issuer is `issuer`, with endpoints under it.
fn description(issuer: &str) -> String {
    format!(
        r#"{{"issuer":"{issuer}","authorization_endpoint":"{issuer}/authorize","token_endpoint":"{issuer}/token","scopes_supported":["openid","profile","email","offline_access"],"code_challenge_methods_supported":["plain","S256"]}}"#
    )
}

#[test]
fn a_description_names_the_service_at_its_own_address() {
    // The mail server sends on to its service's description, under a path (a Keycloak realm).
    let base = service(|base, line| {
        if line.starts_with("GET /.well-known/openid-configuration ") {
            format!("HTTP/1.1 302 Found\r\nLocation: {base}/realms/mail/.well-known/openid-configuration\r\nContent-Length: 0\r\n\r\n").into_bytes()
        } else {
            json(&description(&format!("{base}/realms/mail")))
        }
    });
    let d = describe(&description_url_for_server(&format!("{base}/jmap/session")).unwrap()).unwrap();
    assert_eq!(d.issuer, format!("{base}/realms/mail"));
    assert_eq!(
        (d.authorization_endpoint.as_str(), d.token_endpoint.as_str()),
        (&*format!("{base}/realms/mail/authorize"), &*format!("{base}/realms/mail/token"))
    );
    let p = provider_from(&d, "mailcraft".into(), "");
    assert_eq!((p.id.as_str(), p.name.as_str(), p.client_id.as_str()), ("oidc", "127.0.0.1", "mailcraft"));
    assert_eq!(p.scope, "openid profile email offline_access");
    assert_eq!(p.client_secret, None);
    // Its issuer written with a trailing `/` is the same one.
    let base = service(|base, _| json(&description(&format!("{base}/"))));
    assert!(describe(&description_url(&base)).is_ok());
}

#[test]
fn a_description_that_cant_be_trusted_is_refused() {
    let refused = |answer: fn(&str) -> Vec<u8>| {
        let base = service(move |base, _| answer(base));
        match describe(&description_url(&base)) {
            Ok(d) => panic!("taken: {d:?} from {}", String::from_utf8_lossy(&answer(&base)).chars().take(600).collect::<String>()),
            Err(e) => e.to_string(),
        }
    };
    // Another service's description, sent from this one.
    let e = refused(|_| json(&description("https://sso.example.com")));
    assert!(e.contains("says it's https://sso.example.com"), "{e}");
    // A plain http endpoint anywhere but this computer.
    let e = refused(|base| json(&description(base).replace(&format!("{base}/token"), "http://sso.example.com/token")));
    assert!(e.contains("token_endpoint isn't https"), "{e}");
    // PKCE without S256, no token endpoint, not JSON, no description at all.
    let e = refused(|base| json(&description(base).replace(r#""plain","S256""#, r#""plain""#)));
    assert!(e.contains("PKCE"), "{e}");
    let e = refused(|base| json(&description(base).replace("token_endpoint", "nothing")));
    assert!(e.contains("no `token_endpoint`"), "{e}");
    let e = refused(|_| json("<html>"));
    assert!(e.contains("isn't JSON"), "{e}");
    let e = refused(|_| b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_vec());
    assert!(e.contains("HTTP 404"), "{e}");
    // An issuer with a query, or with a line break inside it.
    let e = refused(|base| json(&description(base).replacen(&format!(r#""issuer":"{base}""#), &format!(r#""issuer":"{base}?x=1""#), 1)));
    assert!(e.contains("says it's"), "{e}");
    let e = refused(|base| json(&description(base).replacen(&format!(r#""issuer":"{base}""#), &format!(r#""issuer":"{base}/a\nb""#), 1)));
    assert!(e.contains("no `issuer`"), "{e}");
    // A description too large to be one.
    let e = refused(|base| json(&format!("{}{}", description(base), " ".repeat(300 << 10))));
    assert!(!e.is_empty());
    // Over plain http to anywhere but this computer: not even asked.
    let e = describe("http://sso.example.com/.well-known/openid-configuration").unwrap_err().to_string();
    assert!(e.contains("address isn't https"), "{e}");
}

/// An account signs in with the service it names: not with another, that the address it names
/// sends the request on to. Its issuer written with a trailing `/` is the same one.
#[test]
fn an_account_signs_in_with_the_service_it_names() {
    let sso = service(|base, _| json(&description(base)));
    let to = sso.clone();
    let elsewhere = service(move |_, _| format!("HTTP/1.1 302 Found\r\nLocation: {to}{WELL_KNOWN}\r\nContent-Length: 0\r\n\r\n").into_bytes());
    let named = |issuer: String| OidcSettings { issuer, client_id: "mailcraft".into(), scope: String::new() };
    let e = provider(&named(elsewhere.clone())).unwrap_err().to_string();
    assert!(e.contains(&format!("at {elsewhere} says it's {sso}")), "{e}");
    let p = provider(&named(format!("{sso}/"))).unwrap();
    assert_eq!((p.auth_url, p.token_url), (format!("{sso}/authorize"), format!("{sso}/token")));
}

proptest::proptest! {
    #![proptest_config(proptest::prelude::ProptestConfig::with_cases(400))]
    /// Whatever a service sends as its description is read within the caps, or refused. One
    /// that's taken names the service where it came from, and its endpoints are https unless it
    /// came over plain http from this computer; one that names its own service, within the caps
    /// and with PKCE's S256, is taken.
    #[test]
    fn a_description_is_read_within_its_caps(
        from in proptest::sample::select(vec!["https://sso.example.com", "https://kc.example.org/realms/mail", "http://127.0.0.1:8080"]),
        own in proptest::prelude::any::<bool>(),
        slash in proptest::prelude::any::<bool>(),
        other in "[ -~]{0,40}",
        long in 0usize..2100,
        scopes in proptest::collection::vec("[ -~]{0,12}", 0..300),
        methods in proptest::option::of(proptest::collection::vec("S256|plain|[a-z0-9]{0,4}", 0..3)),
    ) {
        let at = Url::parse(&format!("{from}{WELL_KNOWN}")).unwrap();
        let (issuer, authorize, token) = if own {
            (format!("{from}{}", if slash { "/" } else { "" }), format!("{from}/authorize{}", "a".repeat(long)), format!("{from}/token"))
        } else {
            (other.clone(), format!("{other}/authorize"), other.clone())
        };
        let mut body = serde_json::json!({"issuer": issuer, "authorization_endpoint": authorize, "token_endpoint": token, "scopes_supported": scopes});
        if let (Some(m), Some(o)) = (&methods, body.as_object_mut()) {
            o.insert("code_challenge_methods_supported".into(), serde_json::json!(m));
        }
        let read = description_from(body.to_string().as_bytes(), &at);
        let s256 = methods.as_ref().is_none_or(|m| m.iter().any(|m| m == "S256"));
        if own && authorize.len() <= MAX_FIELD && s256 {
            proptest::prop_assert!(read.is_ok(), "{read:?}");
        }
        if let Ok(d) = read {
            proptest::prop_assert!(same_issuer(from, &d.issuer) && s256, "{d:?}");
            for f in [&d.issuer, &d.authorization_endpoint, &d.token_endpoint] {
                proptest::prop_assert!(!f.is_empty() && f.len() <= MAX_FIELD && !f.chars().any(|c| c.is_whitespace() || c.is_control()));
            }
            for e in [&d.authorization_endpoint, &d.token_endpoint] {
                let u = Url::parse(e).unwrap();
                proptest::prop_assert!(u.https || (!at.https && loopback(&u.host)), "{e}");
            }
            proptest::prop_assert!(d.scopes_supported.len() <= MAX_SUPPORTED && d.scopes_supported.iter().all(|s| scope_token(s)));
        }
    }
}

/// A JMAP session's refusal of a request without a sign-in, asking for `challenges`.
fn refusal(challenges: &[&str]) -> Vec<u8> {
    let asks: String = challenges.iter().map(|c| format!("WWW-Authenticate: {c}\r\n")).collect();
    format!("HTTP/1.1 401 Unauthorized\r\n{asks}Content-Length: 0\r\nConnection: close\r\n\r\n").into_bytes()
}

#[test]
fn a_server_says_how_it_is_signed_in_to() {
    // Bearer tokens only (as a server behind a single sign-on asks), and the service its own
    // address describes. Its other ways, unknown to MailCraft, aside.
    let base = service(|base, line| {
        if line.starts_with("GET /.well-known/openid-configuration ") {
            json(&description(base))
        } else {
            refusal(&[r#"Bearer realm="mail", error="invalid_token", scope="openid  profile email \"x\"""#, r#"Ticket realm="mail""#])
        }
    });
    let p = probe(&format!("{base}/jmap/session"));
    assert!(p.bearer && !p.password, "{p:?}");
    assert_eq!((p.issuer.as_deref(), p.scope.as_str(), p.error.as_deref()), (Some(base.as_str()), "openid profile email", None));
    assert_eq!(p.client_id, None, "none given for a service on this computer");
    // The session's address sends on to the server's own: its service is the one asked.
    let server = base.clone();
    let start = service(move |_, line| {
        if line.starts_with("GET /.well-known/jmap ") {
            format!("HTTP/1.1 307 Temporary Redirect\r\nLocation: {server}/jmap/session\r\nContent-Length: 0\r\n\r\n").into_bytes()
        } else {
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_vec()
        }
    });
    let p = probe(&start);
    assert_eq!((p.bearer, p.issuer.as_deref()), (true, Some(base.as_str())), "{p:?}");

    // Passwords too, or only: no service asked for.
    let both = service(|_, _| refusal(&["Basic realm=\"mail\"", "Bearer"]));
    let p = probe(&format!("{both}/jmap/session"));
    assert!(p.password && p.bearer, "{p:?}");
    for answer in [refusal(&["Basic realm=\"mail\""]), refusal(&[]), json("{}"), b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_vec()] {
        let server = service(move |_, _| answer.clone());
        let p = probe(&format!("{server}/jmap/session"));
        assert_eq!(p, JmapProbe { password: true, ..JmapProbe::default() }, "{server}");
    }
    // No server there, or one that fails: a password, whose sign-in says what's wrong, and why it
    // couldn't be asked, which may be passing (the add dialog asks again).
    let gone = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
    let p = probe(&format!("http://{gone}/jmap/session"));
    assert!(p.password && !p.bearer && p.issuer.is_none() && p.error.is_some(), "{p:?}");
    let failing = service(|_, _| b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n\r\n".to_vec());
    let p = probe(&format!("{failing}/jmap/session"));
    assert_eq!((p.password, p.bearer, p.error.as_deref()), (true, false, Some("the server answered HTTP 503")));
    // Bearer tokens, and no service to be found: said why.
    let lost = service(|_, line| {
        if line.contains("openid-configuration") { b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_vec() } else { refusal(&["Bearer"]) }
    });
    let p = probe(&format!("{lost}/jmap/session"));
    assert!(p.bearer && p.issuer.is_none() && p.error.as_deref().is_some_and(|e| e.contains("HTTP 404")), "{p:?}");
}

#[test]
fn what_is_asked_for() {
    let supported = |s: &[&str]| s.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(scope_for("", &[]), DEFAULT_SCOPE);
    assert_eq!(scope_for("  ", &supported(&["offline_access"])), "openid profile email offline_access");
    assert_eq!(scope_for("openid email", &supported(&["openid", "email"])), "openid email");
    // What the server named, without what isn't a scope or comes twice.
    assert_eq!(scope_for("openid \"x\" mail\\ email openid é", &[]), "openid email");
    assert_eq!(scope_for("openid offline_access", &supported(&["offline_access"])), "openid offline_access");
    let many = (0..100).map(|i| format!("s{i}")).collect::<Vec<_>>().join(" ");
    assert_eq!(scope_for(&many, &[]).split(' ').count(), MAX_SCOPES);
}

#[test]
fn client_ids_are_given_by_issuer() {
    let given =
        parse_clients(" https://sso.example.com/ = mailcraft ;https://kc.example.org/realms/mail=mc=desktop;;junk; =x;https://empty.example=");
    assert_eq!(
        given,
        vec![("https://sso.example.com".to_string(), "mailcraft".to_string()), ("https://kc.example.org/realms/mail".into(), "mc=desktop".into())]
    );
    assert_eq!(given_in(&given, "https://sso.example.com/").as_deref(), Some("mailcraft"));
    assert_eq!(given_in(&given, "https://kc.example.org/realms/mail").as_deref(), Some("mc=desktop"));
    // Compared as written: another path, scheme or case is another issuer.
    for other in ["https://kc.example.org/realms", "http://sso.example.com", "https://SSO.example.com"] {
        assert_eq!(given_in(&given, other), None, "{other}");
    }
    // Run time's first.
    let both = clients_from("https://sso.example.com=run", "https://sso.example.com=built;https://other.example=b");
    assert_eq!(given_in(&both, "https://sso.example.com").as_deref(), Some("run"));
    assert_eq!(given_in(&both, "https://other.example").as_deref(), Some("b"));
    // The account's own comes first; one that couldn't be sent isn't one.
    let own = |c: &str| OidcSettings { issuer: "https://sso.invalid".into(), client_id: c.into(), ..Default::default() };
    assert_eq!(client_id(&own(" mine ")).as_deref(), Some("mine"));
    assert_eq!(client_id(&own("a\nb")), None);
    assert_eq!(client_id(&own(&"x".repeat(MAX_CLIENT_ID + 1))), None);
    // With none, the account is told to ask for one, and the service isn't asked anything.
    assert!(matches!(provider(&own("")), Err(NetError::Auth(m)) if m == mailcraft_model::NO_CLIENT_ID));
}
