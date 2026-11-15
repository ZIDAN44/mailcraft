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

fn sent(s: &mut Session, subject: &str) -> std::sync::Arc<mime::Parsed> {
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
    let config = s.default_signature("sample", false).unwrap();
    assert!(config.code.starts_with("<table"), "{}", config.code);
    for gone in ["<script", "<title", "onclick", "javascript:", "<body"] {
        assert!(!config.code.contains(gone), "{gone}: {}", config.code);
    }
    assert!(config.text.contains("Riley Moreno") && config.text.contains("Producer"), "{:?}", config.text);
    assert!(!config.text.contains(['│', '─']), "no table drawn in lines: {:?}", config.text);

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
    let (text, html, _) = c.bodies_with(&c.signature_code);
    assert!(text.contains("Director"), "{text}");
    assert!(!html.unwrap_or_default().contains("border-left"), "the card as it was isn't sent");

    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.format", &json!({"plain": true})).unwrap();
    let c = s.compose.clone().unwrap();
    assert_eq!(c.bodies_with(&c.signature_code).1, None);

    s.run("account.setSignature", &json!({"account": "sample", "signature": "Riley", "html": "<b>Riley</b>"})).unwrap();
    assert!(s.default_signature("sample", false).unwrap().code.is_empty());
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
    let (text, html, pictures) = c.bodies_with(&c.signature_code);
    let html = html.unwrap();
    let (card_at, quote_at) = (html.find("border-left:2px solid #c03").unwrap(), html.find("festival poster").unwrap_or(0));
    assert!(card_at < quote_at, "the card, then the quote: {html}");
    assert!(text.contains("Riley Moreno") && text.contains("festival poster"));
    assert_eq!(pictures.iter().filter(|p| p.cid.starts_with("signature")).count(), 1, "the photo once");

    s.run("account.setSignature", &json!({"account": "sample", "signature": "", "code": "<span style=\"letter-spacing:2px\">RILEY</span>"})).unwrap();
    s.run("mail.new", &json!({})).unwrap();
    let c = s.compose.clone().unwrap();
    assert!(c.body.contains("RILEY"));
    let html = c.bodies_with(&c.signature_code).1.unwrap_or_default();
    assert!(html.contains("letter-spacing:2px"), "{html}");
}

/// Issue #90: the window draws the code, laid out, where the signature's text stands, so the
/// engine says which characters those are: the signature's text after its `-- ` line, while it's
/// there as it was put in; with the code as it goes and its pictures. Not for a message sent as
/// plain text, a signature changed in the message, or one with no code. Text written before
/// and after it moves the place and nothing else, and what's sent is what it was.
#[test]
fn the_code_stands_for_the_signatures_text_where_it_was_put() {
    let mut s = sample_session("signature-card");
    let png = crate::sample::sample_png();
    s.run("account.setSignature", &json!({"account": "sample", "signature": "", "code": card(&png)})).unwrap();
    s.run("mail.new", &json!({})).unwrap();
    let c = s.compose.clone().unwrap();
    let chars: Vec<char> = c.body.chars().collect();
    let range = c.signature_card().expect("the signature's text");
    let stood_for: String = chars.get(range.clone()).unwrap().iter().collect();
    assert_eq!(range.end, chars.len(), "to the end of a new message");
    assert_eq!(format!("-- \n{stood_for}"), c.signature, "its text, after the line that starts it");
    let before: String = chars.get(..range.start).unwrap().iter().collect();
    assert!(before.ends_with("-- \n"), "{before:?}");
    let (html, pictures) = c.signature_card_parts();
    assert!(html.starts_with("<table") && pictures.len() == 1 && html.contains(&format!("cid:{}", pictures[0].cid)), "{html}");
    let (_, sent_html, sent_pictures) = c.bodies_with(&c.signature_code);
    assert!(sent_html.unwrap().contains(&html), "what's drawn is what goes");
    assert_eq!(sent_pictures.iter().map(|p| &p.cid).collect::<Vec<_>>(), pictures.iter().map(|p| &p.cid).collect::<Vec<_>>());

    // Text before and after it: the same text, further on.
    let written = format!("Hello Ana,\n{}\nP.S. see you there", c.body);
    s.run("compose.update", &json!({"body": written})).unwrap();
    let c = s.compose.clone().unwrap();
    let moved = c.signature_card().expect("still as it was put in");
    assert_eq!(c.body.chars().skip(moved.start).take(moved.len()).collect::<String>(), stood_for);
    assert_eq!(moved.start, range.start + "Hello Ana,\n".chars().count());
    assert!(moved.end < c.body.chars().count());

    // As plain text, changed in the message, or with no code: its text, like any other.
    s.run("compose.format", &json!({"plain": true})).unwrap();
    assert_eq!(s.compose.as_ref().unwrap().signature_card(), None);
    s.run("compose.format", &json!({"plain": false})).unwrap();
    assert!(s.compose.as_ref().unwrap().signature_card().is_some());
    s.run("compose.update", &json!({"body": c.body.replace("Producer", "Director")})).unwrap();
    assert_eq!(s.compose.as_ref().unwrap().signature_card(), None);
    s.run("compose.discard", &json!({})).unwrap();
    s.run("account.setSignature", &json!({"account": "sample", "signature": "Riley\nStudio"})).unwrap();
    s.run("mail.new", &json!({})).unwrap();
    assert_eq!(s.compose.as_ref().unwrap().signature_card(), None);
}

/// Audit #257: with many web pictures loaded, those let go to make room are never the ones on
/// screen: showing the signature's pictures keeps the open message's, and opening a message
/// keeps the signature's. Only what was asked for is fetched.
#[test]
fn pictures_let_go_are_never_the_ones_on_screen() {
    let mut s = sample_session("signature-pictures-kept");
    let (open, sig) = ("https://cdn.example/open.png".to_string(), "https://cdn.example/sig.png".to_string());
    let k = key_of(&s, "Your proof for the festival poster is ready");
    s.run("list.open", &k).unwrap();
    let r = s.reading.as_mut().unwrap();
    let mut p = (**r.parsed.as_ref().unwrap()).clone();
    p.images = vec![open.clone()];
    r.parsed = Some(std::sync::Arc::new(p));
    s.run("message.showPictures", &json!({})).unwrap();
    assert!(matches!(s.take_jobs().as_slice(), [Job::FetchUrl { url }] if *url == open));
    let ready = || crate::Picture::Ready(std::sync::Arc::new(vec![1, 2, 3]));
    let fill = |s: &mut Session| {
        for n in 0..450 {
            s.pictures.insert(format!("https://old.example/{n}.png"), ready());
        }
    };
    s.pictures.insert(open.clone(), ready());
    fill(&mut s);

    let code = format!("<table><tr><td><img src=\"{sig}\" width=\"40\"></td><td><b>Riley Moreno</b><br>Producer</td></tr></table>");
    s.run("account.setSignature", &json!({"account": "sample", "signature": "", "code": code})).unwrap();
    s.run("mail.new", &json!({})).unwrap();
    s.take_jobs();
    assert_eq!(s.run("compose.signaturePictures", &json!({})).unwrap()["pictures"], 1);
    assert!(matches!(s.take_jobs().as_slice(), [Job::FetchUrl { url }] if *url == sig), "the signature's only");
    assert_eq!(s.pictures.get(&open), Some(&ready()), "the open message's still shown");
    assert!(s.pictures.len() < 10, "the others let go: {}", s.pictures.len());

    s.pictures.insert(sig.clone(), ready());
    fill(&mut s);
    s.run("message.showPictures", &json!({})).unwrap();
    assert!(s.take_jobs().is_empty(), "nothing fetched again");
    assert_eq!((s.pictures.get(&open), s.pictures.get(&sig)), (Some(&ready()), Some(&ready())));
    assert!(s.pictures.len() < 10, "{}", s.pictures.len());
}
