//! Message submission (RFC 6409) through `lettre`: implicit TLS on 465 (preferred, RFC 8314),
//! STARTTLS on 587, or plain only for local test servers.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs, UdpSocket};
use std::sync::Arc;
use std::time::Duration;

use lettre::address::{Address, Envelope};
use lettre::transport::smtp::authentication::{Credentials, DEFAULT_MECHANISMS, Mechanism};
use lettre::transport::smtp::client::{Certificate, SmtpConnection, TlsParameters};
use lettre::transport::smtp::extension::ClientId;
use mailcraft_model::{Security, ServerConfig};
use rustls::pki_types::CertificateDer;

use crate::stream::trust::{self, Distrusting};
use crate::{NetError, Result};

#[cfg(test)]
mod tls_tests;

/// Submit `raw` (an RFC 5322 message) from `from` to `recipients`.
pub fn send(server: &ServerConfig, user: &str, password: &str, from: &str, recipients: &[String], raw: &[u8]) -> Result<()> {
    send_with(server, user, password, false, from, recipients, raw)
}

/// [`send`], signing in with an OAuth access token (XOAUTH2) when `oauth`.
pub fn send_with(server: &ServerConfig, user: &str, password: &str, oauth: bool, from: &str, recipients: &[String], raw: &[u8]) -> Result<()> {
    let from: Address = from.trim().parse().map_err(|e| NetError::Smtp(format!("sender {from}: {e}")))?;
    let mut to = Vec::with_capacity(recipients.len());
    for r in recipients {
        to.push(r.trim().parse::<Address>().map_err(|e| NetError::Smtp(format!("recipient {r}: {e}")))?);
    }
    if to.is_empty() {
        return Err(NetError::Smtp("no recipients".into()));
    }
    let envelope = Envelope::new(Some(from), to).map_err(|e| NetError::Smtp(e.to_string()))?;
    let trust = Trust { roots: crate::stream::system_roots(), verifier: crate::stream::system_verifier().map_err(NetError::Tls)? };
    submit(server, SignIn { user, password, oauth }, &envelope, raw, recipients, &trust)
}

/// Who signs in, and how: a password, or an OAuth access token (XOAUTH2).
struct SignIn<'a> {
    user: &'a str,
    password: &'a str,
    oauth: bool,
}

/// What sending trusts: `roots` for lettre's handshake (the system's), and `verifier`, the check
/// every connection's certificates get ([`trust::check_chain`]).
struct Trust<'a> {
    roots: &'a [CertificateDer<'static>],
    verifier: Arc<Distrusting>,
}

/// One message over one connection, as lettre's `SmtpTransport` sends it (connect and EHLO,
/// STARTTLS, AUTH, MAIL / RCPT / DATA, QUIT), with the server's certificates checked between
/// the handshake and the sign-in: lettre takes no verifier of ours, and only so are a server or
/// intermediate certificate the system distrusts refused before the password goes out.
fn submit(server: &ServerConfig, sign_in: SignIn<'_>, envelope: &Envelope, raw: &[u8], recipients: &[String], trust: &Trust<'_>) -> Result<()> {
    let failed = |e: lettre::transport::smtp::Error| failure(e, recipients);
    let host = server.host.as_str();
    let wrapper = match server.security {
        Security::Tls => Some(tls_parameters(host, trust.roots)?),
        Security::StartTls | Security::None => None,
    };
    // Each of the server's addresses in turn, as lettre would try them, greeted with this
    // computer's own address on the way to it.
    let addrs: Vec<SocketAddr> = (host, server.port).to_socket_addrs().map_err(NetError::Io)?.take(MAX_ADDRESSES).collect();
    let mut tried = None;
    let mut connected = None;
    for addr in addrs {
        let hello = hello_for(addr);
        match SmtpConnection::connect(addr, Some(TIMEOUT), &hello, wrapper.as_ref(), None) {
            Ok(c) => {
                connected = Some((c, hello));
                break;
            }
            Err(e) => tried = Some(e),
        }
    }
    let Some((mut conn, hello)) = connected else {
        return Err(match tried {
            Some(e) => failed(e),
            None => NetError::Io(std::io::Error::other(format!("{host}: no address"))),
        });
    };
    if server.security == Security::StartTls {
        conn.starttls(&tls_parameters(host, trust.roots)?, &hello).map_err(failed)?;
    }
    if server.security != Security::None {
        // After a handshake there's always a chain: lettre's call can't come back empty.
        let chain = conn.certificate_chain().map_err(failed)?;
        if let Err(e) = trust::check_chain(&trust.verifier, &chain, host) {
            conn.abort();
            return Err(NetError::Tls(e.to_string()));
        }
    }
    if !sign_in.user.is_empty() {
        let mechanisms: &[Mechanism] = if sign_in.oauth { &[Mechanism::Xoauth2] } else { DEFAULT_MECHANISMS };
        conn.auth(mechanisms, &Credentials::new(sign_in.user.to_string(), sign_in.password.to_string())).map_err(failed)?;
    }
    let sent = conn.send(envelope, raw);
    conn.abort();
    sent.map_err(failed)?;
    Ok(())
}

/// How long the server may take to answer.
const TIMEOUT: Duration = Duration::from_secs(60);

/// The most of a server's addresses tried.
const MAX_ADDRESSES: usize = 8;

/// What MailCraft calls itself in its EHLO to `server` (RFC 5321 §4.1.4): the address literal
/// of this computer's address on the way there, as Thunderbird sends, since a desktop has no name
/// the world knows. lettre's own, without a host name, is `[127.0.0.1]`, and some providers'
/// spam filters drop mail greeted so after taking it (#50). The address is the one the system
/// would send from: a socket "connected" without sending anything says it.
fn hello_for(server: SocketAddr) -> ClientId {
    let any: SocketAddr = if server.is_ipv4() { (Ipv4Addr::UNSPECIFIED, 0).into() } else { (Ipv6Addr::UNSPECIFIED, 0).into() };
    let local = UdpSocket::bind(any).and_then(|s| {
        s.connect(server)?;
        s.local_addr()
    });
    match local.map(|a| a.ip()) {
        Ok(IpAddr::V4(ip)) if !ip.is_unspecified() => ClientId::Ipv4(ip),
        Ok(IpAddr::V6(ip)) if !ip.is_unspecified() => ip.to_ipv4_mapped().map_or(ClientId::Ipv6(ip), ClientId::Ipv4),
        // No way to the server: the connection fails anyway.
        _ => ClientId::default(),
    }
}

/// What a failure from lettre means for the message: refused for good, to try again later, or
/// no answer at all.
fn failure(e: lettre::transport::smtp::Error, recipients: &[String]) -> NetError {
    let msg = e.to_string();
    if e.is_permanent() {
        let code = e.status().and_then(|c| c.to_string().parse::<u16>().ok()).unwrap_or(550);
        refusal(code, &msg, recipients)
    } else if e.is_transient() {
        // 4xx: the server asks to try again later.
        NetError::Later(msg)
    } else {
        // No answer at all: no network, refused connection, TLS failure or a timeout.
        NetError::Io(std::io::Error::other(msg))
    }
}

/// TLS for `host`: lettre's bundled Mozilla roots and `roots` (the system's, which IMAP and
/// HTTPS trust too). lettre fails the whole build on a root it can't read, so `roots` holds
/// only usable ones, the system's distrusted ones already left out
/// ([`crate::stream::system_roots`]). What else the system distrusts, [`submit`] refuses.
fn tls_parameters(host: &str, roots: &[CertificateDer<'static>]) -> Result<TlsParameters> {
    let mut p = TlsParameters::builder(host.to_string());
    for der in roots {
        if let Ok(c) = Certificate::from_der(der.to_vec()) {
            p = p.add_root_certificate(c);
        }
    }
    p.build_rustls().map_err(|e| NetError::Tls(e.to_string()))
}

/// What a 5xx reply means, by its code (RFC 4954 §6, RFC 3463), never by words in its text
/// ("553 5.7.1 sender not owned by authenticated user" is a refusal of the message):
/// - 530 / 534 / 535 / 538, or the enhanced codes 5.7.8 (bad credentials) and 5.7.9 (mechanism
///   too weak): the sign-in was refused. Not the message's fault: it waits for a new sign-in.
/// - anything else (550/551/552/553/554 on MAIL, RCPT or DATA): this message is refused for good,
///   naming the recipients the server's text names.
fn refusal(code: u16, msg: &str, recipients: &[String]) -> NetError {
    // Enhanced status codes stand alone in the text, maybe wrapped in `( )` or followed by `:`.
    let says = |status: &str| msg.split(|c: char| c.is_whitespace() || ":()".contains(c)).any(|token| token == status);
    let sign_in_code = matches!(code, 530 | 534 | 535 | 538);
    if sign_in_code || says("5.7.8") || says("5.7.9") {
        return NetError::Auth(msg.to_string());
    }
    let lower = msg.to_ascii_lowercase();
    let named: Vec<String> =
        recipients.iter().map(|r| r.trim().to_ascii_lowercase()).filter(|r| !r.is_empty() && lower.contains(r.as_str())).collect();
    if named.is_empty() { NetError::Smtp(msg.to_string()) } else { NetError::Smtp(format!("{} refused: {msg}", named.join(", "))) }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::io::{BufRead, BufReader, Write};
    use std::net::{TcpListener, UdpSocket};

    use super::*;

    /// A minimal SMTP server on localhost that accepts one message.
    pub(crate) fn serve_once() -> (u16, std::thread::JoinHandle<String>) {
        serve_once_on(TcpListener::bind("127.0.0.1:0").unwrap())
    }

    /// The same, on a given listener.
    pub(crate) fn serve_once_on(l: TcpListener) -> (u16, std::thread::JoinHandle<String>) {
        let port = l.local_addr().unwrap().port();
        let h = std::thread::spawn(move || {
            let (s, _) = l.accept().unwrap();
            let mut w = s.try_clone().unwrap();
            let mut r = BufReader::new(s);
            let mut log = String::new();
            w.write_all(b"220 test ESMTP\r\n").unwrap();
            let mut line = String::new();
            let mut in_data = false;
            while r.read_line(&mut line).unwrap_or(0) > 0 {
                log.push_str(&line);
                let cmd = line.trim_end().to_ascii_uppercase();
                if in_data {
                    if cmd == "." {
                        in_data = false;
                        w.write_all(b"250 queued\r\n").unwrap();
                    }
                } else if cmd.starts_with("EHLO") {
                    w.write_all(b"250-test\r\n250 AUTH PLAIN\r\n").unwrap();
                } else if cmd.starts_with("AUTH") {
                    w.write_all(b"235 ok\r\n").unwrap();
                } else if cmd == "DATA" {
                    in_data = true;
                    w.write_all(b"354 go\r\n").unwrap();
                } else if cmd == "QUIT" {
                    w.write_all(b"221 bye\r\n").unwrap();
                    break;
                } else {
                    w.write_all(b"250 ok\r\n").unwrap();
                }
                line.clear();
            }
            log
        });
        (port, h)
    }

    /// Issue #50: the EHLO names this computer's address on the way to the server, as an address
    /// literal: the loopback address only for a server on this computer, and this computer's own
    /// network address for one reached through it (lettre's default was `[127.0.0.1]` for all).
    #[test]
    fn ehlo_names_this_computers_address_on_the_way() {
        let raw = b"From: z@x.example\r\nTo: a@y.example\r\nSubject: hi\r\n\r\nbody\r\n";
        let to = ["a@y.example".to_string()];
        let (port, h) = serve_once();
        let server = ServerConfig { host: "127.0.0.1".into(), port, security: Security::None };
        send(&server, "", "", "z@x.example", &to, raw).unwrap();
        let log = h.join().unwrap();
        assert!(log.starts_with("EHLO [127.0.0.1]\r\n"), "{log}");
        // IPv6 loopback, where the system has it.
        if let Ok(l) = TcpListener::bind("[::1]:0") {
            let (port, h) = serve_once_on(l);
            let server = ServerConfig { host: "::1".into(), port, security: Security::None };
            send(&server, "", "", "z@x.example", &to, raw).unwrap();
            let log = h.join().unwrap();
            assert!(log.starts_with("EHLO [IPv6:::1]\r\n"), "{log}");
        }
        // This computer's network address (the one it would reach a documentation address from;
        // nothing is sent there), where it has one: a server reached through it hears that.
        let own = UdpSocket::bind("0.0.0.0:0").and_then(|s| {
            s.connect("192.0.2.1:9")?;
            s.local_addr()
        });
        let Some(own) = own.ok().map(|a| a.ip()).filter(|ip| !ip.is_loopback() && !ip.is_unspecified()) else {
            eprintln!("no network address: the rest skipped");
            return;
        };
        let (port, h) = serve_once_on(TcpListener::bind((own, 0)).unwrap());
        let server = ServerConfig { host: own.to_string(), port, security: Security::None };
        send(&server, "", "", "z@x.example", &to, raw).unwrap();
        let log = h.join().unwrap();
        assert!(log.starts_with(&format!("EHLO [{own}]\r\n")), "{log}");
        assert!(!log.contains("127.0.0.1"), "{log}");
    }

    #[test]
    fn submits_to_a_local_server() {
        let (port, h) = serve_once();
        let server = ServerConfig { host: "127.0.0.1".into(), port, security: Security::None };
        let raw = b"From: z@x.example\r\nTo: a@y.example\r\nSubject: hi\r\n\r\nbody\r\n";
        send(&server, "z@x.example", "pw", "z@x.example", &["a@y.example".into(), "b@y.example".into()], raw).unwrap();
        let log = h.join().unwrap();
        assert!(log.contains("MAIL FROM:<z@x.example>"), "{log}");
        assert!(log.contains("RCPT TO:<a@y.example>") && log.contains("RCPT TO:<b@y.example>"));
        assert!(log.contains("Subject: hi"));
    }

    /// A refusal is classified by its reply code, not by the word "auth" in its text.
    #[test]
    fn refusals_are_classified_by_code() {
        let to = ["bob@y.example".to_string(), "ann@y.example".to_string()];
        let class = |code: u16, text: &str| refusal(code, text, &to);
        // Sign-in refusals: kept for after signing in again, not refused for good.
        for (code, text) in [
            (535, "5.7.8 Username and Password not accepted"),
            (534, "5.7.9 Please log in with your web browser"),
            (530, "5.7.0 Authentication required"),
            (550, "5.7.8 authentication credentials invalid"),
        ] {
            let e = class(code, text);
            assert!(matches!(e, NetError::Auth(_)), "{code} {text}: {e:?}");
            assert!(e.is_retryable(), "{code}: kept in the Outbox");
        }
        // The message refused for good.
        for (code, text) in [
            (553, "5.7.1 sender not owned by authenticated user"),
            (554, "5.7.1 relay access denied: authentication needed for relaying elsewhere"),
            (552, "5.3.4 message too big"),
        ] {
            let e = class(code, text);
            assert!(matches!(e, NetError::Smtp(_)), "{code} {text}: {e:?}");
            assert!(!e.is_retryable());
        }
        // The refused recipient is named when the server says which.
        let e = class(550, "5.1.1 <Ann@y.example>: Recipient address rejected: User unknown");
        assert!(matches!(&e, NetError::Smtp(m) if m.contains("ann@y.example") && !m.contains("bob@")), "{e:?}");
    }

    /// Plays a scripted SMTP server for one connection on a free local port: greets, then answers
    /// each command line (upper-cased, line end removed) with what `script` returns, until QUIT.
    fn scripted(script: fn(&str) -> &'static str) -> u16 {
        let socket = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = socket.local_addr().unwrap();
        let play = move || {
            let Ok((sock, _)) = socket.accept() else { return };
            let mut out = sock.try_clone().unwrap();
            let _ = out.write_all(b"220 scripted.test ready\r\n");
            for command in BufReader::new(sock).lines().map_while(std::result::Result::ok) {
                let command = command.to_ascii_uppercase();
                let _ = out.write_all(script(&command).as_bytes());
                if command == "QUIT" {
                    break;
                }
            }
        };
        std::thread::spawn(play);
        address.port()
    }

    /// End to end: the server turns the sender away at MAIL FROM with a 553, which refuses this
    /// message; it isn't taken for a failed sign-in.
    #[test]
    fn a_refused_sender_is_not_a_sign_in_failure() {
        let port = scripted(|command| match command.split(' ').next().unwrap_or_default() {
            "EHLO" => "250-scripted.test\r\n250 AUTH PLAIN\r\n",
            "AUTH" => "235 2.7.0 welcome\r\n",
            "MAIL" => "553 5.7.1 <z@x.example>: the authenticated account may not send as this address\r\n",
            "QUIT" => "221 2.0.0 closing\r\n",
            _ => "250 2.0.0 fine\r\n",
        });
        let server = ServerConfig { security: Security::None, port, host: "127.0.0.1".into() };
        let message = b"Subject: refused\r\n\r\nnot delivered\r\n";
        let e = send(&server, "z@x.example", "pw", "z@x.example", &["a@y.example".into()], message).unwrap_err();
        assert!(matches!(e, NetError::Smtp(_)), "{e:?}");
    }

    #[test]
    fn rejects_bad_addresses_before_connecting() {
        let server = ServerConfig { host: "127.0.0.1".into(), port: 1, security: Security::None };
        assert!(matches!(send(&server, "", "", "not an address", &["a@b.c".into()], b""), Err(NetError::Smtp(_))));
        assert!(matches!(send(&server, "", "", "a@b.c", &[], b""), Err(NetError::Smtp(_))));
    }

    /// The system's roots reach SMTP's TLS, and one the store holds but rustls can't read
    /// doesn't stop sending: given to lettre as it is, it failed every message to every server.
    #[test]
    fn an_unreadable_root_in_the_store_does_not_stop_sending() {
        use crate::stream::trust::test_certs::{internal_ca, unreadable};
        use crate::stream::trust::{Distrusted, usable_roots};
        let store = vec![unreadable(), internal_ca()];
        assert!(tls_parameters("mail.corp.test", &usable_roots(store.clone(), &Distrusted::new())).is_ok());
        assert!(tls_parameters("mail.corp.test", &[]).is_ok());
        // Why the store is sorted first: lettre refuses the lot.
        assert!(tls_parameters("mail.corp.test", &store).is_err());
    }
}
