//! How a server asks to be signed in to: the input is a `WWW-Authenticate` value, read as
//! challenges within their caps, then sent with a JMAP server's 401 to a password and to a
//! token, which must both be refused as a sign-in that failed.
#![no_main]

use libfuzzer_sys::fuzz_target;
use mailcraft_jmap::{self as jmap, Client, Http, Request, Response, challenge};

struct Refusing(String);

impl Http for Refusing {
    async fn send(&self, _req: Request) -> jmap::Result<Response> {
        Ok(Response { status: 401, headers: vec![("WWW-Authenticate".into(), self.0.clone())], body: b"{}".to_vec() })
    }
}

fuzz_target!(|data: &[u8]| {
    let header = String::from_utf8_lossy(data).into_owned();
    let found = challenge::parse(&header);
    assert!(found.len() <= challenge::MAX_CHALLENGES);
    for c in &found {
        assert!(!c.scheme.is_empty() && c.params.len() <= challenge::MAX_PARAMS);
        let _ = c.param("scope");
    }
    for bearer in [false, true] {
        let refused = jmap::block_on(Client::connect(Refusing(header.clone()), "https://jmap.example/", "me", "pw", bearer));
        assert!(matches!(refused, Err(jmap::Error::Auth(_))));
    }
});
