use super::item::{File, item_of};
use super::props::{decode, one_off_address, val};
use super::rebuild::to_raw;
use super::*;

/// A one-off entry id holding `name`, `kind` and `address`, in UTF-16 or 8-bit.
fn one_off(name: &str, kind: &str, address: &str, unicode: bool) -> Vec<u8> {
    let mut b = vec![0, 0, 0, 0, 0x81, 0x2B, 0x1F, 0xA4, 0xBE, 0xA3, 0x10, 0x19, 0x9D, 0x6E, 0x00, 0xDD, 0x01, 0x0F, 0x54, 0x02, 0, 0];
    b.extend_from_slice(&(if unicode { 0x8000u16 } else { 0 }).to_le_bytes());
    for s in [name, kind, address] {
        if unicode {
            b.extend(s.encode_utf16().chain([0]).flat_map(u16::to_le_bytes));
        } else {
            b.extend(s.bytes().chain([0]));
        }
    }
    b
}

/// A recipient with no address but in its one-off entry id still comes in, by that address;
/// anything else in that property is no address.
#[test]
fn a_recipient_s_address_from_its_one_off_entry_id() {
    let got = |b: &[u8]| one_off_address(b);
    let pair = |n: &str, a: &str| Some((n.to_string(), a.to_string()));
    assert_eq!(got(&one_off("Cy Lima", "SMTP", "cy@example.com", true)), pair("Cy Lima", "cy@example.com"));
    assert_eq!(got(&one_off("Dee", "SMTP", "dee@example.com", false)), pair("Dee", "dee@example.com"));
    assert_eq!(got(&one_off("Ex", "EX", "/o=Org/cn=Recipients/cn=ex", true)), None, "a directory path is no address");
    let good = one_off("Cy", "SMTP", "cy@example.com", true);
    for n in 0..good.len() {
        let _ = got(&good[..n]);
    }
    // No NUL at the end: the address runs to the end.
    assert_eq!(got(&good[..good.len() - 2]), pair("Cy", "cy@example.com"));
    let mut other = good.clone();
    other[4] = 0;
    assert_eq!(got(&other), None, "another provider");
    let mut big = good.clone();
    big.resize(2000, b'a');
    assert_eq!(got(&big), None, "too long");

    let props: HashMap<u16, Val<'static>> = HashMap::from([(0x001A, Val::Text("IPM.Note".into()))]);
    let entry = one_off("Cy Lima", "SMTP", "cy@example.com", true);
    let cy = HashMap::from([(0x3001, Val::Text("".into())), (0x0FFF, Val::Bin(&entry)), (0x0C15, Val::Int(2))]);
    fn get<'a>(m: &HashMap<u16, Val<'a>>, id: u16) -> Option<Val<'a>> {
        m.get(&id).cloned()
    }
    let it = item_of(&|id| get(&props, id), &[|id| get(&cy, id)], Vec::new(), &crate::pst_pim::Named::default());
    assert_eq!(it.cc.iter().map(|a| (a.name.as_str(), a.email.as_str())).collect::<Vec<_>>(), [("Cy Lima", "cy@example.com")]);
}

/// Outlook's notes come in as notes: their text, colour and time; an empty one doesn't.
#[test]
fn notes_come_in_as_notes() {
    let dir = std::env::temp_dir().join(format!("mailcraft-pst-notes-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    let mut progress = |_| {};
    let mut out = Out::new(&store, "d1", &ImportOptions::default(), &mut progress);
    let note = |body: &str, color: Option<u8>| Item {
        class: "IPM.StickyNote".into(),
        subject: body.lines().next().unwrap_or_default().into(),
        body: body.into(),
        date: 1_700_000_000,
        extra: Some(crate::pst_pim::Extra { note_color: color, modified: Some(1_700_000_500), ..Default::default() }),
        ..Default::default()
    };
    out.add_pim(note("Poster\r\nAsk about 170 gsm", Some(1)));
    out.add_pim(note("Plain", None));
    out.add_pim(note("  ", Some(2)));
    let done = out.finish();
    assert_eq!((done.notes, done.skipped), (2, 1));
    assert_eq!(done.skips.into_iter().collect::<Vec<_>>(), [(SkipReason::ItemNotUsable, 1)], "an empty note isn't an unreadable message");
    let pim: Pim = store.load_account_doc("d1", PIM_DOC).unwrap();
    let n = &pim.notes[0].note;
    assert_eq!((n.text.as_str(), n.color, n.modified), ("Poster\nAsk about 170 gsm", 1, 1_700_000_500));
    assert_eq!(pim.notes[1].note.color, NOTE_YELLOW, "no colour: Outlook's yellow");
    assert!(pim.notes.iter().all(|s| !s.dirty && s.href.is_empty()), "kept on this computer");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A message too big for the batch being gathered goes into a transaction of its own, not on top
/// of the batch (which made one far over the cap): a write of it that fails costs only it.
#[test]
fn a_big_message_goes_alone() {
    let store = Store::failing();
    store.fail(None, Some("2.eml"));
    let mut progress = |_| {};
    let mut out = Out::new(&store, "d1", &ImportOptions::default(), &mut progress);
    let inbox = out.add_folder(None, "Inbox");
    let mail = |body: String| Item { class: "IPM.Note".into(), subject: "s".into(), date: 1_704_067_200, body, ..Default::default() };
    out.add_message(&inbox, mail("small".into()));
    out.add_message(&inbox, mail("x".repeat(mailcraft_store::BACKGROUND_COMMIT)));
    let done = out.finish();
    assert_eq!((done.messages, done.skipped), (1, 1), "the small one was written in a batch of its own");
}

#[test]
fn windows_1252_and_utf8() {
    assert_eq!(decode(b"caf\xe9 \x80 \x93q\x94", 1252, false), "café € “q”");
    assert_eq!(decode("café".as_bytes(), 0, false), "café");
    assert_eq!(decode(b"caf\xe9", 0, false), "café", "no page: Windows-1252 when it isn't UTF-8");
    assert_eq!(decode(b"caf\xe9", 99999, true), "café", "a page this doesn't know: the same");
    assert_eq!(decode(b"\xfe\xff", 65001, false), "\u{FFFD}\u{FFFD}");
    // The five bytes Windows-1252 leaves undefined stay the C1 controls they number, as before
    // the code pages came in: none turns into U+FFFD.
    for page in [0, 1252, 20127, 99999] {
        assert_eq!(decode(b"a\x81\x8d\x8f\x90\x9d\xe9", page, true), "a\u{81}\u{8D}\u{8F}\u{90}\u{9D}é", "page {page}");
    }
}

/// An HTML body and an 8-bit string are read in their code page, not as Windows-1252.
#[test]
fn text_is_read_in_its_code_page() {
    assert_eq!(decode(b"<p>\xcf\xf0\xe8\xe2\xe5\xf2</p>", 1251, true), "<p>Привет</p>");
    assert_eq!(decode(b"\xcf\xf0\xe8\xe2\xe5\xf2", 1251, false), "Привет", "an old data file's subject");
    assert_eq!(decode(b"\x1b$B$3$s$K$A$O\x1b(B", 50220, true), "こんにちは", "Japanese mail keeps no escapes");
    assert_eq!(decode(b"\xc4\xe3\xba\xc3", 936, true), "你好");
    assert_eq!(decode(b"\xb0\xa1", 51949, true), "가");
    // An HTML body's byte-order mark says what its bytes are, whatever the page.
    assert_eq!(decode(b"\xef\xbb\xbfcaf\xc3\xa9", 1251, true), "café");
    assert_eq!(decode(b"\xff\xfec\x00\xe9\x00", 1252, true), "cé");
    assert_eq!(decode(b"\xfe\xff\x00h\x00\xe9", 1252, true), "hé");
    // A string's bytes are its page's, even when they start like a mark.
    assert_eq!(decode(b"\xff\xfeab", 1252, false), "ÿþab");
    assert_eq!(decode(b"\xef\xbb\xbfa", 1252, false), "\u{FEFF}a", "valid UTF-8, read whole");
}

/// UTF-16 is read as UTF-16, though its bytes are often valid UTF-8 too (ASCII as `xx 00`, and
/// many CJK units).
#[test]
fn utf16_is_never_taken_for_utf8() {
    let html = "<p>识别</p>";
    let le: Vec<u8> = html.encode_utf16().flat_map(u16::to_le_bytes).collect();
    assert!(std::str::from_utf8(&le).is_ok_and(|s| !s.is_ascii()), "the case this is about");
    assert_eq!(decode(&le, 1200, true), html);
    let be: Vec<u8> = "<p>스요</p>".encode_utf16().flat_map(u16::to_be_bytes).collect();
    assert_eq!(decode(&be, 1201, true), "<p>스요</p>");
    assert_eq!(decode(b"\x00<\x00p\x00>", 1201, true), "<p>", "all ASCII bytes");
    assert_eq!(decode(b"\xfe\xff\x00h\x00\xe9", 1201, true), "hé", "a mark of the page's own");
}

/// An 8-bit string is never UTF-16 or UTF-7, whatever its message's page says: it reads as with
/// no page (it came out as CJK, short of its last letter, or without its pluses).
#[test]
fn an_8_bit_string_is_never_utf16_or_utf7() {
    for cp in [1200, 1201, 65000] {
        assert_eq!(decode(b"Hello", cp, false), "Hello", "code page {cp}");
        assert_eq!(decode(b"C++ tips", cp, false), "C++ tips", "code page {cp}");
        assert_eq!(decode("café".as_bytes(), cp, false), "café", "code page {cp}");
        assert_eq!(decode(b"caf\xe9", cp, false), "café", "code page {cp}");
    }
}

/// Valid UTF-8 that isn't plain ASCII is UTF-8 (Outlook labels some UTF-8 mail with the
/// sender's code page) unless the HTML declares another charset near its top.
#[test]
fn utf8_is_utf8_unless_another_charset_is_declared() {
    let body = "<p>café — ünï</p>";
    assert_eq!(decode(body.as_bytes(), 28591, true), body, "nothing declared");
    assert_eq!(decode(body.as_bytes(), 1252, false), body, "an 8-bit string declares nothing");
    for meta in ["<meta charset=\"utf-8\">", "<META HTTP-EQUIV=Content-Type CONTENT=\"text/html; CHARSET = 'UTF8'\">"] {
        let html = format!("{meta}{body}");
        assert_eq!(decode(html.as_bytes(), 1252, true), html, "{meta}");
    }
    // The sender said Windows-1252: the bytes are read that way, as Outlook shows them.
    let html = format!("<meta http-equiv=Content-Type content=\"text/html; charset=windows-1252\">{body}");
    assert_eq!(decode(html.as_bytes(), 1252, true), mime::decode_code_page(html.as_bytes(), 1252));
    assert_ne!(decode(html.as_bytes(), 1252, true), html);
    // The word without a value declares nothing; a declaration past the first 4 KB doesn't count.
    let html = format!("<p>the charset</p>{body}");
    assert_eq!(decode(html.as_bytes(), 1252, true), html);
    let html = format!("{}<meta charset=windows-1252>{body}", " ".repeat(5000));
    assert_eq!(decode(html.as_bytes(), 1252, true), html);
    // A declaration that starts in the first 4 KB is read whole, past them (it was cut to "utf").
    for pad in [4070, 4080, 4085, 4094] {
        let html = format!("{}<meta charset=utf-8>{body}", " ".repeat(pad));
        assert_eq!(decode(html.as_bytes(), 1252, true), html, "padded by {pad}");
    }
    let html = format!("{}<meta charset=windows-1252>{body}", " ".repeat(4090));
    assert_ne!(decode(html.as_bytes(), 1252, true), html, "a declaration across the 4 KB still counts");
    // A charset that can't say what bytes beyond ASCII are declares nothing: US-ASCII, and the
    // "unicode" Word writes in its HTML.
    for meta in [
        "<meta http-equiv=Content-Type content=\"text/html; charset=unicode\">",
        "<meta charset=\"us-ascii\">",
        "<meta charset=ascii>",
        "<meta charset=utf-16le>",
        "<meta charset=UCS-2>",
        "<meta charset=utf-7>",
    ] {
        let html = format!("{meta}{body}");
        assert_eq!(decode(html.as_bytes(), 1252, true), html, "{meta}");
    }
    // `charset=` outside a `<meta>` tag isn't a declaration: in a comment, text or a link.
    for html in [
        format!("{body}<!-- charset=iso-8859-1 -->"),
        format!("<p>set charset=latin1 in the file</p>{body}"),
        format!("<a href=\"https://studio.example/?charset=windows-1252\">link</a>{body}"),
    ] {
        assert_eq!(decode(html.as_bytes(), 28591, true), html, "{html}");
    }
    // Plain ASCII and bytes that aren't UTF-8 are in the page.
    assert_eq!(decode(b"caf\xe9", 1252, true), "café");
}

/// ISO-2022-KR has no decoder: its ASCII is kept, and the message is counted.
#[test]
fn an_unsupported_charset_is_counted() {
    let none: [fn(u16) -> Option<Val<'static>>; 0] = [];
    let named = crate::pst_pim::Named::default();
    let props: HashMap<u16, Val<'static>> =
        HashMap::from([(0x001A, Val::Text("IPM.Note".into())), (0x1013, Val::Bin(b"<p>Hi \x0e\x30\x21\x0f</p>")), (0x3FDE, Val::Int(50225))]);
    let it = item_of(&|id| props.get(&id).cloned(), &none, Vec::new(), &named);
    assert!(it.charset_unsupported);
    assert!(it.html.as_deref().is_some_and(|h| h.starts_with("<p>Hi ")));
    let props: HashMap<u16, Val<'static>> = HashMap::from([(0x1013, Val::Bin(b"<p>Hi</p>")), (0x3FDE, Val::Int(1252))]);
    assert!(!item_of(&|id| props.get(&id).cloned(), &none, Vec::new(), &named).charset_unsupported);
    // Labelled ISO-2022-KR but read whole (plain ASCII, or UTF-8): nothing lost, not counted.
    for body in [b"<p>Hi</p>".as_slice(), "<p>café</p>".as_bytes()] {
        let props: HashMap<u16, Val<'_>> = HashMap::from([(0x1013, Val::Bin(body)), (0x3FDE, Val::Int(50225))]);
        let it = item_of(&|id| props.get(&id).cloned(), &none, Vec::new(), &named);
        assert!(!it.charset_unsupported, "{:?}", it.html);
    }
    // A message attached to one that lost text counts for it.
    let lost = Item { charset_unsupported: true, ..Default::default() };
    let props: HashMap<u16, Val<'static>> = HashMap::from([(0x001A, Val::Text("IPM.Note".into()))]);
    let it = item_of(&|id| props.get(&id).cloned(), &none, vec![item::File::Message(Box::new(lost))], &named);
    assert!(it.charset_unsupported);
}

/// A message whose text was in ISO-2022-KR is in the import's losses (it was only logged), and
/// still came in: it isn't among the items left out.
#[test]
fn an_unsupported_charset_is_in_the_report() {
    let dir = std::env::temp_dir().join(format!("mailcraft-pst-charset-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    let mut progress = |_| {};
    let mut out = Out::new(&store, "d1", &ImportOptions::default(), &mut progress);
    let inbox = out.add_folder(None, "Inbox");
    let mail = |lost: bool| Item {
        class: "IPM.Note".into(),
        subject: "Hi".into(),
        html: Some("<p>Hi \u{e}0!\u{f}</p>".into()),
        date: 1_700_000_000,
        charset_unsupported: lost,
        ..Default::default()
    };
    out.add_message(&inbox, mail(true));
    out.add_message(&inbox, mail(false));
    let done = out.finish();
    assert_eq!((done.messages, done.skipped), (2, 0));
    assert_eq!(done.skips.into_iter().collect::<Vec<_>>(), [(SkipReason::CharsetUnsupported, 1)]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn filetime() {
    // 2024-01-01T00:00:00Z
    assert_eq!(val(&PropertyValue::Time(133_485_408_000_000_000), 0).time(), Some(1_704_067_200));
    assert_eq!(val(&PropertyValue::Time(0), 0).time(), None);
}

/// Only a 32-bit integer reads as a number, and a value of a type the import does not read is
/// still there, as it was before items were read through `Val`.
#[test]
fn only_a_32_bit_integer_is_a_number() {
    assert_eq!(val(&PropertyValue::Integer32(2), 0), Val::Int(2));
    for v in [PropertyValue::Integer16(2), PropertyValue::Integer64(2), PropertyValue::ErrorCode(2), PropertyValue::Null] {
        assert_eq!(val(&v, 0), Val::Other, "{v:?}");
    }
}

/// An item is read from its values alone: the subject's prefix marker goes, flags, time and
/// recipients come through, and an HTML body in bytes is read in its code page.
#[test]
fn an_item_reads_from_its_values() {
    let props: HashMap<u16, Val<'static>> = HashMap::from([
        (0x001A, Val::Text("IPM.Note".into())),
        (0x0037, Val::Text("\u{1}\u{5}FW: Hello".into())),
        (0x0C1A, Val::Text("Ana".into())),
        (0x0C1F, Val::Text("ana@example.com".into())),
        (0x0E06, Val::Time(133_485_408_000_000_000)),
        (0x0E07, Val::Int(0x1)),
        (0x1090, Val::Int(2)),
        (0x1081, Val::Int(0x0066)),
        (0x1013, Val::Bin(b"caf\xe9")),
        (0x3FDE, Val::Int(65001)),
    ]);
    let ben = HashMap::from([(0x3001, Val::Text("Ben".into())), (0x39FE, Val::Text("ben@example.com".into())), (0x0C15, Val::Int(2))]);
    let nobody = HashMap::from([(0x3001, Val::Text("No address".into()))]);
    let get = |m: &HashMap<u16, Val<'static>>, id: u16| m.get(&id).cloned();
    let recipients = [&ben, &nobody].map(|r| move |id| get(r, id));
    let it = item_of(&|id| get(&props, id), &recipients, Vec::new(), &crate::pst_pim::Named::default());
    assert_eq!(it.subject, "FW: Hello");
    assert_eq!((it.from.name.as_str(), it.from.email.as_str(), it.date), ("Ana", "ana@example.com", 1_704_067_200));
    assert!(it.flags.seen && !it.flags.draft && it.flags.flagged && it.flags.answered);
    assert_eq!(it.html.as_deref(), Some("caf\u{FFFD}"), "the HTML is read in its code page");
    assert_eq!(it.cc.iter().map(|a| a.email.as_str()).collect::<Vec<_>>(), ["ben@example.com"]);
    assert!(it.to.is_empty() && it.bcc.is_empty() && it.extra.is_none());
}

/// `PidTagRtfCompressed` stored as it is (`MELA`), as Outlook may keep a short body.
fn stored_rtf(rtf: &[u8]) -> Vec<u8> {
    let len = u32::try_from(rtf.len()).unwrap();
    let mut out = (len + 12).to_le_bytes().to_vec();
    out.extend(len.to_le_bytes());
    out.extend(0x414C_454Du32.to_le_bytes());
    out.extend(0u32.to_le_bytes());
    out.extend(rtf);
    out
}

/// An item from its properties alone, with no recipients.
fn item_with<'a>(props: &HashMap<u16, Val<'a>>, files: Vec<File>) -> Item {
    let none: [fn(u16) -> Option<Val<'a>>; 0] = [];
    item_of(&|id| props.get(&id).cloned(), &none, files, &crate::pst_pim::Named::default())
}

/// A body Outlook kept only as RTF comes in as the HTML it wraps, and the picture that HTML
/// names by `cid:` goes inline instead of being a file.
#[test]
fn an_rtf_only_body_comes_in_with_its_pictures_inline() {
    let rtf = stored_rtf(
        br#"{\rtf1\ansi\ansicpg1252\fromhtml1 \deff0{\fonttbl{\f0\fswiss Calibri;}}{\*\htmltag19 <html>}{\*\htmltag34 <body>}{\*\htmltag <p>}\htmlrtf {\htmlrtf0 Hi Riley, the {\*\htmltag <b>}\htmlrtf\b\htmlrtf0 storyboard{\*\htmltag </b>}\htmlrtf\b0\htmlrtf0  is attached.{\*\htmltag <img src="cid:logo@studio.example">}\htmlrtf\par\htmlrtf0}{\*\htmltag </p></body></html>}}"#,
    );
    let props = HashMap::from([(0x001A, Val::Text("IPM.Note".into())), (0x0037, Val::Text("Storyboard".into())), (0x1009, Val::Bin(&rtf))]);
    let logo =
        File::Data { name: "logo.png".into(), mime: "image/png".into(), cid: "logo@studio.example".into(), data: b"\x89PNG\r\n\x1a\nfake".to_vec() };
    let it = item_with(&props, vec![logo]);
    let html = it.html.as_deref().unwrap_or_default();
    assert!(html.contains("<b>storyboard</b>") && html.contains("cid:logo@studio.example"), "{html}");
    let raw = to_raw(it, "rtf@x").unwrap();
    let p = mime::parse(&raw).unwrap();
    assert!(p.had_html && p.text.contains("storyboard"), "{}", p.text);
    let inline: Vec<_> = p.attachments.iter().map(|a| (a.inline, a.content_id.as_deref())).collect();
    assert_eq!(inline, [(true, Some("logo@studio.example"))], "the picture is inline");

    // Compressed (LZFu, MS-OXRTFCP 3.1.1): RTF of its own reads as simple HTML.
    let compressed = [
        0x2d, 0x00, 0x00, 0x00, 0x2b, 0x00, 0x00, 0x00, 0x4c, 0x5a, 0x46, 0x75, 0xf1, 0xc5, 0xc7, 0xa7, 0x03, 0x00, 0x0a, 0x00, 0x72, 0x63, 0x70,
        0x67, 0x31, 0x32, 0x35, 0x42, 0x32, 0x0a, 0xf3, 0x20, 0x68, 0x65, 0x6c, 0x09, 0x00, 0x20, 0x62, 0x77, 0x05, 0xb0, 0x6c, 0x64, 0x7d, 0x0a,
        0x80, 0x0f, 0xa0,
    ];
    let it = item_with(&HashMap::from([(0x1009, Val::Bin(&compressed[..]))]), Vec::new());
    assert!(it.html.as_deref().is_some_and(|h| h.contains("hello world")), "{:?}", it.html);
    let p = mime::parse(&to_raw(it, "lzfu@x").unwrap()).unwrap();
    assert!(p.text.contains("hello world"), "{}", p.text);
}

/// RTF fills in only what the item lacks: plain text keeps its place and gains the HTML the RTF
/// wraps, an HTML body wins over the RTF, wrapped text fills an empty body, and RTF that can't be
/// read leaves the item as it was.
#[test]
fn rtf_fills_in_only_what_is_missing() {
    let wrapped = stored_rtf(br#"{\rtf1\ansi\fromhtml1 {\*\htmltag <p>}\htmlrtf {\htmlrtf0 From the RTF}{\*\htmltag </p>}}"#);
    let with = |extra: &[(u16, Val<'static>)], rtf: &[u8]| {
        let mut props: HashMap<u16, Val<'_>> = extra.iter().cloned().collect();
        props.insert(0x1009, Val::Bin(rtf));
        item_with(&props, Vec::new())
    };
    let it = with(&[(0x1000, Val::Text("Plain text".into()))], &wrapped);
    assert_eq!(it.body, "Plain text");
    assert!(it.html.as_deref().is_some_and(|h| h.contains("From the RTF")), "{:?}", it.html);
    let it = with(&[(0x1000, Val::Text("Plain text".into())), (0x0E1F, Val::Bool(true))], &wrapped);
    assert!(it.html.as_deref().is_some_and(|h| h.contains("From the RTF")), "{:?}", it.html);
    // The plain text was changed after the RTF: the HTML the RTF wraps is older, and left out.
    let it = with(&[(0x1000, Val::Text("Plain text".into())), (0x0E1F, Val::Bool(false))], &wrapped);
    assert_eq!((it.body.as_str(), it.html.as_deref()), ("Plain text", None));
    let it = with(&[(0x0E1F, Val::Bool(false))], &wrapped);
    assert!(it.html.as_deref().is_some_and(|h| h.contains("From the RTF")), "the only body: {:?}", it.html);

    let it = with(&[(0x1013, Val::Text("<p>From the HTML</p>".into()))], &wrapped);
    assert_eq!(it.html.as_deref(), Some("<p>From the HTML</p>"));

    let text = stored_rtf(b"{\\rtf1\\ansi\\fromtext \\deff0{\\fonttbl{\\f0 Courier;}}Line one\\par\nLine\\tab two\\par}");
    let it = with(&[], &text);
    assert_eq!((it.body.as_str(), it.html.as_deref()), ("Line one\r\nLine\ttwo\r\n", None));
    let it = with(&[(0x1000, Val::Text("Kept".into()))], &text);
    assert_eq!(it.body, "Kept");

    // RTF of its own (an RTF-format mail, a meeting request) doesn't stand in for the plain text:
    // its simple HTML would run a table's cells together and drop its pictures and objects.
    let own = stored_rtf(br"{\rtf1\ansi\deff0 Name\cell Value\cell\row Alpha\cell Beta\cell\row Before{\pict\pngblip 89504e47}After\par}");
    let plain = "Name\tValue\r\nAlpha\tBeta\r\nBefore [picture] After";
    let it = with(&[(0x1000, Val::Text(plain.into()))], &own);
    assert_eq!((it.body.as_str(), it.html.as_deref()), (plain, None));
    let it = with(&[], &own);
    assert!(it.body.is_empty() && it.html.as_deref().is_some_and(|h| h.contains("Before")), "{:?}", it.html);

    for junk in [&b""[..], b"0123456789abcdefghij", &stored_rtf(b"not rtf at all"), &wrapped[..20]] {
        let it = with(&[], junk);
        assert!(it.body.is_empty() && it.html.is_none(), "{junk:?}");
        assert!(it.body_not_converted, "{junk:?}");
        assert!(to_raw(it, "junk@x").is_ok());
        let it = with(&[(0x1000, Val::Text("Plain text".into()))], junk);
        assert!(it.body == "Plain text" && !it.body_not_converted, "{junk:?}");
    }
    assert!(!with(&[], &wrapped).body_not_converted && !with(&[], &text).body_not_converted);
}

/// An item with no headers of its own keeps that it was forwarded, its importance and the ids
/// it answers, however the file wrote them.
#[test]
fn a_headerless_item_keeps_forwarded_importance_and_references() {
    let read = |props: &[(u16, Val<'static>)]| {
        let props: HashMap<u16, Val<'static>> = props.iter().cloned().collect();
        let none: [fn(u16) -> Option<Val<'static>>; 0] = [];
        item_of(&|id| props.get(&id).cloned(), &none, Vec::new(), &crate::pst_pim::Named::default())
    };
    // Forwarded by its last verb, or by its icon when it has none; a reply stays a reply.
    let it = read(&[(0x1081, Val::Int(0x0068))]);
    assert!(it.flags.forwarded && !it.flags.answered);
    assert!(read(&[(0x1080, Val::Int(0x0106))]).flags.forwarded);
    let reply = read(&[(0x1081, Val::Int(0x0066)), (0x1080, Val::Int(0x0106))]);
    assert!(reply.flags.answered && !reply.flags.forwarded);
    // Importance: low, normal, high.
    for (n, want) in [(0, -1), (1, 0), (2, 1)] {
        assert_eq!(read(&[(0x0017, Val::Int(n))]).importance, want, "{n}");
    }
    // References however they're separated.
    for refs in ["<a@x><b@x>", "<a@x>,<b@x>", "<a@x> , <b@x>", "a@x b@x", "a@x,b@x", " <a@x>\r\n\t<b@x> "] {
        let it = read(&[(0x1039, Val::Text(refs.into())), (0x1042, Val::Text(refs.into()))]);
        assert_eq!(it.references, ["a@x", "b@x"], "{refs:?}");
        assert_eq!(it.in_reply_to, "a@x", "{refs:?}");
    }
    let many = (0..300).map(|i| format!("<{i}@x>")).collect::<String>();
    let it = read(&[(0x1039, Val::Text(many.into()))]);
    assert_eq!((it.references.len(), it.references.first().map(String::as_str)), (256, Some("44@x")), "the latest are kept");

    // …and they reach the message and its summary.
    let it = Item { subject: "Fwd".into(), body: "x".into(), date: 1_704_067_200, ..read(&[(0x1081, Val::Int(0x0068)), (0x0017, Val::Int(2))]) };
    let raw = to_raw(Item { references: vec!["a@x".into(), "b@x".into()], ..it }, "id@x").unwrap();
    let text = String::from_utf8_lossy(&raw);
    assert!(text.contains("References: <a@x> <b@x>"), "{text}");
    assert!(text.contains("Importance: high"), "{text}");
}

/// An item's categories, a property named by a string, come in as its keywords: once each,
/// none that would be a system keyword, and only from a list of texts.
#[test]
fn categories_named_by_a_string_become_keywords() {
    let read = |v: Val<'static>| {
        let props: HashMap<u16, Val<'static>> = HashMap::from([(0x8010, v)]);
        let none: [fn(u16) -> Option<Val<'static>>; 0] = [];
        item_of(&|id| props.get(&id).cloned(), &none, Vec::new(), &crate::pst_pim::Named::with_keywords(0x8010)).flags.keywords
    };
    let names = ["Work", "To do", "work", " ", "$Junk"].map(String::from).to_vec();
    assert_eq!(read(Val::Texts(names)), ["work", "to=20do"]);
    assert!(read(Val::Text("Work".into())).is_empty());
    assert_eq!(read(Val::Texts((0..100).map(|i| format!("c{i}")).collect())).len(), 64);
    // A file without the name knows no categories.
    let props: HashMap<u16, Val<'static>> = HashMap::from([(0x8010, Val::Texts(vec!["Work".into()]))]);
    let none: [fn(u16) -> Option<Val<'static>>; 0] = [];
    assert!(item_of(&|id| props.get(&id).cloned(), &none, Vec::new(), &crate::pst_pim::Named::default()).flags.keywords.is_empty());
}

/// The header of a rebuilt message: its lines, each ASCII without control characters (but tab),
/// at most 998 bytes.
fn clean_header(raw: &[u8]) -> Vec<String> {
    let end = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let head = std::str::from_utf8(&raw[..end]).unwrap();
    let lines: Vec<String> = head.split("\r\n").map(str::to_string).collect();
    for l in &lines {
        assert!(l.len() <= 998 && l.chars().all(|c| c == '\t' || (c.is_ascii() && !c.is_ascii_control())), "{l:?}");
    }
    lines
}

/// Outlook's conversation index and topic and the sensitivity become headers on an item that
/// has none; hostile text in them stays inside its own line, short and encoded.
#[test]
fn a_headerless_item_gets_thread_and_sensitivity_headers() {
    let index: Vec<u8> = (0..22 + 5 * 60 + 3).map(|i: u32| i as u8).collect();
    let topic = "Plan\r\nBcc: evil@example.com\u{0}\u{7}  for the café";
    let props = [
        (0x001A, Val::Text("IPM.Note".into())),
        (0x0037, Val::Text("Plan".into())),
        (0x0071, Val::Bin(&index)),
        (0x0070, Val::Text(topic.into())),
        (0x0036, Val::Int(3)),
        (0x1035, Val::Text("<id@x>\r\nBcc: evil@example.com".into())),
    ];
    let (raw, _) = rebuilt(&props, &[]).unwrap();
    let lines = clean_header(&raw);
    assert!(!lines.iter().any(|l| l.to_ascii_lowercase().starts_with("bcc")), "{lines:#?}");
    let index_line = lines.iter().find(|l| l.starts_with("Thread-Index: ")).unwrap();
    let kept = crate::ews::unb64(&index_line["Thread-Index: ".len()..]).unwrap();
    assert_eq!(kept.len(), 22 + 5 * 50, "whole blocks, at most 50");
    assert_eq!(kept[..], index[..kept.len()]);
    assert!(lines.contains(&"Sensitivity: Company-Confidential".to_string()), "{lines:#?}");
    let m = mailparse_header(&raw, "Thread-Topic");
    assert_eq!(m, "Plan Bcc: evil@example.com for the café");
    // A long topic is cut short, and plain ASCII stays as it is.
    let long = "x".repeat(2000);
    let (raw, _) = rebuilt(&[(0x0070, Val::Text(long.as_str().into()))], &[]).unwrap();
    assert!(clean_header(&raw).contains(&format!("Thread-Topic: {}", "x".repeat(MAX_TOPIC))));
    // Nothing but spaces: no header.
    let (raw, _) = rebuilt(&[(0x0070, Val::Text("\r\n \t".into())), (0x0036, Val::Int(9))], &[]).unwrap();
    assert!(!clean_header(&raw).iter().any(|l| l.starts_with("Thread-Topic") || l.starts_with("Sensitivity")));
}

/// Names, addresses and ids from a header-less item stay inside their own header lines: a Bcc
/// name with a line break (from the recipient's one-off entry id or its name), an address with
/// one, ids too long for a line.
#[test]
fn names_addresses_and_ids_stay_inside_their_lines() {
    let entry = one_off("x\r\nX-Injected: 1", "SMTP", "a@example.com", true);
    let hidden = HashMap::from([(0x3001, Val::Text("".into())), (0x0FFF, Val::Bin(&entry)), (0x0C15, Val::Int(3))]);
    let named =
        HashMap::from([(0x3001, Val::Text("Bo\r\nX-Injected: 2".into())), (0x39FE, Val::Text("bo@example.com".into())), (0x0C15, Val::Int(3))]);
    let quoted = HashMap::from([(0x3001, Val::Text("Park, Zoe \"Z\"".into())), (0x39FE, Val::Text("zoe@example.com".into())), (0x0C15, Val::Int(3))]);
    let broken = HashMap::from([(0x39FE, Val::Text("c@example.com\r\nX-Injected: 3".into())), (0x0C15, Val::Int(3))]);
    let to = HashMap::from([(0x3001, Val::Text("To\r\nX-Injected: 4".into())), (0x39FE, Val::Text("to@example.com".into()))]);
    let long = format!("<{}>", "a".repeat(2000));
    let references = format!("{long} <r@x>");
    let props: HashMap<u16, Val<'_>> = HashMap::from([
        (0x001A, Val::Text("IPM.Note".into())),
        (0x0037, Val::Text("Hi\u{1}\r\nX-Injected: 5".into())),
        (0x0C1A, Val::Text("Me\r\nX-Injected: 6".into())),
        (0x5D01, Val::Text("me@example.com\r\nX-Injected: 7".into())),
        (0x0065, Val::Text("me@example.com".into())),
        (0x1035, Val::Text(long.as_str().into())),
        (0x1042, Val::Text(long.as_str().into())),
        (0x1039, Val::Text(references.as_str().into())),
    ]);
    fn get<'a>(m: &HashMap<u16, Val<'a>>, id: u16) -> Option<Val<'a>> {
        m.get(&id).cloned()
    }
    let rows = [&hidden, &named, &quoted, &broken, &to].map(|r| move |id| get(r, id));
    let it = item_of(&|id| get(&props, id), &rows, Vec::new(), &crate::pst_pim::Named::default());
    assert_eq!(it.bcc.len(), 3, "an address with a line break is none");
    let raw = to_raw(it, "f@x").unwrap();
    let lines = clean_header(&raw);
    assert!(!lines.iter().any(|l| l.to_ascii_lowercase().starts_with("x-injected")), "{lines:#?}");
    let bcc: Vec<&String> = lines.iter().skip_while(|l| !l.starts_with("Bcc: ")).take(3).collect();
    assert_eq!(bcc, ["Bcc: \"x X-Injected: 1\" <a@example.com>,", " \"Bo X-Injected: 2\" <bo@example.com>,", " \"Park, Zoe 'Z'\" <zoe@example.com>"]);
    let p = mime::parse(&raw).unwrap();
    assert_eq!(p.message_id.len(), MAX_ID, "{}", p.message_id);
    assert_eq!((p.from.email.as_str(), p.from.name.as_str()), ("me@example.com", "Me X-Injected: 6"));
    // A blank line in an original header doesn't end the message's header there.
    let it = Item { headers: "From: a@example.com\r\n\r\nX-Later: 1\r\n".into(), thread_topic: "T".into(), ..Default::default() };
    let head = String::from_utf8_lossy(&to_raw(it, "f@x").unwrap()).split("\r\n\r\n").next().unwrap_or_default().to_string();
    assert!(head.contains("X-Later: 1") && head.contains("Thread-Topic: T") && head.contains("Content-Type"), "{head}");
}

/// A header's value as a mail reader decodes it (read as a subject).
fn mailparse_header(raw: &[u8], name: &str) -> String {
    let head = String::from_utf8_lossy(raw);
    let start = head.find(&format!("\r\n{name}: ")).unwrap() + 2 + name.len();
    let field: String = head[start..].split("\r\n").enumerate().take_while(|(i, l)| *i == 0 || l.starts_with(' ')).map(|(_, l)| l).collect();
    mime::parse(format!("Subject{field}\r\n\r\n").as_bytes()).unwrap().subject
}

/// An item with its own header gets only the lines that header lacks.
#[test]
fn an_original_header_gets_only_what_it_lacks() {
    let it = || Item {
        subject: "Hello".into(),
        from: EmailAddress::new("Ana", "ana@example.com"),
        date: 1_704_067_200,
        body: "Hi".into(),
        headers: "From: ana@example.com\r\nSubject: Hello\r\nthread-topic: Theirs\r\n".into(),
        thread_index: vec![1; 22],
        thread_topic: "Ours".into(),
        sensitivity: 2,
        ..Default::default()
    };
    let raw = to_raw(it(), "f@x").unwrap();
    let lines = clean_header(&raw);
    let named = |n: &str| lines.iter().filter(|l| l.to_ascii_lowercase().starts_with(&n.to_ascii_lowercase())).count();
    assert_eq!((named("thread-topic:"), named("thread-index:"), named("sensitivity:")), (1, 1, 1), "{lines:#?}");
    assert!(lines.contains(&"thread-topic: Theirs".to_string()));
    assert!(lines.contains(&"Sensitivity: Private".to_string()));
    // A signature's folded list of names is no line of those names.
    let folded = "DKIM-Signature: v=1; h=From:To:Subject:\r\n Thread-Index:Thread-Topic:Date; b=x\r\nSubject: s\r\n";
    let raw = to_raw(Item { headers: folded.into(), ..it() }, "f@x").unwrap();
    let lines = clean_header(&raw);
    assert!(lines.contains(&"Thread-Topic: Ours".to_string()) && lines.iter().any(|l| l.starts_with("Thread-Index: ")), "{lines:#?}");
}

/// An unusual address from the file that is one still comes in as the file has it: a quoted
/// local part with a space, a domain ending in a dot.
#[test]
fn an_unusual_address_still_comes_in() {
    let quoted = HashMap::from([(0x39FE, Val::Text("\"a b\"@x.example".into()))]);
    let dotted = HashMap::from([(0x3003, Val::Text("first.last+tag@x.example.".into())), (0x0C15, Val::Int(2))]);
    let props: HashMap<u16, Val<'static>> = HashMap::from([(0x001A, Val::Text("IPM.Note".into()))]);
    fn get<'a>(m: &HashMap<u16, Val<'a>>, id: u16) -> Option<Val<'a>> {
        m.get(&id).cloned()
    }
    let rows = [&quoted, &dotted].map(|r| move |id| get(r, id));
    let it = item_of(&|id| get(&props, id), &rows, Vec::new(), &crate::pst_pim::Named::default());
    assert_eq!(it.to.iter().map(|a| a.email.as_str()).collect::<Vec<_>>(), ["\"a b\"@x.example"]);
    assert_eq!(it.cc.iter().map(|a| a.email.as_str()).collect::<Vec<_>>(), ["first.last+tag@x.example."]);
    let raw = to_raw(Item { subject: "s".into(), body: "b".into(), date: 1_704_067_200, ..it }, "f@x").unwrap();
    let lines = clean_header(&raw);
    assert!(lines.iter().any(|l| l.contains("\"a b\"@x.example")) && lines.iter().any(|l| l.contains("first.last+tag@x.example.")), "{lines:#?}");
}

/// A header-less item's recipient with no address anywhere is counted among what the import
/// couldn't bring in; a header item's own To and Cc still have it.
#[test]
fn a_recipient_without_an_address_is_counted() {
    let rows = [
        HashMap::from([(0x3001, Val::Text("Only a name".into()))]),
        HashMap::from([(0x39FE, Val::Text("/o=Org/cn=Recipients/cn=ex".into()))]),
        HashMap::from([(0x39FE, Val::Text("cy@x.example".into()))]),
    ];
    fn get<'a>(m: &HashMap<u16, Val<'a>>, id: u16) -> Option<Val<'a>> {
        m.get(&id).cloned()
    }
    let read = |headers: &str| {
        let props: HashMap<u16, Val<'_>> = HashMap::from([(0x001A, Val::Text("IPM.Note".into())), (0x007D, Val::Text(headers.into()))]);
        let rows = rows.each_ref().map(|r| move |id| get(r, id));
        item_of(&|id| get(&props, id), &rows, Vec::new(), &crate::pst_pim::Named::default())
    };
    let it = read("");
    assert_eq!((it.recipients_without_address, it.to.len()), (2, 1));
    assert_eq!(read("From: a@x.example\r\nTo: b@x.example\r\n").recipients_without_address, 0);

    // The report counts them, not as items left out.
    let dir = std::env::temp_dir().join(format!("mailcraft-pst-recipients-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    let mut progress = |_| {};
    let mut out = Out::new(&store, "r1", &ImportOptions::default(), &mut progress);
    let path = out.add_folder(None, "Inbox");
    out.add_message(&path, Item { subject: "s".into(), body: "b".into(), date: 1_704_067_200, ..read("") });
    let done = out.finish();
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!((done.messages, done.skipped), (1, 0));
    assert_eq!(done.skips.into_iter().collect::<Vec<_>>(), [(SkipReason::RecipientWithoutAddress, 2)]);
}

#[test]
fn original_headers_are_kept_over_the_rebuilt_body() {
    let it = Item {
        subject: "Hello".into(),
        from: EmailAddress::new("Ana", "ana@example.com"),
        to: vec![EmailAddress::new("", "me@example.com")],
        date: 1_704_067_200,
        body: "Hi there".into(),
        message_id: "<orig@example.com>".into(),
        headers: "Received: from x\r\n by y\r\nFrom: Ana <ana@example.com>\r\nTo: me@example.com\r\nSubject: Hello\r\nList-Id: <l.example.com>\r\nContent-Type: multipart/mixed;\r\n boundary=zzz\r\nMIME-Version: 1.0\r\n"
            .into(),
        ..Default::default()
    };
    let raw = to_raw(it, "fallback@x").unwrap();
    let text = String::from_utf8_lossy(&raw);
    assert!(text.contains("List-Id: <l.example.com>"));
    assert!(text.contains("Received: from x\r\n by y"));
    assert!(!text.contains("boundary=zzz"), "the original body's description goes");
    assert_eq!(text.matches("MIME-Version").count(), 1);
    let p = mime::parse(&raw).unwrap();
    assert_eq!(p.subject, "Hello");
    assert!(p.text.contains("Hi there"));
}

#[test]
fn a_sent_item_keeps_its_bcc_and_gets_an_id() {
    let it = Item {
        subject: "Plans".into(),
        from: EmailAddress::new("Me", "me@example.com"),
        bcc: vec![EmailAddress::new("", "secret@example.com")],
        date: 1_704_067_200,
        body: "Hi".into(),
        ..Default::default()
    };
    let raw = to_raw(it, "fallback@x").unwrap();
    let text = String::from_utf8_lossy(&raw);
    assert!(text.starts_with("Bcc: secret@example.com\r\n"));
    assert!(text.contains("<fallback@x>"));
}

#[test]
fn an_offline_copy_is_explained() {
    let dir = std::env::temp_dir().join(format!("mailcraft-ost-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let file = dir.join("me@example.com.ost");
    std::fs::write(&file, b"!BDN\0\0\0\0SO$\0rest").unwrap();
    let store = Store::open(dir.join("cache")).unwrap();
    let e = import(&file, &store, "a1", &mut |_| {}).unwrap_err();
    assert!(e.contains(".ost") && e.contains("add the account"), "{e}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn junk_is_refused_and_nothing_is_written() {
    let dir = std::env::temp_dir().join(format!("mailcraft-pst-junk-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("junk.pst");
    std::fs::write(&file, b"!BDN this is not really a data file").unwrap();
    let store = Store::open(dir.join("cache")).unwrap();
    assert!(import(&file, &store, "a1", &mut |_| {}).is_err());
    assert!(import(&dir.join("missing.pst"), &store, "a1", &mut |_| {}).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn imports_a_data_file() {
    use pst_rw::{UnicodePstAttachment, UnicodePstFile as Writer, UnicodePstMessage, UnicodePstRecipient, UnicodePstRecipientType};
    let dir = std::env::temp_dir().join(format!("mailcraft-pst-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("Archive 2024.pst");
    let recipients = [
        UnicodePstRecipient { name: "Ana Ruiz", email: "ana@example.com", recipient_type: UnicodePstRecipientType::To },
        UnicodePstRecipient { name: "Ben", email: "ben@example.com", recipient_type: UnicodePstRecipientType::Cc },
    ];
    let message = UnicodePstMessage {
        subject: "Quarterly plan — café",
        sender_name: "Riley Moreno",
        sender_email: "riley@example.com",
        recipients: &recipients,
        body: "Here is the plan.",
        html_body: Some("<p>Here is <b>the plan</b>.</p><img src=\"cid:chart@example.com\">"),
        message_id: "<plan@example.com>",
        // 2024-01-01T00:00:00Z
        delivery_time: 133_485_408_000_000_000,
        message_class: "IPM.Note",
        properties: &[],
    };
    let files = [
        UnicodePstAttachment {
            filename: "chart.png",
            mime_type: "image/png",
            content_id: Some("<chart@example.com>"),
            data: b"\x89PNG\r\n\x1a\nfake",
        },
        UnicodePstAttachment { filename: "notes.txt", mime_type: "text/plain", content_id: None, data: b"remember the milk" },
    ];
    drop(Writer::create_with_attachments(&file, &message, &files).unwrap());
    let nested = UnicodePstMessage { subject: "Old thread", message_id: "<old@example.com>", html_body: None, ..message };
    drop(Writer::append_in_folder(&file, &["Projects", "2024"], &nested).unwrap());

    let store = Store::open(dir.join("cache")).unwrap();
    let mut ticks = 0;
    let done = import(&file, &store, "a1", &mut |_| ticks += 1).unwrap();
    assert_eq!(done.messages, 2, "{done:?}");
    assert_eq!(done.skipped, 0);
    let inbox = done.folders.iter().find(|f| f.role == FolderRole::Inbox).unwrap();
    assert_eq!(inbox.path, "INBOX");
    let all = store.load_folders("a1").unwrap();
    assert_eq!(all, done.folders);
    let nested_folder = all.iter().find(|f| f.path == "Projects/2024").unwrap();
    assert_eq!(nested_folder.total, 1);
    assert!(all.iter().any(|f| f.path == "Projects"));

    // The message where the writer put it, with its addresses, body, picture and file.
    let (path, st) = all
        .iter()
        .filter(|f| f.path != "Projects/2024")
        .find_map(|f| store.load_folder("a1", &f.path).ok().filter(|s| !s.messages.is_empty()).map(|s| (f.path.clone(), s)))
        .unwrap();
    let s = &st.messages[0];
    assert_eq!(s.subject, "Quarterly plan — café");
    assert_eq!(s.date, 1_704_067_200);
    let key = MessageKey { account: "a1".into(), folder: path, validity: 1, uid: s.key.uid };
    let raw = store.read_body(&key).unwrap();
    let p = mime::parse(&raw).unwrap();
    assert_eq!(p.from.email, "riley@example.com");
    assert_eq!(p.to.iter().map(|a| a.email.as_str()).collect::<Vec<_>>(), ["ana@example.com"]);
    assert_eq!(p.cc.iter().map(|a| a.email.as_str()).collect::<Vec<_>>(), ["ben@example.com"]);
    assert_eq!(p.message_id, "plan@example.com");
    assert!(p.had_html && p.text.contains("the plan"), "{}", p.text);
    let names: Vec<&str> = p.attachments.iter().map(|a| a.name.as_str()).collect();
    assert!(names.contains(&"notes.txt"), "{names:?}");
    // The picture goes inline, under its own name.
    assert!(p.attachments.iter().any(|a| a.name == "chart.png" && a.inline), "{names:?}");

    // Into a data file, each source compressed before the writer takes it, and the file sealed at
    // the end: the same folders and mail.
    let packed = Store::open_packed(dir.join("app"), dir.join("Files")).unwrap();
    let again = import(&file, &packed, "a1", &mut |_| {}).unwrap();
    assert_eq!((again.messages, again.skipped), (2, 0));
    assert_eq!(packed.load_folders("a1").unwrap(), all);
    assert_eq!(packed.read_body(&key), Some(raw));
    drop(packed);
    let _ = std::fs::remove_dir_all(&dir);
}

mod chunked;
mod embedded;
mod files;
mod indexing;
mod losses;
mod options;
mod pim_gaps;
mod pipeline;
mod protected;
mod reader;
mod rebuilt;
mod report;
mod resume;
mod sent_ids;

mod real_files {
    /// Import a real data file named by `MAILCRAFT_PST` (counts only).
    #[test]
    #[ignore]
    fn imports_the_file_in_mailcraft_pst() {
        let Ok(path) = std::env::var("MAILCRAFT_PST") else { return };
        let dir = std::env::temp_dir().join(format!("mailcraft-real-pst-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = mailcraft_store::Store::open(&dir).unwrap();
        let t = std::time::Instant::now();
        let r = super::import(std::path::Path::new(&path), &store, "r1", &mut |_| {});
        match r {
            Ok(done) => {
                eprintln!(
                    "messages {} contacts {} events {} skipped {} in {:?}",
                    done.messages,
                    done.contacts,
                    done.events,
                    done.skipped,
                    t.elapsed()
                );
                eprintln!("skips {:?}", done.skips);
                let (mut parsed, mut html, mut files, mut dated) = (0, 0, 0, 0);
                // Attached messages by level (0 = attached to an item) and the files inside them.
                let (mut attached, mut inner) = ([0usize; 4], 0);
                for f in &done.folders {
                    eprintln!("  {:30} {:?} total {} unread {}", f.path, f.role, f.total, f.unread);
                    for m in store.load_folder("r1", &f.path).unwrap().messages {
                        dated += usize::from(m.date > 0);
                        let raw = store.read_body(&m.key).unwrap_or_default();
                        if let Some(p) = mailcraft_mime::parse(&raw) {
                            parsed += 1;
                            html += usize::from(p.had_html);
                            files += p.attachments.len();
                            count_attached(&raw, 0, &mut attached, &mut inner);
                        }
                    }
                }
                eprintln!("parsed {parsed} (html {html}, attachments {files}, dated {dated})");
                eprintln!("attached messages by level {attached:?}, files inside them {inner}");
            }
            Err(e) => eprintln!("FAILED: {e}"),
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Counts the attached messages in `raw`, `level` down, and the other files inside them.
    fn count_attached(raw: &[u8], level: usize, attached: &mut [usize; 4], inner: &mut usize) {
        let Some(p) = mailcraft_mime::parse(raw) else { return };
        for a in &p.attachments {
            if a.mime.eq_ignore_ascii_case("message/rfc822") {
                if let Some(n) = attached.get_mut(level) {
                    *n += 1;
                }
                if let Some(data) = mailcraft_mime::attachment_data(raw, a.index) {
                    count_attached(&data, level + 1, attached, inner);
                }
            } else if level > 0 {
                *inner += 1;
            }
        }
    }
}
