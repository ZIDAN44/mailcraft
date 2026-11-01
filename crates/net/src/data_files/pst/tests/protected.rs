//! Signed and encrypted items: put back as the messages they were sent as.

use mailcraft_mime::smime::{self, Protection};
use mailcraft_model::EmailAddress;

use super::super::item::{File, Item};
use super::super::rebuild::{message, summary_of};

/// What MailCraft signs, as the entity a signature covers.
const ENTITY: &[u8] = b"Content-Type: text/plain; charset=utf-8\r\n\r\nApproved.\r\n";

/// An S/MIME item as Outlook keeps it: its content as its one file.
fn smime_item(class: &str, mime: &str, content: &[u8]) -> Item {
    Item {
        class: class.into(),
        subject: "Proof".into(),
        from: EmailAddress::new("Riley", "riley@studio.example"),
        to: vec![EmailAddress::new("", "ana@posteo.example")],
        date: 1_700_000_000,
        body: "Approved.".into(),
        files: vec![File::Data { name: "smime.p7m".into(), mime: mime.into(), cid: String::new(), data: content.to_vec() }],
        ..Default::default()
    }
}

/// The `multipart/signed` entity Outlook keeps for a clear-signed message.
fn signed_entity() -> Vec<u8> {
    smime::signed_message(b"", ENTITY, &[0xAB; 64], "=_s1")
}

/// A CMS `ContentInfo` of this type (2 signed data, 3 enveloped data), its content made up.
fn cms(kind: u8) -> Vec<u8> {
    let mut b = vec![0x30, 0x80, 0x06, 0x09, 0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x07, kind, 0xA0, 0x80];
    b.extend([0x5A; 40]);
    b
}

/// The names of a message's files, read back from its source.
fn files_of(raw: &[u8]) -> Vec<String> {
    mailcraft_mime::parse(raw).map(|p| p.attachments.into_iter().map(|a| a.name).collect()).unwrap_or_default()
}

/// A clear-signed item comes back as the `multipart/signed` message it was, its signature over
/// exactly the bytes that were signed, under the item's own header fields; it carries no file.
#[test]
fn a_signed_item_comes_back_signed() {
    let it = smime_item("IPM.Note.SMIME.MultipartSigned", "multipart/signed", &signed_entity());
    let (raw, glance) = message(it, "id@x.invalid").unwrap();
    assert_eq!(smime::protection(&raw), Protection::Signed { content: ENTITY.to_vec(), signature: vec![0xAB; 64] });
    let text = String::from_utf8_lossy(&raw);
    assert!(text.contains("Subject: Proof\r\n") && text.contains("riley@studio.example"), "{text}");
    assert_eq!(text.matches("MIME-Version:").count(), 1, "{text}");
    assert!(!text.contains("multipart/mixed"));
    let key = mailcraft_model::MessageKey::default();
    assert!(!summary_of(&raw, glance, key, 0).has_attachments, "the signature isn't a file");
    // A received one keeps its original header fields, not its original content's description.
    let mut it = smime_item("IPM.Note.SMIME.MultipartSigned", "multipart/signed", &signed_entity());
    it.headers = "Received: from relay\r\nSubject: Proof\r\nContent-Type: multipart/signed; boundary=old\r\n".into();
    let (raw, _) = message(it, "id@x.invalid").unwrap();
    let text = String::from_utf8_lossy(&raw);
    assert!(text.starts_with("Received: from relay\r\n") && !text.contains("boundary=old"), "{text}");
    assert!(matches!(smime::protection(&raw), Protection::Signed { .. }));
}

/// An opaque-signed or encrypted item comes back as `application/pkcs7-mime` of its type, the CMS
/// whole; one whose content isn't a CMS of a known type keeps it as a file, as before.
#[test]
fn an_opaque_or_encrypted_item_comes_back_as_its_cms() {
    let (raw, _) = message(smime_item("IPM.Note.SMIME", "application/pkcs7-mime", &cms(2)), "id@x.invalid").unwrap();
    assert_eq!(smime::protection(&raw), Protection::Opaque(cms(2)));
    assert!(String::from_utf8_lossy(&raw).contains("smime-type=signed-data"));
    let (raw, _) = message(smime_item("IPM.Note.SMIME", "application/x-pkcs7-mime", &cms(3)), "id@x.invalid").unwrap();
    assert_eq!(smime::protection(&raw), Protection::Encrypted(cms(3)));
    assert!(String::from_utf8_lossy(&raw).contains("Subject: Proof\r\n"));
    let (raw, _) = message(smime_item("IPM.Note.SMIME", "application/pkcs7-mime", &cms(1)), "id@x.invalid").unwrap();
    assert_eq!(smime::protection(&raw), Protection::None);
    assert_eq!(files_of(&raw), ["smime.p7m"]);
}

/// A signed item's content that would add header fields to the message, or isn't signed, stays
/// a file: the file can't write the message's header.
#[test]
fn a_signed_item_s_content_cannot_add_header_fields() {
    let entity = signed_entity();
    let hostile = [b"Bcc: all@studio.example\r\n".to_vec(), entity.clone()].concat();
    let broken = [&entity[..20], b"\x07", &entity[20..]].concat();
    let plain = ENTITY.to_vec();
    for content in [hostile, broken, plain, b"no header at all".to_vec()] {
        let (raw, _) = message(smime_item("IPM.Note.SMIME.MultipartSigned", "multipart/signed", &content), "id@x.invalid").unwrap();
        assert_eq!(smime::protection(&raw), Protection::None);
        assert!(!String::from_utf8_lossy(&raw).contains("Bcc:"));
        assert_eq!(files_of(&raw), ["smime.p7m"]);
    }
    // Plain mail with such a file is plain mail.
    let (raw, _) = message(smime_item("IPM.Note", "multipart/signed", &entity), "id@x.invalid").unwrap();
    assert_eq!(smime::protection(&raw), Protection::None);
}
