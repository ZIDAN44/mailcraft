use std::net::TcpListener;

use super::*;
use crate::oauth::oidc::tests::{description, json, service};

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

    // Passwords too, in one header or several, or only: no service asked for.
    let both = service(|_, _| refusal(&["Basic realm=\"mail\"", "Bearer"]));
    let p = probe(&format!("{both}/jmap/session"));
    assert!(p.password && p.bearer, "{p:?}");
    let one = service(|_, _| refusal(&["Basic realm=\"mail\", Bearer"]));
    let p = probe(&format!("{one}/jmap/session"));
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
