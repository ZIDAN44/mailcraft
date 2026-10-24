//! Engine tests: what keeps the user safe as mail comes and goes (photos' places, lookalike
//! addresses, the checks before sending, trackers, hidden text, replies still owed).

use super::*;

/// A JPEG whose EXIF block has a GPS block with a latitude (51° 30' 41.23").
fn geotagged_jpeg() -> Vec<u8> {
    let mut tiff = b"MM\0\x2a\0\0\0\x08".to_vec();
    tiff.extend_from_slice(&[0, 1, 0x88, 0x25, 0, 4, 0, 0, 0, 1, 0, 0, 0, 26, 0, 0, 0, 0]);
    tiff.extend_from_slice(&[0, 1, 0, 2, 0, 5, 0, 0, 0, 3, 0, 0, 0, 44, 0, 0, 0, 0]);
    for v in [51u32, 1, 30, 1, 4123, 100] {
        tiff.extend_from_slice(&v.to_be_bytes());
    }
    let body = [b"Exif\0\0".as_slice(), &tiff].concat();
    let mut j = vec![0xFF, 0xD8, 0xFF, 0xE1];
    j.extend_from_slice(&u16::try_from(body.len() + 2).unwrap().to_be_bytes());
    j.extend_from_slice(&body);
    j.extend_from_slice(&[0xFF, 0xDA, 0, 2, 1, 2, 0xFF, 0xD9]);
    j
}

/// The sent copy of the message with `subject`, as it was sent.
fn sent_attachment(s: &mut Session, subject: &str) -> Vec<u8> {
    s.run("folder.open", &json!({"account": "sample", "folder": "Sent"})).unwrap();
    let m = s.list.iter().find(|m| m.subject == subject).unwrap().key.clone();
    let raw = s.store.as_ref().unwrap().read_body(&m).unwrap();
    mailcraft_mime::attachment_data(&raw, 0).unwrap()
}

#[test]
fn photos_are_sent_without_where_they_were_taken() {
    let mut s = sample_session("photo-place");
    let photo = geotagged_jpeg();
    let latitude = 4123u32.to_be_bytes();
    for (subject, keep) in [("Harbour", false), ("Harbour again", true)] {
        s.run("view.photoLocation", &json!({"on": keep})).unwrap();
        s.run("mail.new", &json!({})).unwrap();
        s.run("compose.update", &json!({"to": "ana@soundlab.example", "subject": subject})).unwrap();
        s.run("compose.attach", &json!({"name": "harbour.jpg", "data": photo})).unwrap();
        s.run("compose.send", &json!({})).unwrap();
        let sent = sent_attachment(&mut s, subject);
        assert_eq!(sent.len(), photo.len(), "the photo is all there");
        assert_eq!(sent.windows(4).any(|w| w == latitude), keep, "{subject}");
    }
    done(s);
}

/// Put a message in the sample Inbox and open it.
fn deliver(s: &mut Session, n: u32, from: (&str, &str), subject: &str, html: Option<&str>) -> mailcraft_model::MessageKey {
    let d = mailcraft_mime::Draft {
        to: vec![mailcraft_model::EmailAddress::new("Riley", "riley@studio.example")],
        subject: subject.into(),
        body: "Please look at this.".into(),
        html: html.map(str::to_string),
        ..Default::default()
    };
    let raw = mailcraft_mime::build(&d, &mailcraft_model::EmailAddress::new(from.0, from.1), NOW, &format!("<s{n}@x>")).unwrap();
    let store = s.store.clone().unwrap();
    let mut inbox = store.load_folder("sample", "INBOX").unwrap();
    let key = mailcraft_model::MessageKey { account: "sample".into(), folder: "INBOX".into(), validity: inbox.validity, uid: 9100 + n };
    inbox.messages.insert(0, mailcraft_mime::summary_from_full(&raw, key.clone(), mailcraft_model::Flags::default(), 0, NOW));
    store.save_folder("sample", "INBOX", &inbox).unwrap();
    store.write_body(&key, &raw).unwrap();
    s.apply(Event::FolderChanged { account: "sample".into(), folder: "INBOX".into() });
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    s.run("list.open", &json!({"key": key.to_id()})).unwrap();
    key
}

#[test]
fn senders_passing_for_someone_else_are_pointed_out() {
    use mailcraft_model::lookalike::SenderWarning;
    let mut s = sample_session("lookalike");
    let warning = |s: &Session| s.reading.as_ref().and_then(|r| r.warning.clone());
    // The sample mailbox writes to the print shop: a lookalike of its domain is caught.
    deliver(&mut s, 1, ("Print Shop", "orders@printsh0p.example"), "Your invoice", None);
    assert_eq!(warning(&s), Some(SenderWarning::Lookalike { domain: "printsh0p.example".into(), like: "printshop.example".into() }));
    // So is the shop's address in the name of someone else's.
    deliver(&mut s, 2, ("orders@printshop.example", "billing@invoices-now.example"), "Your invoice", None);
    assert!(matches!(warning(&s), Some(SenderWarning::NameShowsOther { shown, .. }) if shown == "orders@printshop.example"));
    // The shop itself, and people the mailbox has never dealt with, are fine.
    deliver(&mut s, 3, ("Print Shop", "orders@printshop.example"), "Your proof", None);
    assert_eq!(warning(&s), None);
    deliver(&mut s, 4, ("Someone", "hello@orchestra.example"), "Hello", None);
    assert_eq!(warning(&s), None);
    // A message in its own window says the same.
    let k = deliver(&mut s, 5, ("Print Shop", "orders@printsh0p.example"), "Again", None);
    s.run("message.popOut", &json!({"key": k.to_id()})).unwrap();
    assert!(s.windows.iter().any(|w| w.key == k && w.warning.is_some()));
    done(s);
}

#[test]
fn sending_holds_for_a_lookalike_address_or_many_strangers() {
    let mut s = sample_session("send-checks");
    let send = |s: &mut Session, p: Value| s.run("compose.send", &p).unwrap();
    // A typo of the print shop's domain (the sample mailbox writes to it): held, and said why.
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "orders@printsh0p.example", "subject": "Proof approved"})).unwrap();
    let held = send(&mut s, json!({}));
    assert_eq!(held["check"], json!([{"kind": "lookalike", "address": "orders@printsh0p.example", "like": "printshop.example"}]));
    assert!(s.compose.as_ref().is_some_and(|c| !c.sending), "nothing went");
    // Sent anyway once looked at.
    assert!(send(&mut s, json!({"checked": true})).get("check").is_none());
    assert!(s.compose.is_none());

    // Nine friends at a mail service, and people at two firms: each sees the others' addresses.
    let friends: Vec<String> = (1..=9).map(|i| format!("friend{i}@gmail.com")).collect();
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": friends.join("; "), "cc": "a@firm-one.example; b@firm-one.example", "subject": "Party"})).unwrap();
    assert_eq!(send(&mut s, json!({}))["check"], json!([{"kind": "exposed", "count": 10}]));
    // Everyone in Bcc, the sender in To: nothing to ask.
    s.run("compose.hideRecipients", &json!({})).unwrap();
    let c = s.compose.clone().unwrap();
    assert_eq!(c.to, "Riley Moreno <riley@studio.example>; ");
    assert!(c.cc.is_empty() && c.bcc.contains("friend9@gmail.com") && c.bcc.contains("b@firm-one.example"), "{c:?}");
    assert!(send(&mut s, json!({})).get("check").is_none());

    // Colleagues and someone known: nothing held.
    s.run("mail.new", &json!({})).unwrap();
    let team: Vec<String> = (1..=12).map(|i| format!("p{i}@studio.example")).collect();
    s.run("compose.update", &json!({"to": format!("{}; orders@printshop.example", team.join("; ")), "subject": "Wrap"})).unwrap();
    assert!(send(&mut s, json!({})).get("check").is_none());
    done(s);
}

#[test]
fn tracking_pictures_stay_unloaded_when_pictures_are_shown() {
    let mut s = sample_session("trackers");
    let html = r#"<p>New prints are in. <a href="https://shop.example/prints?utm_source=news&amp;fbclid=x">See them</a>
        <img src="https://shop.example/banner.png" width="600" alt="Prints">
        <img src="https://shop.example/o.gif?u=riley" width="1" height="1"></p>"#;
    deliver(&mut s, 6, ("Shop", "news@shop.example"), "New prints", Some(html));
    let p = s.reading.as_ref().unwrap().parsed.clone().unwrap();
    assert_eq!(p.trackers.pictures, ["https://shop.example/o.gif?u=riley"]);
    assert_eq!(p.trackers.links, 1);
    s.run("message.showPictures", &json!({})).unwrap();
    let asked: Vec<String> = s.take_jobs().into_iter().filter_map(|j| if let Job::FetchUrl { url } = j { Some(url) } else { None }).collect();
    assert_eq!(asked, ["https://shop.example/banner.png"], "the tracking picture is never asked for");
    done(s);
}
