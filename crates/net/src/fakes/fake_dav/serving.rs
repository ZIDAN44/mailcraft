//! The fake server's requests: each read, signed in (Basic, or Digest when set to), and answered.

use std::io::BufReader;
use std::net::TcpStream;

use super::*;
use crate::fakes::http;

/// A request: what the server heeds of it.
pub(super) struct Request {
    method: String,
    path: String,
    /// The `Authorization` header, as given.
    given: String,
    /// Signed in with the right Basic credentials.
    basic_ok: bool,
    if_match: Option<String>,
    if_none: bool,
    body: String,
}

/// An answer: its status line's code and text, its headers, its body.
pub(super) type Reply = (&'static str, Vec<(&'static str, String)>, String);

fn reply(status: &'static str) -> Reply {
    (status, Vec::new(), String::new())
}

/// The connection's request; `None` when it's closed before one came.
pub(super) fn read_request(s: &TcpStream, auth: &str) -> Option<Request> {
    let r = http::read_request(&mut BufReader::new(s.try_clone().ok()?))?;
    let given = r.header("authorization").unwrap_or_default().to_string();
    Some(Request {
        basic_ok: given == auth,
        given,
        if_match: r.header("if-match").map(str::to_string),
        if_none: r.header("if-none-match").is_some(),
        body: String::from_utf8_lossy(&r.body).to_string(),
        method: r.method,
        path: r.target,
    })
}

impl Data {
    /// One request, logged: refused without the right sign-in (a Digest challenge, stale when
    /// the password was right for a nonce gone), else answered.
    pub(super) fn answer(&mut self, r: &Request) -> Reply {
        self.log.push(format!("{} {}", r.method, r.path));
        if !r.given.is_empty() {
            self.offered += 1;
        }
        let mut authorized = r.basic_ok;
        if let Some(nonce) = self.digest.clone() {
            authorized = digest_ok(&r.given, &r.method, &r.path, &nonce);
            if !authorized {
                // The right password with a nonce no longer current: stale, as RFC 7616 says.
                let old = crate::dav::digest::challenges(&r.given)
                    .into_iter()
                    .next()
                    .and_then(|(_, p)| p.into_iter().find(|(k, _)| k == "nonce"))
                    .map(|(_, v)| v);
                let stale = old.is_some_and(|o| o != nonce && digest_ok(&r.given, &r.method, &r.path, &o));
                let challenge = format!(
                    "Basic realm=\"fake\", Digest realm=\"fake\", qop=\"auth\", nonce=\"{nonce}\"{}",
                    if stale { ", stale=true" } else { "" }
                );
                self.refused += 1;
                return ("401 Unauthorized", vec![("WWW-Authenticate", challenge)], String::new());
            }
        }
        if !authorized {
            return ("401 Unauthorized", vec![("WWW-Authenticate", "Basic realm=\"fake\"".to_string())], String::new());
        }
        if matches!(r.method.as_str(), "PROPFIND" | "REPORT") && self.unreadable.iter().any(|p| r.path.starts_with(p.as_str())) {
            return reply("403 Forbidden");
        }
        match r.method.as_str() {
            "OPTIONS" => {
                let dav = if self.schedules { "1, 2, access-control, calendar-access, calendar-auto-schedule" } else { "1, 2, 3, calendar-access" };
                ("200 OK", vec![("DAV", dav.into())], String::new())
            }
            "PROPFIND" => self.propfind(&r.path),
            "REPORT" => self.report(&r.path, &r.body),
            "PUT" => self.put_item(r),
            "DELETE" => self.delete_item(&r.path),
            _ => reply("404 Not Found"),
        }
    }

    /// The well-known redirect, the principal and its homes, the collections and their items.
    fn propfind(&self, path: &str) -> Reply {
        let found = |body: String| ("207 Multi-Status", Vec::new(), body);
        match path {
            "/.well-known/caldav" | "/.well-known/carddav" => ("301 Moved", vec![("Location", "/dav/".into())], String::new()),
            "/dav/" => {
                found(multistatus(&[response("/dav/", "<d:current-user-principal><d:href>/dav/principals/me/</d:href></d:current-user-principal>")]))
            }
            "/dav/principals/me/" => {
                let props = "<c:calendar-home-set><d:href>/dav/cal/</d:href></c:calendar-home-set><card:addressbook-home-set><d:href>/dav/card/</d:href></card:addressbook-home-set>";
                found(multistatus(&[response("/dav/principals/me/", props)]))
            }
            "/dav/cal/" | "/dav/card/" => {
                let (kind, child, name) = if path == "/dav/cal/" { ("<c:calendar/>", CAL, "Work") } else { ("<card:addressbook/>", BOOK, "Studio") };
                let col = format!(
                    "<d:resourcetype><d:collection/>{kind}</d:resourcetype><d:displayname>{name}</d:displayname><cs:getctag>{}</cs:getctag><d:current-user-privilege-set><d:privilege><d:write/></d:privilege></d:current-user-privilege-set>",
                    self.ctag
                );
                found(multistatus(&[response(path, "<d:resourcetype><d:collection/></d:resourcetype>"), response(child, &col)]))
            }
            p if p == CAL || p == BOOK => {
                let mut rs = vec![response(p, "<d:resourcetype><d:collection/></d:resourcetype>")];
                for (href, (etag, _)) in self.items.iter().filter(|(h, _)| h.starts_with(p)) {
                    rs.push(response(href, &format!("<d:getetag>{}</d:getetag><d:resourcetype/>", esc(etag))));
                }
                found(multistatus(&rs))
            }
            _ => reply("404 Not Found"),
        }
    }

    /// A calendar-query by uid, or a multiget of the items named.
    fn report(&self, p: &str, body: &str) -> Reply {
        // A control character XML 1.0 doesn't allow: not well-formed, refused, as Radicale does.
        if body.contains(|c: char| c < ' ' && !matches!(c, '\t' | '\n' | '\r')) {
            return reply("400 Bad Request");
        }
        if p == CAL && body.contains("calendar-query") {
            // By uid: its text-match finds the uid inside longer ones too, as RFC 4791's does.
            let query = crate::xml::parse(body);
            let wanted = query.find("text-match").map(|t| t.text.as_str());
            let mut rs = Vec::new();
            for (href, (etag, data)) in self.items.iter().filter(|(h, _)| h.starts_with(p)) {
                if wanted.is_some_and(|w| uid_of(data).is_some_and(|u| u.contains(w))) {
                    rs.push(response(href, &format!("<d:getetag>{}</d:getetag>", esc(etag))));
                }
            }
            return ("207 Multi-Status", Vec::new(), multistatus(&rs));
        }
        if p != CAL && p != BOOK {
            return reply("404 Not Found");
        }
        let field = if p == CAL { "c:calendar-data" } else { "card:address-data" };
        let mut rs = Vec::new();
        for href in body.split("<d:href>").skip(1).filter_map(|x| x.split("</d:href>").next()) {
            if let Some((etag, data)) = self.items.get(href) {
                rs.push(response(href, &format!("<d:getetag>{}</d:getetag><{field}>{}</{field}>", esc(etag), esc(data))));
            }
        }
        ("207 Multi-Status", Vec::new(), multistatus(&rs))
    }

    /// An item stored, as its conditions allow: refused where the user may not change it, or
    /// (409) where its uid is another item's in the collection, as SOGo does.
    fn put_item(&mut self, r: &Request) -> Reply {
        let p = r.path.as_str();
        if self.forbidden.iter().any(|f| f == p) {
            return reply("403 Forbidden");
        }
        let taken = !self.items.contains_key(p)
            && uid_of(&r.body).is_some_and(|u| {
                let folder = p.rsplit_once('/').map_or("", |(f, _)| f);
                self.items.iter().any(|(h, (_, v))| h.rsplit_once('/').map_or("", |(f, _)| f) == folder && uid_of(v).as_ref() == Some(&u))
            });
        if taken {
            return ("409 Conflict", Vec::new(), "<D:error xmlns:D=\"DAV:\">Event UID already in use.</D:error>".into());
        }
        let current = self.items.get(p).map(|(e, _)| e.clone());
        let ok = match (&current, &r.if_match, r.if_none) {
            (Some(_), _, true) => self.overwrites,
            (Some(c), Some(m), _) => c == m,
            (None, Some(_), _) => false,
            _ => true,
        };
        if ok {
            let etag = self.put(p, &r.body);
            ("201 Created", vec![("ETag", etag)], String::new())
        } else {
            reply("412 Precondition Failed")
        }
    }

    fn delete_item(&mut self, p: &str) -> Reply {
        if self.forbidden.iter().any(|f| f == p) {
            return reply("403 Forbidden");
        }
        if self.items.remove(p).is_some() {
            self.ctag += 1;
            reply("204 No Content")
        } else {
            reply("404 Not Found")
        }
    }
}
