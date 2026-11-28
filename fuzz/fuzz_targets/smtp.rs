//! An SMTP server's side of a submission: the input is everything the server sends, read by the
//! client through the greeting, `EHLO`, a sign-in (a password, NTLM, an OAuth token), `MAIL`,
//! `RCPT`, `DATA` and `QUIT`, as MailCraft sends a message. Each sign-in reads the same bytes.
#![no_main]

use std::io::{self, Read, Write};
use std::time::Duration;

use libfuzzer_sys::fuzz_target;
use mailcraft_model::SignInMethod;
use mailcraft_net::smtp::{self, SignIn};
use mailcraft_net::stream::Timeouts;

/// The server: its bytes read in small pieces (as a socket hands them over), writes dropped.
struct Server<'a> {
    data: &'a [u8],
}

impl Read for Server<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = buf.len().min(self.data.len()).min(97);
        let (now, rest) = self.data.split_at(n);
        buf[..n].copy_from_slice(now);
        self.data = rest;
        Ok(n)
    }
}

/// Whatever the client sends is accepted whole and forgotten.
impl Write for Server<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Timeouts for Server<'_> {
    fn set_read_timeout(&mut self, _: Option<Duration>) -> io::Result<()> {
        Ok(())
    }
}

fuzz_target!(|data: &[u8]| {
    let to = ["ana@studio.example".to_string(), "jürgen@studio.example".to_string()];
    let message = "Subject: Plan\r\n\r\n.a line starting with a dot\r\nGrüße\r\n".as_bytes();
    for (oauth, method) in [(false, SignInMethod::Automatic), (false, SignInMethod::Ntlm), (true, SignInMethod::Automatic)] {
        let sign_in = SignIn { user: "riley@studio.example", password: "pw", oauth, method };
        let _ = smtp::session_over(Server { data }, "[127.0.0.1]", sign_in, "riley@studio.example", &to, message);
    }
});
