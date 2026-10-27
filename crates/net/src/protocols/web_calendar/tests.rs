//! Reading a calendar's link from a server on this computer: the calendar with its version, not
//! again while unchanged, a server's refusal, and an answer too large to keep.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use super::*;

const ICS: &str =
    "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:a\r\nDTSTART:20270101T090000Z\r\nSUMMARY:Stand-up\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

/// A server answering each request by its path, keeping the requests' header lines.
fn serve() -> (String, Arc<Mutex<Vec<Vec<String>>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let kept = seen.clone();
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(mut conn) = conn else { continue };
            let mut lines = Vec::new();
            let mut reader = BufReader::new(conn.try_clone().unwrap());
            loop {
                let mut l = String::new();
                if reader.read_line(&mut l).unwrap_or(0) == 0 || l.trim().is_empty() {
                    break;
                }
                lines.push(l.trim().to_string());
            }
            let first = lines.first().cloned().unwrap_or_default();
            let cached = lines.iter().any(|l| l.eq_ignore_ascii_case("If-None-Match: \"v1\""));
            let answer = if first.contains("/cal.ics") && cached {
                "HTTP/1.1 304 Not Modified\r\nETag: \"v1\"\r\n\r\n".to_string()
            } else if first.contains("/cal.ics") {
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/calendar\r\nETag: \"v1\"\r\nLast-Modified: Wed, 28 Oct 2026 10:00:00 GMT\r\nContent-Length: {}\r\n\r\n{ICS}",
                    ICS.len()
                )
            } else if first.contains("/moved") {
                "HTTP/1.1 301 Moved\r\nLocation: /cal.ics\r\nContent-Length: 0\r\n\r\n".to_string()
            } else if first.contains("/huge") {
                format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\nBEGIN", MAX_CALENDAR + 1)
            } else {
                "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_string()
            };
            kept.lock().unwrap().push(lines);
            let _ = conn.write_all(answer.as_bytes());
        }
    });
    (base, seen)
}

#[test]
fn a_calendar_is_read_from_its_link_and_not_again_while_unchanged() {
    let (base, seen) = serve();
    let url = format!("{base}/cal.ics");
    let Fetched::Calendar { ics, etag, modified } = fetch(&url, "", "").unwrap() else { panic!("not read") };
    assert_eq!((ics.as_str(), etag.as_str(), modified.as_str()), (ICS, "\"v1\"", "Wed, 28 Oct 2026 10:00:00 GMT"));
    assert_eq!(fetch(&url, &etag, &modified).unwrap(), Fetched::Unchanged);
    {
        let seen = seen.lock().unwrap();
        let second = &seen[1];
        assert!(second.iter().any(|l| l == "If-Modified-Since: Wed, 28 Oct 2026 10:00:00 GMT"), "{second:?}");
        assert!(seen[0].iter().any(|l| l.starts_with("Accept: text/calendar")), "{:?}", seen[0]);
    }
    // Moved: followed.
    assert!(matches!(fetch(&format!("{base}/moved"), "", ""), Ok(Fetched::Calendar { .. })));
    // A version that can't go in a header isn't sent.
    assert!(matches!(fetch(&url, "\"v1\"\r\nX-Evil: 1", ""), Ok(Fetched::Calendar { .. })));
    assert!(seen.lock().unwrap().iter().all(|r| r.iter().all(|l| !l.starts_with("X-Evil"))));
    // Refused, and too large: said, nothing kept.
    let e = fetch(&format!("{base}/gone.ics"), "", "").unwrap_err().to_string();
    assert!(e.contains("404"), "{e}");
    assert!(fetch(&format!("{base}/huge"), "", "").unwrap_err().to_string().contains("larger"));
    // Not a web address at all.
    assert!(fetch("webcal://x.example/a.ics", "", "").is_err());
}
