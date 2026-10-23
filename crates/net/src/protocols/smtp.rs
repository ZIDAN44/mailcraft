//! Message submission (RFC 6409) through `lettre`: implicit TLS on 465 (preferred, RFC 8314),
//! STARTTLS on 587, or plain only for local test servers.

use std::time::Duration;

use lettre::address::{Address, Envelope};
use lettre::transport::smtp::authentication::Credentials;
use lettre::transport::smtp::client::{Tls, TlsParameters};
use lettre::{SmtpTransport, Transport};
use mailcraft_model::{Security, ServerConfig};

use crate::{NetError, Result};

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
    let tls = |host: &str| TlsParameters::new_rustls(host.to_string()).map_err(|e| NetError::Tls(e.to_string()));
    let mut b = SmtpTransport::builder_dangerous(server.host.as_str()).port(server.port).timeout(Some(Duration::from_secs(60)));
    b = match server.security {
        Security::Tls => b.tls(Tls::Wrapper(tls(&server.host)?)),
        Security::StartTls => b.tls(Tls::Required(tls(&server.host)?)),
        Security::None => b.tls(Tls::None),
    };
    if !user.is_empty() {
        b = b.credentials(Credentials::new(user.to_string(), password.to_string()));
        if oauth {
            b = b.authentication(vec![lettre::transport::smtp::authentication::Mechanism::Xoauth2]);
        }
    }
    let transport = b.build();
    transport.send_raw(&envelope, raw).map_err(|e| {
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
    })?;
    Ok(())
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
    use std::net::TcpListener;

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
}
