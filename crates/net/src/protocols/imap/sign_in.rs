//! Signing in: a password (`AUTHENTICATE PLAIN` or `LOGIN`) or an OAuth token (XOAUTH2), and
//! the extensions turned on after.

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

impl<S: Read + Write> Client<S> {
    /// Sign in: `AUTHENTICATE PLAIN` with SASL-IR when offered, else `LOGIN`.
    pub fn login(&mut self, user: &str, pass: &str) -> Result<()> {
        let res = if self.has("AUTH=PLAIN") && self.has("SASL-IR") {
            let token = mailcraft_model::base64::encode(format!("\0{user}\0{pass}").as_bytes());
            self.cmd(&[Part::Text("AUTHENTICATE PLAIN "), Part::Text(&token)])
        } else if self.has("LOGINDISABLED") {
            return Err(NetError::Auth("the server does not allow sign-in on this connection (use TLS)".into()));
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
