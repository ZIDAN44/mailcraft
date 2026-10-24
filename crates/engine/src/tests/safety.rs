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
