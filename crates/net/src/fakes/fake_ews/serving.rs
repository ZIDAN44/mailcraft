//! The fake server's connections: each request read, signed in (NTLM once a connection, or
//! Basic), and answered: Autodiscover, or one EWS operation in its SOAP envelope.

use std::io::{BufReader, Write};
use std::net::{TcpListener, TcpStream};

use super::*;
use crate::fakes::http;

/// What came to the server and went out: connections made, and each request, when it came (since
/// the first), what it asked for, and whether it was answered.
#[derive(Default)]
pub(super) struct Traffic {
    began: Option<std::time::Instant>,
    connections: usize,
    requests: Vec<(std::time::Duration, String, bool)>,
}

impl Data {
    /// Where the server's side stood (a test's wait that timed out says it): the connections, the
    /// requests answered and the one still being answered, and the last few, each with when it
    /// came. A request that came and was never answered is the server's to explain; none
    /// waiting, the client never sent the next.
    pub fn where_it_stopped(&self) -> String {
        let t = &self.traffic;
        let answered = t.requests.iter().filter(|(_, _, done)| *done).count();
        let last: Vec<String> = t
            .requests
            .iter()
            .rev()
            .take(8)
            .rev()
            .map(|(at, what, done)| format!("{} at {:.2} s{}", what, at.as_secs_f64(), if *done { "" } else { " (not answered)" }))
            .collect();
        format!("{} connections, {} requests, {answered} answered; the last: {}", t.connections, t.requests.len(), last.join(", "))
    }

    /// A request came: what it asks for (its SOAP operation, or what it is), and its number.
    fn came(&mut self, body: &[u8]) -> usize {
        let text = String::from_utf8_lossy(body);
        let what = match text.find("<m:") {
            Some(at) => text[at + 3..].split(|c: char| !c.is_ascii_alphanumeric()).next().unwrap_or("?").to_string(),
            None if text.contains("Autodiscover") => "Autodiscover".to_string(),
            None if body.is_empty() => "(no body)".to_string(),
            None => "?".to_string(),
        };
        let began = *self.traffic.began.get_or_insert_with(std::time::Instant::now);
        self.traffic.requests.push((began.elapsed(), what, false));
        self.traffic.requests.len() - 1
    }

    /// Request `n` answered.
    fn answered(&mut self, n: usize) {
        if let Some(r) = self.traffic.requests.get_mut(n) {
            r.2 = true;
        }
    }
}

/// An answer, in XML; the connection stays open.
fn respond(s: &mut impl Write, status: u16, headers: &[(&str, String)], body: &[u8]) {
    let mut all = vec![("Content-Type", "text/xml; charset=utf-8".to_string())];
    all.extend_from_slice(headers);
    http::respond(s, &format!("{status} X"), &all, body);
}

/// Start the server on an address of its own.
pub(super) fn serve_at(data: Data, at: &str) -> Server {
    let l = TcpListener::bind(at).unwrap();
    let url = format!("http://127.0.0.1:{}/EWS/Exchange.asmx", l.local_addr().unwrap().port());
    let mut data = data;
    data.url = url.clone();
    let data = Arc::new(Mutex::new(data));
    let shared = data.clone();
    std::thread::spawn(move || {
        for conn in l.incoming() {
            let Ok(s) = conn else { continue };
            let shared = shared.clone();
            if let Ok(mut d) = shared.lock() {
                d.traffic.connections += 1;
            }
            std::thread::spawn(move || serve_connection(s, &shared));
        }
    });
    Server { url, data }
}

/// A request: the headers the server heeds, and its body.
#[derive(Default)]
struct Request {
    auth: String,
    /// `X-PublicFolderMailbox`.
    routed: String,
    /// `X-AnchorMailbox`.
    anchor: String,
    agent: String,
    /// `X-PreferServerAffinity`, and the `Cookie` sent (issue #145).
    affinity: String,
    cookie: String,
    body: Vec<u8>,
}

/// The next request on a connection; `None` once it's closed.
fn read_request(reader: &mut BufReader<TcpStream>) -> Option<Request> {
    let r = http::read_request(reader)?;
    let header = |name: &str| r.header(name).unwrap_or_default().to_string();
    Some(Request {
        auth: header("authorization"),
        routed: header("x-publicfoldermailbox"),
        anchor: header("x-anchormailbox"),
        agent: header("user-agent"),
        affinity: header("x-preferserveraffinity"),
        cookie: header("cookie"),
        body: r.body,
    })
}

/// One connection's sign-in: NTLM's handshake, once (or Basic on every request, when set to).
#[derive(Default)]
struct SignIn {
    signed_in: bool,
    challenge: Option<crate::ntlm::Challenge>,
    /// A request was answered on it, signed in.
    answered: bool,
    /// Its sign-in was dropped ([`Data::expire_ntlm_once`]): the next handshake is one again.
    expired: bool,
}

impl SignIn {
    /// Whether the request may be answered; when not, its 401 (with a challenge) is sent.
    fn admits(&mut self, s: &mut impl Write, shared: &Arc<Mutex<Data>>, auth: &str) -> bool {
        let (basic, open, lower) = {
            let mut d = shared.lock().unwrap();
            // The sign-in dropped once, for the whole server (not again on each new connection).
            if !d.basic && !d.open && d.expire_ntlm_once && self.signed_in && self.answered {
                d.expire_ntlm_once = false;
                self.signed_in = false;
                self.challenge = None;
                self.expired = true;
            }
            (d.basic, d.open, d.lower_case_schemes)
        };
        let named = |scheme: &str| if lower { scheme.to_ascii_lowercase() } else { scheme.to_string() };
        if basic && !open {
            let expected = format!("Basic {}", mailcraft_model::base64::encode(format!("{DOMAIN}\\{USER}:{PASSWORD}").as_bytes()));
            if auth != expected {
                respond(s, 401, &[("WWW-Authenticate", "Basic realm=\"x\"".into())], b"");
                return false;
            }
        } else if !self.signed_in && !open {
            match auth.strip_prefix("NTLM ").and_then(unb64) {
                Some(m) if m.get(8) == Some(&1) => {
                    let c = crate::ntlm::make_challenge([9; 8], &[]);
                    self.challenge = crate::ntlm::challenge(&c).ok();
                    respond(s, 401, &[("WWW-Authenticate", format!("{} {}", named("NTLM"), mailcraft_model::base64::encode(&c)))], b"");
                    return false;
                }
                Some(m) if m.get(8) == Some(&3) => {
                    let ok = self.challenge.as_ref().is_some_and(|c| crate::ntlm::verify(c, &m, USER, DOMAIN, PASSWORD));
                    if !ok {
                        respond(s, 401, &[("WWW-Authenticate", "NTLM".into()), ("WWW-Authenticate", "Negotiate".into())], b"");
                        return false;
                    }
                    self.signed_in = true;
                    let mut d = shared.lock().unwrap();
                    d.handshakes += 1;
                    if std::mem::take(&mut self.expired) {
                        d.ntlm_reauthentications += 1;
                    }
                }
                _ => {
                    respond(s, 401, &[("WWW-Authenticate", named("Negotiate")), ("WWW-Authenticate", named("NTLM"))], b"");
                    return false;
                }
            }
        }
        self.answered = self.signed_in;
        true
    }
}

/// Requests on one kept-open connection: NTLM signs it in once.
fn serve_connection(mut s: TcpStream, shared: &Arc<Mutex<Data>>) {
    let mut reader = BufReader::new(s.try_clone().unwrap());
    let mut sign_in = SignIn::default();
    // The request in hand, answered by the time the loop comes round again (every way through
    // it answers), and marked so before the next is waited for.
    let mut in_hand: Option<usize> = None;
    loop {
        if let Some(n) = in_hand.take() {
            shared.lock().unwrap().answered(n);
        }
        let Some(r) = read_request(&mut reader) else { return };
        let only = {
            let mut d = shared.lock().unwrap();
            d.agents.push(r.agent.clone());
            in_hand = Some(d.came(&r.body));
            d.only_agent.clone()
        };
        if only.is_some_and(|o| o != r.agent) {
            respond(&mut s, 403, &[], b"Forbidden");
            continue;
        }
        if !sign_in.admits(&mut s, shared, &r.auth) {
            continue;
        }
        let doc = parse(&String::from_utf8_lossy(&r.body));
        let name = doc.find("Body").and_then(|b| b.kids.first()).map(|k| k.name.clone()).unwrap_or_default();
        let affine = r.affinity.eq_ignore_ascii_case("true");
        if shared.lock().unwrap().lands_elsewhere(&name, affine, &r.cookie) {
            let answer = envelope(&name, &[super::err("ErrorSubscriptionNotFound")]);
            respond(&mut s, 200, &[], answer.as_bytes());
            continue;
        }
        let (status, answer) = if doc.find("Autodiscover").is_some_and(|a| a.find("EMailAddress").is_some()) {
            (200, autodiscover(shared, &doc))
        } else {
            operation(shared, &doc, r.routed, r.anchor)
        };
        let balanced = shared.lock().unwrap().balanced;
        let cookie = [("Set-Cookie", format!("{}; path=/EWS", super::watching::BACKEND))];
        let headers: &[(&str, String)] = if balanced && affine && name == "Subscribe" { &cookie } else { &[] };
        respond(&mut s, status, headers, answer.as_bytes());
    }
}

/// Autodiscover: this server's own address; for the public folder mailbox (asked for by its
/// GUID), its address.
fn autodiscover(shared: &Arc<Mutex<Data>>, doc: &El) -> String {
    let (url, pf, hierarchy) = {
        let d = shared.lock().unwrap();
        (d.url.clone(), d.pf_mailbox.clone(), d.pf_hierarchy.clone())
    };
    // The public folder mailbox, asked for by its GUID: its address.
    let asked = doc.find("EMailAddress").map(|e| e.text.clone()).unwrap_or_default();
    if let Some(pf) = pf.filter(|_| asked.starts_with(PF_GUID)) {
        return format!(
            "<?xml version=\"1.0\"?><Autodiscover><Response><User><DisplayName>Public folders</DisplayName>\
             <AutoDiscoverSMTPAddress>{pf}</AutoDiscoverSMTPAddress></User><Account><AccountType>email</AccountType><Action>settings</Action>\
             <Protocol><Type>EXCH</Type><EwsUrl>{url}</EwsUrl></Protocol></Account></Response></Autodiscover>"
        );
    }
    // The account's: its server, and its public folder hierarchy mailbox.
    let public = hierarchy.map(|h| format!("<PublicFolderInformation><SmtpAddress>{h}</SmtpAddress></PublicFolderInformation>")).unwrap_or_default();
    // Mailboxes mapped to the account (full access, automapped), as Outlook is told.
    let mapped: String = shared
        .lock()
        .unwrap()
        .shared
        .iter()
        .filter(|b| b.automap)
        .map(|b| {
            let id = if b.legacy {
                format!("<LegacyDN>{}</LegacyDN><Server>mbx.contoso.example</Server>", legacy_dn(&b.address))
            } else {
                format!("<SmtpAddress>{}</SmtpAddress><OwnerSmtpAddress>{}</OwnerSmtpAddress>", b.address, b.address)
            };
            format!("<AlternativeMailbox><Type>Delegate</Type><DisplayName>{}</DisplayName>{id}</AlternativeMailbox>", esc(&b.name))
        })
        .collect();
    let public = format!(
        "{public}{mapped}<AlternativeMailbox><Type>Archive</Type><DisplayName>Online Archive</DisplayName><SmtpAddress>archive@contoso.example</SmtpAddress></AlternativeMailbox>"
    );
    format!(
        "<?xml version=\"1.0\"?><Autodiscover><Response><User><DisplayName>Riley</DisplayName></User><Account><AccountType>email</AccountType><Action>settings</Action>\
         <Protocol><Type>EXCH</Type><EwsUrl>{url}</EwsUrl></Protocol>{public}</Account></Response></Autodiscover>"
    )
}

/// One EWS operation, in its SOAP envelope: (status, the answer). A slow server holds a mail
/// folder's listing; an operation the version asked for doesn't have is a fault.
fn operation(shared: &Arc<Mutex<Data>>, doc: &El, routed: String, anchor: String) -> (u16, String) {
    let Some(op) = doc.find("Body").and_then(|b| b.kids.first()).cloned() else {
        return (
            500,
            "<soap:Envelope><soap:Body><soap:Fault><faultstring>no operation</faultstring></soap:Fault></soap:Body></soap:Envelope>".into(),
        );
    };
    // As Exchange: a time zone whose two changes fall on the same day is refused whole (issue #140).
    if op.name == "GetUserAvailabilityRequest" && super::availability::zone_refused(&op) {
        let fault = "<soap:Envelope><soap:Body><soap:Fault><faultcode>a:ErrorTimeZone</faultcode>\
                     <faultstring>The specified time zone isn't valid.</faultstring></soap:Fault></soap:Body></soap:Envelope>";
        return (500, fault.into());
    }
    // A slow server: listing a mail folder waits while the test holds it (without holding up
    // the others); a minute at most, so a test that never lets go fails rather than hangs.
    let hold = {
        let d = shared.lock().unwrap();
        d.hold && op.name == "FindItem" && !d.pim_folder(op.child("ParentFolderIds").and_then(|p| p.kids.first()))
    };
    if hold {
        shared.lock().unwrap().held += 1;
        let end = std::time::Instant::now() + std::time::Duration::from_secs(60);
        while shared.lock().unwrap().hold && std::time::Instant::now() < end {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        shared.lock().unwrap().held -= 1;
    }
    // A big calendar: listing it takes a while (the server not held meanwhile).
    let slow = {
        let d = shared.lock().unwrap();
        d.pim_slow.filter(|_| op.name == "FindItem" && d.pim_folder(op.child("ParentFolderIds").and_then(|p| p.kids.first())))
    };
    if let Some(wait) = slow {
        std::thread::sleep(wait);
    }
    let mut d = shared.lock().unwrap();
    d.calls.push(op.name.clone());
    let version = doc.find("RequestServerVersion").and_then(|v| v.attr("Version")).unwrap_or_default().to_string();
    if d.refuses_version(&op.name, &version) {
        drop(d);
        let fault = "<soap:Envelope><soap:Body><soap:Fault><faultcode>a:ErrorInvalidServerVersion</faultcode>\
                     <faultstring>The specified server version is invalid.</faultstring></soap:Fault></soap:Body></soap:Envelope>";
        return (500, fault.into());
    }
    // As Exchange: a field its schema doesn't know refuses the request whole (issue #191).
    if let Some(field) = super::schema::unknown_field(&op, &d.refused_fields) {
        if d.field_refusals.len() < 1000 {
            d.field_refusals.push(field);
        }
        drop(d);
        let fault = "<soap:Envelope><soap:Body><soap:Fault><faultcode>a:ErrorInvalidRequest</faultcode>\
                     <faultstring>The request is invalid.</faultstring></soap:Fault></soap:Body></soap:Envelope>";
        return (500, fault.into());
    }
    if let Some(id) = newer_id(&op).filter(|_| d.exchange_2010) {
        d.schema_refusals += 1;
        drop(d);
        let fault = format!(
            "<soap:Envelope><soap:Body><soap:Fault><faultcode>a:ErrorSchemaValidation</faultcode>\
             <faultstring>The request failed schema validation: The 'Id' attribute is invalid - The value '{id}' is invalid \
             according to its datatype 'http://schemas.microsoft.com/exchange/services/2006/types:DistinguishedFolderIdNameType' \
             - The Enumeration constraint failed.</faultstring></soap:Fault></soap:Body></soap:Envelope>"
        );
        return (500, fault);
    }
    d.routed = routed;
    if !anchor.is_empty() && d.anchors.len() < 1000 {
        d.anchors.push(anchor);
    }
    // A person's photo (issue #223): as Exchange, the answer is the response itself, and one
    // with none is HTTP 404 (its picture empty, as Gromox's).
    if op.name == "GetUserPhoto" {
        let who = op.text_of("Email").unwrap_or("").trim().to_lowercase();
        let photo = d.photos.iter().find(|(e, _)| e.eq_ignore_ascii_case(&who)).map(|(_, p)| mailcraft_model::base64::encode(p));
        drop(d);
        let (status, data) = match photo {
            Some(p) => (200, p),
            None => (404, String::new()),
        };
        return (
            status,
            format!(
                "<soap:Envelope><soap:Body><m:GetUserPhotoResponse ResponseClass=\"Success\"><m:ResponseCode>NoError</m:ResponseCode>\
                 <m:HasChanged>true</m:HasChanged><m:PictureData>{data}</m:PictureData></m:GetUserPhotoResponse></soap:Body></soap:Envelope>"
            ),
        );
    }
    let (name, parts) = d.route(&op);
    drop(d);
    (200, envelope(&name, &parts))
}

/// The first distinguished folder an operation names that Exchange 2010 doesn't know.
fn newer_id(op: &El) -> Option<String> {
    let mut ids = Vec::new();
    op.all("DistinguishedFolderId", &mut ids);
    ids.iter().filter_map(|f| f.attr("Id")).find(|id| ["archive", "conversationhistory", "syncissues"].contains(id)).map(str::to_string)
}

/// An operation's response element in its envelope: its response messages (each named for the
/// operation), or, for automatic replies and reminders, its parts as they are.
fn envelope(name: &str, parts: &[String]) -> String {
    // Answers whose result and response code are on the answer itself, not in response messages.
    let reminders = ["GetReminders", "PerformReminderAction", "GetInboxRules", "UpdateInboxRules"].iter().any(|n| name.starts_with(n));
    let class = if !reminders {
        String::new()
    } else if parts.concat().contains(">NoError<") {
        " ResponseClass=\"Success\"".to_string()
    } else {
        " ResponseClass=\"Error\"".to_string()
    };
    let inner = if name.starts_with("GetUserOof") || name.starts_with("SetUserOof") || name.starts_with("GetUserAvailability") || reminders {
        parts.concat()
    } else {
        let tag = name.trim_end_matches("Response").to_string() + "ResponseMessage";
        format!(
            "<m:ResponseMessages>{}</m:ResponseMessages>",
            parts.concat().replace("<m:R ", &format!("<m:{tag} ")).replace("</m:R>", &format!("</m:{tag}>"))
        )
    };
    format!(
        "<?xml version=\"1.0\"?><s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\"><s:Body><m:{name}{class} xmlns:m=\"m\" xmlns:t=\"t\">{inner}</m:{name}></s:Body></s:Envelope>"
    )
}
