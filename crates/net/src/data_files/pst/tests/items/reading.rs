//! An item read from its values: its recipients' addresses, its body (from RTF where that's all
//! it has), its flags, importance, references and categories.

use super::*;

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
    let it = item_of(&|id| get(&props, id), &[|id| get(&cy, id)], Vec::new(), &Named::default());
    assert_eq!(it.cc.iter().map(|a| (a.name.as_str(), a.email.as_str())).collect::<Vec<_>>(), [("Cy Lima", "cy@example.com")]);
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
    let recipients = [&ben, &nobody].map(|r| move |id| get(r, id));
    let it = item_of(&|id| get(&props, id), &recipients, Vec::new(), &Named::default());
    assert_eq!(it.subject, "FW: Hello");
    assert_eq!((it.from.name.as_str(), it.from.email.as_str(), it.date), ("Ana", "ana@example.com", 1_704_067_200));
    assert!(it.flags.seen && !it.flags.draft && it.flags.flagged && it.flags.answered);
    assert_eq!(it.html.as_deref(), Some("caf\u{FFFD}"), "the HTML is read in its code page");
    assert_eq!(it.cc.iter().map(|a| a.email.as_str()).collect::<Vec<_>>(), ["ben@example.com"]);
    assert!(it.to.is_empty() && it.bcc.is_empty() && it.extra.is_none());
}

/// A body Outlook kept only as RTF comes in as the HTML it wraps, and the picture that HTML
/// names by `cid:` goes inline instead of being a file.
#[test]
fn an_rtf_only_body_comes_in_with_its_pictures_inline() {
    let rtf = stored_rtf(STORYBOARD_RTF);
    let props = HashMap::from([(0x001A, Val::Text("IPM.Note".into())), (0x0037, Val::Text("Storyboard".into())), (0x1009, Val::Bin(&rtf))]);
    let logo =
        File::Data { name: "logo.png".into(), mime: "image/png".into(), cid: "logo@studio.example".into(), data: b"\x89PNG\r\n\x1a\nfake".to_vec() };
    let it = item_with(&props, vec![logo], &Named::default());
    let html = it.html.as_deref().unwrap_or_default();
    assert!(html.contains("<b>storyboard</b>") && html.contains("cid:logo@studio.example"), "{html}");
    let raw = to_raw(it, "rtf@x").unwrap();
    let p = mime::parse(&raw).unwrap();
    assert!(p.had_html && p.text.contains("storyboard"), "{}", p.text);
    let inline: Vec<_> = p.attachments.iter().map(|a| (a.inline, a.content_id.as_deref())).collect();
    assert_eq!(inline, [(true, Some("logo@studio.example"))], "the picture is inline");

    // Compressed (LZFu, MS-OXRTFCP 3.1.1): RTF of its own reads as simple HTML.
    let it = item_with(&HashMap::from([(0x1009, Val::Bin(&SPEC_LZFU[..]))]), Vec::new(), &Named::default());
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
        item_with(&props, Vec::new(), &Named::default())
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
        item_with(&props, Vec::new(), &Named::default())
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
        item_with(&props, Vec::new(), &Named::with_keywords(0x8010)).flags.keywords
    };
    let names = ["Work", "To do", "work", " ", "$Junk"].map(String::from).to_vec();
    assert_eq!(read(Val::Texts(names)), ["work", "to=20do"]);
    assert!(read(Val::Text("Work".into())).is_empty());
    assert_eq!(read(Val::Texts((0..100).map(|i| format!("c{i}")).collect())).len(), 64);
    // A file without the name knows no categories.
    let props: HashMap<u16, Val<'static>> = HashMap::from([(0x8010, Val::Texts(vec!["Work".into()]))]);
    assert!(item_with(&props, Vec::new(), &Named::default()).flags.keywords.is_empty());
}

/// An item's References keep as many ids as threading reads, the last ones (nearest the item).
#[test]
fn references_keep_the_ids_threading_reads() {
    let cap = mailcraft_model::thread::MAX_REFERENCES;
    let all: String = (0..cap + 40).map(|n| format!("<r{n}@x> ")).collect();
    let ids = super::super::super::item::message_ids(&all);
    assert_eq!(ids.len(), cap);
    assert_eq!((ids[0].as_str(), ids[cap - 1].as_str()), ("r40@x", &*format!("r{}@x", cap + 39)));
}

/// An unusual address from the file that is one still comes in as the file has it: a quoted
/// local part with a space, a domain ending in a dot.
#[test]
fn an_unusual_address_still_comes_in() {
    let quoted = HashMap::from([(0x39FE, Val::Text("\"a b\"@x.example".into()))]);
    let dotted = HashMap::from([(0x3003, Val::Text("first.last+tag@x.example.".into())), (0x0C15, Val::Int(2))]);
    let props: HashMap<u16, Val<'static>> = HashMap::from([(0x001A, Val::Text("IPM.Note".into()))]);
    let rows = [&quoted, &dotted].map(|r| move |id| get(r, id));
    let it = item_of(&|id| get(&props, id), &rows, Vec::new(), &Named::default());
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
    let read = |headers: &str| {
        let props: HashMap<u16, Val<'_>> = HashMap::from([(0x001A, Val::Text("IPM.Note".into())), (0x007D, Val::Text(headers.into()))]);
        let rows = rows.each_ref().map(|r| move |id| get(r, id));
        item_of(&|id| get(&props, id), &rows, Vec::new(), &Named::default())
    };
    let it = read("");
    assert_eq!((it.recipients_without_address, it.to.len()), (2, 1));
    assert_eq!(read("From: a@x.example\r\nTo: b@x.example\r\n").recipients_without_address, 0);

    // The report counts them, not as items left out.
    let dir = scratch("pst-recipients");
    let store = Store::open(&dir).unwrap();
    let mut progress = |_| {};
    let mut out = Out::new(&store, "r1", &ImportOptions::default(), &mut progress);
    let path = out.add_folder(None, "Inbox");
    out.add_message(&path, Item { subject: "s".into(), body: "b".into(), date: 1_704_067_200, ..read("") });
    let done = out.finish();
    assert_eq!((done.messages, done.skipped), (1, 0));
    assert_eq!(done.skips.into_iter().collect::<Vec<_>>(), [(SkipReason::RecipientWithoutAddress, 2)]);
}
