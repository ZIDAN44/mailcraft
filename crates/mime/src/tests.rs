/// An HTML body's links read whole from its `href`s: a 900-character Teams link that the
/// text's footnote wraps (it came out cut at 395), quoted either way or not, with entities,
/// once each; nothing that isn't a web address, and nothing runaway.
#[test]
fn an_html_bodys_links_come_whole() {
    let long = format!("https://teams.microsoft.com/l/meetup-join/19%3ameeting_{}%40thread.v2/0?context=%7b%22Tid%22%7d", "Nj".repeat(400));
    let html = format!(
        "<p>Hi</p><a href=\"{long}\">Join</a> <A HREF='https://x.example/a?b=1&amp;c=2'>x</A> <a href=https://y.example/u>y</a>             <a href = \"javascript:alert(1)\">no</a><a href=\"mailto:z@x.example\">m</a><a href=\"{long}\">again</a><a href>bare</a>"
    );
    assert_eq!(html_links(&html), [long.clone(), "https://x.example/a?b=1&c=2".into(), "https://y.example/u".into()]);
    let cut = mailcraft_model::ical::CalEvent { description: html_to_text(&html), ..Default::default() }.join_link().unwrap_or_default();
    assert!(cut.len() < long.len(), "the text alone cuts it ({}), which is why the HTML is read", cut.len());
    assert!(html_links(&"<a href=\"".repeat(10_000)).is_empty());
    assert!(html_links("href").is_empty() && html_links("").is_empty());
    let many: String = (0..200).map(|i| format!("<a href=\"https://h{i}.example/\">l</a>")).collect();
    assert_eq!(html_links(&many).len(), 50);
}

/// Issue #1: a Teams invitation's "Join the meeting now" is a link in its HTML body; made
/// text, the address stays (a footnote), so the calendar can offer it to join.
#[test]
fn a_teams_invitations_join_link_survives_as_text() {
    let html = "<html><body><p>Agenda: the release.</p><div>________________</div>\
            <div><span>Microsoft Teams <a href=\"https://aka.ms/JoinTeamsMeeting\">Need help?</a></span></div>\
            <div><a href=\"https://teams.microsoft.com/l/meetup-join/19%3ameeting_NjA3%40thread.v2/0?context=%7b%22Tid%22%3a%22c1%22%7d\">\
            <span>Join the meeting now</span></a></div><div>Meeting ID: 366 254 181 924</div></body></html>";
    let text = html_to_text(html);
    let event = mailcraft_model::ical::CalEvent { description: text.clone(), ..Default::default() };
    let join = event.join_link().unwrap_or_default();
    assert!(join.starts_with("https://teams.microsoft.com/l/meetup-join/19%3ameeting_NjA3%40thread.v2/0?context="), "{join} in {text}");
    assert!(event.links().contains(&"https://aka.ms/JoinTeamsMeeting".to_string()), "{text}");
}

use super::*;

/// Issue #11 (#16's test): a signature's pictures are written into the HTML kept with the account, and come
/// back from it as pictures of its text, with its look; pictures from the web, kinds a signature
/// can't hold, and broken ones don't.
#[test]
fn a_signatures_pictures_go_into_its_html_and_back() {
    let png = vec![0x89, b'P', b'N', b'G', 1, 2, 3, 250];
    let (text, mut styles) = rich::from_markup("**Riley Moreno**\nStudio Lantern");
    let mut text = text;
    rich::splice(&mut text, &mut styles, 0..0, "\u{FFFC}\n", &rich::Styles::plain("\u{FFFC}\n"));
    styles.restyle(&text, 0..1, |s| rich::Style { picture: Some("logo@x".into()), ..s.clone() });
    let pictures = vec![InlinePicture { cid: "logo@x".into(), name: "logo.png".into(), mime: "image/png".into(), data: png.clone() }];
    let html = embed_pictures(&rich::to_html(&text, &styles), &pictures);
    assert!(html.contains("<img src=\"data:image/png;base64,iVBORwECA/o=\"") && !html.contains("cid:"), "{html}");
    let (back, back_styles, back_pictures) = signature_html(&html);
    assert_eq!(back, text);
    assert_eq!(back_pictures.len(), 1);
    assert_eq!((back_pictures[0].mime.as_str(), &back_pictures[0].data), ("image/png", &png));
    assert_eq!(back_styles.style(0).picture.as_deref(), Some(back_pictures[0].cid.as_str()));
    assert!(back_styles.style(2).bold, "the name stays bold");
    // What a signature can't keep.
    let others =
        "<p>Hi<img src=\"https://x.example/t.png\"><img src=\"data:image/svg+xml;base64,PHN2Zz4=\"><img src=\"data:image/png;base64,***\"></p>";
    let (t, _, p) = signature_html(others);
    assert_eq!((t.as_str(), p.len()), ("Hi", 0));
    // As before, without pictures: the same text and look as pasting it.
    let plain = "<p><b>Riley</b> · <a href=\"https://studio.example/\">studio</a></p>";
    let (t, s, p) = signature_html(plain);
    assert_eq!((t, s, p.len()), (pasted_html(plain).0, pasted_html(plain).1, 0));
}

/// Issue #1: a signature copied from Outlook (Word's HTML on the clipboard, only the copied
/// fragment) keeps its look when pasted.
#[test]
fn a_signature_pasted_from_outlook_keeps_its_formatting() {
    let html = "<!--StartFragment--><p class=MsoNormal><b><span style='font-size:14.0pt;font-family:\"Arial\",sans-serif;color:#BFBFBF'>Company bv<o:p></o:p></span></b></p>\r\n\
<p class=MsoNormal><span style='font-size:10.0pt;font-family:\"Tahoma\",sans-serif'><o:p>&nbsp;</o:p></span></p>\r\n\
<p class=MsoNormal><span style='font-size:10.0pt;font-family:\"Tahoma\",sans-serif'>An street 1<br>B-1000 Brussel<br>Belgium<br>VAT BE 01234 567 890<br>Mob&nbsp;&nbsp;&nbsp;&nbsp; +32 (0)490 12 34 56<br>\
<a href=\"http://www.company.com/\"><span style='color:#44546A'>www.company.com</span></a><o:p></o:p></span></p>\r\n\
<p class=MsoNormal><span style='font-size:10.0pt'><o:p>&nbsp;</o:p></span></p>\r\n\
<p class=MsoNormal><i><span style='font-size:14.0pt;color:#BFBFBF'>\u{2018}Choose a job you like' \u{2013} Confucius<o:p></o:p></span></i></p>\r\n\
<p class=MsoNormal><span style='font-size:10.0pt'><o:p>&nbsp;</o:p></span></p>\r\n\
<p class=MsoNormal><i><span style='font-size:10.0pt'>The information contained in this message is confidential &amp; intended for the addressee.<o:p></o:p></span></i></p>\r\n\
<p class=MsoNormal><img width=120 height=40 src=\"file:///C:/Users/x/AppData/Local/Temp/msohtmlclip1/01/clip_image001.png\"><o:p></o:p></p><!--EndFragment-->";
    let (text, styles) = pasted_html(html);
    let lines: Vec<&str> = text.split('\n').collect();
    assert_eq!(lines.first().map(|l| l.trim()), Some("Company bv"));
    assert_eq!(lines.get(1).map(|l| l.trim()), Some(""), "one empty line, as in Outlook");
    assert_eq!(lines.get(2).map(|l| l.trim()), Some("An street 1"));
    assert!(lines.iter().any(|l| l.contains("Mob") && l.contains("+32")));
    assert!(!text.contains("[picture]") && !text.contains('\u{FFFC}'), "the other program's pictures aren't brought along");
    let at = |needle: &str| styles.style(text.find(needle).map(|b| text[..b].chars().count()).unwrap());
    let company = at("Company");
    assert!(company.bold && company.color == Some([0xBF, 0xBF, 0xBF]), "{company:?}");
    assert_eq!(at("www.company").link.as_deref(), Some("http://www.company.com/"));
    let quote = at("Choose");
    assert!(quote.italic && quote.color == Some([0xBF, 0xBF, 0xBF]), "{quote:?}");
    assert!(at("The information").italic);
    // Issue #1 again: each part keeps its size and typeface (Arial 14, Tahoma 10, 14, 10).
    assert_eq!((company.size, company.font.as_deref()), (Some(14), Some("Arial")));
    assert_eq!((at("Belgium").size, at("Belgium").font.as_deref()), (Some(10), Some("Tahoma")));
    assert_eq!((quote.size, at("The information").size), (Some(14), Some(10)));
    let html = rich::to_html(&text, &styles);
    assert!(html.contains("font-family:'Tahoma',sans-serif") && html.contains("font-size:14pt"), "{html}");
    // As kept and read again (a saved signature): the same.
    let (again, again_styles) = pasted_html(&html);
    assert_eq!(again, text);
    assert_eq!(again_styles.style(text.find("Belgium").map(|b| text[..b].chars().count()).unwrap()).size, Some(10));
    assert!(!at("Belgium").bold && !at("Belgium").italic);
    // As the message sends it: HTML with the look, and a plain-text twin.
    let out = rich::to_html(&text, &styles);
    assert!(out.contains("<b") || out.contains("font-weight"), "{out}");
    assert!(out.contains("http://www.company.com/"));
    // Into a message, between what was there.
    let (mut body, mut st) = ("Hi\n\nBye".to_string(), rich::Styles::plain("Hi\n\nBye"));
    rich::splice(&mut body, &mut st, 3..3, &text, &styles);
    assert!(body.starts_with("Hi\nCompany bv") && body.ends_with("\nBye"), "{body}");
    assert_eq!(st.chars.len(), body.chars().count());
    assert!(st.style(3).bold && !st.style(0).bold && !st.style(body.chars().count() - 1).italic);
    // Word's paragraphs are lines (one line per paragraph isn't double-spaced); a web
    // page's keep their space.
    assert_eq!(pasted_html("<p class=MsoNormal>Riley<o:p></o:p></p>\r\n<p class=MsoNormal>Studio</p>").0, "Riley\nStudio");
    assert_eq!(pasted_html("<p>Riley</p><p>Studio</p>").0, "Riley\n\nStudio");
    assert_eq!(pasted_html("<P CLASS=MsoNormal>a</P><pre class=MsoNormal>b</pre>").0.matches('a').count(), 1);
    assert!(pasted_html("").0.is_empty());
    assert!(pasted_html("<img src=\"https://x.example/a.png\">").0.is_empty());
}

const SIMPLE: &[u8] = b"From: Zoe Park <zoe@studio.example>\r\n\
To: Ann <ann@studio.example>, ops@studio.example\r\n\
Cc: Kim <kim@studio.example>\r\n\
Subject: =?UTF-8?Q?Caf=C3=A9_plans?=\r\n\
Date: Wed, 14 Oct 2026 09:30:00 +0200\r\n\
Message-ID: <m1@studio.example>\r\n\
References: <m0@studio.example>\r\n\
In-Reply-To: <m0@studio.example>\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
\r\n\
Hello team,\r\n\
\r\n\
> quoted earlier text\r\n\
Let's meet at the caf\xc3\xa9.\r\n\
-- \r\n\
Zoe\r\n";

const MULTI: &[u8] = b"From: a@x.example\r\n\
Subject: Pictures\r\n\
Message-ID: <p@x>\r\n\
MIME-Version: 1.0\r\n\
Content-Type: multipart/mixed; boundary=\"B\"\r\n\
\r\n\
--B\r\n\
Content-Type: text/html; charset=utf-8\r\n\
\r\n\
<html><body><p>See <b>attached</b>.</p><img src=\"https://tracker.example/p.gif\"></body></html>\r\n\
--B\r\n\
Content-Type: image/png; name=\"../../evil.png\"\r\n\
Content-Disposition: attachment; filename=\"../../evil.png\"\r\n\
Content-Transfer-Encoding: base64\r\n\
\r\n\
iVBORw0KGgo=\r\n\
--B--\r\n";

fn key() -> MessageKey {
    MessageKey { account: "a1".into(), folder: "INBOX".into(), validity: 1, uid: 9 }
}

#[test]
fn summary_fields() {
    let s = summary_from_header(SIMPLE, key(), Flags::default(), 321, 5);
    assert_eq!(s.subject, "Café plans");
    assert_eq!(s.from, EmailAddress::new("Zoe Park", "zoe@studio.example"));
    assert_eq!(s.to.len(), 2);
    assert_eq!(s.cc[0].name, "Kim");
    assert_eq!(s.message_id, "m1@studio.example");
    assert_eq!(s.in_reply_to, "m0@studio.example");
    assert_eq!(s.references, vec!["m0@studio.example".to_string()]);
    assert_eq!(s.date, 1_791_963_000);
    assert_eq!(s.preview, "Hello team, Let's meet at the café.");
    assert!(!s.has_attachments);
    assert_eq!(s.size, 321);
}

#[test]
fn bulk_mail_is_recognised() {
    let s = |head: &str| summary_from_header(format!("{head}From: a@x.example\r\nSubject: s\r\n\r\nb").as_bytes(), key(), Flags::default(), 0, 0);
    assert!(!s("").bulk);
    assert!(s("List-Id: <news.x.example>\r\n").bulk);
    assert!(s("List-Unsubscribe: <mailto:u@x.example>\r\n").bulk);
    assert!(s("Precedence: bulk\r\n").bulk);
    assert!(s("Auto-Submitted: auto-generated\r\n").bulk);
    assert!(!s("Auto-Submitted: no\r\n").bulk);
    assert!(summary_from_header(b"From: noreply@shop.example\r\n\r\nx", key(), Flags::default(), 0, 0).bulk);
}

#[test]
fn garbage_never_panics() {
    for raw in [&b""[..], b"\xff\xfe\x00", b"Subject", b"Content-Type: multipart/mixed; boundary=\r\n\r\n--\r\n--", b"From: <<<@>>>\r\n\r\n"] {
        let _ = summary_from_header(raw, key(), Flags::default(), 0, 7);
        let _ = parse(raw);
        let _ = attachment_data(raw, 3);
    }
    assert_eq!(html_to_text(""), "");
    let _ = html_to_text("<table><tr><td><table><tr><td>x</td></tr></table></td></tr></table><<<>>>&#xFFFFFFFF;");
}

/// A message inside a message inside a multipart, ended by the outer boundary (found by
/// `fuzz/`): mail-parser trips its own `debug_assert!`, a panic in every debug build until the
/// workspace's profiles turned its debug assertions off (Cargo.toml).
#[test]
fn nested_messages_ended_by_the_outer_boundary_never_panic() {
    let raw = b"Content-Type:multipart;boundary=b1\n\n--b1Content-Type:message/rfc822\n\nContent-Type:message/rfc822\n\n\n--b1--";
    let _ = parse(raw);
    let _ = summary_from_full(raw, key(), Flags::default(), 0, 0);
    let _ = search_text(raw);
    let _ = attachment_data(raw, 0);
}

#[test]
fn rich_html_keeps_formatting_links_and_pictures() {
    let r = html_to_rich(
        "<p>Hello <b>bold</b> and <i>slanted</i> <a href=\"https://x.example/a\">link</a>.</p><p><img src=\"cid:logo@x\" alt=\"Logo\"><img src=\"https://t.example/p.gif\" alt=\"\"></p><pre>code</pre><script>alert(1)</script>",
    );
    let all: Vec<&Span> = r.iter().flatten().collect();
    assert!(all.iter().any(|s| s.bold && s.text.contains("bold")));
    assert!(all.iter().any(|s| s.italic && s.text.contains("slanted")));
    assert!(all.iter().any(|s| s.link.as_deref() == Some("https://x.example/a") && s.text.contains("link")));
    assert!(all.iter().any(|s| s.image.as_deref() == Some("cid:logo@x")));
    assert!(all.iter().any(|s| s.code && s.text.contains("code")));
    assert!(!all.iter().any(|s| s.text.contains("alert")), "scripts never show");
    let h = html_to_rich("<h2>Big news</h2><p># not a heading</p>");
    assert_eq!((h[0][0].text.as_str(), h[0][0].heading), ("Big news", 2));
    assert!(h.iter().flatten().any(|s| s.heading == 0 && s.text.starts_with('#')), "{h:?}");
    // A picture without a description keeps its place (shown once the web is allowed).
    assert_eq!(remote_images(&r), ["https://t.example/p.gif"]);
    assert!(all.iter().filter(|s| s.image.is_some()).all(|s| !s.text.contains(PICTURE_MARK)));
    assert!(html_to_rich("").is_empty());
    let p = parse(MULTI).unwrap();
    assert!(!p.rich.is_empty());
    assert_eq!(p.images, vec!["https://tracker.example/p.gif".to_string()]);
    assert_eq!(
        html_images("<IMG alt=x SRC='https://a.example/1.png?x=1&amp;y=2'><img src=cid:k@x><img src=\"cid:k@x\"><img>"),
        vec!["https://a.example/1.png?x=1&y=2".to_string(), "cid:k@x".to_string()]
    );
    assert_eq!(remote_images(&html_to_rich("<img src=\"https://a.example/x.png\" alt=\"Map\">")), vec!["https://a.example/x.png".to_string()]);
    assert!(parse(SIMPLE).unwrap().rich.is_empty(), "plain mail has no rich body");
}

/// Issue #11: a WebP sent without a name, or with a name without an extension, is named with its
/// type's extension (it was saved as `attachment-1`, which nothing would open); one sent as
/// `application/octet-stream` is still a picture; a name with its own extension is left as it is.
#[test]
fn attachments_are_named_for_their_type() {
    let webp = b"RIFF\x1a\0\0\0WEBPVP8L\x0d\0\0\0\x2f\0\0\0\x10\x07\x10\x11\x11\x88\x88\xfe\x07\0";
    let part = |headers: &str| {
        format!("--b\r\n{headers}\r\nContent-Transfer-Encoding: base64\r\n\r\nUklGRhoAAABXRUJQVlA4TA0AAAAvAAAAEAcQERGIiP4HAA==\r\n").into_bytes()
    };
    let mut raw = b"From: a@x.example\r\nTo: b@x.example\r\nSubject: Pictures\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"b\"\r\n\r\n--b\r\nContent-Type: text/plain\r\n\r\nHere.\r\n".to_vec();
    raw.extend(part("Content-Type: image/webp\r\nContent-Disposition: attachment"));
    raw.extend(part("Content-Type: image/webp; name=\"holiday\""));
    raw.extend(part("Content-Type: application/octet-stream; name=\"beach.webp\""));
    raw.extend(part("Content-Type: image/webp; name=\"scan.pdf\""));
    raw.extend(b"--b--\r\n");
    let p = parse(&raw).unwrap();
    let names: Vec<&str> = p.attachments.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, ["attachment-1.webp", "holiday.webp", "beach.webp", "scan.pdf"]);
    assert!(p.attachments.iter().take(3).all(|a| a.kind() == AttachmentKind::Image), "{:?}", p.attachments);
    assert_eq!(attachment_data(&raw, 0).as_deref(), Some(&webp[..]), "saved as it came");
}

#[test]
fn html_and_attachments() {
    let p = parse(MULTI).unwrap();
    assert!(p.had_html);
    assert!(p.text.contains("See") && p.text.contains("attached"));
    assert!(!p.text.contains("<b>"));
    assert_eq!(p.attachments.len(), 1);
    let a = &p.attachments[0];
    assert_eq!(a.name, "evil.png");
    assert_eq!(a.mime, "image/png");
    assert_eq!(a.kind(), AttachmentKind::Image);
    assert_eq!(attachment_data(MULTI, 0).unwrap()[..4], [0x89, b'P', b'N', b'G']);
    let s = summary_from_full(MULTI, key(), Flags::default(), 0, 0);
    assert!(s.has_attachments);
    assert!(s.preview.contains("attached"));
}

#[test]
fn build_round_trips() {
    let d = Draft {
        to: vec![EmailAddress::new("Ann", "ann@studio.example")],
        cc: vec![EmailAddress::new("", "kim@studio.example")],
        bcc: vec![EmailAddress::new("", "secret@studio.example")],
        subject: "Storyboard v2 ✏️".into(),
        body: "Hi Ann,\nhere it is.\n".into(),
        in_reply_to: "m1@studio.example".into(),
        references: vec!["m0@studio.example".into(), "m1@studio.example".into()],
        attachments: vec![OutAttachment { name: "board.png".into(), mime: String::new(), data: vec![1, 2, 3, 4] }],
        ..Default::default()
    };
    let raw = build(&d, &EmailAddress::new("Zoe Park", "zoe@studio.example"), 1_791_963_000, "x1@studio.example").unwrap();
    let text = String::from_utf8_lossy(&raw);
    assert!(!text.contains("secret@"), "Bcc must not be written");
    let p = parse(&raw).unwrap();
    assert_eq!(p.subject, d.subject);
    assert_eq!(p.to, d.to);
    assert_eq!(p.cc, d.cc);
    assert_eq!(p.message_id, "x1@studio.example");
    assert_eq!(p.text.trim(), "Hi Ann,\nhere it is.");
    assert_eq!(p.attachments[0].name, "board.png");
    assert_eq!(p.attachments[0].mime, "image/png");
    assert_eq!(attachment_data(&raw, 0).unwrap(), vec![1, 2, 3, 4]);
    let s = summary_from_header(&raw, key(), Flags::default(), 0, 0);
    assert_eq!(s.references, d.references);
    assert_eq!(s.in_reply_to, "m1@studio.example");
    assert_eq!(d.recipients(), vec!["ann@studio.example", "kim@studio.example", "secret@studio.example"]);
    // The same message builds the same bytes: nothing comes from the clock (which a browser
    // doesn't have), and each message's boundaries are its own.
    assert_eq!(build(&d, &EmailAddress::new("Zoe Park", "zoe@studio.example"), 1_791_963_000, "x1@studio.example").unwrap(), raw);
    let other = build(&d, &EmailAddress::new("Zoe Park", "zoe@studio.example"), 1_791_963_000, "x2@studio.example").unwrap();
    assert_ne!(other, raw);
    // Replies sent elsewhere; a held-back message dated when it goes.
    let to_desk = Draft { reply_to: vec![EmailAddress::new("Desk", "desk@studio.example")], ..d.clone() };
    let raw2 = build(&to_desk, &EmailAddress::new("Zoe Park", "zoe@studio.example"), 1_791_963_000, "x3@studio.example").unwrap();
    assert_eq!(parse(&raw2).unwrap().reply_to, to_desk.reply_to);
    let later = with_date(&raw2, 1_792_000_000);
    let p2 = parse(&later).unwrap();
    assert_eq!((p2.date, p2.subject.as_str(), p2.text.trim()), (1_792_000_000, d.subject.as_str(), "Hi Ann,\nhere it is."));
    assert_eq!(String::from_utf8_lossy(&later).matches("Date:").count(), 1);
    assert_eq!(with_date(b"no header block", 5), b"no header block".to_vec());
    let rich = Draft {
        html: Some("<p>Hi <b>Ann</b><img src=\"cid:pic@x\"></p>".into()),
        inline: vec![InlinePicture { cid: "pic@x".into(), name: "pic.png".into(), mime: "image/png".into(), data: vec![9, 9] }],
        ..d.clone()
    };
    let raw = build(&rich, &EmailAddress::new("", "zoe@studio.example"), 5, "x3@studio.example").unwrap();
    let text = String::from_utf8_lossy(&raw);
    for kind in ["multipart/mixed", "multipart/related", "multipart/alternative"] {
        assert!(text.contains(kind), "{kind}");
    }
    assert_eq!(text.matches("boundary=").count(), 3);
    let p = parse(&raw).unwrap();
    assert!(p.had_html && p.text.contains("Ann"));
    assert_eq!(p.attachments.iter().filter(|a| !a.inline).map(|a| a.name.as_str()).collect::<Vec<_>>(), ["board.png"]);
    // Plain text alone is a single part.
    let plain = Draft { attachments: Vec::new(), ..d };
    let raw = build(&plain, &EmailAddress::new("", "zoe@studio.example"), 5, "x4@studio.example").unwrap();
    assert!(!String::from_utf8_lossy(&raw).contains("multipart"));
}

#[test]
fn formatted_mail_with_inline_pictures() {
    let d = Draft {
        to: vec![EmailAddress::new("", "ann@studio.example")],
        subject: "Look".into(),
        body: "See the picture.".into(),
        html: Some("<p>See the <b>picture</b>.</p><img src=\"cid:pic1@mailcraft\" alt=\"pic\">".into()),
        inline: vec![InlinePicture { cid: "pic1@mailcraft".into(), name: "p.png".into(), mime: "image/png".into(), data: vec![9, 9] }],
        ..Default::default()
    };
    let raw = build(&d, &EmailAddress::new("", "z@studio.example"), 0, "f@x").unwrap();
    let text = String::from_utf8_lossy(&raw).to_ascii_lowercase();
    assert!(text.contains("multipart/alternative") && text.contains("multipart/related"), "{text}");
    let p = parse(&raw).unwrap();
    assert_eq!(p.text.trim(), "See the picture.");
    assert!(p.rich.iter().flatten().any(|s| s.bold && s.text == "picture"));
    assert_eq!(p.images, vec!["cid:pic1@mailcraft".to_string()]);
    let pic = p.attachments.iter().find(|a| a.content_id.as_deref() == Some("pic1@mailcraft")).unwrap();
    assert!(pic.inline);
    assert_eq!(attachment_data(&raw, pic.index).unwrap(), vec![9, 9]);
}

#[test]
fn replies() {
    let p = parse(SIMPLE).unwrap();
    assert!(!p.had_html, "plain text is not HTML");
    let clock = Clock::new(1_792_000_000, 0);
    let me = vec!["ann@studio.example".to_string()];
    let r = reply(&p, ReplyMode::Reply, &me, &clock, "");
    assert_eq!(r.subject, "Re: Café plans");
    assert_eq!(r.to, vec![p.from.clone()]);
    assert!(r.cc.is_empty());
    assert_eq!(r.in_reply_to, "m1@studio.example");
    assert_eq!(r.references, vec!["m0@studio.example".to_string(), "m1@studio.example".into()]);
    // Outlook's layout: a rule, who sent it to whom and when, then the original as it was.
    let tail = &r.body[r.body.find(markup::RULE).unwrap()..];
    assert!(tail.starts_with(&format!("{}\nFrom: Zoe Park <zoe@studio.example>\nSent: ", markup::RULE)), "{tail}");
    assert!(
        tail.contains("\nTo: Ann <ann@studio.example>; ops@studio.example\nCc: Kim <kim@studio.example>\nSubject: Café plans\n\nHello team,"),
        "{tail:?}"
    );
    assert!(!r.body.contains("wrote:"));
    // Sent formatted: the rule is a rule, the labels bold, the rest plain text.
    let html = markup::to_html(&r.body);
    assert!(html.contains("<hr") && html.contains("<b>From:</b> Zoe Park &lt;zoe@studio.example&gt;") && html.contains("<b>Subject:</b> Café plans"));
    assert!(html.contains("Hello team,") && !html.contains("<b>Hello"));
    assert!(markup::has_formatting(&r.body));
    assert_eq!(markup::header_lines("x\n____________\nFrom: a\n  b\nTo: c\n\nFrom: not a header"), [false, false, true, true, true, false, false]);
    let all = reply(&p, ReplyMode::ReplyAll, &me, &clock, "Ann");
    let cc: Vec<&str> = all.cc.iter().map(|a| a.email.as_str()).collect();
    assert_eq!(cc, vec!["ops@studio.example", "kim@studio.example"]);
    assert!(all.body.starts_with("\n\n-- \nAnn"));
    let f = reply(&p, ReplyMode::Forward, &me, &clock, "");
    assert_eq!(f.subject, "Fwd: Café plans");
    assert!(f.to.is_empty() && f.in_reply_to.is_empty());
    assert!(f.body.contains("Subject: Café plans"));
    let rr = reply(&parse(&build(&r, &EmailAddress::new("", "ann@studio.example"), 0, "r@x").unwrap()).unwrap(), ReplyMode::Reply, &me, &clock, "");
    assert_eq!(rr.subject, "Re: Café plans");
    // Replying to my own message addresses its recipients.
    assert_eq!(rr.to[0].email, "zoe@studio.example");
}

#[test]
fn html_tables() {
    let html = "<p>Before</p><table><tr><th>Item</th><th>Qty</th></tr><tr><td>Poster A2</td><td>2</td></tr></table>                    <table width=600><tr><td><p>Layout cell</p></td></tr></table><p>After</p>";
    let rich = html_to_rich(html);
    let lines: Vec<String> = rich.iter().map(|l| l.iter().map(|s| s.text.as_str()).collect()).collect();
    let row = rich.iter().find(|l| l.iter().any(|s| s.text.contains("Poster A2"))).unwrap();
    assert!(row.iter().all(|s| s.code), "data rows line up in monospace");
    let layout = rich.iter().find(|l| l.iter().any(|s| s.text.contains("Layout cell"))).unwrap();
    assert!(!layout.iter().any(|s| s.code), "a layout table is just text");
    assert!(!lines.iter().any(|l| !l.is_empty() && l.chars().all(|c| c == '─')), "no stray rules: {lines:?}");
}

#[test]
fn html_colours_and_hidden_parts() {
    let html = r#"<style>span.warn { font-weight: normal; color: #cc0000 }</style><div style="display:none">preheader text</div>
<p>Plain <span style="color:#ff0000">red</span> <span style="background-color: yellow">marked</span> <span class="warn">styled</span> <font color="blue">old</font></p>"#;
    let rich = html_to_rich(html);
    let spans: Vec<&Span> = rich.iter().flatten().collect();
    let find = |t: &str| spans.iter().find(|s| s.text.contains(t)).copied();
    assert_eq!(find("red").and_then(|s| s.color), Some([255, 0, 0]));
    assert_eq!(find("marked").and_then(|s| s.background), Some([255, 255, 0]));
    assert_eq!(find("styled").and_then(|s| s.color), Some([0xcc, 0, 0]), "<style> rules");
    assert!(find("old").and_then(|s| s.color).is_some(), "<font color>");
    assert_eq!(find("Plain").and_then(|s| s.color), None);
    assert!(find("preheader").is_none(), "display:none stays hidden");
}

/// A style sheet that stops inside an attribute selector (found by `fuzz/`): html2text panicked
/// inside its fence and the message lost its formatting; now it reads, and the rules before it
/// still apply.
#[test]
fn a_style_sheet_cut_off_in_a_selector_still_reads() {
    for html in
        ["<style>[s=", "<style>.w{color:#cc0000}[s=</style><p class=w>styled</p>", "<style>.w{color:#cc0000}\n[s= </style><p class=w>styled</p>"]
    {
        assert!(
            std::panic::catch_unwind(|| html2text::config::rich().use_doc_css().lines_from_read(close_css_rules(html).as_bytes(), 80)).is_ok(),
            "{html}"
        );
    }
    let rich = html_to_rich("<style>.w{color:#cc0000}[s=</style><p class=w>styled</p>");
    let styled = rich.iter().flatten().find(|s| s.text.contains("styled"));
    assert_eq!(styled.and_then(|s| s.color), Some([0xcc, 0, 0]), "{rich:?}");
}

#[test]
fn helpers() {
    assert_eq!(sanitize_filename("..\\..\\a:b?.txt"), "a_b_.txt");
    assert_eq!(sanitize_filename("..."), "attachment");
    assert_eq!(mime_from_name("X.JPG"), "image/jpeg");
    assert_eq!(new_message_id("z@studio.example", 255, 1), "ff.0000000000000001@studio.example");
    assert_eq!(new_message_id("bad", -5, 0), "0.0000000000000000@mailcraft.invalid");
    assert_eq!(preview("a\n\n  b   c\n> q\n-- \nsig"), "a b c");
    assert_eq!(preview(&"x".repeat(500)).chars().count(), PREVIEW_CHARS + 1);
}

/// A hostile Message-ID with a bare CR inside its brackets: the reply's In-Reply-To and
/// References carry the id without it, and nothing is written outside a line ending.
#[test]
fn a_message_id_cannot_break_a_replys_headers() {
    let raw = b"From: Ana <ana@studio.example>\r\nTo: ann@studio.example\r\nSubject: hi\r\nMessage-ID: <abc\rBcc: x@evil.example@host>\r\nReferences: <r1\r@host> <r2@host>\r\n\r\nbody\r\n";
    let p = parse(raw).unwrap();
    assert!(!p.message_id.contains(['\r', '\n', ' ']), "{:?}", p.message_id);
    let clock = Clock::new(1_792_000_000, 0);
    let r = reply(&p, ReplyMode::Reply, &["ann@studio.example".to_string()], &clock, "");
    let out = build(&r, &EmailAddress::new("Ann", "ann@studio.example"), 1_792_000_000, "n@studio.example").unwrap();
    let text = String::from_utf8_lossy(&out);
    let head = text.split("\r\n\r\n").next().unwrap_or_default();
    for (i, w) in head.as_bytes().windows(2).enumerate() {
        assert!(w[0] != b'\r' || w[1] == b'\n', "a bare CR at {i}: {head:?}");
    }
    assert!(!head.lines().any(|l| l.starts_with("Bcc:")), "no header smuggled in: {head}");
    // A draft's ids, from anywhere, are cleaned as they're written too.
    let mut d = r.clone();
    d.in_reply_to = "<x\r\nBcc: y@evil.example>".into();
    d.references = vec!["<\r\n>".into(), "<ok@host>".into()];
    let out = build(&d, &EmailAddress::new("Ann", "ann@studio.example"), 1_792_000_000, "n2@studio.example").unwrap();
    let text = String::from_utf8_lossy(&out);
    assert!(!text.split("\r\n\r\n").next().unwrap_or_default().lines().any(|l| l.starts_with("Bcc:")), "{text}");
}

/// Issue #10: pictures carry the size their `<img>` sets (attributes, or the style's, which wins),
/// each one its own when several share an address; none when nothing is set.
#[test]
fn pictures_carry_the_size_the_message_sets() {
    use layout::Width;
    let html = r#"<p><img src="cid:logo" width="200" height="50" alt="Studio">
        <img src="cid:hero" width="600" style="display:block; width:100%; max-width:600px; height:auto" alt="Hero">
        <img src="cid:dot" width="20" height="20" alt="">x
        <img src="cid:tall" height="100px" alt="H">
        <img src="cid:dot" width="40" alt="">y
        <img src="cid:plain" alt="P">
        <img src="cid:odd" width="-5" height="1e9" alt="O"></p>"#;
    let rich = html_to_rich(html);
    let sizes: Vec<(String, Option<ImageSize>)> = rich.iter().flatten().filter_map(|sp| Some((sp.image.clone()?, sp.image_size))).collect();
    let of = |src: &str, n: usize| sizes.iter().filter(|(s, _)| s == src).nth(n).and_then(|(_, z)| *z);
    assert_eq!(of("cid:logo", 0), Some(ImageSize { width: Some(Width::Px(200)), height: Some(50), max_width: None }));
    assert_eq!(of("cid:hero", 0), Some(ImageSize { width: Some(Width::Percent(100)), height: None, max_width: Some(600) }), "the style wins");
    assert_eq!(of("cid:dot", 0), Some(ImageSize { width: Some(Width::Px(20)), height: Some(20), max_width: None }));
    assert_eq!(of("cid:dot", 1), Some(ImageSize { width: Some(Width::Px(40)), height: None, max_width: None }), "each its own");
    assert_eq!(of("cid:tall", 0), Some(ImageSize { width: None, height: Some(100), max_width: None }));
    assert_eq!(of("cid:plain", 0), None);
    let odd = of("cid:odd", 0).unwrap_or_default();
    assert!(odd.width.is_none() && odd.height == Some(4000), "nonsense is left out or held to 4000: {odd:?}");
}

/// Issue #11: a signature keeps its pictures with it, written into its HTML, and they come back
/// as pictures in its text at their widths; what isn't a picture, or is too big, is left out.
#[test]
fn a_signature_keeps_its_pictures() {
    let png = vec![0x89, b'P', b'N', b'G', 1, 2, 3, 4, 5];
    let text = "Riley Park\n\u{FFFC} Studio Lantern";
    let mut st = rich::Styles::plain(text);
    st.restyle(text, 0..10, |s| rich::Style { bold: true, ..s.clone() });
    st.restyle(text, 11..12, |s| rich::Style { picture: Some("logo@x".into()), picture_width: Some(120), ..s.clone() });
    let pic = InlinePicture { cid: "logo@x".into(), name: "logo.png".into(), mime: "image/png".into(), data: png.clone() };
    let html = embed_pictures(&rich::to_html(text, &st), &[pic]);
    assert!(html.contains("src=\"data:image/png;base64,iVBORwECAwQF\"") && !html.contains("cid:"), "{html}");
    let (t, s, pictures) = signature_html(&html);
    assert_eq!(t, text);
    assert!(s.style(0).bold);
    let p = s.style(11);
    assert_eq!((p.picture.as_deref(), p.picture_width), (Some("signature1@mailcraft"), Some(120)));
    assert_eq!(pictures.len(), 1);
    assert_eq!((pictures[0].data.clone(), pictures[0].mime.as_str()), (png, "image/png"));
    // Not pictures, too big, or on the web: left out; the text stays.
    let big = "A".repeat(SIGNATURE_PICTURE_MAX / 3 * 4 + 400);
    let odd = format!(
        "<p>Hi<img src=\"data:image/svg+xml;base64,PHN2Zz4=\"><img src=\"data:image/png;base64,{big}\"><img src=\"https://x.example/a.png\"><img src=\"data:image/png;base64,!!!\">\u{E002}</p>"
    );
    let (t, _, pictures) = signature_html(&odd);
    assert_eq!((t.as_str(), pictures.len()), ("Hi", 0));
}

/// Issue #11: a signature copied from Word or Outlook with its logo: Word writes the picture into
/// the copied HTML (`data:`) after its own drawing of it (VML, in a comment that only Office reads,
/// pointing at a file of its own); the picture comes along, at the size it was given there.
#[test]
fn a_logo_copied_from_word_comes_along() {
    let png = crate::smime::base64(&[0x89, b'P', b'N', b'G', 9, 9]);
    let png = String::from_utf8(png).unwrap();
    let html = format!(
        "<html><body><!--StartFragment--><p class=MsoNormal><span style='mso-no-proof:yes'><!--[if gte vml 1]><v:shape \
         style='width:135pt;height:40.5pt'><v:imagedata src=\"file:///C:/Users/x/AppData/Local/Temp/msohtmlclip1/01/clip_image001.png\" \
         o:title=\"\"/></v:shape><![endif]--><![if !vml]><img width=180 height=54\r\nsrc=\"data:image/png;base64,{png}\"\r\n\
         v:shapes=\"Picture_x0020_1\"><![endif]></span><o:p></o:p></p>\r\n<p class=MsoNormal><b>Riley Moreno<o:p></o:p></b></p>\r\n\
         <p class=MsoNormal>Studio Lantern<o:p></o:p></p><!--EndFragment--></body></html>"
    );
    let (text, styles, pictures) = signature_html(&html);
    assert_eq!(text, "\u{FFFC}\nRiley Moreno\nStudio Lantern");
    assert_eq!(pictures.len(), 1);
    assert_eq!(pictures[0].data, vec![0x89, b'P', b'N', b'G', 9, 9]);
    assert_eq!((styles.style(0).picture.as_deref(), styles.style(0).picture_width), (Some(pictures[0].cid.as_str()), Some(180)));
    assert!(styles.style(2).bold && !styles.style(15).bold);
}

/// Issue #11: pictures older Word and Outlook copy as files of their own are written into the
/// HTML as the reader gives them, however their address is quoted; other addresses, kinds a mail
/// program doesn't show, and those past [`LOCAL_PICTURES`] are left as they are.
#[test]
fn local_pictures_are_written_into_copied_html() {
    let read = |src: &str| match src {
        "file:///a.png" | "file:///b.png" | "file:///c.png" => Some(("image/png".to_string(), vec![1, 2, 3])),
        "file:///d.svg" => Some(("image/svg+xml".to_string(), b"<svg/>".to_vec())),
        _ => None,
    };
    let html = "<img src=\"file:///a.png\" width=9><img\r\nsrc='file:///b.png'><img src=file:///c.png><img src=\"file:///d.svg\"><img src=\"file:///gone.png\"><img src=\"https://x.example/e.png\">";
    let out = local_pictures(html, read);
    assert_eq!(out.matches("src=\"data:image/png;base64,AQID\"").count(), 3, "{out}");
    assert!(out.contains("file:///d.svg") && out.contains("file:///gone.png") && out.contains("https://x.example/e.png"), "{out}");
    let many = "<img src=\"file:///a.png\">".repeat(LOCAL_PICTURES + 2);
    assert_eq!(local_pictures(&many, read).matches("data:").count(), LOCAL_PICTURES);
    assert_eq!(local_pictures("<p>no pictures</p>", read), "<p>no pictures</p>");
    assert_eq!(local_pictures("<img src=\"file:", read), "<img src=\"file:");
}

/// Mail in the East Asian code pages reads as text: Shift_JIS, GBK, Big5 and EUC-KR came out as
/// U+FFFD (found testing `winmail.dat`, issue #18, whose strings can be in them too).
#[test]
fn east_asian_mail_reads() {
    let raw = b"From: a@b.example\r\nSubject: =?shift_jis?B?gqA=?=\r\nContent-Type: text/plain; charset=shift_jis\r\n\r\n\x82\xa0\x82\xa2\r\n";
    let p = parse(raw).unwrap();
    assert_eq!((p.subject.as_str(), p.text.trim()), ("あ", "あい"));
    for (charset, bytes, text) in
        [("gbk", &b"\xc4\xe3\xba\xc3"[..], "你好"), ("big5", &b"\xa7\x41\xa6\x6e"[..], "你好"), ("euc-kr", &b"\xbe\xc8\xb3\xe7"[..], "안녕")]
    {
        let mut raw = format!("From: a@b.example\r\nSubject: x\r\nContent-Type: text/plain; charset={charset}\r\n\r\n").into_bytes();
        raw.extend(bytes);
        assert_eq!(parse(&raw).map(|p| p.text.trim().to_string()).as_deref(), Some(text), "{charset}");
    }
}
