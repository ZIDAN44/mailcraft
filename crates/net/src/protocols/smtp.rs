//! Message submission (RFC 6409) through `lettre`: implicit TLS on 465 (preferred, RFC 8314),
//! STARTTLS on 587, or plain only for local test servers.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs, UdpSocket};
use std::sync::Arc;
use std::time::Duration;

use lettre::address::{Address, Envelope};
use lettre::transport::smtp::authentication::{Credentials, DEFAULT_MECHANISMS, Mechanism};
use lettre::transport::smtp::client::{Certificate, SmtpConnection, TlsParameters};
use lettre::transport::smtp::commands::{Data, Mail, Rcpt};
use lettre::transport::smtp::extension::{ClientId, Extension, MailBodyParameter, MailParameter};
use mailcraft_model::{Security, ServerConfig, SignInMethod};
use rustls::pki_types::CertificateDer;

use crate::stream::Trusted;
use crate::stream::trust::{self, Distrusting};
use crate::{NetError, Result};

#[cfg(test)]
mod sign_in_tests;
#[cfg(test)]
mod tls_tests;

/// Submit `raw` (an RFC 5322 message) from `from` to `recipients`, signing in with a password:
/// the tests' way to put mail on a server.
#[cfg(test)]
pub fn send(server: &ServerConfig, user: &str, password: &str, from: &str, recipients: &[String], raw: &[u8]) -> Result<()> {
    send_trusting(server, SignIn { user, password, oauth: false, method: SignInMethod::Automatic }, from, recipients, raw, Trusted::default())
}

/// Submit `raw` (an RFC 5322 message) from `from` to `recipients` through an account's server,
/// signing in as `sign_in` says: `trusted` is what the account trusts besides the usual
/// authorities, as [`crate::stream::tls_trusting`] takes it.
pub fn send_trusting(server: &ServerConfig, sign_in: SignIn<'_>, from: &str, recipients: &[String], raw: &[u8], trusted: Trusted<'_>) -> Result<()> {
    let from: Address = from.trim().parse().map_err(|e| NetError::Smtp(format!("sender {from}: {e}")))?;
    let mut to = Vec::with_capacity(recipients.len());
    for r in recipients {
        to.push(r.trim().parse::<Address>().map_err(|e| NetError::Smtp(format!("recipient {r}: {e}")))?);
    }
    if to.is_empty() {
        return Err(NetError::Smtp("no recipients".into()));
    }
    let envelope = Envelope::new(Some(from), to).map_err(|e| NetError::Smtp(e.to_string()))?;
    let trust = Trust { roots: crate::stream::system_roots(), verifier: crate::stream::system_verifier().map_err(NetError::Tls)?, trusted };
    submit(server, sign_in, &envelope, raw, recipients, &trust)
}

/// Who signs in, and how: a password as `method` says, or an OAuth access token (XOAUTH2) when
/// `oauth`. An empty `user` sends without signing in.
pub struct SignIn<'a> {
    pub user: &'a str,
    /// The password, or the access token when `oauth`.
    pub password: &'a str,
    pub oauth: bool,
    pub method: SignInMethod,
}

/// What sending trusts: `roots` for lettre's handshake (the system's), and `verifier`, the check
/// every connection's certificates get, with what the account trusts besides ([`trust::Own`]).
struct Trust<'a> {
    roots: &'a [CertificateDer<'static>],
    verifier: Arc<Distrusting>,
    trusted: Trusted<'a>,
}

/// One message over one connection, as lettre's `SmtpTransport` sends it (connect and EHLO,
/// STARTTLS, AUTH, MAIL / RCPT / DATA, QUIT), with the server's certificates checked between
/// the handshake and the sign-in: lettre takes no verifier of ours, and only so are a server or
/// intermediate certificate the system distrusts refused before the password goes out.
fn submit(server: &ServerConfig, sign_in: SignIn<'_>, envelope: &Envelope, raw: &[u8], recipients: &[String], trust: &Trust<'_>) -> Result<()> {
    let failed = |e: lettre::transport::smtp::Error| failure(e, recipients);
    let host = server.host.as_str();
    // Each of the server's addresses in turn, as lettre would try them, greeted with this
    // computer's own address on the way to it.
    let addrs: Vec<SocketAddr> = (host, server.port).to_socket_addrs().map_err(NetError::Io)?.take(MAX_ADDRESSES).collect();
    // A server may show a certificate the account trusts and no authority vouches for (one on
    // this computer, a bridge to a mail service; a company's own, issue #105): lettre's handshake
    // lets it by, and the account's check below decides, before any sign-in. Not a major mail
    // service's server, whose certificate an authority always vouches for. lettre greets the
    // server (EHLO, this computer's address) before that check: the greeting STARTTLS already
    // says in the clear, never the password or the message.
    let here = !addrs.is_empty() && addrs.iter().all(|a| crate::stream::on_this_computer(a.ip()));
    let local = here || !trust::public_service(host);
    let wrapper = match server.security {
        Security::Tls => Some(tls_parameters(host, trust.roots, local)?),
        Security::StartTls | Security::None => None,
    };
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
        conn.starttls(&tls_parameters(host, trust.roots, local)?, &hello).map_err(failed)?;
    }
    if server.security != Security::None {
        // After a handshake there's always a chain: lettre's call can't come back empty.
        let chain = conn.certificate_chain().map_err(failed)?;
        // The usual check, then what the account trusts; or it's said which certificate it was.
        let check = trust::Own::new(Arc::clone(&trust.verifier), trust.trusted, host, here);
        let verdict = check.check(&chain).map_err(|e| match check.refused() {
            Some(shown) => NetError::Untrusted(Box::new(shown)),
            None => NetError::Tls(e.to_string()),
        });
        if let Err(e) = verdict {
            conn.abort();
            return Err(e);
        }
    }
    if let Err(e) = sign_in_to(&mut conn, &hello, &sign_in, recipients) {
        conn.abort();
        return Err(e);
    }
    let sent = hand_over(&mut conn, envelope, raw, recipients);
    conn.abort();
    sent
}

/// Sign in as `sign_in` says, once the connection is secured (issue #123). An OAuth token goes
/// as XOAUTH2. With a password: `PLAIN` or `LOGIN` (lettre's), or NTLM, Outlook's Secure Password
/// Authentication; left to MailCraft, the password when the server takes it, NTLM when it takes
/// only that, and no sign-in when it offers none (a company's server that takes mail from where
/// it's sent). An empty user name or [`SignInMethod::None`] sends without signing in.
fn sign_in_to(conn: &mut SmtpConnection, hello: &ClientId, sign_in: &SignIn<'_>, recipients: &[String]) -> Result<()> {
    let failed = |e: lettre::transport::smtp::Error| failure(e, recipients);
    if sign_in.user.is_empty() || sign_in.method == SignInMethod::None {
        return Ok(());
    }
    let credentials = Credentials::new(sign_in.user.to_string(), sign_in.password.to_string());
    if sign_in.oauth {
        return conn.auth(&[Mechanism::Xoauth2], &credentials).map(drop).map_err(failed);
    }
    // What the connection's `EHLO` named that lettre speaks. Asked again only for what it doesn't
    // (NTLM): IONOS's servers refuse a second `EHLO` in a session ("503 Bad sequence of
    // commands", issue #165), and every message was refused. One refused counts as no more.
    let info = conn.server_info();
    let known: Vec<String> = [(Mechanism::Plain, "PLAIN"), (Mechanism::Login, "LOGIN"), (Mechanism::Xoauth2, "XOAUTH2")]
        .into_iter()
        .filter(|(m, _)| info.supports_auth_mechanism(*m))
        .map(|(_, name)| name.to_string())
        .collect();
    let password_known = known.iter().any(|m| m == "PLAIN" || m == "LOGIN");
    let offered = if password_known && sign_in.method != SignInMethod::Ntlm {
        known
    } else {
        match offered_sign_ins(conn, hello) {
            Ok(all) => all,
            Err(e) => {
                log::info!("the outgoing server refused EHLO asked again ({e}): what it named first is used");
                known
            }
        }
    };
    let has = |m: &str| offered.iter().any(|o| o == m);
    let password = has("PLAIN") || has("LOGIN");
    let names = || if offered.is_empty() { "none".to_string() } else { offered.join(", ") };
    match sign_in.method {
        SignInMethod::Automatic if offered.is_empty() => Ok(()),
        SignInMethod::Automatic | SignInMethod::Password if password => conn.auth(DEFAULT_MECHANISMS, &credentials).map(drop).map_err(failed),
        SignInMethod::Automatic | SignInMethod::Ntlm if has("NTLM") => ntlm(conn, sign_in, recipients),
        SignInMethod::Ntlm => {
            Err(NetError::Auth(format!("the outgoing server doesn't offer NTLM (Secure Password Authentication); it offers: {}", names())))
        }
        SignInMethod::Password if offered.is_empty() => Err(NetError::Auth(
            "the outgoing server offers no sign-in: if it takes mail without one, choose No sign-in in the account's Server settings".into(),
        )),
        _ => Err(NetError::Auth(format!("the outgoing server offers no sign-in MailCraft can use; it offers: {}", names()))),
    }
}

/// The sign-in methods the server names (`AUTH PLAIN LOGIN NTLM`, or the older `AUTH=`), upper
/// case: `EHLO` asked again, as lettre keeps only those it speaks (RFC 5321 lets it be asked any
/// time).
fn offered_sign_ins(conn: &mut SmtpConnection, hello: &ClientId) -> std::result::Result<Vec<String>, lettre::transport::smtp::Error> {
    let r = conn.command(lettre::transport::smtp::commands::Ehlo::new(hello.clone()))?;
    let mut out: Vec<String> = Vec::new();
    for line in r.message() {
        let line = line.trim();
        let Some(rest) = line.get(..5).filter(|p| p.eq_ignore_ascii_case("AUTH ") || p.eq_ignore_ascii_case("AUTH=")).and(line.get(5..)) else {
            continue;
        };
        for m in rest.split_whitespace().take(32) {
            let m = m.to_ascii_uppercase();
            if !out.contains(&m) {
                out.push(m);
            }
        }
    }
    Ok(out)
}

/// A line MailCraft writes itself (a step of a sign-in lettre doesn't speak).
struct Line(String);

impl std::fmt::Display for Line {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}\r\n", self.0)
    }
}

/// NTLM's messages through RFC 4954's `AUTH` exchange (as [MS-SMTPNTLM] has it): the server's
/// challenge answered, the password never sent.
fn ntlm(conn: &mut SmtpConnection, sign_in: &SignIn<'_>, recipients: &[String]) -> Result<()> {
    let failed = |e: lettre::transport::smtp::Error| failure(e, recipients);
    conn.command(Line("AUTH NTLM".into())).map_err(failed)?;
    let r = conn.command(Line(mailcraft_model::base64::encode(&crate::ntlm::negotiate()))).map_err(failed)?;
    let answer = r
        .first_word()
        .and_then(crate::ews::unb64)
        .ok_or_else(|| NetError::Protocol("bad NTLM challenge".into()))
        .and_then(|c| crate::ntlm::answer(&c, sign_in.user, sign_in.password));
    let answer = match answer {
        Ok(a) => a,
        Err(e) => {
            // Cancelled, as RFC 4954 cancels; the server's refusal of that is expected.
            let _ = conn.command(Line("*".into()));
            return Err(e);
        }
    };
    // Signed in only on 235: a server asking on (334) isn't, and is cancelled.
    let r = conn.command(Line(mailcraft_model::base64::encode(&answer))).map_err(failed)?;
    if r.has_code(235) {
        return Ok(());
    }
    let _ = conn.command(Line("*".into()));
    Err(NetError::Auth("the outgoing server didn't finish the NTLM sign-in".into()))
}

/// MAIL, RCPT, DATA and the message, as lettre's `SmtpConnection::send` goes through them, one
/// at a time so it's known how far it got. Up to DATA nothing is sent, and a failure can be
/// tried again. Once the message itself has gone, only the server's answer says what became of
/// it: with none (it stopped answering, the connection dropped), the message may be on its way,
/// and trying again by itself is how it arrives twice.
fn hand_over(conn: &mut SmtpConnection, envelope: &Envelope, raw: &[u8], recipients: &[String]) -> Result<()> {
    let failed = |e: lettre::transport::smtp::Error| failure(e, recipients);
    let lacks = |what: &str| NetError::Io(std::io::Error::other(format!("the server does not support {what}")));
    let mut options = Vec::new();
    let ascii = |a: &Address| AsRef::<str>::as_ref(a).is_ascii();
    if !envelope.from().is_none_or(ascii) || !envelope.to().iter().all(ascii) {
        if !conn.server_info().supports_feature(Extension::SmtpUtfEight) {
            return Err(lacks("addresses outside ASCII (SMTPUTF8)"));
        }
        options.push(MailParameter::SmtpUtfEight);
    }
    if !raw.is_ascii() {
        if !conn.server_info().supports_feature(Extension::EightBitMime) {
            return Err(lacks("text outside ASCII (8BITMIME)"));
        }
        options.push(MailParameter::Body(MailBodyParameter::EightBitMime));
    }
    // Turned away at MAIL, it's the sender the server refuses: said, with the address. "550 Invalid
    // Sender" alone hid an account's mistyped address (Outlook, with the right one, sent).
    conn.command(Mail::new(envelope.from().cloned(), options)).map_err(|e| match failed(e) {
        NetError::Smtp(msg) => match envelope.from() {
            Some(from) => NetError::Smtp(format!("the server refused the sender, {from}: {msg}")),
            None => NetError::Smtp(msg),
        },
        other => other,
    })?;
    for to in envelope.to() {
        conn.command(Rcpt::new(to.clone(), Vec::new())).map_err(failed)?;
    }
    conn.command(Data).map_err(failed)?;
    // The server may check the whole message before it answers (RFC 5321 §4.5.3.2.6: wait ten
    // minutes).
    conn.set_timeout(Some(SENT_TIMEOUT)).map_err(NetError::Io)?;
    match conn.message(raw) {
        Ok(_) => Ok(()),
        // An answer, whatever it says, is the server's word on the message.
        Err(e) if e.status().is_some() => Err(failed(e)),
        Err(e) => Err(NetError::Unconfirmed(e.to_string())),
    }
}

/// How long the server may take to answer.
const TIMEOUT: Duration = Duration::from_secs(60);

/// How long it may take over a message it's been sent.
const SENT_TIMEOUT: Duration = Duration::from_secs(600);

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
fn tls_parameters(host: &str, roots: &[CertificateDer<'static>], unchecked: bool) -> Result<TlsParameters> {
    // `unchecked`: lettre's own check is left out, for [`submit`]'s to decide alone.
    let mut p = TlsParameters::builder(host.to_string()).dangerous_accept_invalid_certs(unchecked);
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
        // Gmail's "534-5.7.9 Application-specific password required": said plainly (issue #164).
        if mailcraft_model::google_wants_app_password(msg) {
            log::info!("the outgoing server refused the sign-in: {msg}");
            return NetError::Auth(mailcraft_model::GOOGLE_APP_PASSWORD_NEEDED.into());
        }
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

    /// The same, on a given listener. A client that doesn't come, or goes quiet, within
    /// [`crate::http::tests::TEST_SERVER_WAIT`] fails the thread rather than hanging its joiner.
    pub(crate) fn serve_once_on(l: TcpListener) -> (u16, std::thread::JoinHandle<String>) {
        let port = l.local_addr().unwrap().port();
        let h = std::thread::spawn(move || {
            let wait = crate::http::tests::TEST_SERVER_WAIT;
            let s = crate::http::tests::accept_within(&l, wait).expect("no client came to the test SMTP server");
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
        // Gmail asking for an app password (issue #164): said plainly, not in Google's words.
        let e = class(
            534,
            "5.7.9 Application-specific password required. For more information, go to 5.7.9 https://support.google.com/mail/?p=InvalidSecondFactor",
        );
        assert!(matches!(&e, NetError::Auth(m) if m == mailcraft_model::GOOGLE_APP_PASSWORD_NEEDED), "{e:?}");
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
                let answer = script(&command);
                // "CLOSE": the connection drops there, nothing said.
                if answer == "CLOSE" {
                    break;
                }
                let _ = out.write_all(answer.as_bytes());
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

    /// A sender turned away is said with its address: a mistyped one shows where it's wrong
    /// ("550 Invalid Sender" alone, as a tester's server put it, said nothing of which).
    #[test]
    fn a_refused_sender_is_named() {
        let port = scripted(|command| match command.split(' ').next().unwrap_or_default() {
            "EHLO" => "250-scripted.test\r\n250 AUTH PLAIN LOGIN\r\n",
            "AUTH" => "235 2.7.0 welcome\r\n",
            "MAIL" => "550 Invalid Sender  - Please Contact with your mail service provider\r\n",
            "QUIT" => "221 2.0.0 closing\r\n",
            _ => "250 2.0.0 fine\r\n",
        });
        let server = ServerConfig { security: Security::None, port, host: "127.0.0.1".into() };
        let e = send(&server, "zoe@studio.example", "pw", "zoe@stuido.example", &["a@y.example".into()], b"Subject: s\r\n\r\nx\r\n").unwrap_err();
        let NetError::Smtp(said) = e else { panic!("{e:?}") };
        assert!(said.starts_with("the server refused the sender, zoe@stuido.example: ") && said.contains("Invalid Sender"), "{said}");
    }

    /// The server takes the whole message and the connection drops before it answers: the
    /// message may have gone, so it isn't one to send again unasked (it was, and arrived twice).
    /// Dropped before the message is handed over, nothing has gone, and it's tried again.
    #[test]
    fn no_answer_after_the_message_is_not_tried_again_unasked() {
        let message = b"Subject: gone\r\n\r\ntext\r\n";
        let to = ["a@y.example".to_string()];
        let port = scripted(|command| match command.split(' ').next().unwrap_or_default() {
            "EHLO" => "250-scripted.test\r\n250 AUTH PLAIN\r\n",
            "AUTH" => "235 2.7.0 welcome\r\n",
            "DATA" => "354 go on\r\n",
            "SUBJECT:" | "" | "TEXT" => "",
            "." => "CLOSE",
            _ => "250 2.0.0 fine\r\n",
        });
        let server = ServerConfig { security: Security::None, port, host: "127.0.0.1".into() };
        let e = send(&server, "z@x.example", "pw", "z@x.example", &to, message).unwrap_err();
        assert!(matches!(e, NetError::Unconfirmed(_)), "{e:?}");
        assert!(!e.is_retryable(), "{e}");
        let port = scripted(|command| match command.split(' ').next().unwrap_or_default() {
            "EHLO" => "250-scripted.test\r\n250 AUTH PLAIN\r\n",
            "AUTH" => "235 2.7.0 welcome\r\n",
            "DATA" => "CLOSE",
            _ => "250 2.0.0 fine\r\n",
        });
        let server = ServerConfig { security: Security::None, port, host: "127.0.0.1".into() };
        let e = send(&server, "z@x.example", "pw", "z@x.example", &to, message).unwrap_err();
        assert!(matches!(e, NetError::Io(_)) && e.is_retryable(), "{e:?}");
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
        assert!(tls_parameters("mail.corp.test", &usable_roots(store.clone(), &Distrusted::new()), false).is_ok());
        assert!(tls_parameters("mail.corp.test", &[], false).is_ok());
        // Why the store is sorted first: lettre refuses the lot.
        assert!(tls_parameters("mail.corp.test", &store, false).is_err());
    }
}
