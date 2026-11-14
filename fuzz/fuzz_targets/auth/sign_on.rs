//! A single sign-on service's description (OIDC Discovery §3): the input's first byte picks the
//! address it came from, the rest is what was sent from there. What's taken is within its caps,
//! names the service at that address, has https endpoints unless it came over plain http from
//! this computer, and is signed in with asking for a few scopes.
#![no_main]

use libfuzzer_sys::fuzz_target;
use mailcraft_net::http::Url;
use mailcraft_net::oauth::oidc;

/// Where a description can come from: a service, one under a path (a Keycloak realm), a test
/// service on this computer over plain http, and an address that isn't a description's.
const FROM: [&str; 4] = [
    "https://sso.example.com/.well-known/openid-configuration",
    "https://kc.example.org/realms/mail/.well-known/openid-configuration",
    "http://127.0.0.1:8080/.well-known/openid-configuration",
    "https://sso.example.com/description.json",
];

fuzz_target!(|data: &[u8]| {
    let Some((&pick, body)) = data.split_first() else { return };
    let at = Url::parse(FROM[usize::from(pick) % FROM.len()]).unwrap();
    let Ok(d) = oidc::description_from(body, &at) else { return };
    assert_eq!(Url::parse(&d.issuer).ok().map(|u| u.origin()), Some(at.origin()));
    for field in [&d.issuer, &d.authorization_endpoint, &d.token_endpoint] {
        assert!(!field.is_empty() && field.len() <= oidc::MAX_FIELD && !field.chars().any(char::is_whitespace));
    }
    for endpoint in [&d.authorization_endpoint, &d.token_endpoint] {
        assert!(Url::parse(endpoint).is_ok_and(|u| u.https || !at.https));
    }
    assert!(d.scopes_supported.len() <= oidc::MAX_SUPPORTED);
    let p = oidc::provider_from(&d, "mailcraft".into(), &String::from_utf8_lossy(body));
    assert!(p.scope.split(' ').count() <= oidc::MAX_SCOPES + 1);
});
