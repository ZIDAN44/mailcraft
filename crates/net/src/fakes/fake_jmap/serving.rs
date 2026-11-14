//! The fake's connections: one request each (`Connection: close`), or an event stream held
//! open; the session, the API's method calls, blobs down and up.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;

use super::*;

fn respond(s: &mut impl Write, status: u16, body: &[u8], kind: &str) {
    let head = format!("HTTP/1.1 {status} X\r\nContent-Type: {kind}\r\nContent-Length: {}\r\n{CORS}Connection: close\r\n\r\n", body.len());
    let _ = s.write_all(head.as_bytes());
    let _ = s.write_all(body);
}

/// Any web page may call (a test server on this computer).
const CORS: &str = "Access-Control-Allow-Origin: *\r\nAccess-Control-Allow-Headers: Authorization, Content-Type, Accept\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\n";

/// A request: its first line, whether it came with the right sign-in, its body.
struct Request {
    first: String,
    authorized: bool,
    body: Vec<u8>,
}

/// The connection's request; `None` when it's closed before one came.
fn read_request(s: &TcpStream, auth: &str) -> Option<Request> {
    let mut reader = BufReader::new(s.try_clone().unwrap());
    let mut first = String::new();
    if reader.read_line(&mut first).unwrap_or(0) == 0 {
        return None;
    }
    let (mut len, mut authorized) = (0usize, false);
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
            break;
        }
        let lower = line.to_ascii_lowercase();
        if let Some(v) = lower.strip_prefix("content-length:") {
            len = v.trim().parse().unwrap_or(0);
        }
        if line.split_once(':').is_some_and(|(k, v)| k.eq_ignore_ascii_case("authorization") && v.trim() == auth) {
            authorized = true;
        }
    }
    let mut body = vec![0u8; len];
    let _ = reader.read_exact(&mut body);
    Some(Request { first, authorized, body })
}

/// One connection: one request (`Connection: close`), or an event stream held open.
pub(super) fn handle(mut s: TcpStream, shared: &Arc<Mutex<Data>>, auth: &str) {
    let Some(r) = read_request(&s, auth) else { return };
    // A browser's CORS preflight carries no credentials.
    if r.first.starts_with("OPTIONS ") {
        respond(&mut s, 204, b"", "text/plain");
        return;
    }
    if !r.authorized {
        respond(&mut s, 401, b"{}", "application/json");
        return;
    }
    let path = r.first.split(' ').nth(1).unwrap_or("/").to_string();
    if path.starts_with("/events/") {
        stream_events(s, shared, &path);
        return;
    }
    let (status, body, kind) = shared.lock().unwrap().answer(&path, &r.body);
    respond(&mut s, status, &body, kind);
}

/// Push (RFC 8620 §7.3): a state change sent as each comes, a ping each second between.
fn stream_events(mut s: TcpStream, shared: &Arc<Mutex<Data>>, path: &str) {
    let mut d = shared.lock().unwrap();
    // Kept for the test to fail on, and refused: not asserted here, which would
    // poison the data's lock and leave every request after it unanswered.
    if !path.contains("types=%2A") && !path.contains("types=*") {
        d.violations.push(format!("an event stream not asked for every type: {path}"));
        drop(d);
        respond(&mut s, 400, br#"{"type": "urn:ietf:params:jmap:error:notRequest"}"#, "application/json");
        return;
    }
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    d.listeners.push(tx);
    drop(d);
    let _ = s.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n{CORS}Transfer-Encoding: chunked\r\n\r\n").as_bytes());
    loop {
        let msg = match rx.recv_timeout(std::time::Duration::from_secs(1)) {
            Ok(m) => format!("event: state\r\ndata: {m}\r\n\r\n"),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => ": keep-alive\n\nevent: ping\ndata: {\"interval\":1}\n\n".to_string(),
            Err(_) => break,
        };
        if s.write_all(format!("{:x}\r\n{msg}\r\n", msg.len()).as_bytes()).is_err() {
            break;
        }
    }
}

impl Data {
    /// A request signed in: (status, body, content type).
    fn answer(&mut self, path: &str, body: &[u8]) -> (u16, Vec<u8>, &'static str) {
        if path == "/.well-known/jmap" {
            let session = json!({
                "capabilities": {"urn:ietf:params:jmap:core": {"maxObjectsInGet": MAX_GET}, "urn:ietf:params:jmap:mail": {}, "urn:ietf:params:jmap:submission": {}, "urn:ietf:params:jmap:quota": {}, "urn:ietf:params:jmap:vacationresponse": {}},
                "accounts": {ACCOUNT: {"name": USER}},
                "primaryAccounts": {"urn:ietf:params:jmap:mail": ACCOUNT, "urn:ietf:params:jmap:submission": ACCOUNT},
                "username": USER,
                "apiUrl": "/api/",
                "downloadUrl": "/download/{accountId}/{blobId}/{name}?accept={type}",
                "uploadUrl": "/upload/{accountId}/",
                "eventSourceUrl": "/events/?types={types}&closeafter={closeafter}&ping={ping}",
                "state": "1",
            });
            (200, session.to_string().into_bytes(), "application/json")
        } else if path == "/api/" {
            (200, self.api(body).to_string().into_bytes(), "application/json")
        } else if let Some(rest) = path.strip_prefix(&format!("/download/{ACCOUNT}/")) {
            let blob = rest.split('/').next().unwrap_or_default();
            match self.blobs.get(blob) {
                Some(raw) => (200, raw.clone(), "message/rfc822"),
                None => (404, Vec::new(), "text/plain"),
            }
        } else if path == format!("/upload/{ACCOUNT}/") {
            let blob = self.id("B");
            self.blobs.insert(blob.clone(), body.to_vec());
            let r = json!({"accountId": ACCOUNT, "blobId": blob, "type": "message/rfc822", "size": body.len()});
            (201, r.to_string().into_bytes(), "application/json")
        } else {
            (404, Vec::new(), "text/plain")
        }
    }

    /// The API: each method call in turn, each able to name an earlier one's result.
    fn api(&mut self, body: &[u8]) -> Value {
        let req: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
        let mut earlier = BTreeMap::new();
        let mut out = Vec::new();
        for call in req["methodCalls"].as_array().cloned().unwrap_or_default() {
            let (name, args, id) =
                (call[0].as_str().unwrap_or_default().to_string(), call[1].clone(), call[2].as_str().unwrap_or_default().to_string());
            let (rname, result) = self.method(&name, &args, &earlier);
            earlier.insert(id.clone(), result.clone());
            out.push(json!([rname, result, id]));
        }
        json!({"methodResponses": out, "sessionState": "1"})
    }
}
