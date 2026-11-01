//! A report that came in by mail is a `multipart/report` again, its status first; anything that
//! doesn't fit one stays `multipart/mixed` with every part.

use super::super::item::{File, Item};
use super::super::rebuild::to_raw;
use super::super::report::{as_report, report_type};
use super::*;

const STATUS: &[u8] =
    b"Reporting-MTA: dns; mx.studio.example\r\n\r\nFinal-Recipient: rfc822; gone@example.com\r\nAction: failed\r\nStatus: 5.1.1\r\n";

fn header(content_type: &str) -> String {
    format!("From: Mail Delivery <postmaster@studio.example>\r\nTo: me@studio.example\r\nSubject: Undeliverable\r\nContent-Type: {content_type}\r\n")
}

const BOUNCE: &str = "multipart/report; report-type=delivery-status;\r\n\tboundary=\"b1\"";

fn status() -> File {
    File::Data { name: "details.txt".into(), mime: "message/delivery-status".into(), cid: String::new(), data: STATUS.to_vec() }
}

fn returned() -> File {
    File::Message(Box::new(Item { subject: "Lunch".into(), body: "See you at noon.".into(), ..Default::default() }))
}

fn bounce(content_type: &str, files: Vec<File>) -> Item {
    Item { headers: header(content_type), body: "Your message couldn't be delivered.".into(), files, ..Default::default() }
}

/// The rebuilt top-level Content-Type, its folded lines joined.
fn top_type(raw: &[u8]) -> String {
    let text = String::from_utf8_lossy(raw);
    let head = text.split("\r\n\r\n").next().unwrap();
    let start = head.find("\r\nContent-Type: ").unwrap() + 2;
    head[start..]
        .split("\r\n")
        .enumerate()
        .take_while(|(i, l)| *i == 0 || l.starts_with([' ', '\t']))
        .map(|(_, l)| l.trim())
        .collect::<Vec<_>>()
        .join(" ")
}

/// A bounce whose files are its status and the message it returns (in the file's order, the
/// message first) is a report again: status first, its boundary kept, and read back with both
/// files whole.
#[test]
fn a_bounce_is_a_report_again() {
    let raw = to_raw(bounce(BOUNCE, vec![returned(), status()]), "f@x").unwrap();
    let top = top_type(&raw);
    assert!(top.starts_with("Content-Type: multipart/report; report-type=delivery-status;"), "{top}");
    assert!(top.contains("boundary="), "{top}");
    assert_eq!(top.matches("Content-Type").count(), 1, "{top}");
    let p = mime::parse(&raw).unwrap();
    let got: Vec<&str> = p.attachments.iter().map(|a| a.mime.as_str()).collect();
    assert_eq!(got, ["message/delivery-status", "message/rfc822"]);
    assert_eq!(mime::attachment_data(&raw, 0).unwrap(), STATUS);
    assert!(p.text.contains("couldn't be delivered"), "{}", p.text);
    // The status is readable as it is (RFC 3464).
    assert!(String::from_utf8_lossy(&raw).contains("Action: failed\r\nStatus: 5.1.1"));
}

/// A read receipt with only its status, a quoted report-type in another case, a returned header
/// instead of a message, an HTML body: each is a report of its kind.
#[test]
fn a_report_of_every_kind() {
    let receipt = File::Data {
        name: "MDNPart2.txt".into(),
        mime: "Message/Disposition-Notification".into(),
        cid: String::new(),
        data: b"Disposition: manual-action/MDN-sent-manually; displayed\r\n".to_vec(),
    };
    let raw = to_raw(bounce("Multipart/Report; Report-Type=\"Disposition-Notification\"; boundary=b2", vec![receipt]), "f@x").unwrap();
    assert!(top_type(&raw).starts_with("Content-Type: multipart/report; report-type=disposition-notification;"), "{}", top_type(&raw));

    let headers =
        File::Data { name: "headers.txt".into(), mime: "text/rfc822-headers".into(), cid: String::new(), data: b"Subject: Lunch\r\n".to_vec() };
    let mut it = bounce(BOUNCE, vec![headers, status()]);
    it.html = Some("<p>Your message couldn't be delivered.</p>".into());
    let raw = to_raw(it, "f@x").unwrap();
    assert!(top_type(&raw).starts_with("Content-Type: multipart/report; report-type=delivery-status;"), "{}", top_type(&raw));
    let p = mime::parse(&raw).unwrap();
    let got: Vec<&str> = p.attachments.iter().map(|a| a.mime.as_str()).collect();
    assert_eq!(got, ["message/delivery-status", "text/rfc822-headers"]);
}

/// What doesn't fit a report stays mixed, every file kept: another file beside the status, two
/// statuses, two returned messages, no status, no or a broken report-type, another kind of
/// message, and an item without its own header.
#[test]
fn what_isnt_a_report_stays_mixed() {
    let pdf = || File::Data { name: "a.pdf".into(), mime: "application/pdf".into(), cid: String::new(), data: b"%PDF".to_vec() };
    let cases: Vec<(String, Vec<File>)> = vec![
        (BOUNCE.into(), vec![status(), pdf()]),
        (BOUNCE.into(), vec![status(), status()]),
        (BOUNCE.into(), vec![status(), returned(), returned()]),
        (BOUNCE.into(), vec![returned()]),
        ("multipart/report; boundary=b1".into(), vec![status()]),
        ("multipart/report; report-type=\"delivery status\"; boundary=b1".into(), vec![status()]),
        (format!("multipart/report; report-type={}; boundary=b1", "x".repeat(65)), vec![status()]),
        ("multipart/mixed; boundary=b1".into(), vec![status(), returned()]),
    ];
    for (content_type, files) in cases {
        let n = files.len();
        let raw = to_raw(bounce(&content_type, files), "f@x").unwrap();
        assert!(top_type(&raw).starts_with("Content-Type: multipart/mixed;"), "{content_type}: {}", top_type(&raw));
        assert_eq!(mime::parse(&raw).unwrap().attachments.len(), n, "{content_type}");
    }
    let headerless = Item { body: "Undeliverable".into(), files: vec![returned(), status()], ..Default::default() };
    assert!(top_type(&to_raw(headerless, "f@x").unwrap()).starts_with("Content-Type: multipart/mixed;"));
}

/// The header is hostile: its Content-Type is read only from its own field (folded or not), and
/// a message whose top isn't mixed is left as it is.
#[test]
fn the_report_type_is_read_from_its_own_field() {
    let mut files = vec![returned(), status()];
    let folded = header("multipart/report;\r\n report-type=delivery-status;\r\n boundary=b1");
    assert_eq!(report_type(&folded, &mut files).as_deref(), Some("delivery-status"));
    assert!(matches!(files[0], File::Data { .. }), "status first");
    let elsewhere =
        "Subject: multipart/report; report-type=delivery-status\r\nX-Content-Type: multipart/report; report-type=x\r\nContent-Type: text/plain\r\n";
    assert_eq!(report_type(elsewhere, &mut files), None);
    assert_eq!(report_type("", &mut files), None);
    assert_eq!(report_type("Content-Type: multipart/report; report-type=", &mut files), None);
    let plain = b"Subject: Hi\r\nContent-Type: text/plain\r\n\r\nHi".to_vec();
    assert_eq!(as_report(plain.clone(), "delivery-status"), plain);
    assert_eq!(as_report(b"no header end".to_vec(), "delivery-status"), b"no header end");
}

proptest::proptest! {
    /// Whatever the header says and whatever the files are (a report's parts or others, any data,
    /// a returned message or not), the rebuilt message reads back with every file, each whole.
    #[test]
    fn every_file_survives_a_report(
        kind in proptest::sample::select(vec!["multipart/report", "Multipart/Report", "multipart/mixed", "text/plain", ""]),
        param in "[a-z.+-]{1,12}|[ -~]{0,40}",
        files in proptest::collection::vec(
            (
                proptest::sample::select(vec!["message/delivery-status", "Message/Disposition-Notification", "message/feedback-report", "text/rfc822-headers", "application/pdf", ""]),
                proptest::collection::vec(proptest::prelude::any::<u8>(), 0..300),
                proptest::prelude::any::<bool>(),
            ),
            0..4,
        ),
    ) {
        let make = || -> Vec<File> {
            files
                .iter()
                .map(|(mime, data, message)| {
                    if *message { returned() } else { File::Data { name: "f.bin".into(), mime: (*mime).into(), cid: String::new(), data: data.clone() } }
                })
                .collect()
        };
        let mut sorted = make();
        let content_type = format!("{kind}; report-type={param}; boundary=b1");
        let kept = report_type(&header(&content_type), &mut sorted);
        let raw = to_raw(bounce(&content_type, make()), "f@x").unwrap();
        let p = mime::parse(&raw).unwrap();
        proptest::prop_assert_eq!(p.attachments.len(), sorted.len(), "{:?}", kept);
        for (i, f) in sorted.iter().enumerate() {
            if let File::Data { data, .. } = f {
                proptest::prop_assert_eq!(&mime::attachment_data(&raw, i).unwrap(), data, "{}", i);
            }
        }
    }
}
