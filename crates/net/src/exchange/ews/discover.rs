//! Autodiscover (POX, MS-OXDSCLI): where a mailbox's EWS address is found.

use super::*;

/// Where Autodiscover is asked (POX, MS-OXDSCLI): the domain's `autodiscover.` host, then the
/// domain itself.
pub(super) fn autodiscover_urls(email: &str) -> Vec<String> {
    let domain = email.rsplit_once('@').map(|(_, d)| d.trim().to_ascii_lowercase()).unwrap_or_default();
    if domain.is_empty() || !domain.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-') {
        return Vec::new();
    }
    vec![format!("https://autodiscover.{domain}/autodiscover/autodiscover.xml"), format!("https://{domain}/autodiscover/autodiscover.xml")]
}

/// What Autodiscover is asked about an address (POX's request).
pub(super) fn autodiscover_request(address: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?><Autodiscover xmlns=\"http://schemas.microsoft.com/exchange/autodiscover/outlook/requestschema/2006\">\
         <Request><EMailAddress>{}</EMailAddress><AcceptableResponseSchema>http://schemas.microsoft.com/exchange/autodiscover/outlook/responseschema/2006a</AcceptableResponseSchema></Request></Autodiscover>",
        esc(address)
    )
}

/// Whether the address Autodiscover was asked at is this computer: the user's own server on it,
/// the only one whose answer may name a plain http address (on this computer too).
pub(super) fn asked_here(url: &str) -> bool {
    http::Url::parse(url).is_ok_and(|u| http::is_loopback(&u.host))
}

/// The EWS address in an Autodiscover answer (the outside one first), or the address to ask
/// about instead (a redirect). `here`: the answer is from a server on this computer
/// ([`asked_here`]); one from the network never names anything but https.
pub(super) fn autodiscover_answer(xml: &str, here: bool) -> (Option<String>, Option<String>) {
    let doc = parse(xml);
    let Some(account) = doc.find("Account") else { return (None, None) };
    if account.text_of("Action") == Some("redirectAddr") {
        return (None, account.text_of("RedirectAddr").map(|r| r.trim().to_string()));
    }
    let protocols: Vec<&El> = account.children("Protocol").collect();
    let url = ["EXPR", "EXCH", "WEB"]
        .iter()
        .find_map(|kind| protocols.iter().find(|p| p.text_of("Type") == Some(kind)).and_then(|p| p.text_of("EwsUrl")))
        .map(|u| u.trim().to_string())
        // https; plain http only to this computer, and only when a server on it says so.
        .filter(|u| http::Url::parse(u).is_ok_and(|p| p.https || (here && http::is_loopback(&p.host))));
    (url, None)
}

/// Find a mailbox's EWS address with Autodiscover, signed in as the account. `base`: where to
/// ask instead of the usual addresses (tests).
pub fn autodiscover(email: &str, user: &str, password: &str, base: Option<&str>) -> Result<String> {
    autodiscover_as(email, user, password, base, None)
}

/// [`autodiscover`], naming itself `agent` to the servers asked.
pub fn autodiscover_as(email: &str, user: &str, password: &str, base: Option<&str>, agent: Option<&str>) -> Result<String> {
    let mut address = email.to_string();
    for _ in 0..3 {
        let body = autodiscover_request(&address);
        let urls = base.map_or_else(|| autodiscover_urls(&address), |b| vec![b.to_string()]);
        let mut redirect = None;
        let mut last = NetError::No("Autodiscover found no Exchange server for this address".into());
        for url in urls {
            let c = match Client::new(&url, user, password, email) {
                Ok(c) => c.with_agent(agent),
                Err(e) => {
                    last = e;
                    continue;
                }
            };
            match c.post(body.clone().into_bytes()) {
                Ok(resp) if resp.ok() => match autodiscover_answer(&String::from_utf8_lossy(&resp.body), asked_here(&url)) {
                    (Some(url), _) => return Ok(url),
                    (None, Some(r)) => {
                        redirect = Some(r);
                        break;
                    }
                    _ => {}
                },
                Ok(resp) => last = NetError::Protocol(format!("Autodiscover answered HTTP {}", resp.status)),
                Err(NetError::Auth(m)) => return Err(NetError::Auth(m)),
                Err(e) => last = e,
            }
        }
        match redirect {
            Some(r) if r != address && r.contains('@') => address = r,
            _ => return Err(last),
        }
    }
    Err(NetError::No("Autodiscover sent MailCraft round in circles".into()))
}
