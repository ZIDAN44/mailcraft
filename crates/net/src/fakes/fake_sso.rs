//! A single sign-on service of the tests' own (OpenID Connect, on this computer): its
//! description, a sign-in page that signs the user in at once (it sends the browser back to the
//! app with a code), and a token endpoint that checks the client id and PKCE, then hands out
//! access tokens a fake JMAP server takes ([`fake_jmap::Data::tokens`]).

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use serde_json::json;

use super::fake_jmap;
use crate::oauth::{b64url, unpct};

/// The client id it knows MailCraft by.
pub const CLIENT_ID: &str = "mailcraft-test";

#[derive(Default)]
pub struct Data {
    /// The fake JMAP server the tokens are for: each access token handed out is one it takes.
    pub jmap: Option<Arc<Mutex<fake_jmap::Data>>>,
    /// Whether it says it takes `offline_access`.
    pub offline_access: bool,
    /// Each sign-in page's query, and each form sent for tokens.
    pub pages: Vec<String>,
    pub forms: Vec<String>,
    /// (code, PKCE challenge, redirect address), until the code is used.
    codes: Vec<(String, String, String)>,
    refresh: Vec<String>,
    next: u32,
}

pub struct Sso {
    /// Its address, which is its issuer.
    pub base: String,
    pub data: Arc<Mutex<Data>>,
}

/// Start the service; it runs until the test process ends.
pub fn serve(data: Data) -> Sso {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://127.0.0.1:{}", l.local_addr().unwrap().port());
    let data = Arc::new(Mutex::new(data));
    let (shared, issuer) = (data.clone(), base.clone());
    std::thread::spawn(move || {
        for conn in l.incoming() {
            let Ok(s) = conn else { continue };
            let (shared, issuer) = (shared.clone(), issuer.clone());
            std::thread::spawn(move || handle(s, &shared, &issuer));
        }
    });
    Sso { base, data }
}

/// `k=v&…` (a query or a form), decoded.
fn params(s: &str) -> Vec<(String, String)> {
    s.split('&').filter_map(|kv| kv.split_once('=')).map(|(k, v)| (unpct(k), unpct(&v.replace('+', " ")))).collect()
}

fn answer(s: &mut impl Write, status: &str, extra: &str, body: &str) {
    let head = format!("HTTP/1.1 {status}\r\n{extra}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
    let _ = s.write_all(head.as_bytes());
    let _ = s.write_all(body.as_bytes());
}

fn handle(mut s: std::net::TcpStream, shared: &Arc<Mutex<Data>>, issuer: &str) {
    let mut reader = BufReader::new(s.try_clone().unwrap());
    let mut first = String::new();
    if reader.read_line(&mut first).unwrap_or(0) == 0 {
        return;
    }
    let mut len = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
            break;
        }
        if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            len = v.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0u8; len];
    let _ = reader.read_exact(&mut body);
    let target = first.split(' ').nth(1).unwrap_or("/").to_string();
    let (path, query) = target.split_once('?').unwrap_or((&target, ""));
    let mut d = shared.lock().unwrap();
    match path {
        "/.well-known/openid-configuration" => {
            let mut scopes = vec!["openid", "profile", "email"];
            if d.offline_access {
                scopes.push("offline_access");
            }
            let description = json!({
                "issuer": issuer,
                "authorization_endpoint": format!("{issuer}/authorize"),
                "token_endpoint": format!("{issuer}/token"),
                "scopes_supported": scopes,
                "response_types_supported": ["code"],
                "code_challenge_methods_supported": ["S256"],
            });
            answer(&mut s, "200 OK", "", &description.to_string());
        }
        // The sign-in page: the user is signed in at once, and the browser sent back with a code.
        "/authorize" => {
            d.pages.push(query.to_string());
            let q = params(query);
            let get = |k: &str| q.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone()).unwrap_or_default();
            let (redirect, state) = (get("redirect_uri"), get("state"));
            let then = if get("client_id") != CLIENT_ID || get("response_type") != "code" || get("code_challenge_method") != "S256" {
                format!("{redirect}?error=unauthorized_client&state={state}")
            } else {
                d.next += 1;
                let code = format!("code{}", d.next);
                d.codes.push((code.clone(), get("code_challenge"), redirect.clone()));
                format!("{redirect}?code={code}&state={state}")
            };
            answer(&mut s, "302 Found", &format!("Location: {then}\r\n"), "");
        }
        "/token" => {
            let form = String::from_utf8_lossy(&body).to_string();
            d.forms.push(form.clone());
            let f = params(&form);
            let get = |k: &str| f.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone()).unwrap_or_default();
            let granted = get("client_id") == CLIENT_ID
                && match get("grant_type").as_str() {
                    // The code once, from the address it was sent to, with the verifier its
                    // challenge was made from (RFC 7636 §4.6).
                    "authorization_code" => {
                        let challenge = b64url(ring::digest::digest(&ring::digest::SHA256, get("code_verifier").as_bytes()).as_ref());
                        let found = d.codes.iter().position(|(c, ch, r)| *c == get("code") && *ch == challenge && *r == get("redirect_uri"));
                        found.map(|i| d.codes.remove(i)).is_some()
                    }
                    "refresh_token" => d.refresh.contains(&get("refresh_token")),
                    _ => false,
                };
            if !granted {
                return answer(&mut s, "400 Bad Request", "", r#"{"error":"invalid_grant"}"#);
            }
            d.next += 1;
            let access = format!("at{}", d.next);
            if let Some(jmap) = &d.jmap {
                jmap.lock().unwrap().tokens.get_or_insert_with(Vec::new).push(access.clone());
            }
            let mut reply = json!({"access_token": access, "token_type": "Bearer", "expires_in": 3600, "scope": get("scope")});
            if get("grant_type") == "authorization_code" {
                let refresh = format!("rt{}", d.next);
                d.refresh.push(refresh.clone());
                reply["refresh_token"] = json!(refresh);
                let claims = json!({"iss": issuer, "sub": "u1", "email": fake_jmap::USER}).to_string();
                reply["id_token"] = json!(format!("e30.{}.sig", b64url(claims.as_bytes())));
            }
            answer(&mut s, "200 OK", "", &reply.to_string());
        }
        _ => answer(&mut s, "404 Not Found", "", "{}"),
    }
}
