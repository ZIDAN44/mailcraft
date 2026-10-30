//! Engine tests: a signature given as HTML code, a business card in two columns, sent as written
//! (issue #11).

use super::*;
use mailcraft_mime as mime;
use mailcraft_model::MessageKey;

/// A card as signature makers write one: the photo on the left, the details on the right.
fn card(png: &[u8]) -> String {
    let pic = mime::InlinePicture { cid: "photo".into(), name: "photo.png".into(), mime: "image/png".into(), data: png.to_vec() };
    let img = mime::embed_pictures("<img src=\"cid:photo\" width=\"80\" alt=\"Riley\">", &[pic]);
    format!(
        "<html><head><title>sig</title><script>alert(1)</script></head><body>\
         <table cellpadding=\"0\" cellspacing=\"0\"><tr>\
         <td style=\"padding-right:12px\">{img}</td>\
         <td style=\"border-left:2px solid #c03;padding-left:12px\" onclick=\"steal()\"><b>Riley Moreno</b><br>Producer<br>\
         <a href=\"https://studio.example\">studio.example</a> <a href=\"javascript:x()\">x</a></td>\
         </tr></table></body></html>"
    )
}

fn sent(s: &mut Session, subject: &str) -> mime::Parsed {
    s.run("folder.open", &json!({"account": "sample", "folder": "Sent"})).unwrap();
    let k = key_of(s, subject);
    s.run("list.open", &k).unwrap();
    s.reading.as_ref().unwrap().parsed.clone().unwrap()
}

/// Kept as written (its head, scripts and handlers left out), shown in the editor as its text
/// (the photo, then the details: no table drawn in lines), and sent as written with its photo a
/// part of the message under a `cid:` of its own.
#[test]
fn a_signature_in_html_code_is_sent_as_written() {
    let mut s = sample_session("signature-code");
    let png = crate::sample::sample_png();
    s.run("account.setSignature", &json!({"account": "sample", "signature": "", "code": card(&png)})).unwrap();
    let config = s.account("sample").unwrap().config.clone();
    assert!(config.signature_code.starts_with("<table"), "{}", config.signature_code);
    for gone in ["<script", "<title", "onclick", "javascript:", "<body"] {
        assert!(!config.signature_code.contains(gone), "{gone}: {}", config.signature_code);
    }
    assert!(config.signature.contains("Riley Moreno") && config.signature.contains("Producer"), "{:?}", config.signature);
    assert!(!config.signature.contains(['│', '─']), "no table drawn in lines: {:?}", config.signature);

    s.run("mail.new", &json!({})).unwrap();
    let c = s.compose.clone().unwrap();
    assert!(c.body.contains("Riley Moreno") && c.body.contains(mime::rich::PICTURE), "{:?}", c.body);
    s.run("compose.update", &json!({"to": "ann@x.example", "subject": "Card", "body": format!("Hello Ann,{}", c.body)})).unwrap();
    s.run("compose.send", &json!({})).unwrap();
    let p = sent(&mut s, "Card");
    // The card's two columns, side by side, as read.
    let columns = p.layout.iter().any(|b| matches!(b, mime::layout::Block::Table(t) if t.rows.iter().any(|r| r.len() == 2)));
    assert!(columns, "the card as written: {:?}", p.layout);
    assert!(p.text.contains("Hello Ann"), "{}", p.text);
    assert_eq!(p.images.len(), 1, "{:?}", p.images);
    let cid = p.images[0].trim_start_matches("cid:").to_string();
    assert!(cid.starts_with("signature1.") && cid.ends_with("@mailcraft"), "{cid}");
    let photo = p.attachments.iter().find(|a| a.content_id.as_deref() == Some(cid.as_str())).expect("the photo goes with it");
    let raw = s.store.as_ref().unwrap().read_body(&photo_key(&s, "Card")).unwrap();
    assert_eq!(mime::attachment_data(&raw, photo.index), Some(png));
    assert!(p.text.contains("Riley Moreno"), "the text part has its text: {}", p.text);
    done(s);
}

/// Changed in the message, the signature goes as it now reads; a plain-text message, as text;
/// saved again from the formatted editor, the code is forgotten.
#[test]
fn a_changed_signature_goes_as_changed() {
    let mut s = sample_session("signature-code-changed");
    let png = crate::sample::sample_png();
    s.run("account.setSignature", &json!({"account": "sample", "signature": "", "code": card(&png)})).unwrap();
    s.run("mail.new", &json!({})).unwrap();
    let body = s.compose.as_ref().unwrap().body.replace("Producer", "Director");
    s.run("compose.update", &json!({"to": "ann@x.example", "subject": "Changed", "body": body})).unwrap();
    let c = s.compose.clone().unwrap();
    let (text, html, _) = c.bodies_with(&s.signature_code("sample"));
    assert!(text.contains("Director"), "{text}");
    assert!(!html.unwrap_or_default().contains("border-left"), "the card as it was isn't sent");

    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.format", &json!({"plain": true})).unwrap();
    let c = s.compose.clone().unwrap();
    assert_eq!(c.bodies_with(&s.signature_code("sample")).1, None);

    s.run("account.setSignature", &json!({"account": "sample", "signature": "Riley", "html": "<b>Riley</b>"})).unwrap();
    assert!(s.account("sample").unwrap().config.signature_code.is_empty());
    done(s);
}

fn photo_key(s: &Session, subject: &str) -> MessageKey {
    s.list.iter().find(|m| m.subject == subject).map(|m| m.key.clone()).unwrap()
}

/// In a reply the quoted message follows the card; code of plain text alone still goes as HTML.
#[test]
fn a_reply_has_the_card_then_the_quote() {
    let mut s = sample_session("signature-code-reply");
    let png = crate::sample::sample_png();
    s.run("account.setSignature", &json!({"account": "sample", "signature": "", "code": card(&png)})).unwrap();
    let k = key_of(&s, "Your proof for the festival poster is ready");
    s.run("list.open", &k).unwrap();
    s.run("mail.reply", &json!({})).unwrap();
    let c = s.compose.clone().unwrap();
    let (text, html, pictures) = c.bodies_with(&s.signature_code("sample"));
    let html = html.unwrap();
    let (card_at, quote_at) = (html.find("border-left:2px solid #c03").unwrap(), html.find("festival poster").unwrap_or(0));
    assert!(card_at < quote_at, "the card, then the quote: {html}");
    assert!(text.contains("Riley Moreno") && text.contains("festival poster"));
    assert_eq!(pictures.iter().filter(|p| p.cid.starts_with("signature")).count(), 1, "the photo once");

    s.run("account.setSignature", &json!({"account": "sample", "signature": "", "code": "<span style=\"letter-spacing:2px\">RILEY</span>"})).unwrap();
    s.run("mail.new", &json!({})).unwrap();
    let c = s.compose.clone().unwrap();
    assert!(c.body.contains("RILEY"));
    let html = c.bodies_with(&s.signature_code("sample")).1.unwrap_or_default();
    assert!(html.contains("letter-spacing:2px"), "{html}");
    done(s);
}
