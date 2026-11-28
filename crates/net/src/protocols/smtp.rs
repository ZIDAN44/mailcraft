//! Message submission (RFC 6409): implicit TLS on 465 (preferred, RFC 8314), STARTTLS on 587
//! (RFC 3207), or plain only for local test servers. MailCraft's own client, on the streams IMAP,
//! POP and ManageSieve use ([`crate::stream`]): the server's certificates are checked in the
//! handshake, as theirs are, and its replies read as [`wire`] reads them.

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, UdpSocket};
use std::sync::Arc;
use std::time::Duration;

use mailcraft_model::{Security, ServerConfig, SignInMethod};

use crate::stream::trust::Distrusting;
use crate::stream::{Stream, Timeouts, Trusted};
use crate::{NetError, Result};

#[cfg(test)]
mod sign_in_tests;
#[cfg(test)]
mod tls_tests;
mod wire;
#[cfg(test)]
mod wire_tests;

use wire::{Conn, Offers, Reply};

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
    let envelope = Envelope::new(from, recipients)?;
    let trust = Trust { verifier: crate::stream::system_verifier().map_err(NetError::Tls)?, trusted };
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

/// Who the message is from and to, as `MAIL FROM` and `RCPT TO` name them.
struct Envelope {
    from: String,
    to: Vec<String>,
}

impl Envelope {
    /// The addresses checked before anything is sent: each one address, `local@domain`, with
    /// nothing in it that would end a command or its brackets.
    fn new(from: &str, recipients: &[String]) -> Result<Envelope> {
        let from = address(from).ok_or_else(|| NetError::Smtp(format!("sender {from}: not an email address")))?;
        let mut to = Vec::with_capacity(recipients.len());
        for r in recipients {
            to.push(address(r).ok_or_else(|| NetError::Smtp(format!("recipient {r}: not an email address")))?);
        }
        if to.is_empty() {
            return Err(NetError::Smtp("no recipients".into()));
        }
        Ok(Envelope { from, to })
    }

    /// Whether every address is ASCII: one that isn't needs the server's SMTPUTF8 (RFC 6531).
    fn ascii(&self) -> bool {
        self.from.is_ascii() && self.to.iter().all(|t| t.is_ascii())
    }
}

/// `text` as an address for the envelope, when it is one: `local@domain`, both parts there, no
/// spaces, controls, brackets or a second `@` outside the local part's quotes left unquoted.
fn address(text: &str) -> Option<String> {
    let a = text.trim();
    let (local, domain) = a.rsplit_once('@')?;
    let bad = |c: char| c.is_whitespace() || c.is_control() || matches!(c, '<' | '>');
    let fits = !local.is_empty() && !domain.is_empty() && local.len() <= 256 && domain.len() <= 255;
    (fits && !a.chars().any(bad) && !domain.contains(['@', '"', ','])).then(|| a.to_string())
}

/// What sending trusts: `verifier`, the check every connection's certificates get (the
/// system's), with what the account trusts besides ([`crate::stream::trust::Own`]).
struct Trust<'a> {
    verifier: Arc<Distrusting>,
    trusted: Trusted<'a>,
}

/// One message over one connection: connect (and secure it), the greeting, `EHLO`, STARTTLS,
/// the sign-in, then `MAIL` / `RCPT` / `DATA` and `QUIT`. The server's certificates are checked
/// in the handshake, so one the system distrusts, or the account doesn't, never hears a word.
fn submit(server: &ServerConfig, sign_in: SignIn<'_>, envelope: &Envelope, raw: &[u8], recipients: &[String], trust: &Trust<'_>) -> Result<()> {
    let host = server.host.as_str();
    let sock = crate::stream::tcp(host, server.port)?;
    // Greeted with this computer's own address on the way to the server (#50).
    let hello = sock.peer_addr().map(hello_for).unwrap_or_else(|_| "[127.0.0.1]".into());
    let secure = |sock| crate::stream::tls_checked(sock, host, Arc::clone(&trust.verifier), trust.trusted);
    let stream = match server.security {
        Security::Tls => secure(sock)?,
        Security::StartTls | Security::None => Stream::Plain(sock),
    };
    let mut conn = Conn::new(stream);
    let mut offers = greet(&mut conn, &hello, recipients)?;
    if server.security == Security::StartTls {
        if !offers.has("STARTTLS") {
            conn.quit();
            return Err(NetError::Tls("the outgoing server does not offer STARTTLS".into()));
        }
        expect(conn.command("STARTTLS")?, recipients)?;
        let Stream::Plain(sock) = conn.into_stream()? else {
            return Err(NetError::Tls("connection already encrypted".into()));
        };
        conn = Conn::new(secure(sock)?);
        // What the server offers is asked again over TLS (RFC 3207 §4.2): what it said in the
        // clear isn't to be relied on.
        offers = ehlo(&mut conn, &hello, recipients)?;
    }
    let sent = after_greeting(&mut conn, &offers, sign_in, envelope, raw, recipients);
    conn.quit();
    sent
}

/// MailCraft's side of a session over a connection already open (and secured, where it's to be):
/// the greeting, `EHLO` naming `hello`, the sign-in, the message and `QUIT`. The fuzz target's
/// way in, as a server's bytes reach the client.
pub fn session_over<S: Read + Write + Timeouts>(
    stream: S,
    hello: &str,
    sign_in: SignIn<'_>,
    from: &str,
    recipients: &[String],
    raw: &[u8],
) -> Result<()> {
    let envelope = Envelope::new(from, recipients)?;
    let mut conn = Conn::new(stream);
    let offers = greet(&mut conn, hello, recipients)?;
    let sent = after_greeting(&mut conn, &offers, sign_in, &envelope, raw, recipients);
    conn.quit();
    sent
}

/// The server's greeting, then `EHLO`: what it offers.
fn greet<S: Read + Write + Timeouts>(conn: &mut Conn<S>, hello: &str, recipients: &[String]) -> Result<Offers> {
    let greeting = conn.reply()?;
    if greeting.code != 220 {
        return Err(refused(&greeting, recipients));
    }
    ehlo(conn, hello, recipients)
}

fn ehlo<S: Read + Write + Timeouts>(conn: &mut Conn<S>, hello: &str, recipients: &[String]) -> Result<Offers> {
    let r = expect(conn.command(&format!("EHLO {hello}"))?, recipients)?;
    Ok(Offers::of(&r))
}

fn after_greeting<S: Read + Write + Timeouts>(
    conn: &mut Conn<S>,
    offers: &Offers,
    sign_in: SignIn<'_>,
    envelope: &Envelope,
    raw: &[u8],
    recipients: &[String],
) -> Result<()> {
    sign_in_to(conn, offers, &sign_in, recipients)?;
    hand_over(conn, offers, envelope, raw, recipients)
}

/// `r` when it's a success (2xx), else what it means ([`refused`]).
fn expect(r: Reply, recipients: &[String]) -> Result<Reply> {
    if r.done() { Ok(r) } else { Err(refused(&r, recipients)) }
}

/// Sign in as `sign_in` says, once the connection is secured (issue #123). An OAuth token goes
/// as XOAUTH2. With a password: `PLAIN` or `LOGIN`, or NTLM, Outlook's Secure Password
/// Authentication; left to MailCraft, the password when the server takes it, NTLM when it takes
/// only that, and no sign-in when it offers none (a company's server that takes mail from where
/// it's sent). An empty user name or [`SignInMethod::None`] sends without signing in. What the
/// server offers is what its one `EHLO` named: IONOS's servers refuse a second `EHLO` in a
/// session ("503 Bad sequence of commands", issue #165).
fn sign_in_to<S: Read + Write + Timeouts>(conn: &mut Conn<S>, offers: &Offers, sign_in: &SignIn<'_>, recipients: &[String]) -> Result<()> {
    if sign_in.user.is_empty() || sign_in.method == SignInMethod::None {
        return Ok(());
    }
    let offered = offers.sign_ins();
    let has = |m: &str| offered.iter().any(|o| o == m);
    if sign_in.oauth {
        if !has("XOAUTH2") {
            return Err(NetError::Auth(format!("the outgoing server doesn't offer XOAUTH2; it offers: {}", names(&offered))));
        }
        return xoauth2(conn, sign_in, recipients);
    }
    let password = has("PLAIN") || has("LOGIN");
    match sign_in.method {
        SignInMethod::Automatic if offered.is_empty() => Ok(()),
        SignInMethod::Automatic | SignInMethod::Password if password => password_sign_in(conn, sign_in, has("PLAIN"), recipients),
        SignInMethod::Automatic | SignInMethod::Ntlm if has("NTLM") => ntlm(conn, sign_in, recipients),
        SignInMethod::Ntlm => {
            Err(NetError::Auth(format!("the outgoing server doesn't offer NTLM (Secure Password Authentication); it offers: {}", names(&offered))))
        }
        SignInMethod::Password if offered.is_empty() => Err(NetError::Auth(
            "the outgoing server offers no sign-in: if it takes mail without one, choose No sign-in in the account's Server settings".into(),
        )),
        _ => Err(NetError::Auth(format!("the outgoing server offers no sign-in MailCraft can use; it offers: {}", names(&offered)))),
    }
}

/// The sign-in methods a server offers, for an error to name.
fn names(offered: &[String]) -> String {
    if offered.is_empty() { "none".to_string() } else { offered.join(", ") }
}

/// The password, as `PLAIN` (RFC 4616, sent at once) or, when the server doesn't take that,
/// `LOGIN` (the user name, then the password, each when asked).
fn password_sign_in<S: Read + Write + Timeouts>(conn: &mut Conn<S>, sign_in: &SignIn<'_>, plain: bool, recipients: &[String]) -> Result<()> {
    let b64 = |s: &str| mailcraft_model::base64::encode(s.as_bytes());
    let r = if plain {
        conn.command(&format!("AUTH PLAIN {}", b64(&format!("\0{}\0{}", sign_in.user, sign_in.password))))?
    } else {
        let r = conn.command("AUTH LOGIN")?;
        if r.code != 334 {
            return Err(refused(&r, recipients));
        }
        let r = conn.command(&b64(sign_in.user))?;
        if r.code != 334 {
            return Err(refused(&r, recipients));
        }
        conn.command(&b64(sign_in.password))?
    };
    signed_in(conn, r, recipients)
}

/// An OAuth access token, as Gmail and Microsoft 365 take it (SASL XOAUTH2).
fn xoauth2<S: Read + Write + Timeouts>(conn: &mut Conn<S>, sign_in: &SignIn<'_>, recipients: &[String]) -> Result<()> {
    let r = conn.command(&format!("AUTH XOAUTH2 {}", crate::oauth::xoauth2(sign_in.user, sign_in.password)))?;
    signed_in(conn, r, recipients)
}

/// Signed in on `235`. A server asking on (`334`: XOAUTH2's error, as JSON) is answered with an
/// empty line, as the method has it, for its refusal; anything else is the refusal.
fn signed_in<S: Read + Write + Timeouts>(conn: &mut Conn<S>, r: Reply, recipients: &[String]) -> Result<()> {
    match r.code {
        235 => Ok(()),
        334 => {
            let last = conn.command("")?;
            if last.code == 235 {
                return Ok(());
            }
            Err(match refused(&last, recipients) {
                NetError::Auth(m) => NetError::Auth(m),
                _ => NetError::Auth(last.said()),
            })
        }
        _ => Err(refused(&r, recipients)),
    }
}

/// NTLM's messages through RFC 4954's `AUTH` exchange (as [MS-SMTPNTLM] has it): the server's
/// challenge answered, the password never sent.
fn ntlm<S: Read + Write + Timeouts>(conn: &mut Conn<S>, sign_in: &SignIn<'_>, recipients: &[String]) -> Result<()> {
    let r = conn.command("AUTH NTLM")?;
    if r.code != 334 {
        return Err(refused(&r, recipients));
    }
    let r = conn.command(&mailcraft_model::base64::encode(&crate::ntlm::negotiate()))?;
    if r.code != 334 {
        return Err(refused(&r, recipients));
    }
    let answer = r
        .first_word()
        .and_then(crate::ews::unb64)
        .ok_or_else(|| NetError::Protocol("bad NTLM challenge".into()))
        .and_then(|c| crate::ntlm::answer(&c, sign_in.user, sign_in.password, None));
    let answer = match answer {
        Ok(a) => a,
        Err(e) => {
            // Cancelled, as RFC 4954 cancels; the server's refusal of that is expected.
            let _ = conn.command("*");
            return Err(e);
        }
    };
    // Signed in only on 235: a server asking on (334) isn't, and is cancelled.
    let r = conn.command(&mailcraft_model::base64::encode(&answer))?;
    match r.code {
        235 => Ok(()),
        334 => {
            let _ = conn.command("*");
            Err(NetError::Auth("the outgoing server didn't finish the NTLM sign-in".into()))
        }
        _ => Err(refused(&r, recipients)),
    }
}

/// MAIL, RCPT, DATA and the message, one at a time so it's known how far it got. Up to DATA
/// nothing is sent, and a failure can be tried again. Once the message itself has gone, only the
/// server's answer says what became of it: with none (it stopped answering, the connection
/// dropped), the message may be on its way, and trying again by itself is how it arrives twice.
fn hand_over<S: Read + Write + Timeouts>(conn: &mut Conn<S>, offers: &Offers, envelope: &Envelope, raw: &[u8], recipients: &[String]) -> Result<()> {
    let lacks = |what: &str| NetError::Io(std::io::Error::other(format!("the server does not support {what}")));
    let mut options = String::new();
    if !envelope.ascii() {
        if !offers.has("SMTPUTF8") {
            return Err(lacks("addresses outside ASCII (SMTPUTF8)"));
        }
        options.push_str(" SMTPUTF8");
    }
    if !raw.is_ascii() {
        if !offers.has("8BITMIME") {
            return Err(lacks("text outside ASCII (8BITMIME)"));
        }
        options.push_str(" BODY=8BITMIME");
    }
    // Turned away at MAIL, it's the sender the server refuses: said, with the address. "550 Invalid
    // Sender" alone hid an account's mistyped address (Outlook, with the right one, sent).
    let r = conn.command(&format!("MAIL FROM:<{}>{options}", envelope.from))?;
    expect(r, recipients).map_err(|e| match e {
        NetError::Smtp(msg) => NetError::Smtp(format!("the server refused the sender, {}: {msg}", envelope.from)),
        other => other,
    })?;
    for to in &envelope.to {
        expect(conn.command(&format!("RCPT TO:<{to}>"))?, recipients)?;
    }
    let r = conn.command("DATA")?;
    if r.code != 354 {
        return Err(refused(&r, recipients));
    }
    // The server may check the whole message before it answers (RFC 5321 §4.5.3.2.6: wait ten
    // minutes).
    conn.set_timeout(SENT_TIMEOUT)?;
    match conn.message(raw) {
        Ok(r) if r.done() => Ok(()),
        // An answer, whatever it says, is the server's word on the message.
        Ok(r) => Err(refused(&r, recipients)),
        Err(e) => Err(NetError::Unconfirmed(e.to_string())),
    }
}

/// How long it may take over a message it's been sent.
const SENT_TIMEOUT: Duration = Duration::from_secs(600);

/// What MailCraft calls itself in its EHLO to `server` (RFC 5321 §4.1.4): the address literal
/// of this computer's address on the way there, as Thunderbird sends, since a desktop has no name
/// the world knows. `[127.0.0.1]` for every server, as a library's default had it, and some
/// providers' spam filters dropped mail greeted so after taking it (#50). The address is the one
/// the system would send from: a socket "connected" without sending anything says it.
fn hello_for(server: SocketAddr) -> String {
    let any: SocketAddr = if server.is_ipv4() { (Ipv4Addr::UNSPECIFIED, 0).into() } else { (Ipv6Addr::UNSPECIFIED, 0).into() };
    let local = UdpSocket::bind(any).and_then(|s| {
        s.connect(server)?;
        s.local_addr()
    });
    match local.map(|a| a.ip()) {
        Ok(IpAddr::V4(ip)) if !ip.is_unspecified() => format!("[{ip}]"),
        Ok(IpAddr::V6(ip)) if !ip.is_unspecified() => ip.to_ipv4_mapped().map_or_else(|| format!("[IPv6:{ip}]"), |v4| format!("[{v4}]")),
        // No way to the server: the connection fails anyway.
        _ => "[127.0.0.1]".into(),
    }
}

/// What a reply that isn't the one hoped for means for the message: refused for good (5xx), to
/// try again later (4xx), or not SMTP's answer at all.
fn refused(r: &Reply, recipients: &[String]) -> NetError {
    match r.code / 100 {
        5 => refusal(r.code, &r.said(), recipients),
        4 => NetError::Later(r.said()),
        _ => NetError::Protocol(format!("the outgoing server answered: {}", r.said())),
    }
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
    /// network address for one reached through it (it was `[127.0.0.1]` for all).
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
        scripted_bytes(b"220 scripted.test ready\r\n", script)
    }

    /// [`scripted`], greeting with `greeting` (bytes: a server may write what isn't UTF-8).
    fn scripted_bytes(greeting: &'static [u8], script: fn(&str) -> &'static str) -> u16 {
        let socket = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = socket.local_addr().unwrap();
        let play = move || {
            let Ok((sock, _)) = socket.accept() else { return };
            let mut out = sock.try_clone().unwrap();
            let _ = out.write_all(greeting);
            for command in BufReader::new(sock).lines().map_while(std::result::Result::ok) {
                let command = command.to_ascii_uppercase();
                let answer = script(&command);
                // "CLOSE": the connection drops there, nothing said.
                if answer == "CLOSE" {
                    break;
                }
                // "LATIN-1:": the rest written as Latin-1, as a server set up in German may.
                let _ = match answer.strip_prefix("LATIN-1:") {
                    Some(text) => out.write_all(&text.chars().map(|c| u8::try_from(u32::from(c)).unwrap_or(b'?')).collect::<Vec<u8>>()),
                    None => out.write_all(answer.as_bytes()),
                };
                if command == "QUIT" {
                    break;
                }
            }
        };
        std::thread::spawn(play);
        address.port()
    }

    /// A server whose words aren't UTF-8 (Latin-1, as one set up in German may greet: "Grüß
    /// Gott") still takes the message (discussion #309): the whole send failed on the greeting
    /// with "response error: invalid utf-8 sequence", and failed again at each try. Its refusal
    /// in those words is still a refusal, said as the server put it.
    #[test]
    fn a_server_writing_latin_1_is_understood() {
        let port =
            scripted_bytes(b"220 mail.firma.example ESMTP Gr\xFC\xDF Gott\r\n", |command| match command.split(' ').next().unwrap_or_default() {
                "EHLO" => "LATIN-1:250-mail.firma.example schön, Sie zu sehen\r\n250 AUTH PLAIN\r\n",
                "AUTH" => "235 2.7.0 willkommen\r\n",
                "DATA" => "354 los\r\n",
                "QUIT" => "221 2.0.0 tschüss\r\n",
                _ => "250 2.0.0 gut\r\n",
            });
        let server = ServerConfig { security: Security::None, port, host: "127.0.0.1".into() };
        send(&server, "z@x.example", "pw", "z@x.example", &["a@y.example".into()], b"Subject: s\r\n\r\nx\r\n").unwrap();

        let port = scripted_bytes(b"220 mail.firma.example ESMTP\r\n", |command| match command.split(' ').next().unwrap_or_default() {
            "EHLO" => "250-mail.firma.example\r\n250 AUTH PLAIN\r\n",
            "AUTH" => "235 2.7.0 willkommen\r\n",
            "RCPT" => "LATIN-1:550 5.1.1 Empfänger unbekannt\r\n",
            "QUIT" => "221 2.0.0 tschüss\r\n",
            _ => "250 2.0.0 gut\r\n",
        });
        let server = ServerConfig { security: Security::None, port, host: "127.0.0.1".into() };
        let e = send(&server, "z@x.example", "pw", "z@x.example", &["a@y.example".into()], b"Subject: s\r\n\r\nx\r\n").unwrap_err();
        assert!(matches!(&e, NetError::Smtp(m) if m.contains("Empf") && m.contains("nger unbekannt")), "{e:?}");
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
}
