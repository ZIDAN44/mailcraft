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
    }
    let transport = b.build();
    transport.send_raw(&envelope, raw).map_err(|e| {
        let msg = e.to_string();
        if e.is_permanent() && (msg.contains("535") || msg.to_ascii_lowercase().contains("auth")) { NetError::Auth(msg) } else { NetError::Smtp(msg) }
    })?;
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;

    use super::*;

    /// A minimal SMTP server on localhost that accepts one message.
    pub(crate) fn serve_once() -> (u16, std::thread::JoinHandle<String>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
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

    #[test]
    fn rejects_bad_addresses_before_connecting() {
        let server = ServerConfig { host: "127.0.0.1".into(), port: 1, security: Security::None };
        assert!(matches!(send(&server, "", "", "not an address", &["a@b.c".into()], b""), Err(NetError::Smtp(_))));
        assert!(matches!(send(&server, "", "", "a@b.c", &[], b""), Err(NetError::Smtp(_))));
    }
}
