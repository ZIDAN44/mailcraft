//! What the fake checks of the client: a request breaking it is refused and kept for the test
//! to fail on, and the server goes on.

use std::io::{Read, Write};

use super::*;

/// One request to the fake, by hand: the status it's answered with (0: none).
fn status_of(server: &Server, target: &str, immutable: bool) -> u16 {
    let addr = server.base.trim_start_matches("http://").trim_end_matches("/v1.0");
    let mut s = std::net::TcpStream::connect(addr).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let prefer = if immutable { "Prefer: IdType=\"ImmutableId\"\r\n" } else { "" };
    write!(s, "GET /v1.0{target} HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer {TOKEN}\r\n{prefer}\r\n").unwrap();
    let mut out = String::new();
    let _ = s.read_to_string(&mut out);
    out.split(' ').nth(1).and_then(|c| c.parse().ok()).unwrap_or(0)
}

fn with_inbox() -> Server {
    let mut d = Data::default();
    d.folder("Inbox", None, Some("inbox"));
    serve(d)
}

/// Found in review: the fake asserted what it checks of the client on its own thread, so a
/// request breaking it killed the server (with its data's lock poisoned, for one) and the test
/// saw only a timeout. Ids not asked for in their immutable form, or messages asked for in
/// another order, are refused (400) and kept; the next request is answered.
#[test]
fn a_request_the_fake_forbids_is_refused_and_kept() {
    let server = with_inbox();
    assert_eq!(status_of(&server, "/me/mailFolders/inbox/messages", false), 400);
    assert_eq!(status_of(&server, "/me/mailFolders/inbox/messages?$orderby=subject", true), 400);
    assert_eq!(status_of(&server, "/me/mailFolders/inbox/messages?$orderby=receivedDateTime%20desc", true), 200);
    let kept = std::mem::take(&mut server.data.lock().unwrap().violations);
    assert_eq!(kept.len(), 2, "{kept:?}");
}

/// A refusal no one looked at fails the test when its server goes.
#[test]
#[should_panic(expected = "the client broke what the fake checks")]
fn a_refusal_left_unseen_fails_the_test() {
    let server = with_inbox();
    assert_eq!(status_of(&server, "/me/mailFolders/inbox/messages", false), 400);
}
