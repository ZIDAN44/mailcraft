//! Finding an account's server settings from its address, the way mail clients have agreed to:
//!
//! 1. the domain's own configuration: `https://autoconfig.<domain>/mail/config-v1.1.xml` and
//!    `https://<domain>/.well-known/autoconfig/mail/config-v1.1.xml` (Mozilla's autoconfig
//!    format, the de-facto standard);
//! 2. the public ISP database (`https://autoconfig.thunderbird.net/v1.1/<domain>`);
//! 3. for the domain of its mail exchanger (MX), which covers domains hosted by Google,
//!    Microsoft, Spacemail and the like: the same database, then that provider's own
//!    configuration (`https://autoconfig.<provider>/mail/config-v1.1.xml`);
//! 4. the host an `_autodiscover._tcp` SRV record names (where Outlook looks), for the same
//!    configuration file;
//! 5. DNS SRV records (RFC 6186: `_imaps._tcp`, `_imap._tcp`, `_submissions._tcp`,
//!    `_submission._tcp`).
//!
//! The first answer wins; a POP server offered alongside is kept (`pop`) for POP accounts. Only
//! the domain's own server is told the address; the others are asked about the domain, and
//! only when the user types an address into the account dialog. Everything fetched is hostile input: sizes are capped and
//! a bad file is simply skipped.

use mailcraft_model::{AccountConfig, Security, ServerConfig};

/// Largest configuration file read.
const MAX_XML: usize = 256 << 10;

/// Settings found for an address, and where they came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub config: AccountConfig,
    pub source: String,
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

/// The text of the first `<tag>…</tag>` in `xml`.
fn text_of(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml.get(start..)?.find(&format!("</{tag}>"))? + start;
    Some(unescape(xml.get(start..end)?.trim()))
}

/// Every `<tag …>…</tag>` block whose start tag contains `attr` (`type="imap"`).
fn blocks<'a>(xml: &'a str, tag: &str, attr: &str) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut from = 0;
    let close = format!("</{tag}>");
    while let Some(i) = xml.get(from..).and_then(|r| r.find(&format!("<{tag}"))) {
        let start = from + i;
        let Some(head_end) = xml.get(start..).and_then(|r| r.find('>')).map(|e| start + e) else { break };
        let Some(end) = xml.get(head_end..).and_then(|r| r.find(&close)).map(|e| head_end + e) else { break };
        let head = xml.get(start..head_end).unwrap_or("").replace('\'', "\"");
        if head.contains(attr) {
            out.push(xml.get(head_end + 1..end).unwrap_or(""));
        }
        from = end + close.len();
    }
    out
}

/// Whether `host` contains only ASCII hostname characters (`a-z`, `0-9`, `.`, `-`).
fn valid_host(host: &str) -> bool {
    !host.is_empty() && host.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-'))
}

/// Parses one `<incomingServer>` or `<outgoingServer>` block into a `(ServerConfig, username)` pair.
fn server(block: &str, email: &str) -> Option<(ServerConfig, String)> {
    let host = text_of(block, "hostname")?;
    let port: u16 = text_of(block, "port")?.parse().ok()?;
    let security = match text_of(block, "socketType").unwrap_or_default().to_ascii_uppercase().as_str() {
        "SSL" | "TLS" => Security::Tls,
        "STARTTLS" => Security::StartTls,
        _ => Security::None,
    };
    let (local, domain) = email.rsplit_once('@').unwrap_or((email, ""));
    let user = text_of(block, "username").unwrap_or_else(|| "%EMAILADDRESS%".into());
    let user = user.replace("%EMAILADDRESS%", email).replace("%EMAILLOCALPART%", local).replace("%EMAILDOMAIN%", domain);
    let host = host.replace("%EMAILDOMAIN%", domain);
    let ok = port != 0 && valid_host(&host);
    ok.then_some((ServerConfig { host: host.to_ascii_lowercase(), port, security }, user))
}

/// Builds the `/.well-known/jmap` HTTPS discovery URL for `host` and `port`.
fn jmap_url_for(host: &str, port: u16) -> String {
    if port == 443 { format!("https://{host}/.well-known/jmap") } else { format!("https://{host}:{port}/.well-known/jmap") }
}

/// Extracts the first valid HTTPS `<incomingServer type="jmap">` endpoint and expanded username.
fn jmap_server(xml: &str, email: &str) -> Option<(String, String)> {
    let (local, domain) = email.rsplit_once('@').unwrap_or((email, ""));
    for b in blocks(xml, "incomingServer", "type=\"jmap\"") {
        let user = text_of(b, "username")
            .unwrap_or_else(|| "%EMAILADDRESS%".into())
            .replace("%EMAILADDRESS%", email)
            .replace("%EMAILLOCALPART%", local)
            .replace("%EMAILDOMAIN%", domain);
        if let Some(raw_url) = text_of(b, "url") {
            let expanded = raw_url.replace("%EMAILDOMAIN%", domain);
            if let Ok(u) = crate::http::Url::parse(&expanded)
                && u.https
                && valid_host(&u.host)
            {
                return Some((format!("{}{}", u.origin(), u.path), user));
            }
        }
        if let Some((s, u)) = server(b, email)
            && s.security == Security::Tls
        {
            return Some((jmap_url_for(&s.host, s.port), u));
        }
    }
    None
}

/// Parses an `<oAuth2>` autoconfig block when both authorization and token URLs use valid HTTPS hosts.
fn oauth_block(xml: &str) -> Option<mailcraft_model::OAuthConfig> {
    let start = xml.find("<oAuth2")?;
    let body_start = xml.get(start..)?.find('>')? + start + 1;
    let end = xml.get(body_start..)?.find("</oAuth2>")? + body_start;
    let b = xml.get(body_start..end)?;
    let auth_url = text_of(b, "authURL")?;
    let token_url = text_of(b, "tokenURL")?;
    let valid_https = |u: &str| crate::http::Url::parse(u).is_ok_and(|p| p.https && valid_host(&p.host));
    if !valid_https(&auth_url) || !valid_https(&token_url) {
        return None;
    }
    let issuer = text_of(b, "issuer").unwrap_or_default();
    let scope = text_of(b, "scope").unwrap_or_else(|| "mail caldav carddav offline_access".into());
    let client_id = text_of(b, "clientId").unwrap_or_default();
    Some(mailcraft_model::OAuthConfig { issuer, auth_url, token_url, scope, client_id })
}

/// The best IMAP, SMTP and JMAP servers of a Mozilla autoconfig file, and its POP server when it
/// has one (in `pop`). Encrypted servers are preferred; one without encryption is only taken when
/// nothing else is offered (and never for JMAP).
pub fn parse_config(xml: &str, email: &str) -> Option<AccountConfig> {
    let pick = |tag: &str, kind: &str| -> Option<(ServerConfig, String)> {
        let mut all: Vec<(ServerConfig, String)> =
            blocks(xml, tag, &format!("type=\"{kind}\"")).into_iter().filter_map(|b| server(b, email)).collect();
        all.sort_by_key(|(s, _)| match s.security {
            Security::Tls => 0,
            Security::StartTls => 1,
            Security::None => 2,
        });
        all.into_iter().next()
    };
    let jmap = jmap_server(xml, email);
    let oauth_config = oauth_block(xml);
    match (pick("incomingServer", "imap"), pick("outgoingServer", "smtp"), jmap) {
        (Some((imap, user)), Some((smtp, _)), jmap) => {
            let mut c = AccountConfig::guess(email);
            let guessed_jmap = c.jmap.take().filter(|_| imap.host == c.imap.host);
            let nextxch_xml = imap.security == Security::Tls
                && text_of(xml, "displayShortName").is_some_and(|s| s.eq_ignore_ascii_case("nextxch.com") || s.eq_ignore_ascii_case("nextxch"));
            c.jmap = jmap.map(|(u, _)| u).or_else(|| nextxch_xml.then(|| jmap_url_for(&imap.host, 443))).or(guessed_jmap);
            c.imap = imap;
            c.smtp = smtp;
            c.username = user;
            c.pop = pick("incomingServer", "pop3").map(|(s, _)| s);
            c.oauth_config = oauth_config;
            Some(c)
        }
        (_, _, Some((jmap_url, user))) => {
            let mut c = AccountConfig::guess(email);
            c.imap = ServerConfig::default();
            c.smtp = ServerConfig::default();
            c.username = user;
            c.jmap = Some(jmap_url);
            c.oauth_config = oauth_config;
            Some(c)
        }
        _ => None,
    }
}

/// The registrable part of a host name (`aspmx.l.google.com` → `google.com`; two-letter country
/// domains keep one more label: `mx.example.co.uk` → `example.co.uk`).
pub fn base_domain(host: &str) -> String {
    let labels: Vec<&str> = host.trim_end_matches('.').split('.').filter(|l| !l.is_empty()).collect();
    let n = labels.len();
    let keep = if n >= 3
        && labels.get(n - 1).is_some_and(|t| t.len() == 2)
        && labels.get(n - 2).is_some_and(|s| ["co", "com", "net", "org", "ac", "gov", "edu", "ne", "or"].contains(s))
    {
        3
    } else {
        2
    };
    labels[n.saturating_sub(keep)..].join(".")
}

/// The lookups, injectable for tests.
pub struct Lookups<'a> {
    /// GET a URL; the body when it answered 200.
    pub fetch: &'a mut dyn FnMut(&str) -> Option<String>,
    pub mx: &'a mut dyn FnMut(&str) -> Vec<String>,
    pub srv: &'a mut dyn FnMut(&str) -> Option<(String, u16)>,
}

/// Find settings for `email` (see the module docs for the order).
pub fn discover_with(email: &str, l: &mut Lookups<'_>) -> Option<Found> {
    discover_inner(email, l, &mut |_| false)
}

/// Whether an unauthenticated `GET /.well-known/jmap` response proves a JMAP endpoint (RFC 8620
/// section 2.2): either a 200 session resource naming `urn:ietf:params:jmap:core`, or a 401
/// challenge whose `WWW-Authenticate` header names JMAP.
fn is_jmap_session_endpoint(r: &crate::http::Response) -> bool {
    if r.status == 200 {
        r.header("content-type").is_some_and(|t| t.to_ascii_lowercase().contains("json"))
            && String::from_utf8_lossy(&r.body).contains("urn:ietf:params:jmap:core")
    } else if r.status == 401 {
        r.headers.iter().any(|(k, v)| k.eq_ignore_ascii_case("www-authenticate") && v.to_ascii_lowercase().contains("jmap"))
    } else {
        false
    }
}

/// Runs autoconfig XML, `/.well-known/jmap`, SRV, and MX discovery for `email`.
fn discover_inner(email: &str, l: &mut Lookups<'_>, probe_jmap: &mut dyn FnMut(&str) -> bool) -> Option<Found> {
    let email = email.trim();
    let domain = email.rsplit_once('@').map(|(_, d)| d.trim().to_ascii_lowercase()).filter(|d| d.contains('.'))?;
    if !domain.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-')) {
        return None;
    }
    let upgrade = |mut found: Found, probe: &mut dyn FnMut(&str) -> bool| -> Found {
        if found.config.jmap.is_none()
            && found.config.imap.security == Security::Tls
            && valid_host(&found.config.imap.host)
            && mailcraft_model::oauth_provider_for(&found.config.imap.host).is_none()
            && probe(&found.config.imap.host)
        {
            found.config.jmap = Some(jmap_url_for(&found.config.imap.host, 443));
        }
        found
    };
    let enc = crate::http::pct(email);
    let try_url = |l: &mut Lookups<'_>, url: &str| (l.fetch)(url).and_then(|x| parse_config(&x, email));
    for url in [
        format!("https://autoconfig.{domain}/mail/config-v1.1.xml?emailaddress={enc}"),
        format!("https://{domain}/.well-known/autoconfig/mail/config-v1.1.xml"),
    ] {
        if let Some(c) = try_url(l, &url) {
            return Some(upgrade(Found { config: c, source: domain.clone() }, probe_jmap));
        }
    }
    if let Some(c) = try_url(l, &format!("https://autoconfig.thunderbird.net/v1.1/{domain}")) {
        return Some(upgrade(Found { config: c, source: "the ISP database".into() }, probe_jmap));
    }
    let first_mx = (l.mx)(&domain).into_iter().next().map(|m| m.trim_end_matches('.').to_ascii_lowercase());
    if let Some(ref mx) = first_mx {
        let base = base_domain(mx);
        if base != domain {
            if let Some(c) = try_url(l, &format!("https://autoconfig.thunderbird.net/v1.1/{base}")) {
                return Some(upgrade(Found { config: c, source: format!("the ISP database ({base} handles this domain's mail)") }, probe_jmap));
            }
            // The mail provider's own configuration (asked about the domain, not the address).
            if let Some(c) = try_url(l, &format!("https://autoconfig.{base}/mail/config-v1.1.xml?emaildomain={domain}")) {
                return Some(upgrade(Found { config: c, source: base }, probe_jmap));
            }
        }
        if *mx != domain
            && valid_host(mx)
            && let Some(c) = try_url(l, &format!("https://{mx}/.well-known/autoconfig/mail/config-v1.1.xml?emaildomain={domain}"))
        {
            return Some(upgrade(Found { config: c, source: base }, probe_jmap));
        }
    }
    // Where Outlook looks: the host an `_autodiscover._tcp` record names, which providers point
    // at their configuration server.
    if let Some((host, _)) = (l.srv)(&format!("_autodiscover._tcp.{domain}"))
        && valid_host(&host)
        && let Some(c) = try_url(l, &format!("https://{host}/mail/config-v1.1.xml?emaildomain={domain}"))
    {
        return Some(upgrade(Found { config: c, source: base_domain(&host) }, probe_jmap));
    }
    // RFC 8620 §2.2 (`_jmap._tcp`) alongside RFC 6186 (`_imaps._tcp`, `_submissions._tcp`).
    let jmap =
        (l.srv)(&format!("_jmap._tcp.{domain}")).filter(|(h, p)| *p != 0 && valid_host(h)).map(|(h, p)| jmap_url_for(&h.to_ascii_lowercase(), p));
    let imap = (l.srv)(&format!("_imaps._tcp.{domain}"))
        .map(|(h, p)| ServerConfig { host: h, port: p, security: Security::Tls })
        .or_else(|| (l.srv)(&format!("_imap._tcp.{domain}")).map(|(h, p)| ServerConfig { host: h, port: p, security: Security::StartTls }));
    let smtp = (l.srv)(&format!("_submissions._tcp.{domain}"))
        .map(|(h, p)| ServerConfig { host: h, port: p, security: Security::Tls })
        .or_else(|| (l.srv)(&format!("_submission._tcp.{domain}")).map(|(h, p)| ServerConfig { host: h, port: p, security: Security::StartTls }));
    match (imap, smtp, jmap) {
        (Some(imap), Some(smtp), jmap) => {
            let mut c = AccountConfig::guess(email);
            c.imap = imap;
            c.smtp = smtp;
            if let Some(u) = jmap {
                c.jmap = Some(u);
            }
            return Some(upgrade(Found { config: c, source: "DNS (SRV records)".into() }, probe_jmap));
        }
        (_, _, Some(jmap_url)) => {
            let mut c = AccountConfig::guess(email);
            c.imap = ServerConfig::default();
            c.smtp = ServerConfig::default();
            c.jmap = Some(jmap_url);
            return Some(Found { config: c, source: "DNS (SRV records)".into() });
        }
        _ => {}
    }
    // RFC 8620 section 2.2 well-known JMAP check on the domain's mail exchanger or `mail.<domain>`
    // when no XML or SRV records are published at all.
    for candidate in [first_mx.filter(|m| valid_host(m)), Some(format!("mail.{domain}")).filter(|m| valid_host(m))].into_iter().flatten() {
        if mailcraft_model::oauth_provider_for(&candidate).is_none() && probe_jmap(&candidate) {
            let mut c = AccountConfig::guess(email);
            c.imap = ServerConfig::default();
            c.smtp = ServerConfig::default();
            c.jmap = Some(jmap_url_for(&candidate, 443));
            return Some(Found { config: c, source: base_domain(&candidate) });
        }
    }
    None
}

/// Find settings for `email` over the network.
pub fn discover(email: &str) -> Option<Found> {
    let mut fetch = |url: &str| {
        crate::http::get(url, MAX_XML)
            .ok()
            .filter(|r| r.ok() && r.header("content-type").is_none_or(|t| t.contains("xml") || t.contains("text")))
            .map(|r| String::from_utf8_lossy(&r.body).to_string())
    };
    let mut mx = crate::dns::mx;
    let mut srv = crate::dns::srv;
    let mut probe_jmap =
        |host: &str| valid_host(host) && crate::http::get(&jmap_url_for(host, 443), MAX_XML).is_ok_and(|r| is_jmap_session_endpoint(&r));
    discover_inner(email, &mut Lookups { fetch: &mut fetch, mx: &mut mx, srv: &mut srv }, &mut probe_jmap)
}

#[cfg(test)]
mod tests {
    use super::*;

    const XML: &str = r#"<?xml version="1.0"?>
<clientConfig version="1.1">
  <emailProvider id="example.org">
    <incomingServer type="pop3"><hostname>pop.example.org</hostname><port>995</port><socketType>SSL</socketType></incomingServer>
    <incomingServer type="imap"><hostname>plain.example.org</hostname><port>143</port><socketType>plain</socketType><username>%EMAILADDRESS%</username></incomingServer>
    <incomingServer type='imap'>
      <hostname>imap.example.org</hostname>
      <port>993</port>
      <socketType>SSL</socketType>
      <username>%EMAILLOCALPART%</username>
    </incomingServer>
    <outgoingServer type="smtp"><hostname>smtp.%EMAILDOMAIN%</hostname><port>587</port><socketType>STARTTLS</socketType></outgoingServer>
  </emailProvider>
</clientConfig>"#;

    #[test]
    fn reads_mozilla_autoconfig() {
        let c = parse_config(XML, "zoe@example.org").unwrap();
        assert_eq!(c.imap, ServerConfig { host: "imap.example.org".into(), port: 993, security: Security::Tls }, "encrypted IMAP wins");
        assert_eq!(c.smtp, ServerConfig { host: "smtp.example.org".into(), port: 587, security: Security::StartTls });
        assert_eq!(c.username, "zoe");
        assert!(parse_config("<html>nope</html>", "a@b.c").is_none());
        assert!(
            parse_config(&XML.replace("<port>993</port>", "<port>x</port>").replace("<port>143</port>", "<port>0</port>"), "z@example.org").is_none()
        );
        let evil = XML.replace("imap.example.org", "imap.example.org/../x");
        assert_eq!(parse_config(&evil, "z@example.org").unwrap().imap.host, "plain.example.org", "bad host names are skipped");
    }

    /// Live: real DNS and the real ISP database (only domain names are sent). Run by hand with
    /// `cargo test -p mailcraft-net -- --ignored live_`.
    #[test]
    #[ignore]
    fn live_discovery() {
        assert!(!crate::dns::mx("gmail.com").is_empty(), "MX lookup through the system resolver");
        let f = discover("someone@gmail.com").unwrap();
        assert_eq!(f.config.imap.host, "imap.gmail.com");
        println!("gmail.com: {} ({}); smtp {}:{}", f.config.imap.host, f.source, f.config.smtp.host, f.config.smtp.port);
    }

    /// A domain hosted by a provider that publishes its own configuration (not in the ISP
    /// database) and points Outlook at it with an `_autodiscover._tcp` record.
    #[test]
    fn hosted_domains_find_their_provider() {
        const HOSTED: &str = r#"<clientConfig version="1.1"><emailProvider id="mailhost.example">
            <incomingServer type="imap"><hostname>mail.mailhost.example</hostname><port>993</port><socketType>SSL</socketType><username>%EMAILADDRESS%</username></incomingServer>
            <incomingServer type="pop3"><hostname>mail.mailhost.example</hostname><port>995</port><socketType>SSL</socketType><username>%EMAILADDRESS%</username></incomingServer>
            <outgoingServer type="smtp"><hostname>mail.mailhost.example</hostname><port>587</port><socketType>STARTTLS</socketType></outgoingServer>
            <outgoingServer type="smtp"><hostname>mail.mailhost.example</hostname><port>465</port><socketType>SSL</socketType></outgoingServer>
        </emailProvider></clientConfig>"#;
        // Through the mail exchanger's provider.
        let mut asked = Vec::new();
        let mut fetch = |url: &str| {
            asked.push(url.to_string());
            url.starts_with("https://autoconfig.mailhost.example/").then(|| HOSTED.to_string())
        };
        let mut mx = |_: &str| vec!["mx1.mailhost.example".to_string()];
        let mut srv = |_: &str| None;
        let f = discover_with("zoe@studio.example", &mut Lookups { fetch: &mut fetch, mx: &mut mx, srv: &mut srv }).unwrap();
        assert_eq!(f.source, "mailhost.example");
        assert_eq!((f.config.imap.host.as_str(), f.config.smtp.port, f.config.smtp.security), ("mail.mailhost.example", 465, Security::Tls));
        assert_eq!(f.config.pop, Some(ServerConfig { host: "mail.mailhost.example".into(), port: 995, security: Security::Tls }));
        assert_eq!(f.config.username, "zoe@studio.example");
        assert!(
            asked.iter().filter(|u| !u.contains("autoconfig.studio.example")).all(|u| !u.contains("zoe")),
            "only the domain's own server is told the address"
        );
        // Through the `_autodiscover._tcp` record, when the mail exchanger says nothing.
        let mut fetch = |url: &str| url.starts_with("https://autoconfig.mailhost.example/").then(|| HOSTED.to_string());
        let mut no_mx = |_: &str| Vec::new();
        let mut srv = |name: &str| (name == "_autodiscover._tcp.studio.example").then(|| ("autoconfig.mailhost.example".to_string(), 443));
        let f = discover_with("zoe@studio.example", &mut Lookups { fetch: &mut fetch, mx: &mut no_mx, srv: &mut srv }).unwrap();
        assert_eq!((f.source.as_str(), f.config.pop.map(|p| p.port)), ("mailhost.example", Some(995)));
        // A record naming something that isn't a host name is ignored.
        let mut fetch = |_: &str| None;
        let mut srv = |name: &str| name.starts_with("_autodiscover").then(|| ("evil/../x".to_string(), 443));
        assert!(discover_with("zoe@studio.example", &mut Lookups { fetch: &mut fetch, mx: &mut no_mx, srv: &mut srv }).is_none());
    }

    #[test]
    fn lookups_in_order() {
        let mut asked: Vec<String> = Vec::new();
        let mut fetch = |url: &str| {
            asked.push(url.to_string());
            (url == "https://autoconfig.thunderbird.net/v1.1/google.com").then(|| XML.to_string())
        };
        let mut mx = |d: &str| if d == "studio.example" { vec!["aspmx.l.google.com".to_string()] } else { Vec::new() };
        let mut srv = |_: &str| None;
        let f = discover_with("zoe@studio.example", &mut Lookups { fetch: &mut fetch, mx: &mut mx, srv: &mut srv }).unwrap();
        assert!(f.source.contains("google.com"), "{}", f.source);
        assert_eq!(
            asked,
            vec![
                "https://autoconfig.studio.example/mail/config-v1.1.xml?emailaddress=zoe%40studio.example".to_string(),
                "https://studio.example/.well-known/autoconfig/mail/config-v1.1.xml".into(),
                "https://autoconfig.thunderbird.net/v1.1/studio.example".into(),
                "https://autoconfig.thunderbird.net/v1.1/google.com".into(),
            ]
        );

        // SRV when nothing else answers.
        let mut none = |_: &str| None;
        let mut no_mx = |_: &str| Vec::new();
        let mut srv = |name: &str| match name {
            "_imaps._tcp.x.example" => Some(("mail.x.example".to_string(), 993)),
            "_submission._tcp.x.example" => Some(("mail.x.example".to_string(), 587)),
            _ => None,
        };
        let f = discover_with("a@x.example", &mut Lookups { fetch: &mut none, mx: &mut no_mx, srv: &mut srv }).unwrap();
        assert_eq!((f.config.imap.port, f.config.imap.security, f.config.smtp.security), (993, Security::Tls, Security::StartTls));
        let mut srv_none = |_: &str| None;
        assert!(discover_with("nobody", &mut Lookups { fetch: &mut none, mx: &mut no_mx, srv: &mut srv_none }).is_none());
        assert_eq!(base_domain("aspmx.l.google.com"), "google.com");
        assert_eq!(base_domain("mx1.mail.example.co.uk."), "example.co.uk");
    }

    #[test]
    fn discovers_jmap_from_autoconfig_and_rfc8620_srv() {
        let xml = r#"<clientConfig version="1.1"><emailProvider id="nextxch.com">
            <incomingServer type="jmap"><hostname>mail.nextxch.com</hostname><port>443</port><socketType>SSL</socketType><username>%EMAILADDRESS%</username></incomingServer>
            <incomingServer type="imap"><hostname>mail.nextxch.com</hostname><port>993</port><socketType>SSL</socketType><username>%EMAILADDRESS%</username></incomingServer>
            <outgoingServer type="smtp"><hostname>mail.nextxch.com</hostname><port>465</port><socketType>SSL</socketType><username>%EMAILADDRESS%</username></outgoingServer>
        </emailProvider><oAuth2>
            <issuer>nextxch.com</issuer>
            <scope>mail caldav carddav offline_access</scope>
            <authURL>https://mail.nextxch.com/oauth/authorize</authURL>
            <tokenURL>https://mail.nextxch.com/oauth/token</tokenURL>
        </oAuth2></clientConfig>"#;
        let c = parse_config(xml, "zoe@firma.example").unwrap();
        assert_eq!(c.jmap.as_deref(), Some("https://mail.nextxch.com/.well-known/jmap"));
        assert_eq!(c.imap.host, "mail.nextxch.com");
        let oa = c.oauth_config.unwrap();
        assert_eq!(
            (oa.issuer.as_str(), oa.auth_url.as_str(), oa.scope.as_str()),
            ("nextxch.com", "https://mail.nextxch.com/oauth/authorize", "mail caldav carddav offline_access")
        );

        // `<displayShortName>nextxch.com</displayShortName>` (and legacy `Nextxch`) upgrades any host to JMAP.
        let nextxch_xml = r#"<clientConfig version="1.1"><emailProvider id="stadtwerke.example">
            <displayShortName>nextxch.com</displayShortName>
            <incomingServer type="imap"><hostname>mail.stadtwerke.example</hostname><port>993</port><socketType>SSL</socketType><username>%EMAILADDRESS%</username></incomingServer>
            <outgoingServer type="smtp"><hostname>mail.stadtwerke.example</hostname><port>465</port><socketType>SSL</socketType><username>%EMAILADDRESS%</username></outgoingServer>
        </emailProvider></clientConfig>"#;
        assert_eq!(
            parse_config(nextxch_xml, "sabine@stadtwerke.example").unwrap().jmap.as_deref(),
            Some("https://mail.stadtwerke.example/.well-known/jmap")
        );
        let legacy_nextxch = nextxch_xml.replace("<displayShortName>nextxch.com</displayShortName>", "<displayShortName>Nextxch</displayShortName>");
        assert_eq!(
            parse_config(&legacy_nextxch, "sabine@stadtwerke.example").unwrap().jmap.as_deref(),
            Some("https://mail.stadtwerke.example/.well-known/jmap")
        );

        // RFC 8620 section 2.2 `/.well-known/jmap` probe upgrades any arbitrary JMAP domain, even white-labeled or without XML.
        assert!(is_jmap_session_endpoint(&crate::http::Response {
            status: 401,
            headers: vec![("WWW-Authenticate".into(), "Basic realm=\"Nextxch JMAP\", Bearer".into())],
            body: Vec::new(),
        }));
        assert!(!is_jmap_session_endpoint(&crate::http::Response {
            status: 401,
            headers: vec![("WWW-Authenticate".into(), "Basic realm=\"Restricted\"".into())],
            body: Vec::new(),
        }));
        let mut none = |_: &str| None;
        let mut arb_mx = |_: &str| vec!["mx.kanzlei.example.".to_string()];
        let mut srv_none = |_: &str| None;
        let f_probe = discover_inner("max@kanzlei.example", &mut Lookups { fetch: &mut none, mx: &mut arb_mx, srv: &mut srv_none }, &mut |h| {
            h == "mx.kanzlei.example"
        })
        .unwrap();
        assert_eq!(f_probe.config.jmap.as_deref(), Some("https://mx.kanzlei.example/.well-known/jmap"));

        // Plain JMAP without TLS is ignored.
        let plain = r#"<clientConfig version="1.1"><emailProvider id="bad.example">
            <incomingServer type="jmap"><hostname>mail.bad.example</hostname><port>80</port><socketType>plain</socketType></incomingServer>
        </emailProvider></clientConfig>"#;
        assert!(parse_config(plain, "zoe@bad.example").is_none());

        // Self-hosted mail server on `mail.<domain>` answering `.well-known/autoconfig` on the MX host.
        let mut mx_fetch = |url: &str| {
            (url == "https://mail.firma.example/.well-known/autoconfig/mail/config-v1.1.xml?emaildomain=firma.example").then(|| xml.to_string())
        };
        let mut self_mx = |_: &str| vec!["mail.firma.example.".to_string()];
        let mut srv_none = |_: &str| None;
        let f = discover_with("zoe@firma.example", &mut Lookups { fetch: &mut mx_fetch, mx: &mut self_mx, srv: &mut srv_none }).unwrap();
        assert_eq!(f.config.jmap.as_deref(), Some("https://mail.nextxch.com/.well-known/jmap"));

        // RFC 8620 §2.2 `_jmap._tcp` SRV discovery when no XML is published.
        let mut none = |_: &str| None;
        let mut no_mx = |_: &str| Vec::new();
        let mut srv = |name: &str| (name == "_jmap._tcp.firma.example").then(|| ("mail.nextxch.com".to_string(), 443));
        let f = discover_with("zoe@firma.example", &mut Lookups { fetch: &mut none, mx: &mut no_mx, srv: &mut srv }).unwrap();
        assert_eq!(f.config.jmap.as_deref(), Some("https://mail.nextxch.com/.well-known/jmap"));
        assert!(f.config.imap.host.is_empty());
    }
}
