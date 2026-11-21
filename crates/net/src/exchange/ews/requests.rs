//! EWS: SOAP requests on a kept connection, signed in with NTLM or Basic, and their answers.

use super::*;

impl Client {
    /// Connect and check the password (asks for the Inbox).
    pub fn connect(url: &str, user: &str, password: &str, email: &str) -> Result<Client> {
        Client::connect_as(url, user, password, email, None)
    }

    /// [`Client::connect`], naming itself `agent` to the server.
    pub fn connect_as(url: &str, user: &str, password: &str, email: &str, agent: Option<&str>) -> Result<Client> {
        let c = Client::new(url, user, password, email)?.with_agent(agent);
        c.call("<m:GetFolder><m:FolderShape><t:BaseShape>IdOnly</t:BaseShape></m:FolderShape><m:FolderIds><t:DistinguishedFolderId Id=\"inbox\"/></m:FolderIds></m:GetFolder>")?;
        Ok(c)
    }

    pub(super) fn send_on_conn(&self, req: &Request) -> Result<Response> {
        let mut slot = self.conn.borrow_mut();
        if slot.as_ref().is_none_or(|c| !c.usable()) {
            *slot = Some(Conn::open(&self.url)?);
            *self.signed_in.borrow_mut() = false;
        }
        let Some(conn) = slot.as_mut() else { return Err(NetError::Closed("no connection".into())) };
        let r = conn.send(req);
        if r.is_err() {
            *slot = None;
        }
        r
    }

    /// The `WWW-Authenticate` schemes of a 401.
    pub(super) fn offered(resp: &Response) -> Vec<String> {
        resp.headers.iter().filter(|(k, _)| k.eq_ignore_ascii_case("WWW-Authenticate")).map(|(_, v)| v.trim().to_string()).collect()
    }

    /// Send a request signed in the way the server wants.
    pub(crate) fn post(&self, body: Vec<u8>) -> Result<Response> {
        self.post_with(body, &[])
    }

    /// [`Client::post`] with these headers besides.
    pub(super) fn post_with(&self, body: Vec<u8>, extra: &[(&str, String)]) -> Result<Response> {
        let mut base = Request::post(&self.url, "text/xml; charset=utf-8", body, MAX_ANSWER).header("Accept", "text/xml");
        for (name, value) in extra {
            base = base.header(name, value);
        }
        if let Some(agent) = &self.user_agent {
            base = base.header("User-Agent", agent);
        }
        if let Some(mailbox) = self.route.borrow().as_ref() {
            base = base.header("X-AnchorMailbox", mailbox).header("X-PublicFolderMailbox", mailbox);
        } else if let Some(mailbox) = self.anchor.borrow().as_ref() {
            base = base.header("X-AnchorMailbox", mailbox);
        }
        for attempt in 0..4 {
            let scheme = *self.scheme.borrow();
            let was_signed_in = *self.signed_in.borrow();
            let resp = match scheme {
                Scheme::Kerberos if !was_signed_in => match self.kerberos(&base) {
                    Ok(resp) => resp,
                    // No Kerberos server to be found (or none answering): NTLM inside Negotiate,
                    // as before, which a server that also takes NTLM accepts.
                    Err(NetError::No(m) | NetError::Closed(m)) => {
                        log::info!("Kerberos: {m}; trying NTLM in Negotiate");
                        *self.scheme.borrow_mut() = Scheme::Ntlm("Negotiate");
                        continue;
                    }
                    Err(e) => return Err(e),
                },
                Scheme::Basic => {
                    let token = base64::encode(format!("{}:{}", self.user, self.password).as_bytes());
                    self.send_on_conn(&base.clone().header("Authorization", &format!("Basic {token}")))?
                }
                Scheme::Ntlm(name) if !*self.signed_in.borrow() => {
                    let first =
                        self.send_on_conn(&base.clone().header("Authorization", &format!("{name} {}", base64::encode(&crate::ntlm::negotiate()))))?;
                    if first.status != 401 {
                        first
                    } else {
                        let offered = Client::offered(&first);
                        let token = offered.iter().find_map(|v| scheme_token(v, name).filter(|t| !t.is_empty()));
                        let Some(token) = token else {
                            // NTLM named but not done (a front that takes only Kerberos lists it
                            // anyway): Kerberos, when Negotiate is offered too.
                            if name == "NTLM" && offered.iter().any(|v| scheme_token(v, "Negotiate").is_some()) {
                                *self.scheme.borrow_mut() = Scheme::Kerberos;
                                continue;
                            }
                            return Err(NetError::Auth("the server didn't answer the sign-in".into()));
                        };
                        let challenge = unb64(token).ok_or_else(|| NetError::Protocol("bad NTLM challenge".into()))?;
                        let answer = crate::ntlm::answer(&challenge, &self.user, &self.password)?;
                        let resp = self.send_on_conn(&base.clone().header("Authorization", &format!("{name} {}", base64::encode(&answer))))?;
                        if resp.status != 401 {
                            *self.signed_in.borrow_mut() = true;
                        }
                        resp
                    }
                }
                Scheme::Ntlm(_) | Scheme::Kerberos | Scheme::Unknown => self.send_on_conn(&base)?,
            };
            if resp.status != 401 {
                return Ok(resp);
            }
            // Not signed in: learn how, once.
            let offered = Client::offered(&resp);
            let starts = |p: &str| offered.iter().any(|v| scheme_token(v, p).is_some());
            let next = if starts("NTLM") {
                Scheme::Ntlm("NTLM")
            } else if starts("Negotiate") {
                // Negotiate alone: Kerberos first (a server that takes only Kerberos refuses
                // NTLM in it), NTLM in it when there's no Kerberos server to ask.
                if matches!(scheme, Scheme::Ntlm("Negotiate")) { scheme } else { Scheme::Kerberos }
            } else if starts("Basic") {
                Scheme::Basic
            } else {
                return Err(NetError::Auth("the server offers no sign-in MailCraft knows".into()));
            };
            // A server that asks again on a connection already signed in (its sign-in held per
            // request, not per connection), or on the new connection opened in place of one
            // the server closed (NTLM signs in a connection, and this one never was): signed
            // in again, once. Taken for a refused password, that was asked for anew.
            if scheme == next && was_signed_in && attempt < 3 {
                *self.signed_in.borrow_mut() = false;
                continue;
            }
            if scheme == next || attempt >= 3 {
                return Err(NetError::Auth("the server refused the user name or password".into()));
            }
            *self.scheme.borrow_mut() = next;
            *self.signed_in.borrow_mut() = false;
        }
        Err(NetError::Auth("the server refused the user name or password".into()))
    }

    /// Run one operation; the answer's elements (a SOAP fault is an error). It goes to the
    /// account's own mailbox, or where [`Client::in_folder`] sends it.
    pub fn call(&self, body: &str) -> Result<El> {
        self.call_raw(envelope(body))
    }

    /// An operation of Exchange 2013 and later, in the user's own mailbox: the answer's elements.
    pub(crate) fn call_2013(&self, body: &str) -> Result<El> {
        self.call_raw(envelope_as("Exchange2013", body))
    }

    fn call_raw(&self, envelope: String) -> Result<El> {
        self.call_raw_with(envelope, &[]).map(|(doc, _)| doc)
    }

    /// [`Client::call_raw`] with these headers besides: the answer's elements, and its headers.
    pub(super) fn call_raw_with(&self, envelope: String, extra: &[(&str, String)]) -> Result<(El, Vec<(String, String)>)> {
        let resp = self.post_with(envelope.into_bytes(), extra)?;
        let text = String::from_utf8_lossy(&resp.body);
        let doc = parse(&text);
        if let Some(fault) = doc.find("Fault") {
            let why = fault.find("faultstring").map(|f| f.text.clone()).unwrap_or_else(|| "the server refused the request".into());
            return Err(NetError::Protocol(why));
        }
        if !resp.ok() {
            return Err(NetError::Protocol(format!("Exchange answered HTTP {}", resp.status)));
        }
        Ok((doc, resp.headers))
    }

    /// Run an operation on one thing: its single response message, checked.
    pub(crate) fn call_one(&self, body: &str) -> Result<El> {
        let m = into_messages(self.call(body)?).into_iter().next().ok_or_else(|| NetError::Protocol("an empty answer".into()))?;
        check_message(&m)?;
        Ok(m)
    }

    /// Run an operation on many things: every response message checked.
    pub(crate) fn call_all(&self, body: &str) -> Result<()> {
        let doc = self.call(body)?;
        for m in messages(&doc) {
            check_message(m)?;
        }
        Ok(())
    }
}

/// What a `WWW-Authenticate` value gives for the sign-in scheme `scheme`: its token (`NTLM <token>`,
/// `""` when it names the scheme alone), the scheme's name taken in any case (RFC 9110 11.1) and
/// whole (`NTLMfoo` isn't NTLM); `None` for another scheme.
pub(super) fn scheme_token<'a>(value: &'a str, scheme: &str) -> Option<&'a str> {
    let v = value.trim();
    let end = v.find(|c: char| c.is_ascii_whitespace() || c == ',').unwrap_or(v.len());
    let (name, rest) = v.split_at(end);
    // A token goes up to the next challenge in the same value, if any.
    name.eq_ignore_ascii_case(scheme).then(|| rest.split(',').next().unwrap_or("").trim())
}

/// [`messages`], moved out of the answer rather than copied (a message's source can be most of
/// it).
pub(crate) fn into_messages(doc: El) -> Vec<El> {
    fn body(el: El) -> Option<El> {
        for k in el.kids {
            if k.name == "Body" {
                return Some(k);
            }
            if let Some(b) = body(k) {
                return Some(b);
            }
        }
        None
    }
    fn walk(el: El, out: &mut Vec<El>) {
        for k in el.kids {
            if k.name == "ResponseMessages" {
                out.extend(k.kids);
            } else {
                walk(k, out);
            }
        }
    }
    let mut out = Vec::new();
    if let Some(b) = body(doc) {
        walk(b, &mut out);
    }
    out
}

impl Client {
    /// Sign the connection in with Kerberos (Negotiate) and send `base` on it: the server's
    /// answer to the signed request. A token the server sends back (mutual authentication, or a
    /// step more) is taken; one it can't use doesn't undo the sign-in it accepted.
    fn kerberos(&self, base: &Request) -> Result<Response> {
        let host = http::Url::parse(&self.url)?.host;
        let (mut n, token) = crate::kerberos::Negotiation::start(&self.user, &self.email, &self.password, &host)?;
        let mut token = token;
        for _ in 0..3 {
            let resp = self.send_on_conn(&base.clone().header("Authorization", &format!("Negotiate {}", base64::encode(&token))))?;
            let back = Client::offered(&resp).iter().find_map(|v| scheme_token(v, "Negotiate").filter(|t| !t.is_empty())).and_then(unb64);
            if resp.status != 401 {
                if let Some(b) = back
                    && let Err(e) = n.step(&b)
                {
                    log::info!("Kerberos: the server's last token: {e}");
                }
                *self.signed_in.borrow_mut() = true;
                return Ok(resp);
            }
            // Another step, or refused (a 401 with nothing to go on).
            let Some(b) = back else { return Ok(resp) };
            let (next, done) = n.step(&b)?;
            if done && next.is_empty() {
                return Ok(resp);
            }
            token = next;
        }
        Err(NetError::Auth("Kerberos: the server didn't finish signing in".into()))
    }
}
