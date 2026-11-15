//! A property's value as the import reads it: text in its code page (and what it can't read
//! counted), a time, a number.

use super::*;

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
    let named = Named::default();
    let props: HashMap<u16, Val<'static>> =
        HashMap::from([(0x001A, Val::Text("IPM.Note".into())), (0x1013, Val::Bin(b"<p>Hi \x0e\x30\x21\x0f</p>")), (0x3FDE, Val::Int(50225))]);
    let it = item_with(&props, Vec::new(), &named);
    assert!(it.charset_unsupported);
    assert!(it.html.as_deref().is_some_and(|h| h.starts_with("<p>Hi ")));
    let props: HashMap<u16, Val<'static>> = HashMap::from([(0x1013, Val::Bin(b"<p>Hi</p>")), (0x3FDE, Val::Int(1252))]);
    assert!(!item_with(&props, Vec::new(), &named).charset_unsupported);
    // Labelled ISO-2022-KR but read whole (plain ASCII, or UTF-8): nothing lost, not counted.
    for body in [b"<p>Hi</p>".as_slice(), "<p>café</p>".as_bytes()] {
        let props: HashMap<u16, Val<'_>> = HashMap::from([(0x1013, Val::Bin(body)), (0x3FDE, Val::Int(50225))]);
        let it = item_with(&props, Vec::new(), &named);
        assert!(!it.charset_unsupported, "{:?}", it.html);
    }
    // A message attached to one that lost text counts for it.
    let lost = Item { charset_unsupported: true, ..Default::default() };
    let props: HashMap<u16, Val<'static>> = HashMap::from([(0x001A, Val::Text("IPM.Note".into()))]);
    let it = item_with(&props, vec![File::Message(Box::new(lost))], &named);
    assert!(it.charset_unsupported);
}

/// A message whose text was in ISO-2022-KR is in the import's losses (it was only logged), and
/// still came in: it isn't among the items left out.
#[test]
fn an_unsupported_charset_is_in_the_report() {
    let dir = scratch("pst-charset");
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
