//! Engine tests: pictures in the messages written: formatted mail sent as HTML with its
//! pictures, a signature's pictures, wide ones on a line of their own, and those a reply or
//! forward quotes.

use super::*;

#[test]
fn formatted_messages_go_out_as_html_with_pictures() {
    let mut s = sample_session("rich");
    s.run("mail.new", &json!({})).unwrap();
    // Agents and the command line can write with marks.
    s.run("compose.update", &json!({"to": "ann@x.example", "subject": "Plan", "body": "Hi **Ann**,\n- frames\n- sound\n", "marks": true})).unwrap();
    let c = s.compose.as_ref().unwrap();
    assert_eq!(c.body, "Hi Ann,\nframes\nsound\n");
    assert!(c.styles.style(3).bold && c.styles.para(1).list == mailcraft_mime::rich::List::Bullet);
    let png = crate::sample::sample_png();
    let r = s.run("compose.insertPicture", &json!({"name": "board.png", "data": png})).unwrap();
    assert!(r["cid"].as_str().is_some());
    assert!(s.compose.as_ref().unwrap().body.ends_with(mailcraft_mime::rich::PICTURE));
    assert!(s.run("compose.insertPicture", &json!({"name": "notes.txt", "data": [1]})).is_err(), "pictures only");
    // Pictures are shown and sent at the width they're given (issue #10), within limits.
    let cid = r["cid"].as_str().unwrap().to_string();
    let width = |s: &Session| {
        let c = s.compose.as_ref().unwrap();
        let at = c.body.chars().count() - 1;
        c.styles.style(at).picture_width
    };
    assert_eq!(width(&s), None, "its own width");
    s.run("compose.resizePicture", &json!({"cid": cid, "width": 240})).unwrap();
    assert_eq!(width(&s), Some(240));
    assert!(s.compose.as_ref().unwrap().bodies().1.unwrap().contains("width=\"240\""));
    s.run("compose.resizePicture", &json!({"cid": cid, "width": 5})).unwrap();
    assert_eq!(width(&s), Some(16), "never narrower than 16");
    s.run("compose.resizePicture", &json!({"cid": cid, "width": 99_999_999})).unwrap();
    assert_eq!(width(&s), Some(4000));
    s.run("compose.resizePicture", &json!({"cid": cid})).unwrap();
    assert_eq!(width(&s), None, "back to its own");
    assert!(s.run("compose.resizePicture", &json!({"cid": "nope@x"})).is_err());
    let wide =
        s.run("compose.insertPicture", &json!({"name": "wide.png", "data": crate::sample::sample_png(), "width": 300, "insert": false})).unwrap();
    assert!(wide["cid"].is_string());
    s.run("compose.resizePicture", &json!({"cid": cid, "width": 320})).unwrap();
    s.run("compose.send", &json!({})).unwrap();
    s.run("folder.open", &json!({"account": "sample", "folder": "Sent"})).unwrap();
    let k = key_of(&s, "Plan");
    s.run("list.open", &k).unwrap();
    let p = s.reading.as_ref().unwrap().parsed.clone().unwrap();
    assert!(p.had_html);
    assert!(p.rich.iter().flatten().any(|sp| sp.bold && sp.text == "Ann"));
    assert!(p.text.starts_with("Hi Ann,\n• frames"), "{}", p.text);
    assert_eq!(p.images.len(), 1);
    assert!(p.attachments.iter().any(|a| a.inline && a.content_id.is_some()));

    // Plain text: the marks go as typed, no HTML.
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "ann@x.example", "subject": "Plain", "body": "Keep **these** stars"})).unwrap();
    s.run("compose.format", &json!({"plain": true})).unwrap();
    s.run("compose.send", &json!({})).unwrap();
    s.run("folder.open", &json!({"account": "sample", "folder": "Sent"})).unwrap();
    let k = key_of(&s, "Plain");
    s.run("list.open", &k).unwrap();
    let p = s.reading.as_ref().unwrap().parsed.clone().unwrap();
    assert!(!p.had_html && p.text.contains("**these**"));
}

/// Issue #11 (#16's test): a signature can hold pictures. Kept in its HTML with the account (its
/// plain-text twin has none), they go into new messages and replies at their width, each under a
/// name of the message's own, and are sent with them; a signature put in again doesn't add them
/// twice.
#[test]
fn a_signature_holds_pictures() {
    use crate::session::Compose;
    use mailcraft_mime::{self as mime, rich::Styles};
    let mut s = sample_session("signature-pictures");
    let png = sample::sample_png();
    // The signature editor's text: a logo, 96 pixels wide, before the name.
    let mut sig = Compose { body: "Riley".into(), styles: Styles::plain("Riley"), ..Default::default() };
    sig.inline.push(mime::InlinePicture { cid: "logo@x".into(), name: "logo.png".into(), mime: "image/png".into(), data: png.clone() });
    crate::cmd::insert_picture_at(&mut sig, 0, "logo@x", Some(96));
    assert_eq!(sig.body.as_str(), "\u{FFFC}Riley");
    let html = mime::embed_pictures(&mime::rich::to_html(&sig.body, &sig.styles), &sig.inline);
    s.run("account.setSignature", &json!({"account": "sample", "signature": "Riley", "html": html})).unwrap();
    let config = s.default_signature("sample", false).unwrap();
    assert!(config.html.contains("src=\"data:image/png;base64,"), "{}", config.html);
    assert_eq!(config.text, "Riley", "the plain-text twin has no picture");

    s.run("mail.new", &json!({})).unwrap();
    let c = s.compose.clone().unwrap();
    let at = c.body.find('\u{FFFC}').map(|b| c.body[..b].chars().count()).expect("the logo is in the message");
    let style = c.styles.style(at);
    assert_eq!(style.picture_width, Some(96));
    let cid = style.picture.expect("as a picture");
    assert!(cid.starts_with("signature1.") && cid.ends_with("@mailcraft"), "{cid}");
    assert_eq!(c.inline.iter().filter(|p| p.cid == cid).map(|p| &p.data).collect::<Vec<_>>(), [&png]);
    s.run("compose.insertSignature", &json!({})).unwrap();
    let c = s.compose.clone().unwrap();
    assert_eq!((c.body.matches('\u{FFFC}').count(), c.inline.len()), (1, 1), "put in again: replaced, not added twice");

    // Sent with the message: a part of its own, which the HTML shows.
    s.run("compose.update", &json!({"to": "ann@x.example", "subject": "With the logo"})).unwrap();
    s.run("compose.send", &json!({})).unwrap();
    s.run("folder.open", &json!({"account": "sample", "folder": "Sent"})).unwrap();
    let k = s.list.iter().find(|m| m.subject == "With the logo").map(|m| m.key.clone()).unwrap();
    let raw = s.store.as_ref().unwrap().read_body(&k).unwrap();
    let p = mime::parse(&raw).unwrap();
    let logo = p.attachments.iter().find(|a| a.content_id.as_deref() == Some(cid.as_str())).expect("the logo goes with it");
    assert!(logo.inline && logo.mime == "image/png", "{logo:?}");
    assert_eq!(mime::attachment_data(&raw, logo.index), Some(png));
    assert_eq!(p.images, [format!("cid:{cid}")], "the HTML shows it, once");

    // A reply brings it too: under a `cid:` of its own, not the one of the quoted message's
    // picture (it looked the same, so the quote's stood in for it: another sender's photo).
    s.run("list.open", &json!({"key": k.to_id()})).unwrap();
    s.run("mail.reply", &json!({})).unwrap();
    let c = s.compose.as_ref().unwrap();
    let cids: Vec<&str> = c.inline.iter().map(|p| p.cid.as_str()).collect();
    assert_eq!(cids.len(), 2, "the quote's and the signature's: {cids:?}");
    assert!(cids.contains(&cid.as_str()), "{cids:?}");
    let own = cids.iter().find(|c| **c != cid).unwrap();
    assert!(own.starts_with("signature1.") && own.ends_with(&format!(".{}@mailcraft", c.signature_tag)), "{own}");
    let (_, html, pictures) = c.bodies();
    assert_eq!(pictures.len(), 2);
    assert!(html.unwrap().contains(own), "shown in the signature");
}

/// A wide picture goes in on a line of its own (issue #10: a second one after it went past the
/// editor's edge), the cursor after it; on an empty line already, no lines are added.
#[test]
fn wide_pictures_go_on_a_line_of_their_own() {
    let mut s = sample_session("picture-line");
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"body": "the frames are in\n\nthanks"})).unwrap();
    let before = s.compose.as_ref().unwrap().body.clone();
    let at = before.find("frames").map(|b| before[..b].chars().count() + 3).unwrap();
    let r = s.run("compose.insertPicture", &json!({"name": "shot.png", "data": crate::sample::sample_png(), "at": at, "line": true})).unwrap();
    let body = s.compose.as_ref().unwrap().body.clone();
    assert!(body.contains("fra\n\u{FFFC}\nmes"), "{body:?}");
    assert_eq!(r["end"].as_u64(), Some(at as u64 + 2));
    let shot = r["cid"].as_str().unwrap().to_string();
    let c = s.compose.as_ref().unwrap();
    assert_eq!(c.styles.style(at + 1).picture.as_deref(), Some(shot.as_str()));
    assert!(c.styles.style(at).picture.is_none() && c.styles.style(at + 2).picture.is_none(), "the line breaks aren't pictures");
    // At the end, after text: a line break before it only.
    let n = c.body.chars().count();
    let r = s.run("compose.insertPicture", &json!({"name": "end.png", "data": crate::sample::sample_png(), "at": n, "line": true})).unwrap();
    assert_eq!(r["end"].as_u64(), Some(n as u64 + 2));
    let body = s.compose.as_ref().unwrap().body.clone();
    assert!(body.ends_with("\n\u{FFFC}"), "{body:?}");
    // On an empty line already: no lines added.
    let empty = body.find("\n\n").map(|b| body[..b].chars().count() + 1).unwrap();
    let n = body.chars().count();
    s.run("compose.insertPicture", &json!({"name": "in.png", "data": crate::sample::sample_png(), "at": empty, "line": true})).unwrap();
    assert_eq!(s.compose.as_ref().unwrap().body.chars().count(), n + 1, "the picture alone");
}

/// A reply or forward keeps the pictures in the message it quotes (they came out as
/// "[picture]", issue #10), and quotes a message without a `Date` as sent when the server took it in (not
/// on 1 January 1970). Found testing Outlook's `winmail.dat` (issue #18).
#[test]
fn replies_keep_the_quoted_pictures_and_date() {
    let mut s = sample_session("quoted-pictures");
    let raw = b"From: Sam Okafor <sam@studio.example>\r\nTo: riley@studio.example\r\nSubject: Logo\r\nMIME-Version: 1.0\r\n\
Content-Type: multipart/related; boundary=\"R\"\r\n\r\n--R\r\nContent-Type: text/html; charset=utf-8\r\n\r\n\
<p>Here is the <b>logo</b>:</p><p><img src=\"cid:logo@studio.example\" width=\"40\"></p><p><img src=\"https://cdn.example/poster.jpg\"><img src=\"https://tracker.example/p.gif\" width=\"1\" height=\"1\"></p>\r\n\
--R\r\nContent-Type: image/png\r\nContent-ID: <logo@studio.example>\r\nContent-Disposition: inline; filename=\"logo.png\"\r\n\r\nPNGDATA\r\n--R--\r\n";
    let key = mailcraft_model::MessageKey { uid: 9300, ..next_key(&s, "sample", "INBOX") };
    let summary = mailcraft_mime::summary_from_full(raw, key, mailcraft_model::Flags::default(), 0, NOW - 3600);
    let key = file_in(&mut s, summary, Some(raw));
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    s.run("list.open", &json!({"key": key.to_id()})).unwrap();
    assert_eq!(s.reading.as_ref().and_then(|r| r.parsed.as_ref()).map(|p| p.date), Some(NOW - 3600));
    for cmd in ["mail.reply", "mail.forward"] {
        s.run(cmd, &json!({})).unwrap();
        let c = s.compose.clone().unwrap();
        assert!(c.attachments.is_empty(), "{cmd}: the picture isn't attached as a file");
        assert_eq!(
            c.inline.iter().map(|p| (p.cid.as_str(), p.data.as_slice())).collect::<Vec<_>>(),
            [("logo@studio.example", &b"PNGDATA"[..])],
            "{cmd}"
        );
        let (text, html, pictures) = c.bodies();
        let html = html.unwrap_or_default();
        assert!(html.contains("<img src=\"cid:logo@studio.example\" width=\"40\""), "{cmd}: {html}");
        // One on the web by its address, never loaded (issue #10); a tracking one left out.
        assert!(html.contains("<img src=\"https://cdn.example/poster.jpg\""), "{cmd}: {html}");
        assert!(!html.contains("tracker.example"), "{cmd}: {html}");
        assert_eq!(pictures.len(), 1);
        assert!(!text.contains("1970"), "{cmd}: {text}");
        s.run("compose.discard", &json!({})).unwrap();
    }
}

/// A picture sent inline beside plain text (as Apple Mail sends one) goes with a forward as a
/// file: the quote can't show it, and it was left out (issue #10).
#[test]
fn a_forward_keeps_an_inline_picture_beside_plain_text() {
    let mut s = sample_session("inline-beside-text");
    let raw = b"From: Sam Okafor <sam@studio.example>\r\nTo: riley@studio.example\r\nSubject: Photo\r\nDate: Thu, 29 Oct 2026 10:00:00 +0000\r\nMIME-Version: 1.0\r\n\
Content-Type: multipart/mixed; boundary=\"M\"\r\n\r\n--M\r\nContent-Type: text/plain\r\n\r\nThe set, from above.\r\n\
--M\r\nContent-Type: image/jpeg; name=\"set.jpg\"\r\nContent-ID: <set@studio.example>\r\nContent-Disposition: inline; filename=\"set.jpg\"\r\n\r\nJPEGDATA\r\n--M--\r\n";
    let key = mailcraft_model::MessageKey { uid: 9301, ..next_key(&s, "sample", "INBOX") };
    let key = file_whole(&mut s, key, raw);
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    s.run("list.open", &json!({"key": key.to_id()})).unwrap();
    s.run("mail.forward", &json!({})).unwrap();
    let c = s.compose.clone().unwrap();
    assert_eq!(c.attachments.iter().map(|a| (a.name.as_str(), a.data.as_slice())).collect::<Vec<_>>(), [("set.jpg", &b"JPEGDATA"[..])]);
    s.run("compose.discard", &json!({})).unwrap();
    s.run("mail.reply", &json!({})).unwrap();
    assert!(s.compose.as_ref().unwrap().attachments.is_empty(), "a reply leaves files out, as Outlook's");
}
