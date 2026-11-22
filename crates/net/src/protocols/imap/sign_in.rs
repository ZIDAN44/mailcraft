//! Signing in: a password (`AUTHENTICATE PLAIN` or `LOGIN`), NTLM (Outlook's Secure Password
//! Authentication, issue #123), or an OAuth token (XOAUTH2); and the extensions turned on after.

use super::*;

/// A sign-in the server refused (`NO`, `BAD`) as a refused sign-in, in the server's words, or in
/// `unsaid` when it gave none.
pub(super) fn auth_error(e: NetError, unsaid: &str) -> NetError {
    match e {
        NetError::No(m) | NetError::Bad(m) => NetError::Auth(if m.is_empty() { unsaid.into() } else { m }),
        other => other,
    }
}

/// An OAuth sign-in refused without a word.
pub(super) const TOKEN_REFUSED: &str = "the sign-in was refused";

/// An NTLM sign-in refused without a word.
const NTLM_REFUSED: &str = "the server rejected the user name or password (NTLM)";

/// The most times a server may ask during an NTLM sign-in: its challenge, then each time after,
/// cancelled; one that asks more is left.
const MAX_NTLM_STEPS: u8 = 4;

impl<S: Read + Write> Client<S> {
    /// Sign in with the password as `method` says (issue #123): NTLM when asked for, or, left to
    /// MailCraft, when the server offers it and not the password (`AUTH=PLAIN`; Cyrus with NTLM
    /// alone names no `LOGINDISABLED`, and refuses `LOGIN`); else [`Self::login`].
    pub fn login_as(&mut self, user: &str, pass: &str, method: mailcraft_model::SignInMethod) -> Result<()> {
        use mailcraft_model::SignInMethod;
        match method {
            SignInMethod::Ntlm => self.login_ntlm(user, pass),
            SignInMethod::Automatic if self.has("AUTH=NTLM") && !self.has("AUTH=PLAIN") => self.login_ntlm(user, pass),
            _ => self.login(user, pass),
        }
    }

    /// The sign-in methods the server names (`AUTH=…`), for saying what it offers.
    pub(super) fn offered_sign_ins(&self) -> String {
        let mut names: Vec<&str> = self.caps.iter().filter_map(|c| c.strip_prefix("AUTH=")).collect();
        // The `LOGIN` command, when it's allowed, and `AUTH=LOGIN` are the same to the user.
        if !self.has("LOGINDISABLED") {
            names.push("LOGIN");
        }
        names.sort_unstable();
        names.dedup();
        if names.is_empty() { "none".into() } else { names.join(", ") }
    }

    /// Sign in with NTLM (Outlook's Secure Password Authentication, [MS-OXIMAP4]'s exchange over
    /// RFC 3501's `AUTHENTICATE`): the server's challenge answered, the password never sent.
    fn login_ntlm(&mut self, user: &str, pass: &str) -> Result<()> {
        if !self.has("AUTH=NTLM") {
            return Err(NetError::Auth(format!(
                "the server doesn't offer NTLM (Secure Password Authentication); it offers: {}",
                self.offered_sign_ins()
            )));
        }
        let tag = self.conn.next_tag();
        let mut collected = Vec::new();
        self.conn.send(format!("{tag} AUTHENTICATE NTLM\r\n").as_bytes())?;
        self.conn.wait_continue(&tag, &mut collected).map_err(|e| auth_error(e, NTLM_REFUSED))?;
        self.conn.send(format!("{}\r\n", mailcraft_model::base64::encode(&crate::ntlm::negotiate())).as_bytes())?;
        // Each challenge answered: the server's (the first), then any more cancelled (`*`), which
        // a server that's done with NTLM never asks; one asking on and on is given up on.
        let mut answered = false;
        let mut asked = 0u8;
        loop {
            let read = self.conn.read()?;
            if matches!(read, Ok(Response::Continue(_))) {
                asked = asked.saturating_add(1);
                if asked > MAX_NTLM_STEPS {
                    return Err(NetError::Protocol("the server went on asking during the NTLM sign-in".into()));
                }
            }
            match read {
                Ok(Response::Continue(o)) if !answered => {
                    answered = true;
                    let answer = crate::ews::unb64(info(&o).trim())
                        .ok_or_else(|| NetError::Protocol("bad NTLM challenge".into()))
                        .and_then(|c| crate::ntlm::answer(&c, user, pass, None));
                    match answer {
                        Ok(a) => self.conn.send(format!("{}\r\n", mailcraft_model::base64::encode(&a)).as_bytes())?,
                        // A challenge that can't be read: the exchange is cancelled, and the
                        // server's answer to that read below.
                        Err(e) => {
                            self.conn.send(b"*\r\n")?;
                            log::warn!("NTLM sign-in: {e}");
                        }
                    }
                }
                Ok(Response::Continue(_)) => self.conn.send(b"*\r\n")?,
                Ok(Response::Done { tag: t, status, outcome }) if t.0 == tag => {
                    if status != Status::Ok {
                        let said = info(&outcome);
                        return Err(NetError::Auth(if said.is_empty() { NTLM_REFUSED.into() } else { said }));
                    }
                    collected.push(Response::Done { tag: t, status, outcome });
                    break;
                }
                Ok(Response::Data { status: Status::Bye, outcome }) => return Err(NetError::Closed(info(&outcome))),
                Ok(other) => collected.push(other),
                Err(raw) => check_unknown(&tag, &raw).map_err(|e| auth_error(e, NTLM_REFUSED))?,
            }
        }
        self.after_login(&collected)
    }

    /// Sign in: `AUTHENTICATE PLAIN` with SASL-IR when offered, else `LOGIN`.
    pub fn login(&mut self, user: &str, pass: &str) -> Result<()> {
        let res = if self.has("AUTH=PLAIN") && self.has("SASL-IR") {
            let token = mailcraft_model::base64::encode(format!("\0{user}\0{pass}").as_bytes());
            self.cmd(&[Part::Text("AUTHENTICATE PLAIN "), Part::Text(&token)])
        } else if self.has("LOGINDISABLED") {
            return Err(NetError::Auth(format!(
                "the server does not take the password on this connection (use TLS); it offers: {}",
                self.offered_sign_ins()
            )));
        } else {
            let (u, p) = (quoted(user), quoted(pass));
            let mut parts = vec![Part::Text("LOGIN ")];
            match &u {
                Some(q) => parts.push(Part::Text(q)),
                None => parts.push(Part::Literal(user.as_bytes())),
            }
            parts.push(Part::Text(" "));
            match &p {
                Some(q) => parts.push(Part::Text(q)),
                None => parts.push(Part::Literal(pass.as_bytes())),
            }
            self.cmd(&parts)
        };
        let rs = res.map_err(|e| auth_error(e, "the server rejected the user name or password"))?;
        self.after_login(&rs)
    }

    /// Sign in with an OAuth access token (SASL XOAUTH2, as Gmail and Microsoft 365 take it).
    pub fn login_xoauth2(&mut self, user: &str, token: &str) -> Result<()> {
        let ir = crate::oauth::xoauth2(user, token);
        let tag = self.conn.next_tag();
        let mut collected = Vec::new();
        if self.has("SASL-IR") {
            self.conn.send(format!("{tag} AUTHENTICATE XOAUTH2 {ir}\r\n").as_bytes())?;
        } else {
            self.conn.send(format!("{tag} AUTHENTICATE XOAUTH2\r\n").as_bytes())?;
            self.conn.wait_continue(&tag, &mut collected).map_err(|e| auth_error(e, TOKEN_REFUSED))?;
            self.conn.send(format!("{ir}\r\n").as_bytes())?;
        }
        // A refused token comes back as a challenge carrying the error: answer it empty, then
        // the server says NO.
        loop {
            match self.conn.read()? {
                Ok(Response::Continue(_)) => self.conn.send(b"\r\n")?,
                Ok(Response::Done { tag: t, status, outcome }) if t.0 == tag => {
                    if status != Status::Ok {
                        return Err(NetError::Auth(if info(&outcome).is_empty() { "the sign-in token was refused".into() } else { info(&outcome) }));
                    }
                    collected.push(Response::Done { tag: t, status, outcome });
                    break;
                }
                Ok(Response::Data { status: Status::Bye, outcome }) => return Err(NetError::Closed(info(&outcome))),
                Ok(other) => collected.push(other),
                Err(raw) => check_unknown(&tag, &raw).map_err(|e| auth_error(e, TOKEN_REFUSED))?,
            }
        }
        self.after_login(&collected)
    }

    /// After signing in: the new capabilities, and the extensions sync uses.
    pub(super) fn after_login(&mut self, rs: &[Response<'static>]) -> Result<()> {
        // Capabilities often change after login; take them from the completion or ask.
        let mut got = false;
        for r in rs {
            if let Response::Done { outcome, .. } = r
                && let Some(ResponseCode::Capabilities(list)) = &outcome.code
            {
                self.caps = caps_from(list);
                got = true;
            }
        }
        if !got {
            self.capability()?;
        }
        if self.has("ENABLE") && self.has("QRESYNC") {
            // QRESYNC implies CONDSTORE. Only an ENABLED reply naming it turns it on.
            if let Ok(rs) = self.cmd(&[Part::Text("ENABLE QRESYNC")]) {
                self.qresync = rs.iter().any(|r| {
                    matches!(r, Response::Capabilities(list) if list.iter().any(|c| matches!(c, Capability::Atom(a) if a.eq_ignore_ascii_case("QRESYNC"))))
                });
            }
        } else if self.has("ENABLE") && self.has("CONDSTORE") {
            let _ = self.cmd(&[Part::Text("ENABLE CONDSTORE")]);
        }
        Ok(())
    }
}
