//! Signing in to an outgoing server as the account says (issue #123): the password, NTLM
//! (Outlook's Secure Password Authentication) or no sign-in; and what's said when the server
//! offers nothing MailCraft can use.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use super::*;

/// The challenge the test servers send.
const CHALLENGE: [u8; 8] = [1, 2, 3, 4, 5, 6, 7, 8];

/// A server that names `auth` on its `EHLO` (none when empty) and plays NTLM, PLAIN and the
/// message through; each line it was sent is kept.
fn server(auth: &'static str, ntlm_verdict: &'static str) -> (u16, Arc<Mutex<Vec<String>>>) {
    let socket = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = socket.local_addr().unwrap().port();
    let lines = Arc::new(Mutex::new(Vec::new()));
    let kept = Arc::clone(&lines);
    std::thread::spawn(move || {
        let Ok((sock, _)) = socket.accept() else { return };
        let mut out = sock.try_clone().unwrap();
        let _ = out.write_all(b"220 scripted.test ready\r\n");
        let mut in_data = false;
        for line in BufReader::new(sock).lines().map_while(std::result::Result::ok) {
            kept.lock().unwrap().push(line.clone());
            let answer = if in_data {
                in_data = line != ".";
                if in_data { String::new() } else { "250 2.0.0 queued\r\n".into() }
            } else if line.starts_with("EHLO") {
                if auth.is_empty() { "250-scripted.test\r\n250 8BITMIME\r\n".into() } else { format!("250-scripted.test\r\n250 AUTH {auth}\r\n") }
            } else if line == "AUTH NTLM" {
                "334 \r\n".into()
            } else if line.starts_with("TlRMTVNTUAAB") {
                format!("334 {}\r\n", mailcraft_model::base64::encode(&crate::ntlm::make_challenge(CHALLENGE, &[])))
            } else if line.starts_with("TlRMTVNTUAAD") {
                ntlm_verdict.into()
            } else if line.starts_with("AUTH") {
                "235 2.7.0 welcome\r\n".into()
            } else if line == "DATA" {
                in_data = true;
                "354 go on\r\n".into()
            } else if line == "QUIT" {
                let _ = out.write_all(b"221 2.0.0 bye\r\n");
                break;
            } else {
                "250 2.0.0 fine\r\n".into()
            };
            let _ = out.write_all(answer.as_bytes());
        }
    });
    (port, lines)
}

/// A message sent through the server on `port`, signing in as Corp's Riley as `method` says.
fn send_with(port: u16, method: SignInMethod) -> Result<()> {
    let server = ServerConfig { security: Security::None, port, host: "127.0.0.1".into() };
    let sign_in = SignIn { user: "CORP\\riley", password: "my pw!", oauth: false, method };
    send_trusting(&server, sign_in, "riley@corp.test", &["ana@corp.test".into()], b"Subject: s\r\n\r\nhi\r\n", Trusted::default())
}

fn sent(lines: &Arc<Mutex<Vec<String>>>) -> Vec<String> {
    lines.lock().unwrap().clone()
}

/// A server that takes NTLM alone (an Exchange server whose plain passwords are turned off) is
/// signed in to with it, left to MailCraft or chosen: the answer proves the password, which
/// never crosses. It was refused ("No compatible authentication mechanism was found").
#[test]
fn ntlm_is_used_when_its_all_the_server_takes() {
    for method in [SignInMethod::Automatic, SignInMethod::Ntlm] {
        let (port, lines) = server("NTLM GSSAPI", "235 2.7.0 welcome\r\n");
        send_with(port, method).unwrap();
        let lines = sent(&lines);
        assert!(lines.iter().any(|l| l == "AUTH NTLM"), "{method:?}: {lines:?}");
        let answer = lines.iter().find(|l| l.starts_with("TlRMTVNTUAAD")).expect("the NTLM answer");
        let challenge = crate::ntlm::challenge(&crate::ntlm::make_challenge(CHALLENGE, &[])).unwrap();
        let answer = crate::ews::unb64(answer).unwrap();
        assert!(crate::ntlm::verify(&challenge, &answer, "riley", "CORP", "my pw!"), "the answer proves the password");
        assert!(!lines.iter().any(|l| l.contains("my pw!")), "the password never crosses: {lines:?}");
        assert!(lines.iter().any(|l| l.starts_with("MAIL FROM")), "sent: {lines:?}");
    }
    // Refused: a refused sign-in, not a refused message.
    let (port, _) = server("NTLM", "535 5.7.8 authentication credentials invalid\r\n");
    assert!(matches!(send_with(port, SignInMethod::Ntlm), Err(NetError::Auth(_))));
    // Asked on after the answer (334, not 235): not signed in, cancelled, nothing sent.
    let (port, lines) = server("NTLM", "334 more\r\n");
    assert!(matches!(send_with(port, SignInMethod::Ntlm), Err(NetError::Auth(_))));
    let lines = sent(&lines);
    assert!(lines.iter().any(|l| l == "*") && !lines.iter().any(|l| l.starts_with("MAIL")), "{lines:?}");
}

/// Chosen, NTLM is used even where a password would be taken; the password, where both are.
#[test]
fn the_chosen_method_is_the_one_used() {
    let (port, lines) = server("PLAIN LOGIN NTLM", "235 2.7.0 welcome\r\n");
    send_with(port, SignInMethod::Ntlm).unwrap();
    let lines = sent(&lines);
    assert!(lines.iter().any(|l| l == "AUTH NTLM") && !lines.iter().any(|l| l.starts_with("AUTH PLAIN")), "{lines:?}");
    for method in [SignInMethod::Automatic, SignInMethod::Password] {
        let (port, lines) = server("PLAIN LOGIN NTLM", "235 2.7.0 welcome\r\n");
        send_with(port, method).unwrap();
        let lines = sent(&lines);
        assert!(lines.iter().any(|l| l.starts_with("AUTH PLAIN")) && !lines.iter().any(|l| l == "AUTH NTLM"), "{method:?}: {lines:?}");
    }
    // NTLM chosen and not offered: said, with what the server does offer.
    let (port, _) = server("PLAIN LOGIN", "");
    let e = send_with(port, SignInMethod::Ntlm).unwrap_err();
    assert!(matches!(&e, NetError::Auth(m) if m.contains("doesn't offer NTLM") && m.contains("PLAIN, LOGIN")), "{e:?}");
}

/// A server that offers no sign-in (a company's, taking mail from inside it) is sent to without
/// one, left to MailCraft or with No sign-in chosen; MailCraft signed in anyway, and couldn't
/// send at all. Chosen, the password is asked for and said not to be offered.
#[test]
fn a_server_that_offers_no_sign_in_is_sent_to_without_one() {
    for method in [SignInMethod::Automatic, SignInMethod::None] {
        let (port, lines) = server("", "");
        send_with(port, method).unwrap();
        let lines = sent(&lines);
        assert!(!lines.iter().any(|l| l.starts_with("AUTH")) && lines.iter().any(|l| l == "DATA"), "{method:?}: {lines:?}");
    }
    let (port, lines) = server("PLAIN", "");
    send_with(port, SignInMethod::None).unwrap();
    assert!(!sent(&lines).iter().any(|l| l.starts_with("AUTH")), "No sign-in chosen: none, though it's offered");
    let (port, _) = server("", "");
    let e = send_with(port, SignInMethod::Password).unwrap_err();
    assert!(matches!(&e, NetError::Auth(m) if m.contains("offers no sign-in") && m.contains("No sign-in")), "{e:?}");
}

/// Offering only methods MailCraft doesn't speak, the server is said to, naming them: it was
/// lettre's "No compatible authentication mechanism was found".
#[test]
fn methods_mailcraft_doesnt_speak_are_named() {
    let (port, lines) = server("CRAM-MD5 GSSAPI", "");
    let e = send_with(port, SignInMethod::Automatic).unwrap_err();
    assert!(matches!(&e, NetError::Auth(m) if m.contains("CRAM-MD5, GSSAPI")), "{e:?}");
    assert!(!sent(&lines).iter().any(|l| l.starts_with("MAIL")), "nothing sent");
}
