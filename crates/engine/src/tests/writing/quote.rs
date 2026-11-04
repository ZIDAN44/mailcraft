use super::*;

/// A made-up message laid out in nested tables, as business mail writes a signature: an icon
/// column beside the numbers, a coloured warning, a logo cell; with a style sheet, a script, a
/// handler and a tracking picture to leave out.
const MILL: &[u8] = b"From: Morgan Lee <morgan@mill.example>\r\nTo: riley@studio.example\r\nSubject: Invoice\r\nMIME-Version: 1.0\r\n\
Content-Type: multipart/related; boundary=\"R\"\r\n\r\n--R\r\nContent-Type: text/html; charset=utf-8\r\n\r\n\
<html><head><style>.warn{color:#ff0000}</style><script>alert(1)</script></head><body>\
<p>We will share the updated invoice soon.</p>\
<table width=\"640\"><tr><td width=\"40\"><img src=\"cid:phone@mill.example\" width=\"16\" alt=\"phone\"></td>\
<td width=\"420\"><table><tr><td>+00 1234 5678</td></tr><tr><td><b class=\"warn\" onclick=\"steal()\">A warning in red</b></td></tr></table></td>\
<td width=\"180\" align=\"right\">Mill</td></tr></table>\
<img src=\"https://tracker.example/open.gif\" width=\"1\" height=\"1\"></body></html>\r\n\
--R\r\nContent-Type: image/png\r\nContent-ID: <phone@mill.example>\r\nContent-Disposition: inline; filename=\"phone.png\"\r\n\r\nPNGDATA\r\n--R--\r\n";

pub(super) fn open_mill(s: &mut Session) -> mailcraft_model::MessageKey {
    let store = s.store.clone().unwrap();
    let mut inbox = store.load_folder("sample", "INBOX").unwrap();
    let key = mailcraft_model::MessageKey { account: "sample".into(), folder: "INBOX".into(), validity: inbox.validity, uid: 9400 };
    inbox.messages.insert(0, mailcraft_mime::summary_from_full(MILL, key.clone(), mailcraft_model::Flags::default(), 0, NOW - 3600).into());
    store.save_folder("sample", "INBOX", &inbox).unwrap();
    store.write_body(&key, MILL).unwrap();
    s.apply(Event::FolderChanged { account: "sample".into(), folder: "INBOX".into() });
    s.run("folder.open", &json!({"account": "sample", "folder": "INBOX"})).unwrap();
    s.run("list.open", &json!({"key": key.to_id()})).unwrap();
    key
}

/// The owner's report: a reply turned a message laid out in tables into lines of text, its
/// columns gone. Mail written in HTML is quoted as it came: shown from the message, sent as its
/// own HTML (without what runs), its picture with it, its text in the plain twin.
#[test]
fn a_reply_keeps_the_original_as_it_came() {
    let mut s = sample_session("quote-as-it-came");
    let key = open_mill(&mut s);
    for cmd in ["mail.reply", "mail.replyAll", "mail.forward"] {
        s.run(cmd, &json!({})).unwrap();
        let c = s.compose.clone().unwrap();
        let q = c.quote.clone().unwrap_or_else(|| panic!("{cmd}: kept as it came"));
        assert_eq!(q.key, key);
        assert!(!q.parsed.layout.is_empty() && q.cells() >= 4, "{cmd}: shown with its columns, each block of text its own");
        // The text holds who sent it to whom and when; the original is after it, not in it.
        assert!(c.body.contains("Subject: Invoice") && !c.body.contains("A warning in red"), "{cmd}: {}", c.body);
        let (text, html, pictures) = c.bodies();
        let html = html.unwrap_or_default();
        assert!(html.contains("<table width=\"640\"") && html.contains("A warning in red"), "{cmd}: {html}");
        assert!(html.contains("color:#ff0000") || html.contains("color: #ff0000"), "its style sheet written into its tags: {html}");
        for gone in ["<script", "alert(1)", "onclick", "steal()", "<style", "tracker.example", "<head"] {
            assert!(!html.contains(gone), "{cmd}: {gone} in {html}");
        }
        assert!(html.contains("src=\"cid:phone@mill.example\""), "{cmd}: {html}");
        assert_eq!(pictures.iter().map(|p| p.cid.as_str()).collect::<Vec<_>>(), ["phone@mill.example"], "{cmd}");
        assert!(text.contains("Subject: Invoice") && text.contains("A warning in red"), "{cmd}: {text}");
        // As plain text: the text alone.
        s.run("compose.format", &json!({"plain": true})).unwrap();
        let (text, html, _) = s.compose.as_ref().unwrap().bodies();
        assert!(html.is_none() && text.contains("+00 1234 5678"), "{cmd}: {text}");
        s.run("compose.discard", &json!({})).unwrap();
    }
    // Forwarded as an attachment, nothing is quoted.
    s.run("mail.forwardAsAttachment", &json!({})).unwrap();
    assert!(s.compose.as_ref().unwrap().quote.is_none());
    s.run("compose.discard", &json!({})).unwrap();
    // A plain text message has nothing to keep: quoted in the text, as before.
    let k = key_of(&s, "Sound mix, version 3");
    s.run("list.open", &k).unwrap();
    s.run("mail.reply", &json!({})).unwrap();
    let c = s.compose.as_ref().unwrap();
    assert!(c.quote.is_none() && c.body.contains("Version 3 of the mix"), "{}", c.body);
    done(s);
}

/// Gap 6 closed: the original is changed where it stands, a block of its text at a time (its
/// columns stay), as Outlook lets it be. Changed, it goes out as its layout written again around
/// the new text, its pictures with it (one put into it too); unchanged blocks as they were.
#[test]
fn the_original_is_changed_where_it_stands() {
    let mut s = sample_session("quote-changed");
    open_mill(&mut s);
    s.run("mail.reply", &json!({})).unwrap();
    let c = s.compose.as_mut().unwrap();
    let q = c.quote.clone().unwrap();
    let n = (0..q.cells()).find(|n| q.cell(*n).is_some_and(|(t, _)| t.contains("A warning"))).expect("the warning's block");
    // Typed into, as the editor does (the block stands in for the text).
    let typed = c.in_quote_cell(n, |c| {
        c.body = c.body.replace("A warning in red", "A warning in red, paid on Friday");
        c.styles.fit(&c.body);
        c.body.clone()
    });
    assert!(typed.is_some_and(|t| t.contains("paid on Friday")));
    assert!(c.body.contains("Subject: Invoice") && !c.body.contains("Friday"), "the text is the message's again: {}", c.body);
    let q = c.quote.clone().unwrap();
    assert_eq!(q.edits.len(), 1, "only the block changed");
    // Unchanged when it's set back as it was.
    let (t, st) = q.cell(n).unwrap();
    c.set_quote_cell(n, t, st);
    assert_eq!(c.quote.as_ref().unwrap().edits.len(), 1);
    let (text, html, pictures) = c.bodies();
    let html = html.unwrap_or_default();
    assert!(html.contains("<table") && html.contains("width=\"640\"") && html.contains("paid on Friday"), "{html}");
    assert!(html.contains("+00 1234 5678") && html.contains("src=\"cid:phone@mill.example\""), "{html}");
    assert!(html.contains("class=\"mailcraft-quote\""), "{html}");
    assert!(!html.contains("<script") && !html.contains("tracker.example"), "{html}");
    assert_eq!(pictures.len(), 1);
    assert!(text.contains("paid on Friday"), "{text}");
    // A picture put into the block where its cursor is.
    let r = s.run("compose.insertPicture", &json!({"name": "stamp.png", "data": crate::sample::sample_png(), "at": 0, "quoteCell": n})).unwrap();
    let cid = r["cid"].as_str().unwrap().to_string();
    let c = s.compose.as_ref().unwrap();
    assert!(!c.body.contains(mailcraft_mime::rich::PICTURE), "not into the text: {}", c.body);
    let (_, html, pictures) = c.bodies();
    assert!(html.unwrap_or_default().contains(&format!("cid:{cid}")));
    assert!(pictures.iter().any(|p| p.cid == cid));
    assert!(s.run("compose.insertPicture", &json!({"name": "x.png", "data": crate::sample::sample_png(), "quoteCell": 999})).is_err());
    done(s);
}

/// A reply taken back out of the Outbox to change has its formatting, its pictures in place and
/// its original as it was (its columns, the changes made in it), not plain text and files.
#[test]
fn a_reply_from_the_outbox_keeps_its_original() {
    let store = temp("quote-outbox");
    crate::sample::seed_if_empty(&store, NOW).unwrap();
    let mut s = Session::open(Some(store.clone()), mailcraft_model::dates::Clock::new(NOW, 0));
    open_mill(&mut s);
    s.run("mail.reply", &json!({})).unwrap();
    let c = s.compose.as_mut().unwrap();
    let n = {
        let q = c.quote.clone().unwrap();
        (0..q.cells()).find(|n| q.cell(*n).is_some_and(|(t, _)| t.contains("A warning"))).unwrap()
    };
    c.in_quote_cell(n, |c| {
        c.body = c.body.replace("A warning in red", "Paid");
        c.styles.fit(&c.body);
    });
    let draft = s.compose.clone().unwrap();
    let (text, html, inline) = draft.bodies();
    let d = mailcraft_mime::Draft {
        to: vec![mailcraft_model::EmailAddress::new("", "morgan@mill.example")],
        subject: draft.subject.clone(),
        body: text,
        html,
        inline,
        ..Default::default()
    };
    let raw = mailcraft_mime::build(&d, &mailcraft_model::EmailAddress::new("Riley", "riley@studio.example"), NOW, "out@x").unwrap();
    let id = "sample";
    let item = mailcraft_store::OutboxItem {
        id: 77,
        recipients: vec!["morgan@mill.example".into()],
        queued: NOW,
        error: Some("offline".into()),
        ..Default::default()
    };
    store.outbox_add(id, &item, &raw).unwrap();
    s.apply(Event::Queued { account: id.into(), id: 77, error: "offline".into() });
    s.run("compose.discard", &json!({})).unwrap_or_default();
    s.run("folder.outbox", &json!({})).unwrap();
    let key = s.list.iter().find(|m| m.subject == draft.subject).map(|m| m.key.clone()).expect("in the Outbox");
    s.run("outbox.edit", &json!({"key": key.to_id()})).unwrap();
    let c = s.compose.clone().unwrap();
    assert!(c.attachments.is_empty(), "its picture is in its original, not a file: {:?}", c.attachments.iter().map(|a| &a.name).collect::<Vec<_>>());
    let q = c.quote.clone().expect("its original as it was");
    assert!(!q.parsed.layout.is_empty(), "with its columns");
    assert!((0..q.cells()).any(|n| q.cell(n).is_some_and(|(t, _)| t.contains("Paid"))), "with what was changed in it");
    assert!(!c.body.contains("Paid") && c.body.contains("Subject: Invoice"), "{}", c.body);
    let (_, html, pictures) = c.bodies();
    assert!(html.unwrap_or_default().contains("cid:phone@mill.example") && pictures.len() == 1);
    done(s);
}
