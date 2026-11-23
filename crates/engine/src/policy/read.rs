//! A profile read from JSON: each part on its own, so one that can't be read (a misspelt key, a
//! value of the wrong kind) is said and left out, never a reason to refuse the rest. Several
//! sources stack, the later over the earlier; one whose locks don't count (beside a portable
//! copy, which its user can change) has them taken out first.

use super::*;

/// The largest profile read, in bytes: far more than any organisation's.
pub const PROFILE_MAX: usize = 1 << 20;
/// The most profiles in one file, domains of one profile, and settings of one profile.
const PROFILES_MAX: usize = 64;
const DOMAINS_MAX: usize = 256;
const SETTINGS_MAX: usize = 512;
/// The longest text kept: a signature's HTML, a disclaimer; and the longest name or address.
const TEXT_MAX: usize = 64 * 1024;
const NAME_MAX: usize = 2048;
/// The most of what couldn't be read that's said.
const IGNORED_MAX: usize = 100;

/// The keys a profile has: anything else is said as not read.
const KEYS: [&str; 15] = [
    "organisation",
    "logo",
    "contact",
    "settings",
    "theme",
    "language",
    "updates",
    "requireEncryption",
    "certificateExceptions",
    "accounts",
    "signature",
    "disclaimer",
    "profiles",
    "domains",
    "default",
];

impl Organisation {
    /// The profile the sources give, the later over the earlier: `(where, its JSON, whether its
    /// locks count)`.
    pub fn from_sources(sources: Vec<(String, Value, bool)>) -> Organisation {
        let mut merged = Value::Object(serde_json::Map::new());
        let mut places = Vec::new();
        for (place, mut v, locks) in sources {
            if !v.is_object() {
                continue;
            }
            if !locks {
                without_locks(&mut v);
            }
            merge(&mut merged, v, 0);
            // A configuration profile's own keys and its named values are two, from one place.
            if !places.contains(&place) {
                places.push(place);
            }
        }
        let mut o = Organisation::read(&merged);
        o.sources = places;
        o
    }

    /// A profile from its JSON (an object; anything else is an empty one, said).
    pub fn read(v: &Value) -> Organisation {
        let mut ignored = Vec::new();
        let Some(obj) = v.as_object() else {
            return Organisation { ignored: vec!["the profile isn't a JSON object".into()], ..Organisation::default() };
        };
        let top = profile(obj, "", &mut ignored, true);
        let mut profiles = Vec::new();
        if let Some(list) = obj.get("profiles") {
            match list.as_array() {
                Some(items) => {
                    for (i, item) in items.iter().take(PROFILES_MAX).enumerate() {
                        let at = format!("profiles[{i}].");
                        match item.as_object() {
                            Some(p) => {
                                let p = profile(p, &at, &mut ignored, false);
                                if p.domains.is_empty() && !p.default {
                                    note(&mut ignored, format!("{at}domains: a profile for no domain, and not the default, applies to no one"));
                                }
                                profiles.push(p);
                            }
                            None => note(&mut ignored, format!("profiles[{i}]: not an object")),
                        }
                    }
                    if items.len() > PROFILES_MAX {
                        note(&mut ignored, format!("profiles: only the first {PROFILES_MAX} are read"));
                    }
                }
                None => note(&mut ignored, "profiles: not a list".into()),
            }
        }
        if profiles.iter().filter(|p| p.default).count() > 1 {
            note(&mut ignored, "profiles: more than one is the default; the first is taken".into());
            let mut seen = false;
            for p in profiles.iter_mut().filter(|p| p.default) {
                p.default = !seen;
                seen = true;
            }
        }
        Organisation { top, profiles, sources: Vec::new(), ignored }
    }
}

/// Say what wasn't read, up to [`IGNORED_MAX`].
fn note(ignored: &mut Vec<String>, what: String) {
    if ignored.len() < IGNORED_MAX {
        ignored.push(what.chars().take(300).collect());
    }
}

/// `b` over `a`: objects key by key, anything else (lists among them) replaced whole.
fn merge(a: &mut Value, b: Value, depth: usize) {
    match (a, b) {
        (Value::Object(a), Value::Object(b)) if depth < 16 => {
            for (k, v) in b {
                match a.get_mut(&k) {
                    Some(old) => merge(old, v, depth + 1),
                    None => {
                        a.insert(k, v);
                    }
                }
            }
        }
        (a, b) => *a = b,
    }
}

/// A profile's JSON with its locks taken out (for a source its user can change): settings stay
/// as defaults, and the policies, which are nothing but locks, go.
pub fn without_locks(v: &mut Value) {
    let Some(obj) = v.as_object_mut() else { return };
    for k in ["requireEncryption", "certificateExceptions", "disclaimer"] {
        obj.remove(k);
    }
    if let Some(a) = obj.get_mut("accounts").and_then(Value::as_object_mut) {
        a.remove("onlyOrganisationDomains");
        a.remove("lookups");
    }
    for k in ["theme", "language", "updates"] {
        if let Some(g) = obj.get_mut(k).and_then(Value::as_object_mut) {
            g.insert("locked".into(), Value::Bool(false));
        }
    }
    if let Some(s) = obj.get_mut("settings").and_then(Value::as_object_mut) {
        for g in s.values_mut() {
            if let Some(g) = g.as_object_mut().filter(|g| g.contains_key("value")) {
                g.insert("locked".into(), Value::Bool(false));
            }
        }
    }
    if let Some(s) = obj.get_mut("signature").and_then(Value::as_object_mut) {
        s.insert("locked".into(), Value::Bool(false));
    }
    if let Some(list) = obj.get_mut("profiles").and_then(Value::as_array_mut) {
        for p in list {
            without_locks(p);
        }
    }
}

/// Text, trimmed and capped; `None` for none or empty.
fn text(v: Option<&Value>, at: &str, max: usize, ignored: &mut Vec<String>) -> Option<String> {
    let v = v?;
    let Some(s) = v.as_str() else {
        note(ignored, format!("{at}: not text"));
        return None;
    };
    let s = s.trim();
    if s.chars().count() > max {
        note(ignored, format!("{at}: longer than {max} characters, cut"));
    }
    let s: String = s.chars().take(max).collect();
    (!s.is_empty()).then_some(s)
}

fn flag(v: Option<&Value>, at: &str, ignored: &mut Vec<String>) -> Option<bool> {
    let v = v?;
    let b = v.as_bool();
    if b.is_none() {
        note(ignored, format!("{at}: not true or false"));
    }
    b
}

/// A setting given as its value (a default), or as `{"value": …, "locked": true}`.
fn given(v: &Value, at: &str, ignored: &mut Vec<String>) -> Option<Given> {
    match v.as_object() {
        Some(o) if o.contains_key("value") => {
            let locked = match o.get("locked") {
                None => false,
                Some(l) => l.as_bool().unwrap_or_else(|| {
                    note(ignored, format!("{at}.locked: not true or false; taken as a default"));
                    false
                }),
            };
            o.get("value").map(|value| Given { value: value.clone(), locked })
        }
        _ => Some(Given { value: v.clone(), locked: false }),
    }
}

/// A mail domain, lower-case, if it is one.
fn domain(v: &Value) -> Option<String> {
    let d = v.as_str()?.trim().trim_end_matches('.').to_ascii_lowercase();
    let ok = !d.is_empty()
        && d.len() <= 253
        && d.contains('.')
        && d.split('.').all(|l| !l.is_empty() && l.len() <= 63 && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'));
    ok.then_some(d)
}

fn domains(v: Option<&Value>, at: &str, ignored: &mut Vec<String>) -> Vec<String> {
    let Some(v) = v else { return Vec::new() };
    let Some(list) = v.as_array() else {
        note(ignored, format!("{at}: not a list"));
        return Vec::new();
    };
    let mut out = Vec::new();
    for d in list.iter().take(DOMAINS_MAX) {
        match domain(d) {
            Some(d) if !out.contains(&d) => out.push(d),
            Some(_) => {}
            None => note(ignored, format!("{at}: {d} isn't a mail domain")),
        }
    }
    out
}

/// A server as `host:port:security` (security `tls`, `starttls` or `none`; port and security
/// may be left out, for the protocol's usual ones), or `{"host", "port", "security"}`.
fn server(v: &Value, at: &str, usual: (u16, &str), ignored: &mut Vec<String>) -> Option<Server> {
    let (host, port, security) = match v {
        Value::String(s) => {
            let mut parts = s.trim().splitn(3, ':');
            let host = parts.next().unwrap_or_default().to_string();
            let port = parts.next().map(str::to_string);
            let security = parts.next().map(str::to_string);
            (host, port.map(|p| p.parse::<u16>().ok()), security)
        }
        Value::Object(o) => {
            let host = o.get("host").and_then(Value::as_str).unwrap_or_default().to_string();
            let port = o.get("port").map(|p| p.as_u64().and_then(|p| u16::try_from(p).ok()));
            let security = o.get("security").and_then(Value::as_str).map(str::to_string);
            (host, port, security)
        }
        _ => {
            note(ignored, format!("{at}: not a server"));
            return None;
        }
    };
    let host = host.trim().to_ascii_lowercase();
    let host_ok = !host.is_empty() && host.len() <= 253 && host.bytes().all(|b| b.is_ascii_alphanumeric() || b"-.".contains(&b));
    if !host_ok {
        note(ignored, format!("{at}: `{host}` isn't a server's name"));
        return None;
    }
    let port = match port {
        None => usual.0,
        Some(Some(p)) if p > 0 => p,
        Some(_) => {
            note(ignored, format!("{at}: not a port"));
            return None;
        }
    };
    // None named: as the port is used (RFC 8314 §3.3, RFC 6409: submission's 587 and IMAP's
    // and POP's own ports go encrypted with STARTTLS), else as the kind of server's usual port.
    let usual_security = match port {
        587 | 143 | 110 => "starttls",
        _ if port == usual.0 => usual.1,
        _ => "tls",
    };
    let security = security.map(|s| s.trim().to_ascii_lowercase()).unwrap_or_else(|| usual_security.to_string());
    if !["tls", "starttls", "none"].contains(&security.as_str()) {
        note(ignored, format!("{at}: security is tls, starttls or none, not `{security}`"));
        return None;
    }
    Some(Server { host, port, security })
}

/// A web address (https; http only for this computer, as a test server's).
fn web_address(v: Option<&Value>, at: &str, ignored: &mut Vec<String>) -> Option<String> {
    let s = text(v, at, NAME_MAX, ignored)?;
    let lower = s.to_ascii_lowercase();
    let local = ["http://localhost", "http://127.0.0.1", "http://[::1]"].iter().any(|p| lower.starts_with(p));
    if lower.starts_with("https://") || local {
        Some(s)
    } else {
        note(ignored, format!("{at}: an https address, not `{s}`"));
        None
    }
}

fn domain_servers(v: &Value, at: &str, ignored: &mut Vec<String>) -> Option<DomainServers> {
    let Some(o) = v.as_object() else {
        note(ignored, format!("{at}: not an object"));
        return None;
    };
    let mut s = DomainServers::default();
    for (k, val) in o {
        let here = format!("{at}.{k}");
        match k.as_str() {
            "imap" => s.imap = server(val, &here, (993, "tls"), ignored),
            "pop" => s.pop = server(val, &here, (995, "tls"), ignored),
            "smtp" => s.smtp = server(val, &here, (465, "tls"), ignored),
            "caldav" => s.caldav = web_address(Some(val), &here, ignored),
            "carddav" => s.carddav = web_address(Some(val), &here, ignored),
            "ews" => s.ews = web_address(Some(val), &here, ignored),
            "jmap" => s.jmap = web_address(Some(val), &here, ignored),
            "graph" => s.graph = flag(Some(val), &here, ignored).unwrap_or(false),
            _ => note(ignored, format!("{here}: not a kind of server")),
        }
    }
    Some(s)
}

/// One profile from its object: `top` for the file's own (which may have `profiles`), else one
/// of those (which may have `domains` and `default`).
fn profile(o: &serde_json::Map<String, Value>, at: &str, ignored: &mut Vec<String>, top: bool) -> Profile {
    let mut p = Profile::default();
    for k in o.keys() {
        let known = KEYS.contains(&k.as_str());
        let here_only = matches!(k.as_str(), "profiles") && !top || matches!(k.as_str(), "domains" | "default") && top;
        if !known || here_only {
            note(ignored, format!("{at}{k}: not a profile's setting here"));
        }
    }
    if !top {
        p.domains = domains(o.get("domains"), &format!("{at}domains"), ignored);
        p.default = flag(o.get("default"), &format!("{at}default"), ignored).unwrap_or(false);
    }
    p.organisation = text(o.get("organisation"), &format!("{at}organisation"), 200, ignored);
    p.logo = text(o.get("logo"), &format!("{at}logo"), NAME_MAX, ignored);
    p.contact = text(o.get("contact"), &format!("{at}contact"), 500, ignored);
    if let Some(s) = o.get("settings") {
        match s.as_object() {
            Some(s) => {
                for (k, v) in s.iter().take(SETTINGS_MAX) {
                    if let Some(g) = given(v, &format!("{at}settings.{k}"), ignored) {
                        p.settings.insert(k.clone(), g);
                    }
                }
            }
            None => note(ignored, format!("{at}settings: not an object")),
        }
    }
    let choice = |key: &str, allowed: &dyn Fn(&str) -> bool, ignored: &mut Vec<String>| -> Option<Given> {
        let here = format!("{at}{key}");
        let g = given(o.get(key)?, &here, ignored)?;
        match g.value.as_str() {
            Some(v) if allowed(v) => Some(Given { value: Value::String(v.to_ascii_lowercase()), locked: g.locked }),
            _ => {
                note(ignored, format!("{here}: `{}` isn't one it takes", g.value));
                None
            }
        }
    };
    p.theme = choice("theme", &|v| ["light", "dark", "system"].contains(&v.to_ascii_lowercase().as_str()), ignored);
    p.language = choice("language", &|v| !v.is_empty() && v.len() <= 16 && v.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'), ignored);
    if let Some(u) = choice("updates", &|v| Updates::parse(v).is_some(), ignored) {
        p.updates = u.value.as_str().and_then(Updates::parse).map(|v| (v, u.locked));
    }
    p.require_encryption = flag(o.get("requireEncryption"), &format!("{at}requireEncryption"), ignored);
    p.certificate_exceptions = flag(o.get("certificateExceptions"), &format!("{at}certificateExceptions"), ignored);
    if let Some(a) = o.get("accounts") {
        match a.as_object() {
            Some(a) => accounts(a, &format!("{at}accounts."), &mut p, ignored),
            None => note(ignored, format!("{at}accounts: not an object")),
        }
    }
    if let Some(s) = o.get("signature") {
        p.signature = signature(s, &format!("{at}signature"), ignored);
    }
    if let Some(d) = o.get("disclaimer") {
        match d.as_object() {
            Some(d) => {
                let text = text(d.get("text"), &format!("{at}disclaimer.text"), TEXT_MAX, ignored).unwrap_or_default();
                let html = text_html(d.get("html"), &format!("{at}disclaimer.html"), ignored);
                if text.is_empty() && html.is_empty() {
                    note(ignored, format!("{at}disclaimer: no text"));
                } else {
                    p.disclaimer = Some(Disclaimer { text, html });
                }
            }
            None => note(ignored, format!("{at}disclaimer: not an object")),
        }
    }
    p
}

/// HTML as given, kept to [`TEXT_MAX`] characters (it's shown and sent as MailCraft cleans any).
fn text_html(v: Option<&Value>, at: &str, ignored: &mut Vec<String>) -> String {
    text(v, at, TEXT_MAX, ignored).unwrap_or_default()
}

fn signature(v: &Value, at: &str, ignored: &mut Vec<String>) -> Option<SignatureRule> {
    let Some(o) = v.as_object() else {
        note(ignored, format!("{at}: not an object"));
        return None;
    };
    let text = text(o.get("text"), &format!("{at}.text"), TEXT_MAX, ignored).unwrap_or_default();
    let html = text_html(o.get("html"), &format!("{at}.html"), ignored);
    if text.is_empty() && html.is_empty() {
        note(ignored, format!("{at}: no text"));
        return None;
    }
    let name = text_or(o.get("name"), &format!("{at}.name"), ignored).unwrap_or_else(|| "Organisation".into());
    Some(SignatureRule {
        name,
        text,
        html,
        locked: flag(o.get("locked"), &format!("{at}.locked"), ignored).unwrap_or(false),
        new_messages: flag(o.get("newMessages"), &format!("{at}.newMessages"), ignored).unwrap_or(true),
        replies: flag(o.get("replies"), &format!("{at}.replies"), ignored).unwrap_or(true),
    })
}

/// A signature's name: short text.
fn text_or(v: Option<&Value>, at: &str, ignored: &mut Vec<String>) -> Option<String> {
    text(v, at, 64, ignored)
}

fn accounts(a: &serde_json::Map<String, Value>, at: &str, p: &mut Profile, ignored: &mut Vec<String>) {
    for (k, v) in a {
        let here = format!("{at}{k}");
        match k.as_str() {
            "onlyOrganisationDomains" => p.only_organisation_domains = flag(Some(v), &here, ignored),
            "domains" => p.account_domains = domains(Some(v), &here, ignored),
            "autoconfig" => p.autoconfig = web_address(Some(v), &here, ignored),
            "lookups" => match v.as_object() {
                Some(l) => {
                    let mut lookups = Lookups::default();
                    for (lk, lv) in l {
                        let on = flag(Some(lv), &format!("{here}.{lk}"), ignored);
                        match (lk.as_str(), on) {
                            ("domain", Some(on)) => lookups.domain = on,
                            ("thunderbird", Some(on)) => lookups.thunderbird = on,
                            ("mx", Some(on)) => lookups.mx = on,
                            ("dns", Some(on)) => lookups.dns = on,
                            (_, None) => {}
                            _ => note(ignored, format!("{here}.{lk}: not a look-up")),
                        }
                    }
                    p.lookups = Some(lookups);
                }
                None => note(ignored, format!("{here}: not an object")),
            },
            "servers" => match v.as_object() {
                Some(s) => {
                    for (d, servers) in s.iter().take(DOMAINS_MAX) {
                        match domain(&Value::String(d.clone())) {
                            Some(dom) => {
                                if let Some(s) = domain_servers(servers, &format!("{here}.{d}"), ignored) {
                                    p.servers.insert(dom, s);
                                }
                            }
                            None => note(ignored, format!("{here}.{d}: not a mail domain")),
                        }
                    }
                }
                None => note(ignored, format!("{here}: not an object")),
            },
            "microsoft" => match v.as_object() {
                Some(m) => {
                    let id = text(m.get("clientId"), &format!("{here}.clientId"), 100, ignored);
                    let tenant = text(m.get("tenant"), &format!("{here}.tenant"), 200, ignored);
                    let id_ok = id.as_deref().is_some_and(|i| i.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-'));
                    let tenant_ok = tenant.as_deref().is_none_or(|t| t.bytes().all(|b| b.is_ascii_alphanumeric() || b"-.".contains(&b)));
                    match (id, id_ok && tenant_ok) {
                        (Some(client_id), true) => p.microsoft = Some(MicrosoftApp { client_id, tenant }),
                        _ => note(ignored, format!("{here}: a clientId (a GUID) and, if any, a tenant (an id or a domain)")),
                    }
                }
                None => note(ignored, format!("{here}: not an object")),
            },
            _ => note(ignored, format!("{here}: not an accounts setting")),
        }
    }
}
