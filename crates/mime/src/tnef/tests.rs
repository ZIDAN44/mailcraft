//! `winmail.dat` (issue #18): the specification's examples, streams written the way Outlook
//! writes them, and damaged ones.

use super::*;

/// One attribute: level, id, length, data, checksum.
pub(crate) fn attr(out: &mut Vec<u8>, level: u8, id: u32, data: &[u8]) {
    out.push(level);
    out.extend(id.to_le_bytes());
    out.extend((data.len() as u32).to_le_bytes());
    out.extend(data);
    let sum = data.iter().fold(0u16, |a, b| a.wrapping_add(u16::from(*b)));
    out.extend(sum.to_le_bytes());
}

/// A stream: signature, key, version and code page, then `attrs`.
pub(crate) fn stream(attrs: &[u8]) -> Vec<u8> {
    let mut out = SIGNATURE.to_vec();
    out.extend(1u16.to_le_bytes());
    attr(&mut out, 1, 0x0008_9006, &0x0001_0000u32.to_le_bytes());
    attr(&mut out, 1, ATT_OEM_CODEPAGE, &[0xE4, 0x04, 0, 0, 0, 0, 0, 0]);
    out.extend(attrs);
    out
}

fn padded(out: &mut Vec<u8>, b: &[u8]) {
    out.extend((b.len() as u32).to_le_bytes());
    out.extend(b);
    out.extend(std::iter::repeat_n(0, (4 - b.len() % 4) % 4));
}

/// A property list; each entry is its type, id and value as the stream holds it.
pub(crate) fn props(list: &[(u16, u16, Vec<u8>)]) -> Vec<u8> {
    let mut out = (list.len() as u32).to_le_bytes().to_vec();
    for (kind, id, value) in list {
        out.extend(kind.to_le_bytes());
        out.extend(id.to_le_bytes());
        out.extend(value);
    }
    out
}

pub(crate) fn binary(b: &[u8]) -> Vec<u8> {
    let mut out = 1u32.to_le_bytes().to_vec();
    padded(&mut out, b);
    out
}

pub(crate) fn str8(s: &str) -> Vec<u8> {
    let mut b = s.as_bytes().to_vec();
    b.push(0);
    binary(&b)
}

fn unicode(s: &str) -> Vec<u8> {
    let mut b: Vec<u8> = s.encode_utf16().chain([0]).flat_map(u16::to_le_bytes).collect();
    b.truncate(b.len());
    binary(&b)
}

fn long(n: i32) -> Vec<u8> {
    n.to_le_bytes().to_vec()
}

fn boolean(on: bool) -> Vec<u8> {
    vec![u8::from(on), 0, 0, 0]
}

/// RTF as `PidTagRtfCompressed` holds it: literals only, ended by the reference to where the
/// next byte would go (MS-OXRTFCP), so any RTF can be written.
pub(crate) fn compress(rtf: &[u8]) -> Vec<u8> {
    let mut body = Vec::new();
    let mut chunks = rtf.chunks(8).peekable();
    let end = ((DICTIONARY_LEN + rtf.len()) % 4096) as u16;
    let mut ended = false;
    while let Some(c) = chunks.next() {
        if c.len() == 8 {
            body.push(0);
            body.extend(c);
        } else {
            body.push(1 << c.len());
            body.extend(c);
            body.extend((end << 4).to_be_bytes());
            ended = true;
        }
        if chunks.peek().is_none() && !ended {
            body.push(1);
            body.extend((end << 4).to_be_bytes());
            ended = true;
        }
    }
    if !ended {
        body.push(1);
        body.extend((end << 4).to_be_bytes());
    }
    let mut out = ((body.len() + 12) as u32).to_le_bytes().to_vec();
    out.extend((rtf.len() as u32).to_le_bytes());
    out.extend(0x7546_5A4Cu32.to_le_bytes());
    out.extend(0u32.to_le_bytes());
    out.extend(body);
    out
}

const DICTIONARY_LEN: usize = 207;

/// MS-OXRTFCP 3.1.1 and 3.1.2: the specification's own compressed RTF, one with a reference that
/// reaches past where the dictionary was written to when it started.
#[test]
fn the_specifications_compressed_rtf_comes_out() {
    let simple = [
        0x2d, 0x00, 0x00, 0x00, 0x2b, 0x00, 0x00, 0x00, 0x4c, 0x5a, 0x46, 0x75, 0xf1, 0xc5, 0xc7, 0xa7, 0x03, 0x00, 0x0a, 0x00, 0x72, 0x63, 0x70,
        0x67, 0x31, 0x32, 0x35, 0x42, 0x32, 0x0a, 0xf3, 0x20, 0x68, 0x65, 0x6c, 0x09, 0x00, 0x20, 0x62, 0x77, 0x05, 0xb0, 0x6c, 0x64, 0x7d, 0x0a,
        0x80, 0x0f, 0xa0,
    ];
    assert_eq!(decompress_rtf(&simple).as_deref(), Some(&b"{\\rtf1\\ansi\\ansicpg1252\\pard hello world}\r\n"[..]));
    let crossing = [
        0x1a, 0x00, 0x00, 0x00, 0x1c, 0x00, 0x00, 0x00, 0x4c, 0x5a, 0x46, 0x75, 0xe2, 0xd4, 0x4b, 0x51, 0x41, 0x00, 0x04, 0x20, 0x57, 0x58, 0x59,
        0x5a, 0x0d, 0x6e, 0x7d, 0x01, 0x0e, 0xb0,
    ];
    assert_eq!(decompress_rtf(&crossing).as_deref(), Some(&b"{\\rtf1 WXYZWXYZWXYZWXYZWXYZ}"[..]));
    // Stored as it is (`MELA`).
    let mut plain = 16u32.to_le_bytes().to_vec();
    plain.extend(4u32.to_le_bytes());
    plain.extend(0x414C_454Du32.to_le_bytes());
    plain.extend(0u32.to_le_bytes());
    plain.extend(b"{\\rt");
    assert_eq!(decompress_rtf(&plain).as_deref(), Some(&b"{\\rt"[..]));
    assert_eq!(decompress_rtf(&compress(b"{\\rtf1 any RTF at all}")).as_deref(), Some(&b"{\\rtf1 any RTF at all}"[..]));
    // Not compressed RTF; cut short; a size far past what's there.
    assert_eq!(decompress_rtf(b"0123456789abcdefghij"), None);
    for n in 0..simple.len() {
        let _ = decompress_rtf(simple.get(..n).unwrap_or_default());
    }
    let mut huge = simple.to_vec();
    huge.splice(4..8, u32::MAX.to_le_bytes());
    assert_eq!(decompress_rtf(&huge).as_deref(), Some(&b"{\\rtf1\\ansi\\ansicpg1252\\pard hello world}\r\n"[..]));
}

/// MS-OXRTFEX 3.1: the HTML Outlook wrapped in RTF comes back out, without what the RTF added.
#[test]
fn html_wrapped_in_rtf_comes_back_out() {
    let rtf = br#"{\rtf1\ANSI\ansicpg1251\fromhtml1 \deff0
{\fonttbl {\f0\fmodern Courier New;}{\f1\fswiss Arial;}{\f2\fswiss\fcharset0 Arial;}}
{\colortbl\red0\green0\blue0;\red0\green0\blue255;}
{\*\htmltag64}
\uc1\pard\plain\deftab360 \f0\fs24
{\*\htmltag <HTML><head>\par
<style>\par
<!--\par
 p.MsoNormal, li.MsoNormal \{font-family:Arial;\}\par
-->\par
</style>\par
</head>\par
<body>\par
<p\par
class="MsoNormal">}
{\htmlrtf \f1 \htmlrtf0 Note the line break inside a P tag. {\*\htmltag <b>}{\htmlrtf \b
\htmlrtf0 This is a bold text{\*\htmltag </b>}} \htmlrtf\par\htmlrtf0}
\htmlrtf \par \htmlrtf0
{\*\htmltag </p>\par
<p class="MsoNormal">\par}
{\htmlrtf \f1 \htmlrtf0 This is a normal text with a character references:
{\*\htmltag &nbsp;}\htmlrtf \'a0\htmlrtf0 {\*\htmltag &lt;}\htmlrtf <\htmlrtf0 {\*\htmltag
&uml;}\htmlrtf {\f2\'a8}\htmlrtf0{\*\htmltag <br>\par}\htmlrtf\line\htmlrtf0
characters which have special meaning in RTF: \{\}\\{\*\htmltag
<br>\par}\htmlrtf\line\htmlrtf0\htmlrtf\par\htmlrtf0}
{\*\htmltag </p>\par
</body>\par
</HTML>\par }}"#;
    let TnefBody::Html(html) = rtf_body(rtf) else { panic!("HTML: {:?}", rtf_body(rtf)) };
    assert!(html.starts_with("<HTML><head>\r\n<style>"), "{html}");
    assert!(html.contains("p.MsoNormal, li.MsoNormal {font-family:Arial;}"), "{html}");
    assert!(html.contains("<p\r\nclass=\"MsoNormal\">Note the line break inside a P tag. <b>This is a bold text</b> </p>"), "{html}");
    assert!(html.contains("character references:&nbsp;&lt;&uml;<br>"), "{html}");
    assert!(html.contains("in RTF: {}\\<br>"), "{html}");
    assert!(html.trim_end().ends_with("</HTML>"), "{html}");
    for leftover in ["Courier", "fswiss", "htmlrtf", "\u{a0}", "\u{a8}"] {
        assert!(!html.contains(leftover), "{leftover} in {html}");
    }
}

/// RTF from plain text gives the text; RTF of its own gives HTML with its formatting, links,
/// and characters in the fonts' code pages and as Unicode.
#[test]
fn rtf_of_its_own_comes_as_html() {
    assert_eq!(
        rtf_body(b"{\\rtf1\\ansi\\fromtext \\deff0{\\fonttbl{\\f0 Courier;}}Line one\\par\nLine\\tab two\\par}"),
        TnefBody::Text("Line one\r\nLine\ttwo\r\n".into())
    );
    let rtf = br#"{\rtf1\ansi\ansicpg1252\deff0{\fonttbl{\f0\fswiss Calibri;}{\f1\fcharset204 Arial Cyr;}}{\colortbl;\red255\green0\blue0;}
{\*\generator Riched20;}\pard Hello \b bold\b0 , {\i italic} and {\ul under}\ulnone  caf\'e9 \f1\'cf\'f0\'e8\f0  \u8364?\u-10179?\u-8704?.\par
A {\field{\*\fldinst{HYPERLINK "https://studio.example/a?b=1&c=2"}}{\fldrslt{link}}} and 1 < 2 & {\strike gone}.\line
{\pict\pngblip 89504e47}\bin4 {}}}after\par}"#;
    let TnefBody::Html(html) = rtf_body(rtf) else { panic!() };
    assert!(html.contains("Hello <b>bold</b>, <i>italic</i> and <u>under</u> café При €\u{1F600}."), "{html}");
    assert!(html.contains("A <a href=\"https://studio.example/a?b=1&c=2\">link</a> and 1 &lt; 2 &amp; <s>gone</s>.<br>"), "{html}");
    assert!(html.contains("after<br>"), "{html}");
    for leftover in ["Calibri", "Riched", "255", "89504e47", "HYPERLINK"] {
        assert!(!html.contains(leftover), "{leftover} in {html}");
    }
    // Not RTF; empty; nesting far past the cap; a group that never closes.
    assert_eq!(rtf_body(b"plain"), TnefBody::None);
    assert_eq!(rtf_body(b"{\\rtf1}"), TnefBody::None);
    let deep = format!("{{\\rtf1 {}deep{}}}", "{".repeat(5000), "}".repeat(5000));
    assert!(matches!(rtf_body(deep.as_bytes()), TnefBody::Html(h) if h.contains("deep")));
    assert!(matches!(rtf_body(b"{\\rtf1 {\\b open"), TnefBody::Html(h) if h.contains("<b>open</b>")));
    // A link that isn't a web or mail address stays text.
    let TnefBody::Html(h) = rtf_body(br#"{\rtf1 {\field{\*\fldinst HYPERLINK "file:///C:/x"}{\fldrslt here}}}"#) else { panic!() };
    assert_eq!(h, "here");
}

/// A `winmail.dat` as Outlook writes one: a subject, an RTF body wrapping HTML with a picture in
/// it, a file, and an attached message with a file of its own.
pub(crate) fn outlook_stream() -> Vec<u8> {
    let rtf = br#"{\rtf1\ansi\ansicpg1252\fromhtml1 \deff0{\fonttbl{\f0\fswiss Calibri;}}{\*\htmltag19 <html>}{\*\htmltag34 <body>}{\*\htmltag <p>}\htmlrtf {\htmlrtf0 Hi Riley, the {\*\htmltag <b>}\htmlrtf\b\htmlrtf0 storyboard{\*\htmltag </b>}\htmlrtf\b0\htmlrtf0  is attached.{\*\htmltag <img src="cid:logo@studio.example">}\htmlrtf\par\htmlrtf0}{\*\htmltag </p></body></html>}}"#;
    let mut inner_attrs = Vec::new();
    attr(&mut inner_attrs, 1, ATT_SUBJECT, b"Notes from Monday\0");
    attr(
        &mut inner_attrs,
        1,
        ATT_MSG_PROPS,
        &props(&[
            (0x001E, PR_BODY, str8("The notes.")),
            (0x001F, PR_SENDER_NAME, unicode("Zoe Park")),
            (0x001E, PR_SENDER_EMAIL, str8("zoe@studio.example")),
            (0x0040, PR_CLIENT_SUBMIT_TIME, 133_400_000_000_000_000u64.to_le_bytes().to_vec()),
        ]),
    );
    attr(&mut inner_attrs, 2, ATT_REND_DATA, &[1, 0, 0xFF, 0xFF, 0xFF, 0xFF, 0, 0, 0, 0, 0, 0, 0, 0]);
    attr(&mut inner_attrs, 2, ATT_TITLE, b"notes.txt\0");
    attr(&mut inner_attrs, 2, ATT_DATA, b"Monday's notes");
    let inner = stream(&inner_attrs);
    let mut object = vec![0x07, 0x03, 0x02, 0x00, 0, 0, 0, 0, 0xC0, 0, 0, 0, 0, 0, 0, 0x46];
    object.extend(&inner);

    let mut a = Vec::new();
    attr(&mut a, 1, ATT_SUBJECT, b"Storyboard\0");
    attr(&mut a, 1, 0x0007_8008, b"IPM.Microsoft Mail.Note\0");
    attr(
        &mut a,
        1,
        ATT_MSG_PROPS,
        &props(&[
            (0x000B, 0x0002, boolean(true)),
            // A named property, skipped.
            (
                0x001F,
                0x8001,
                [
                    vec![7; 16],
                    1u32.to_le_bytes().to_vec(),
                    {
                        let mut n = Vec::new();
                        padded(&mut n, &[b'x', 0, 0, 0]);
                        n
                    },
                    unicode("skip"),
                ]
                .concat(),
            ),
            (0x0003, PR_INTERNET_CPID, long(1252)),
            (0x1003, 0x1234, [2u32.to_le_bytes(), 1u32.to_le_bytes(), 2u32.to_le_bytes()].concat()),
            (0x0102, PR_RTF_COMPRESSED, binary(&compress(rtf))),
            (0x001E, PR_BODY, str8("Hi Riley, the storyboard is attached.")),
        ]),
    );
    attr(&mut a, 2, ATT_REND_DATA, &[1, 0, 0xFF, 0xFF, 0xFF, 0xFF, 0, 0, 0, 0, 0, 0, 0, 0]);
    attr(&mut a, 2, ATT_TITLE, b"STORYB~1.PDF\0");
    attr(&mut a, 2, ATT_DATA, b"%PDF-1.4 storyboard");
    attr(
        &mut a,
        2,
        ATT_ATTACHMENT,
        &props(&[
            (0x001F, PR_ATTACH_LONG_FILENAME, unicode("Storyboard – scenes 4 to 7.pdf")),
            (0x001E, PR_ATTACH_MIME_TAG, str8("application/pdf")),
        ]),
    );
    attr(&mut a, 2, ATT_REND_DATA, &[1, 0, 0xFF, 0xFF, 0xFF, 0xFF, 0, 0, 0, 0, 0, 0, 0, 0]);
    attr(
        &mut a,
        2,
        ATT_ATTACHMENT,
        &props(&[
            (0x001E, PR_ATTACH_LONG_FILENAME, str8("image001.png")),
            (0x0102, PR_ATTACH_DATA, binary(b"\x89PNG\r\n\x1a\nlogo")),
            (0x001E, PR_ATTACH_CONTENT_ID, str8("logo@studio.example")),
            (0x000B, PR_ATTACHMENT_HIDDEN, boolean(true)),
        ]),
    );
    attr(&mut a, 2, ATT_REND_DATA, &[1, 0, 0xFF, 0xFF, 0xFF, 0xFF, 0, 0, 0, 0, 0, 0, 0, 0]);
    attr(
        &mut a,
        2,
        ATT_ATTACHMENT,
        &props(&[
            (0x001E, PR_DISPLAY_NAME, str8("Notes from Monday")),
            (0x0003, PR_ATTACH_METHOD, long(5)),
            (0x000D, PR_ATTACH_DATA, binary(&object)),
        ]),
    );
    stream(&a)
}

#[test]
fn a_winmail_dat_gives_its_files_and_body() {
    let t = read_tnef(&outlook_stream()).unwrap();
    assert_eq!(t.subject, "Storyboard");
    let TnefBody::Html(html) = &t.body else { panic!("{:?}", t.body) };
    assert!(html.contains("<p>Hi Riley, the <b>storyboard</b> is attached.<img src=\"cid:logo@studio.example\">"), "{html}");
    let names: Vec<&str> = t.attachments.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, ["Storyboard – scenes 4 to 7.pdf", "image001.png", "Notes from Monday.eml"]);
    let [pdf, png, eml] = &t.attachments[..] else { panic!() };
    assert_eq!((pdf.mime.as_str(), pdf.data.as_slice(), pdf.hidden), ("application/pdf", &b"%PDF-1.4 storyboard"[..], false));
    assert_eq!((png.mime.as_str(), png.content_id.as_deref(), png.hidden), ("image/png", Some("logo@studio.example"), true));
    assert_eq!(eml.mime, "message/rfc822");
    let inner = crate::parse(&eml.data).unwrap();
    assert_eq!((inner.subject.as_str(), inner.text.trim(), inner.from.email.as_str()), ("Notes from Monday", "The notes.", "zoe@studio.example"));
    assert!(inner.date > 1_600_000_000, "{}", inner.date);
    assert_eq!(inner.attachments.first().map(|a| a.name.as_str()), Some("notes.txt"));
}

/// The plain text when there's no RTF; the HTML property over the RTF; strings in the message's
/// code page.
#[test]
fn a_winmail_dat_body_falls_back() {
    let mut a = Vec::new();
    attr(&mut a, 1, ATT_BODY, b"Only the text\r\nattribute\0");
    assert_eq!(read_tnef(&stream(&a)).map(|t| t.body), Some(TnefBody::Text("Only the text\nattribute".into())));
    let mut a = Vec::new();
    attr(
        &mut a,
        1,
        ATT_MSG_PROPS,
        &props(&[
            (0x0003, PR_INTERNET_CPID, long(1251)),
            (0x001E, PR_SUBJECT, str8("\u{0}")),
            (0x0102, PR_HTML, binary(b"<p>\xcf\xf0\xe8\xe2\xe5\xf2</p>")),
            (0x0102, PR_RTF_COMPRESSED, binary(&compress(b"{\\rtf1 no}"))),
        ]),
    );
    let t = read_tnef(&stream(&a)).unwrap();
    assert_eq!(t.body, TnefBody::Html("<p>Привет</p>".into()));
    // Bad RTF: the text instead.
    let mut a = Vec::new();
    attr(&mut a, 1, ATT_MSG_PROPS, &props(&[(0x0102, PR_RTF_COMPRESSED, binary(b"junk")), (0x001E, PR_BODY, str8("text"))]));
    assert_eq!(read_tnef(&stream(&a)).map(|t| t.body), Some(TnefBody::Text("text".into())));
}

/// Damaged streams: never a panic, and what came before the damage is kept.
#[test]
fn a_damaged_winmail_dat_never_crashes() {
    let good = outlook_stream();
    for n in 0..good.len() {
        let _ = read_tnef(good.get(..n).unwrap_or_default());
    }
    // Every byte changed in turn (a cheap fuzz).
    for i in (0..good.len()).step_by(3) {
        let mut bad = good.clone();
        if let Some(b) = bad.get_mut(i) {
            *b ^= 0xA5;
        }
        let _ = read_tnef(&bad);
        if let Some(b) = bad.get_mut(i) {
            *b = 0xFF;
        }
        let _ = read_tnef(&bad);
    }
    assert_eq!(read_tnef(b"not tnef"), None);
    assert_eq!(read_tnef(&SIGNATURE), None);
    // Lengths and counts far past what's there.
    let mut a = Vec::new();
    a.extend([1u8]);
    a.extend(ATT_MSG_PROPS.to_le_bytes());
    a.extend(u32::MAX.to_le_bytes());
    let t = read_tnef(&stream(&a)).unwrap();
    assert!(t.attachments.is_empty() && t.body.is_empty());
    let mut a = Vec::new();
    attr(&mut a, 1, ATT_MSG_PROPS, &[0xFF, 0xFF, 0xFF, 0xFF, 0x1E, 0, 0, 0x10, 0xFF, 0xFF, 0xFF, 0x7F]);
    attr(&mut a, 1, ATT_SUBJECT, b"Kept\0");
    assert_eq!(read_tnef(&stream(&a)).map(|t| t.subject), Some("Kept".into()));
    // Attached messages inside attached messages, past the depth read.
    let mut s = outlook_stream();
    for _ in 0..6 {
        let mut object = vec![0x07, 0x03, 0x02, 0x00, 0, 0, 0, 0, 0xC0, 0, 0, 0, 0, 0, 0, 0x46];
        object.extend(&s);
        let mut a = Vec::new();
        attr(&mut a, 2, ATT_REND_DATA, &[1, 0]);
        attr(&mut a, 2, ATT_ATTACHMENT, &props(&[(0x0003, PR_ATTACH_METHOD, long(5)), (0x000D, PR_ATTACH_DATA, binary(&object))]));
        s = stream(&a);
    }
    let _ = read_tnef(&s);
    // More attachments than are read.
    let mut a = Vec::new();
    for _ in 0..MAX_FILES + 10 {
        attr(&mut a, 2, ATT_REND_DATA, &[]);
        attr(&mut a, 2, ATT_DATA, b"x");
    }
    assert_eq!(read_tnef(&stream(&a)).map(|t| t.attachments.len()), Some(MAX_FILES));
}

#[test]
fn code_pages_read() {
    assert_eq!(decode(b"caf\xe9", 1252), "café");
    assert_eq!(decode(b"\xcf\xf0\xe8", 1251), "При");
    assert_eq!(decode(b"\x82\xa0", 932), "あ");
    assert_eq!(decode("ünï".as_bytes(), 65001), "ünï");
    assert_eq!(decode(b"plain", 99999), "plain");
}

/// A message as Outlook sends one in Rich Text Format: a plain-text part and a `winmail.dat`.
pub(crate) fn outlook_message(dat: Vec<u8>) -> Vec<u8> {
    MessageBuilder::new()
        .from(("Sam Okafor", "sam@studio.example"))
        .to("riley@studio.example")
        .subject("Storyboard")
        .text_body("Hi Riley, the storyboard is attached.")
        .attachment("application/ms-tnef", "winmail.dat", dat)
        .write_to_vec()
        .unwrap_or_default()
}

/// The reading pane shows the formatted body and lists the files, by indexes that read them;
/// search finds them; a `winmail.dat` that can't be read stays one.
#[test]
fn a_message_from_outlook_shows_what_its_winmail_dat_holds() {
    let raw = outlook_message(outlook_stream());
    let p = crate::parse(&raw).unwrap();
    assert!(p.had_html, "{p:?}");
    assert!(p.rich.iter().flatten().any(|s| s.bold && s.text.contains("storyboard")), "{:?}", p.rich);
    assert_eq!(p.images, ["cid:logo@studio.example"]);
    let list: Vec<(&str, bool)> = p.attachments.iter().map(|a| (a.name.as_str(), a.inline)).collect();
    assert_eq!(list, [("Storyboard – scenes 4 to 7.pdf", false), ("image001.png", true), ("Notes from Monday.eml", false)]);
    assert_eq!(p.attachments.get(1).and_then(|a| a.content_id.as_deref()), Some("logo@studio.example"));
    assert_eq!(crate::attachment_data(&raw, 0).as_deref(), Some(&b"%PDF-1.4 storyboard"[..]));
    assert_eq!(crate::attachment_data(&raw, 1).as_deref(), Some(&b"\x89PNG\r\n\x1a\nlogo"[..]));
    assert!(crate::attachment_data(&raw, 2).and_then(|e| crate::parse(&e)).is_some_and(|m| m.subject == "Notes from Monday"));
    assert_eq!(crate::attachment_data(&raw, 3), None);
    assert!(crate::search_text(&raw).contains("Storyboard – scenes 4 to 7.pdf"));

    let raw = outlook_message(b"\x78\x9f\x3e\x22 broken".to_vec());
    let p = crate::parse(&raw).unwrap();
    assert_eq!(p.attachments.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["winmail.dat"]);
    assert_eq!(p.text.trim(), "Hi Riley, the storyboard is attached.");
    assert!(!p.had_html);
}

/// Found reading Outlook's own Rich Text messages: the body ends with the document's group (the
/// bytes after it, left from an older body, came out as a second copy with its Russian garbled);
/// a link closes before the line break after it; a picture the body shows is in it, though
/// Outlook didn't mark it hidden.
#[test]
fn outlooks_rich_text_reads_as_outlook_shows_it() {
    let rtf = br#"{\rtf1\ansi\ansicpg1252\deff0{\fonttbl{\f0\fswiss\fcharset0 Arial;}{\f1\fswiss\fcharset204 Arial;}}
\pard\f0 Hi \f1\'cf\'f0\'e8\f0 .\par
{{\field{\*\fldinst{HYPERLINK "https://studio.example/notes"}}{\fldrslt{\ul Notes}}}}\par
}
 {\i \htmlrtf0 attached {\*\htmltag92 </i>}\htmlrtf }\htmlrtf0 \f4\'cf\'f0\'e8}"#;
    assert_eq!(rtf_body(rtf), TnefBody::Html("Hi При.<br>\n<a href=\"https://studio.example/notes\"><u>Notes</u></a><br>\n".into()));

    let mut a = Vec::new();
    attr(&mut a, 1, ATT_MSG_PROPS, &props(&[(0x0102, PR_HTML, binary(b"<p><img src=\"cid:logo@x\"></p>"))]));
    attr(&mut a, 2, ATT_REND_DATA, &[]);
    attr(
        &mut a,
        2,
        ATT_ATTACHMENT,
        &props(&[
            (0x001E, PR_ATTACH_LONG_FILENAME, str8("logo.png")),
            (0x0102, PR_ATTACH_DATA, binary(b"png")),
            (0x001E, PR_ATTACH_CONTENT_ID, str8("logo@x")),
        ]),
    );
    attr(&mut a, 2, ATT_REND_DATA, &[]);
    attr(
        &mut a,
        2,
        ATT_ATTACHMENT,
        &props(&[
            (0x001E, PR_ATTACH_LONG_FILENAME, str8("other.png")),
            (0x0102, PR_ATTACH_DATA, binary(b"png")),
            (0x001E, PR_ATTACH_CONTENT_ID, str8("other@x")),
        ]),
    );
    let p = crate::parse(&outlook_message(stream(&a))).unwrap();
    let inline: Vec<(&str, bool)> = p.attachments.iter().map(|a| (a.name.as_str(), a.inline)).collect();
    assert_eq!(inline, [("logo.png", true), ("other.png", false)]);

    // So in an attached message: its picture stays in its body.
    let mut inner = Vec::new();
    attr(&mut inner, 1, ATT_MSG_PROPS, &props(&[(0x001E, PR_SUBJECT, str8("Inner")), (0x0102, PR_HTML, binary(b"<p><img src=\"cid:logo@x\"></p>"))]));
    attr(&mut inner, 2, ATT_REND_DATA, &[]);
    attr(
        &mut inner,
        2,
        ATT_ATTACHMENT,
        &props(&[
            (0x001E, PR_ATTACH_LONG_FILENAME, str8("logo.png")),
            (0x0102, PR_ATTACH_DATA, binary(b"png")),
            (0x001E, PR_ATTACH_CONTENT_ID, str8("logo@x")),
        ]),
    );
    let mut object = vec![0x07, 0x03, 0x02, 0x00, 0, 0, 0, 0, 0xC0, 0, 0, 0, 0, 0, 0, 0x46];
    object.extend(stream(&inner));
    let mut a = Vec::new();
    attr(&mut a, 2, ATT_REND_DATA, &[]);
    attr(&mut a, 2, ATT_ATTACHMENT, &props(&[(0x0003, PR_ATTACH_METHOD, long(5)), (0x000D, PR_ATTACH_DATA, binary(&object))]));
    let t = read_tnef(&stream(&a)).unwrap();
    let eml = crate::parse(&t.attachments[0].data).unwrap();
    assert_eq!((eml.subject.as_str(), eml.images.as_slice()), ("Inner", &["cid:logo@x".to_string()][..]));
    assert!(eml.attachments.iter().all(|a| a.inline), "{:?}", eml.attachments);
}
