//! What a message carries to learn about its reader. A newsletter's 1-by-1 picture tells its
//! sender when (and from where) it was opened; its links go through a click counter, or carry
//! tags (`utm_source`, `fbclid`…) that say which mailing and which reader clicked.
//!
//! [`trackers`] counts both in an HTML body, and lists the tracking pictures so they stay
//! unloaded even when the user shows the message's pictures. [`clean_link`] takes a link back
//! to where it really goes before it's opened: out of the redirect wrappers that carry the real
//! address in them (Microsoft's Safe Links, Google's `/url`, Proofpoint, Amazon SES and others),
//! and without the tags that only identify the reader. Links counted through a service that
//! keeps the real address to itself can't be cleaned, only counted.

use serde::{Deserialize, Serialize};

/// Pictures and links that report back.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Trackers {
    /// The tracking pictures' addresses (never loaded).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pictures: Vec<String>,
    /// Links that report the click (through a counter, or with tags naming the reader).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub links: usize,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

impl Trackers {
    pub fn is_empty(&self) -> bool {
        self.pictures.is_empty() && self.links == 0
    }
}

/// Query parameters that only say who clicked, from which mailing (`utm_*` too).
const TAGS: &[&str] = &[
    "_hsenc",
    "_hsmi",
    "_kx",
    "__hsfp",
    "__hssc",
    "__hstc",
    "ck_subscriber_id",
    "dclid",
    "fbclid",
    "gbraid",
    "gclid",
    "hsctatracking",
    "igshid",
    "mc_cid",
    "mc_eid",
    "mkt_tok",
    "msclkid",
    "oly_anon_id",
    "oly_enc_id",
    "s_cid",
    "vero_conv",
    "vero_id",
    "wbraid",
    "yclid",
];

/// Services that count clicks and opens for mailings (a host ending in one of these).
const COUNTERS: &[&str] = &[
    "awstrack.me",
    "clicks.aweber.com",
    "clicks.mlsend.com",
    "convertkit-mail.com",
    "convertkit-mail2.com",
    "createsend1.com",
    "ct.sendgrid.net",
    "exct.net",
    "hubspotemail.net",
    "hubspotlinks.com",
    "klclick.com",
    "klclick1.com",
    "klclick2.com",
    "klclick3.com",
    "list-manage.com",
    "mailtrack.io",
    "mandrillapp.com",
    "mcsv.net",
    "mjt.lu",
    "pstmrk.it",
    "rs6.net",
    "sendgrid.net",
    "sparkpostmail.com",
    "t.yesware.com",
    "track.customer.io",
    "track.hubspot.com",
    "via.intercom-mail.com",
];

/// Wrappers that carry the real address in a query parameter: (host or host suffix, path
/// prefix, parameter).
const WRAPPERS: &[(&str, &str, &str)] = &[
    ("safelinks.protection.outlook.com", "/", "url"),
    ("www.google.com", "/url", "q"),
    ("www.google.com", "/url", "url"),
    ("google.com", "/url", "q"),
    ("l.facebook.com", "/l.php", "u"),
    ("lm.facebook.com", "/l.php", "u"),
    ("l.instagram.com", "/", "u"),
    ("www.youtube.com", "/redirect", "q"),
    ("out.reddit.com", "/", "url"),
    ("slack-redir.net", "/link", "url"),
    ("www.linkedin.com", "/redir/redirect", "url"),
    ("steamcommunity.com", "/linkfilter/", "url"),
];

/// The longest link looked at (longer ones are left as they are).
const LINK_MAX: usize = 8192;

/// A web address split where cleaning needs it.
struct Url<'a> {
    scheme: &'a str,
    host: String,
    /// Everything after the host up to `?` (a port included, as written).
    path: &'a str,
    query: Option<&'a str>,
    fragment: Option<&'a str>,
}

fn split(url: &str) -> Option<Url<'_>> {
    let (scheme, rest) = url.split_once("://")?;
    if !scheme.eq_ignore_ascii_case("https") && !scheme.eq_ignore_ascii_case("http") {
        return None;
    }
    let (rest, fragment) = match rest.split_once('#') {
        Some((r, f)) => (r, Some(f)),
        None => (rest, None),
    };
    let (rest, query) = match rest.split_once('?') {
        Some((r, q)) => (r, Some(q)),
        None => (rest, None),
    };
    // An IPv6 address is in brackets, colons and all.
    let host_end = match rest.strip_prefix('[') {
        Some(r) => r.find(']').map_or(rest.len(), |e| e + 2),
        None => rest.find(['/', ':']).unwrap_or(rest.len()),
    };
    let authority = rest.get(..host_end)?;
    // `user@host`: the host is after the `@`.
    let host = authority.rsplit('@').next().unwrap_or(authority).to_ascii_lowercase();
    if host.is_empty() {
        return None;
    }
    Some(Url { scheme, host, path: rest.get(host_end..).unwrap_or(""), query, fragment })
}

fn host_is(host: &str, name: &str) -> bool {
    host == name || host.strip_suffix(name).is_some_and(|h| h.ends_with('.'))
}

/// `%xx` and (in a query) `+` as the characters they stand for; `None` when that isn't text.
fn percent_decode(s: &str, plus: bool) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while let Some(&c) = b.get(i) {
        match c {
            b'%' => {
                let hex = s.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok());
                match hex {
                    Some(v) => {
                        out.push(v);
                        i += 3;
                        continue;
                    }
                    None => out.push(c),
                }
            }
            b'+' if plus => out.push(b' '),
            _ => out.push(c),
        }
        i += 1;
    }
    String::from_utf8(out).ok()
}

/// The address a wrapper carries, when `url` is one.
fn unwrapped(url: &str) -> Option<String> {
    let u = split(url)?;
    let inner = if host_is(&u.host, "urldefense.com") {
        // Proofpoint v3: `/v3/__https://real.example/path__;…`.
        u.path.strip_prefix("/v3/__").and_then(|p| p.split_once("__;")).map(|(real, _)| real.to_string())
    } else if host_is(&u.host, "awstrack.me") {
        // Amazon SES: `/L0/<the address, percent-encoded>/1/…`.
        u.path.strip_prefix("/L0/").and_then(|p| p.split('/').next()).and_then(|p| percent_decode(p, false))
    } else {
        let (_, _, param) = WRAPPERS.iter().find(|(host, path, _)| host_is(&u.host, host) && u.path.starts_with(path))?;
        u.query?.split('&').find_map(|pair| {
            let (k, v) = pair.split_once('=')?;
            k.eq_ignore_ascii_case(param).then(|| percent_decode(v, true)).flatten()
        })
    }?;
    let inner = inner.trim().to_string();
    split(&inner).is_some().then_some(inner)
}

fn is_tag(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.starts_with("utm_") || TAGS.contains(&name.as_str())
}

/// `url` as it really goes: out of the redirect wrappers that carry the real address (a few deep
/// at most), and without the query parameters that only identify the reader. Anything that isn't
/// an `http(s)` address comes back as it was.
pub fn clean_link(url: &str) -> String {
    let mut url = url.trim().to_string();
    if url.len() > LINK_MAX {
        return url;
    }
    for _ in 0..4 {
        match unwrapped(&url) {
            Some(inner) if inner != url => url = inner,
            _ => break,
        }
    }
    let Some(u) = split(&url) else { return url };
    let Some(query) = u.query else { return url };
    let kept: Vec<&str> = query.split('&').filter(|p| !p.is_empty() && !is_tag(p.split('=').next().unwrap_or(p))).collect();
    let mut out = format!("{}://{}{}", u.scheme, original_authority(&url, u.scheme), u.path);
    if !kept.is_empty() {
        out.push('?');
        out.push_str(&kept.join("&"));
    }
    if let Some(f) = u.fragment {
        out.push('#');
        out.push_str(f);
    }
    out
}

/// The host part of `url` as it was written (case and any `user@` kept).
fn original_authority<'a>(url: &'a str, scheme: &str) -> &'a str {
    let rest = url.get(scheme.len() + 3..).unwrap_or("");
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let auth = rest.get(..end).unwrap_or("");
    // A port stays with the path (as `split` keeps it): only the name part here.
    let name_end = match auth.strip_prefix('[') {
        Some(r) => r.find(']').map_or(auth.len(), |e| e + 2),
        None => auth.rfind(':').unwrap_or(auth.len()),
    };
    auth.get(..name_end).unwrap_or(auth)
}

/// Whether following `url` tells someone the reader clicked it.
pub fn tracks_click(url: &str) -> bool {
    let Some(u) = split(url.trim()) else { return false };
    COUNTERS.iter().any(|c| host_is(&u.host, c))
        || unwrapped(url).is_some()
        || u.query.is_some_and(|q| q.split('&').any(|p| is_tag(p.split('=').next().unwrap_or(p))))
}

/// The trackers in an HTML body: tiny pictures (1 or 2 pixels across, or on a counting service)
/// and links that report the click.
pub fn trackers(html: &str) -> Trackers {
    let lower = html.to_ascii_lowercase();
    let mut pictures: Vec<String> = Vec::new();
    let mut from = 0;
    while let Some(i) = lower.get(from..).and_then(|r| r.find("<img")) {
        let start = from + i;
        let Some(end) = lower.get(start..).and_then(|r| r.find('>')).map(|e| start + e) else { break };
        let tag = html.get(start..end).unwrap_or("");
        from = end + 1;
        let Some(src) = crate::html::tag_attr(tag, "src").map(|s| s.trim().replace("&amp;", "&")) else { continue };
        let Some(u) = split(&src) else { continue };
        let tiny =
            |a: &str| crate::html::tag_attr(tag, a).is_some_and(|v| v.trim().trim_end_matches("px").trim().parse::<f32>().is_ok_and(|n| n <= 2.0));
        let style = crate::html::tag_attr(tag, "style").unwrap_or("").to_ascii_lowercase().replace(' ', "");
        let hidden = style.contains("display:none") || style.contains("width:1px") || style.contains("height:1px") || style.contains("width:0");
        if (tiny("width") || tiny("height") || hidden || COUNTERS.iter().any(|c| host_is(&u.host, c)))
            && !pictures.contains(&src)
            && pictures.len() < 50
        {
            pictures.push(src);
        }
    }
    let links = crate::html_links(html).iter().filter(|l| tracks_click(l)).count();
    Trackers { pictures, links }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_lose_their_tags_and_wrappers() {
        for (link, clean) in [
            ("https://shop.example/sale?utm_source=news&utm_medium=email&id=7#top", "https://shop.example/sale?id=7#top"),
            ("https://shop.example/?fbclid=abc", "https://shop.example/"),
            ("https://Shop.Example:8443/a?MC_EID=1&q=x+y", "https://Shop.Example:8443/a?q=x+y"),
            (
                "https://eur01.safelinks.protection.outlook.com/?url=https%3A%2F%2Fdocs.example%2Fplan%3Fv%3D2%26utm_campaign%3Dq3&data=05%7C01&reserved=0",
                "https://docs.example/plan?v=2",
            ),
            ("https://www.google.com/url?q=https://site.example/page&sa=D&ust=1", "https://site.example/page"),
            ("https://urldefense.com/v3/__https://portal.example/login__;!!abc$", "https://portal.example/login"),
            ("https://abcd.r.us-east-1.awstrack.me/L0/https:%2F%2Fapp.example%2Fwelcome/1/0100-x/abc=", "https://app.example/welcome"),
            ("https://l.facebook.com/l.php?u=https%3A%2F%2Fevent.example%2F&h=AT0", "https://event.example/"),
            // Nothing to take out, or not a web address: as it was.
            ("https://site.example/a?b=c", "https://site.example/a?b=c"),
            ("mailto:ana@x.example?subject=Hi", "mailto:ana@x.example?subject=Hi"),
            // A wrapper whose parameter isn't a web address stays wrapped.
            ("https://www.google.com/url?q=javascript:alert(1)", "https://www.google.com/url?q=javascript:alert(1)"),
        ] {
            assert_eq!(clean_link(link), clean, "{link}");
        }
    }

    #[test]
    fn trackers_are_counted() {
        let html = r#"<p><a href="https://news.us1.list-manage.com/track/click?u=1&id=2">Read</a>
            <a href="https://shop.example/?utm_source=x">Shop</a>
            <a href="https://shop.example/about">About</a>
            <img src="https://shop.example/logo.png" width="120" alt="Shop">
            <img src="https://open.shop.example/o.gif?r=9" width="1" height="1">
            <img src="https://news.us1.list-manage.com/track/open.php?u=1" alt="">
            <img src="https://t.example/p.png" style="display: none"></p>"#;
        let t = trackers(html);
        assert_eq!(t.links, 2);
        assert_eq!(
            t.pictures,
            ["https://open.shop.example/o.gif?r=9", "https://news.us1.list-manage.com/track/open.php?u=1", "https://t.example/p.png"]
        );
        assert!(trackers("<p>plain</p>").is_empty());
    }

    #[test]
    fn odd_links_never_panic() {
        for link in [
            "",
            "https://",
            "https://%",
            "https://x.example/?%zz=1&&=&utm_",
            "https://urldefense.com/v3/__",
            "http://[::1]:80/?a",
            "https://é.example/%E2%82",
        ] {
            let _ = clean_link(link);
            let _ = tracks_click(link);
        }
        assert_eq!(clean_link("http://[::1]:8080/a?utm_x=1&b=2"), "http://[::1]:8080/a?b=2");
        let deep = (0..10).fold("https://site.example/".to_string(), |u, _| format!("https://www.google.com/url?q={}", u.replace('&', "%26")));
        let _ = clean_link(&deep);
    }
}
