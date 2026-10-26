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
        let mut base = Request::post(&self.url, "text/xml; charset=utf-8", body, MAX_ANSWER).header("Accept", "text/xml");
        if let Some(agent) = &self.user_agent {
            base = base.header("User-Agent", agent);
        }
        if let Some(mailbox) = self.route.borrow().as_ref() {
            base = base.header("X-AnchorMailbox", mailbox).header("X-PublicFolderMailbox", mailbox);
        } else if let Some(mailbox) = self.anchor.borrow().as_ref() {
            base = base.header("X-AnchorMailbox", mailbox);
        }
        for attempt in 0..3 {
            let scheme = *self.scheme.borrow();
            let resp = match scheme {
                Scheme::Basic => {
                    let token = b64(format!("{}:{}", self.user, self.password).as_bytes());
                    self.send_on_conn(&base.clone().header("Authorization", &format!("Basic {token}")))?
                }
                Scheme::Ntlm(name) if !*self.signed_in.borrow() => {
                    let first = self.send_on_conn(&base.clone().header("Authorization", &format!("{name} {}", b64(&crate::ntlm::negotiate()))))?;
                    if first.status != 401 {
                        first
                    } else {
                        let token = Client::offered(&first)
                            .iter()
                            .find_map(|v| v.strip_prefix(name).map(|t| t.trim().to_string()))
                            .filter(|t| !t.is_empty())
                            .ok_or_else(|| NetError::Auth("the server didn't answer the sign-in".into()))?;
                        let challenge = crate::ntlm::challenge(&unb64(&token).ok_or_else(|| NetError::Protocol("bad NTLM challenge".into()))?)?;
                        let mut client_challenge = [0u8; 8];
                        ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut client_challenge)
                            .map_err(|_| NetError::Protocol("no randomness".into()))?;
                        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
                        let filetime = (now + 11_644_473_600) * 10_000_000;
                        let answer = crate::ntlm::authenticate(&challenge, &self.user, &self.password, filetime, client_challenge);
                        let resp = self.send_on_conn(&base.clone().header("Authorization", &format!("{name} {}", b64(&answer))))?;
                        if resp.status != 401 {
                            *self.signed_in.borrow_mut() = true;
                        }
                        resp
                    }
                }
                Scheme::Ntlm(_) | Scheme::Unknown => self.send_on_conn(&base)?,
            };
            if resp.status != 401 {
                return Ok(resp);
            }
            // Not signed in: learn how, once.
            let offered = Client::offered(&resp);
            let starts = |p: &str| offered.iter().any(|v| v.len() >= p.len() && v.get(..p.len()).is_some_and(|h| h.eq_ignore_ascii_case(p)));
            let next = if starts("NTLM") {
                Scheme::Ntlm("NTLM")
            } else if starts("Negotiate") {
                Scheme::Ntlm("Negotiate")
            } else if starts("Basic") {
                Scheme::Basic
            } else {
                return Err(NetError::Auth("the server offers no sign-in MailCraft knows".into()));
            };
            if scheme == next || attempt == 2 {
                return Err(NetError::Auth("the server refused the user name or password".into()));
            }
            *self.scheme.borrow_mut() = next;
            *self.signed_in.borrow_mut() = false;
        }
        Err(NetError::Auth("the server refused the user name or password".into()))
    }

    /// Run one operation; the answer's elements (a SOAP fault is an error). Something not found
    /// while the account has public folders is asked for again in the public folder mailboxes
    /// holding their content (Exchange 2013 and later answer only there).
    pub fn call(&self, body: &str) -> Result<El> {
        let doc = self.call_here(body)?;
        let missing = |doc: &El| messages(doc).iter().any(|m| matches!(m.text_of("ResponseCode"), Some("ErrorFolderNotFound" | "ErrorItemNotFound")));
        if self.route.borrow().is_some() || self.anchor.borrow().is_some() || !missing(&doc) {
            return Ok(doc);
        }
        for mailbox in self.shared_open.borrow().clone() {
            if let Ok(there) = self.anchored_to(&mailbox, || self.call_here(body))
                && !missing(&there)
            {
                return Ok(there);
            }
        }
        let guids = self.pf_replicas.borrow().clone();
        for guid in guids {
            let Ok(mailbox) = self.public_folder_mailbox(&guid) else { continue };
            if let Ok(there) = self.routed_to(&mailbox, || self.call_here(body))
                && !missing(&there)
            {
                return Ok(there);
            }
        }
        Ok(doc)
    }

    /// [`Client::call`] where the request is sent, without looking elsewhere.
    pub(super) fn call_here(&self, body: &str) -> Result<El> {
        let resp = self.post(envelope(body).into_bytes())?;
        let text = String::from_utf8_lossy(&resp.body);
        let doc = parse(&text);
        if let Some(fault) = doc.find("Fault") {
            let why = fault.find("faultstring").map(|f| f.text.clone()).unwrap_or_else(|| "the server refused the request".into());
            return Err(NetError::Protocol(why));
        }
        if !resp.ok() {
            return Err(NetError::Protocol(format!("Exchange answered HTTP {}", resp.status)));
        }
        Ok(doc)
    }

    /// Run an operation on one thing: its single response message, checked.
    pub(crate) fn call_one(&self, body: &str) -> Result<El> {
        let doc = self.call(body)?;
        let m = messages(&doc).first().map(|m| (*m).clone()).ok_or_else(|| NetError::Protocol("an empty answer".into()))?;
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
