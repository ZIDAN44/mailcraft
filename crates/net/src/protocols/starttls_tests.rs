//! STARTTLS for IMAP, POP and ManageSieve, against a server on this computer: the connection is
//! encrypted before anything else is said, the certificate is checked as on every connection
//! (one no authority vouches for passes only when the user trusted it), and the client stops
//! rather than go on in the clear when the server can't upgrade or slips data in before it.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::thread::JoinHandle;

use mailcraft_model::{Security, ServerConfig};
use rustls::{ServerConnection, StreamOwned};

use crate::stream::Trusted;
use crate::stream::trust::fingerprint;
use crate::stream::trust::test_certs::server_identity;
use crate::{Result, imap, pop, sieve};

#[derive(Clone, Copy, Debug)]
enum Protocol {
    Imap,
    Pop,
    Sieve,
}

const ALL: [Protocol; 3] = [Protocol::Imap, Protocol::Pop, Protocol::Sieve];

/// What the server does when asked to upgrade.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Upgrade {
    /// Agrees, then speaks TLS.
    Agrees,
    /// Agrees, with more plain text in the same breath (an injection).
    Injects,
    /// Doesn't offer it.
    Lacks,
}

/// One line from the client, a byte at a time so nothing of a TLS handshake after it is read.
fn read_line(s: &mut impl Read) -> Option<String> {
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    while line.last() != Some(&b'\n') {
        match s.read(&mut byte) {
            Ok(1) => line.push(byte[0]),
            _ => return None,
        }
    }
    Some(String::from_utf8_lossy(&line).trim_end().to_string())
}

/// What the server says on connecting, in the clear or (`tls`) once encrypted.
fn greeting(p: Protocol, up: Upgrade, tls: bool) -> String {
    let offers = up != Upgrade::Lacks;
    match (p, tls) {
        (Protocol::Imap, false) if offers => "* OK [CAPABILITY IMAP4rev1 STARTTLS] hi\r\n".into(),
        (Protocol::Imap, false) => "* OK hi\r\n".into(),
        (Protocol::Pop, false) => "+OK hi\r\n".into(),
        (Protocol::Sieve, false) if offers => "\"IMPLEMENTATION\" \"test\"\r\n\"STARTTLS\"\r\nOK\r\n".into(),
        (Protocol::Sieve, false) => "\"IMPLEMENTATION\" \"test\"\r\nOK\r\n".into(),
        // ManageSieve repeats its capabilities over TLS (RFC 5804 §2.2); the others wait.
        (Protocol::Sieve, true) => "\"IMPLEMENTATION\" \"test\"\r\n\"SIEVE\" \"vacation\"\r\nOK\r\n".into(),
        (_, true) => String::new(),
    }
}

/// The server's answer to `line`, and whether TLS starts after it.
fn answer(p: Protocol, up: Upgrade, tls: bool, line: &str) -> (String, bool) {
    let mut words = line.split_whitespace();
    let tag = words.next().unwrap_or_default();
    let upgrade = |agreed: &str, slipped: &str| match up {
        Upgrade::Agrees => (agreed.to_string(), true),
        Upgrade::Injects => (format!("{agreed}{slipped}"), true),
        Upgrade::Lacks => (String::new(), false),
    };
    match p {
        Protocol::Imap => {
            let cmd = words.next().unwrap_or_default().to_ascii_uppercase();
            match cmd.as_str() {
                "STARTTLS" if !tls && up != Upgrade::Lacks => upgrade(&format!("{tag} OK begin\r\n"), "* OK slipped in\r\n"),
                "CAPABILITY" if tls => (format!("* CAPABILITY IMAP4rev1 IDLE\r\n{tag} OK\r\n"), false),
                "CAPABILITY" if up == Upgrade::Lacks => (format!("* CAPABILITY IMAP4rev1\r\n{tag} OK\r\n"), false),
                "CAPABILITY" => (format!("* CAPABILITY IMAP4rev1 STARTTLS\r\n{tag} OK\r\n"), false),
                "LOGOUT" => (format!("* BYE bye\r\n{tag} OK\r\n"), false),
                _ => (format!("{tag} BAD unknown\r\n"), false),
            }
        }
        Protocol::Pop => match tag.to_ascii_uppercase().as_str() {
            "STLS" if !tls && up != Upgrade::Lacks => upgrade("+OK begin\r\n", "+OK slipped in\r\n"),
            "QUIT" => ("+OK bye\r\n".into(), false),
            _ => ("-ERR unknown\r\n".into(), false),
        },
        Protocol::Sieve => match tag.to_ascii_uppercase().as_str() {
            "STARTTLS" if !tls && up != Upgrade::Lacks => upgrade("OK begin\r\n", "\"IMPLEMENTATION\" \"slipped in\"\r\n"),
            "LOGOUT" => ("OK bye\r\n".into(), false),
            _ => ("NO unknown\r\n".into(), false),
        },
    }
}

/// Answer `s` until the client goes (or, unless `tls`, until TLS starts: then `true`). Every
/// line the client sent goes into `log`, after `side`.
fn converse(s: &mut (impl Read + Write), p: Protocol, up: Upgrade, tls: bool, log: &mut String) -> bool {
    let side = if tls { "tls" } else { "plain" };
    let hello = greeting(p, up, tls);
    if !hello.is_empty() && s.write_all(hello.as_bytes()).and_then(|()| s.flush()).is_err() {
        return false;
    }
    while let Some(line) = read_line(s) {
        log.push_str(&format!("{side}: {line}\n"));
        let (reply, starts) = answer(p, up, tls, &line);
        if s.write_all(reply.as_bytes()).and_then(|()| s.flush()).is_err() {
            return false;
        }
        if starts {
            return true;
        }
    }
    false
}

/// A server for `p` on 127.0.0.1 presenting the test certificate, for one connection. What the
/// client said, once it's gone, or once it hasn't come, or said nothing, for
/// [`crate::http::tests::TEST_SERVER_WAIT`].
fn serve(p: Protocol, up: Upgrade) -> (u16, JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let (chain, key) = server_identity();
    let config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(chain, key)
        .unwrap();
    let handle = std::thread::spawn(move || {
        let mut log = String::new();
        let Some(mut tcp) = crate::http::tests::accept_within(&listener, crate::http::tests::TEST_SERVER_WAIT) else { return log };
        if !converse(&mut tcp, p, up, false, &mut log) {
            return log;
        }
        let Ok(conn) = ServerConnection::new(Arc::new(config)) else { return log };
        let mut tls = StreamOwned::new(conn, tcp);
        converse(&mut tls, p, up, true, &mut log);
        log
    });
    (port, handle)
}

/// Connect to such a server with STARTTLS, trusting the certificates `trusted`; once connected,
/// check what was learned over TLS and say goodbye. The outcome, and what the server heard.
fn talk(p: Protocol, up: Upgrade, trusted: &[String]) -> (Result<()>, String) {
    let (port, server) = serve(p, up);
    let cfg = ServerConfig { host: "127.0.0.1".into(), port, security: Security::StartTls };
    let outcome: Result<()> = match p {
        Protocol::Imap => imap::Client::connect_trusting(&cfg, Trusted::pinned(trusted)).map(|mut c| {
            // Capabilities from before TLS are forgotten.
            assert!(c.has("IDLE") && !c.has("STARTTLS"), "IMAP kept what it heard in the clear");
            c.logout();
        }),
        Protocol::Pop => pop::Client::connect_trusting(&cfg, Trusted::pinned(trusted)).and_then(pop::Client::quit),
        Protocol::Sieve => sieve::Client::connect_trusting(&cfg, port, Trusted::pinned(trusted)).map(|mut c| {
            assert!(c.has_extension("vacation"), "Sieve kept the capabilities it heard in the clear");
            c.logout();
        }),
    };
    (outcome, server.join().unwrap())
}

fn the_certificate() -> String {
    fingerprint(&server_identity().0[0])
}

/// The upgrade happens, and everything after it goes over TLS.
#[test]
fn starttls_upgrades_to_a_certificate_the_user_trusts() {
    for p in ALL {
        let (outcome, heard) = talk(p, Upgrade::Agrees, &[the_certificate()]);
        assert!(outcome.is_ok(), "{p:?}: {outcome:?} {heard}");
        let plain: Vec<&str> = heard.lines().filter(|l| l.starts_with("plain:")).collect();
        assert_eq!(plain.len(), 1, "{p:?}: only the upgrade is said in the clear: {heard}");
        assert!(plain.iter().all(|l| l.contains("STARTTLS") || l.contains("STLS")), "{p:?}: {heard}");
        assert!(heard.lines().any(|l| l.starts_with("tls:")), "{p:?}: {heard}");
    }
}

/// A certificate no authority vouches for, that the user didn't trust: refused, saying which it
/// was, and nothing more is said.
#[test]
fn starttls_refuses_a_certificate_nobody_vouches_for() {
    for p in ALL {
        let (outcome, heard) = talk(p, Upgrade::Agrees, &[]);
        let Err(crate::NetError::Untrusted(shown)) = outcome else { panic!("{p:?}: {outcome:?}") };
        assert_eq!((shown.host.as_str(), shown.fingerprint, shown.remote), ("127.0.0.1", the_certificate(), false), "{p:?}");
        assert!(!heard.contains("tls:"), "{p:?}: {heard}");
    }
}

/// Plain text after the server's go-ahead and before TLS would be read as if it came over TLS:
/// the client stops.
#[test]
fn starttls_stops_at_data_slipped_in_before_tls() {
    for p in ALL {
        let (outcome, heard) = talk(p, Upgrade::Injects, &[the_certificate()]);
        let why = outcome.err().map(|e| e.to_string()).unwrap_or_default();
        assert!(why.contains("unexpected data before TLS negotiation"), "{p:?}: {why}");
        assert!(!heard.contains("tls:"), "{p:?}: {heard}");
    }
}

/// A server without STARTTLS: the client stops rather than go on in the clear.
#[test]
fn starttls_never_falls_back_to_the_clear() {
    for p in ALL {
        let (outcome, heard) = talk(p, Upgrade::Lacks, &[the_certificate()]);
        let why = outcome.err().map(|e| e.to_string()).unwrap_or_default();
        assert!(why.contains("STARTTLS") || why.contains("STLS"), "{p:?}: {why}");
        assert!(!heard.contains("tls:"), "{p:?}: {heard}");
        // Nothing past the attempt (no sign-in, no other command) went out.
        assert!(heard.lines().all(|l| ["STARTTLS", "STLS", "CAPABILITY"].iter().any(|c| l.contains(c))), "{p:?}: {heard}");
    }
}
