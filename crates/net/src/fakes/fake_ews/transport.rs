use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use super::{DOMAIN, Data, PASSWORD, PF_GUID, USER, legacy_dn};
use crate::ews::{esc, parse, unb64};

pub struct Server {
    pub url: String,
    pub data: Arc<Mutex<Data>>,
}

fn respond(s: &mut impl Write, status: u16, headers: &[(&str, String)], body: &[u8]) {
    let mut head = format!("HTTP/1.1 {status} X\r\nContent-Type: text/xml; charset=utf-8\r\nContent-Length: {}\r\n", body.len());
    for (k, v) in headers {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    let _ = s.write_all(head.as_bytes());
    let _ = s.write_all(body);
}

/// Start the server; it runs until the test process ends.
pub fn serve(data: Data) -> Server {
    serve_at(data, "127.0.0.1:0")
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
            std::thread::spawn(move || serve_connection(s, &shared));
        }
    });
    Server { url, data }
}

/// Requests on one kept-open connection: NTLM signs it in once.
fn serve_connection(mut s: std::net::TcpStream, shared: &Arc<Mutex<Data>>) {
    let mut reader = BufReader::new(s.try_clone().unwrap());
    let mut signed_in = false;
    let mut expired_on_connection = false;
    let mut authenticated_requests = 0usize;
    let mut challenge: Option<crate::ntlm::Challenge> = None;
    loop {
        let mut first = String::new();
        if reader.read_line(&mut first).unwrap_or(0) == 0 {
            return;
        }
        let (mut len, mut auth, mut routed, mut anchor, mut agent) = (0usize, String::new(), String::new(), String::new(), String::new());
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                break;
            }
            if let Some((k, v)) = line.split_once(':') {
                if k.eq_ignore_ascii_case("content-length") {
                    len = v.trim().parse().unwrap_or(0);
                }
                if k.eq_ignore_ascii_case("authorization") {
                    auth = v.trim().to_string();
                }
                if k.eq_ignore_ascii_case("x-publicfoldermailbox") {
                    routed = v.trim().to_string();
                }
                if k.eq_ignore_ascii_case("x-anchormailbox") {
                    anchor = v.trim().to_string();
                }
                if k.eq_ignore_ascii_case("user-agent") {
                    agent = v.trim().to_string();
                }
            }
        }
        let mut body = vec![0u8; len];
        let _ = reader.read_exact(&mut body);
        let only = {
            let mut d = shared.lock().unwrap();
            d.agents.push(agent.clone());
            d.only_agent.clone()
        };
        if only.is_some_and(|o| o != agent) {
            respond(&mut s, 403, &[], b"Forbidden");
            continue;
        }
        let (basic, open, expire_ntlm) = {
            let mut d = shared.lock().unwrap();
            // Consume the expiry globally, not once per TCP connection. Otherwise
            // a subsequent connection can trigger an unrelated second expiry.
            let expire = d.expire_ntlm_once && signed_in && authenticated_requests > 0;
            if expire {
                d.expire_ntlm_once = false;
            }
            (d.basic, d.open, expire)
        };
        if expire_ntlm {
            expired_on_connection = true;
            signed_in = false;
            challenge = None;
        }
        if basic && !open {
            let expected = format!("Basic {}", crate::imap::base64(format!("{DOMAIN}\\{USER}:{PASSWORD}").as_bytes()));
            if auth != expected {
                respond(&mut s, 401, &[("WWW-Authenticate", "Basic realm=\"x\"".into())], b"");
                continue;
            }
        } else if !signed_in && !open {
            match auth.strip_prefix("NTLM ").and_then(unb64) {
                Some(m) if m.get(8) == Some(&1) => {
                    let c = crate::ntlm::make_challenge([9; 8], &[]);
                    challenge = crate::ntlm::challenge(&c).ok();
                    respond(&mut s, 401, &[("WWW-Authenticate", format!("NTLM {}", crate::imap::base64(&c)))], b"");
                    continue;
                }
                Some(m) if m.get(8) == Some(&3) => {
                    let ok = challenge.as_ref().is_some_and(|c| crate::ntlm::verify(c, &m, USER, DOMAIN, PASSWORD));
                    if !ok {
                        respond(&mut s, 401, &[("WWW-Authenticate", "NTLM".into()), ("WWW-Authenticate", "Negotiate".into())], b"");
                        continue;
                    }
                    signed_in = true;
                    let mut d = shared.lock().unwrap();
                    d.handshakes += 1;
                    if expired_on_connection {
                        d.ntlm_reauthentications += 1;
                        expired_on_connection = false;
                    }
                }
                _ => {
                    respond(&mut s, 401, &[("WWW-Authenticate", "Negotiate".into()), ("WWW-Authenticate", "NTLM".into())], b"");
                    continue;
                }
            }
        }
        if signed_in && !basic && !open {
            authenticated_requests += 1;
        }
        let doc = parse(&String::from_utf8_lossy(&body));
        // Autodiscover: this server's own address.
        if doc.find("Autodiscover").is_some_and(|a| a.find("EMailAddress").is_some()) {
            let (url, pf, hierarchy) = {
                let d = shared.lock().unwrap();
                (d.url.clone(), d.pf_mailbox.clone(), d.pf_hierarchy.clone())
            };
            // The public folder mailbox, asked for by its GUID: its address.
            let asked = doc.find("EMailAddress").map(|e| e.text.clone()).unwrap_or_default();
            if let Some(pf) = pf.filter(|_| asked.starts_with(PF_GUID)) {
                let answer = format!(
                    "<?xml version=\"1.0\"?><Autodiscover><Response><User><DisplayName>Public folders</DisplayName>\
                     <AutoDiscoverSMTPAddress>{pf}</AutoDiscoverSMTPAddress></User><Account><AccountType>email</AccountType><Action>settings</Action>\
                     <Protocol><Type>EXCH</Type><EwsUrl>{url}</EwsUrl></Protocol></Account></Response></Autodiscover>"
                );
                respond(&mut s, 200, &[], answer.as_bytes());
                continue;
            }
            // The account's: its server, and its public folder hierarchy mailbox.
            let public =
                hierarchy.map(|h| format!("<PublicFolderInformation><SmtpAddress>{h}</SmtpAddress></PublicFolderInformation>")).unwrap_or_default();
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
            let answer = format!(
                "<?xml version=\"1.0\"?><Autodiscover><Response><User><DisplayName>Riley</DisplayName></User><Account><AccountType>email</AccountType><Action>settings</Action>\
                 <Protocol><Type>EXCH</Type><EwsUrl>{url}</EwsUrl></Protocol>{public}</Account></Response></Autodiscover>"
            );
            respond(&mut s, 200, &[], answer.as_bytes());
            continue;
        }
        let Some(op) = doc.find("Body").and_then(|b| b.kids.first()).cloned() else {
            respond(
                &mut s,
                500,
                &[],
                b"<soap:Envelope><soap:Body><soap:Fault><faultstring>no operation</faultstring></soap:Fault></soap:Body></soap:Envelope>",
            );
            continue;
        };
        // A slow server: listing a mail folder takes its time (without holding up the others).
        let hold = {
            let d = shared.lock().unwrap();
            d.hold.filter(|_| op.name == "FindItem" && !d.pim_folder(op.child("ParentFolderIds").and_then(|p| p.kids.first())))
        };
        if let Some(h) = hold {
            std::thread::sleep(h);
        }
        let mut d = shared.lock().unwrap();
        d.calls.push(op.name.clone());
        let version = doc.find("RequestServerVersion").and_then(|v| v.attr("Version")).unwrap_or_default().to_string();
        if d.refuses_version(&op.name, &version) {
            drop(d);
            let fault = "<soap:Envelope><soap:Body><soap:Fault><faultcode>a:ErrorInvalidServerVersion</faultcode>\
                         <faultstring>The specified server version is invalid.</faultstring></soap:Fault></soap:Body></soap:Envelope>";
            respond(&mut s, 500, &[], fault.as_bytes());
            continue;
        }
        d.routed = routed;
        if !anchor.is_empty() && d.anchors.len() < 1000 {
            d.anchors.push(anchor);
        }
        let (name, parts) = d.route(&op);
        drop(d);
        let reminders = name.starts_with("GetReminders") || name.starts_with("PerformReminderAction");
        let class = if !reminders {
            String::new()
        } else if parts.concat().contains(">NoError<") {
            " ResponseClass=\"Success\"".to_string()
        } else {
            " ResponseClass=\"Error\"".to_string()
        };
        let inner = if name.starts_with("GetUserOof") || name.starts_with("SetUserOof") || reminders {
            parts.concat()
        } else {
            let tag = name.trim_end_matches("Response").to_string() + "ResponseMessage";
            format!(
                "<m:ResponseMessages>{}</m:ResponseMessages>",
                parts.concat().replace("<m:R ", &format!("<m:{tag} ")).replace("</m:R>", &format!("</m:{tag}>"))
            )
        };
        let xml = format!(
            "<?xml version=\"1.0\"?><s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\"><s:Body><m:{name}{class} xmlns:m=\"m\" xmlns:t=\"t\">{inner}</m:{name}></s:Body></s:Envelope>"
        );
        respond(&mut s, 200, &[], xml.as_bytes());
    }
}
