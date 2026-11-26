use super::*;

fn doc(body: &str) -> Doc {
    Doc {
        title: "Storyboard (v2) review".into(),
        header: vec![("From".into(), "Zoe Park <zoe@studio.example>".into()), ("Date".into(), "Wed 14 Oct 2026 09:30".into())],
        body: body.into(),
        ..Default::default()
    }
}

/// Every object starts exactly where the cross-reference table says (on the bytes: the
/// streams are binary).
fn objects_are_where_the_xref_says(pdf: &[u8]) {
    let find_last = |needle: &[u8]| pdf.windows(needle.len()).rposition(|w| w == needle);
    let sx = find_last(b"startxref\n").unwrap() + 10;
    let tail = String::from_utf8_lossy(&pdf[sx..]);
    let start: usize = tail.lines().next().unwrap().trim().parse().unwrap();
    assert!(pdf[start..].starts_with(b"xref"));
    let table = String::from_utf8_lossy(&pdf[start..sx]).to_string();
    let mut n = 0;
    for (i, line) in table.lines().skip(3).take_while(|l| l.ends_with(" n ")).enumerate() {
        let off: usize = line[..10].parse().unwrap();
        assert!(pdf[off..].starts_with(format!("{} 0 obj", i + 1).as_bytes()), "object {} at {off}", i + 1);
        n += 1;
    }
    assert!(n >= 4);
}

/// A laid-out message: its boxes, pictures (a JPEG as it is, another with what shows through
/// it as a mask) and links are in the file, each picture once, and the file holds together.
#[test]
fn a_laid_out_message_has_its_boxes_pictures_and_links() {
    let png = |pixel: [u8; 4]| {
        let mut out = std::io::Cursor::new(Vec::new());
        image::RgbaImage::from_pixel(8, 6, image::Rgba(pixel)).write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    };
    let mut jpeg = std::io::Cursor::new(Vec::new());
    image::RgbImage::from_pixel(16, 8, image::Rgb([200, 100, 50])).write_to(&mut jpeg, image::ImageFormat::Jpeg).unwrap();
    let pictures = vec![
        Picture::decode(&png([9, 9, 9, 100])).unwrap(),
        Picture::decode(&jpeg.into_inner()).unwrap(),
        Picture::decode(&png([1, 1, 1, 255])).unwrap(),
    ];
    let picture = |index| Run { picture: Some((index, 60.0, 45.0)), ..Default::default() };
    let link = Run { text: "Track package".into(), link: Some("https://shop.example/track?a=(1)".into()), underline: true, ..Default::default() };
    let cell = |runs: Vec<Run>| Cell { blocks: vec![Block::Text(vec![runs])], background: Some([240, 240, 240]), radius: 6.0, ..Default::default() };
    // Enough rows to go over a page: the pictures are drawn on several pages, and kept once.
    let rows: Vec<Vec<Cell>> = (0..40).map(|_| vec![cell(vec![picture(0), picture(1)]), cell(vec![link.clone(), picture(7)])]).collect();
    let table = Table { rows, width: Some(Width::Percent(100.0)), spacing: 3.0, padding: 4.0, ..Default::default() };
    let d = Doc { blocks: vec![Block::Table(table)], pictures, ..doc("") };
    let p = pdf(&d, None);
    objects_are_where_the_xref_says(&p);
    let text = String::from_utf8_lossy(&p);
    let count: usize = text.split("/Count ").nth(1).and_then(|r| r.split_whitespace().next()).and_then(|n| n.parse().ok()).unwrap();
    assert!(count >= 3, "{count} pages");
    // The two it shows, each once (and the mask of the one that shows through); not the one
    // it doesn't show, nor one it names that isn't there.
    assert_eq!(text.matches("/Subtype /Image").count(), 3);
    assert_eq!((text.matches("/DCTDecode").count(), text.matches("/SMask ").count()), (1, 1));
    let dict = text.split("/XObject <<").nth(1).and_then(|r| r.split(">>").next()).unwrap();
    assert!(dict.contains("/Im0 ") && dict.contains("/Im1 ") && !dict.contains("/Im2") && !dict.contains("/Im7"), "{dict}");
    assert_eq!(text.matches("/URI (https://shop.example/track?a=\\(1\\))").count(), 40);
    // Nothing but the header on a page of its own, and no blocks: as before.
    let plain = pdf(&doc("Hello"), None);
    assert!(!String::from_utf8_lossy(&plain).contains("/XObject"));
}

#[test]
fn helvetica_pages() {
    let long = (0..400).map(|i| format!("Line {i}: the quick brown fox jumps over the lazy dog (café, “quotes”)")).collect::<Vec<_>>().join("\n");
    let p = pdf(&doc(&long), None);
    assert!(p.starts_with(b"%PDF-1.7"));
    assert!(p.ends_with(b"%%EOF\n"));
    let text = String::from_utf8_lossy(&p);
    assert!(text.contains("/BaseFont /Helvetica"));
    let count: usize = text.split("/Count ").nth(1).and_then(|r| r.split_whitespace().next()).and_then(|n| n.parse().ok()).unwrap();
    assert!(count >= 8, "{count} pages");
    assert!(text.contains("/Title (Storyboard \\(v2\\) review)"));
    objects_are_where_the_xref_says(&p);
}

#[test]
fn wrapping() {
    let f = Face::Helvetica { bold: false };
    let lines = wrap(&f, "a b c", 11.0, 1000.0);
    assert_eq!(lines, vec!["a b c".to_string()]);
    let lines = wrap(&f, &"word ".repeat(100), 11.0, 200.0);
    assert!(lines.len() > 5 && lines.iter().all(|l| f.text_width(l, 11.0) <= 200.0));
    let lines = wrap(&f, &"x".repeat(500), 11.0, 100.0);
    assert!(lines.len() > 5 && lines.iter().all(|l| f.text_width(l, 11.0) <= 100.0), "long words are cut");
    assert_eq!(wrap(&f, "> quoted\n\n  indented", 11.0, 400.0), vec!["> quoted".to_string(), String::new(), "  indented".to_string()]);
    assert_eq!(win_ansi('é'), 0xE9);
    assert_eq!(win_ansi('€'), 0x80);
    assert_eq!(win_ansi('漢'), b'?');
}

#[test]
fn empty_and_hostile_input() {
    let p = pdf(&Doc::default(), None);
    objects_are_where_the_xref_says(&p);
    // Not a font: falls back to Helvetica rather than failing.
    let p = pdf(&doc("x"), Some(Fonts { regular: b"not a font", bold: None }));
    assert!(String::from_utf8_lossy(&p).contains("/Helvetica"));
    let huge = "y\n".repeat(200_000);
    let p = pdf(&doc(&huge), None);
    assert!(String::from_utf8_lossy(&p).contains(&format!("/Count {MAX_PAGES}")));
}

/// A TrueType font the system has, when it has one of these.
fn system_font() -> Option<Vec<u8>> {
    // Windows' fonts are in its folder (`%WINDIR%`), which needn't be `C:\Windows`.
    let windows = std::env::var_os("WINDIR").map_or_else(|| std::path::PathBuf::from("C:\\Windows"), std::path::PathBuf::from).join("Fonts");
    let candidates = [
        windows.join("segoeui.ttf"),
        windows.join("arial.ttf"),
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf".into(),
        "/System/Library/Fonts/Supplemental/Arial.ttf".into(),
    ];
    candidates.iter().find_map(|p| std::fs::read(p).ok())
}

/// How long a word too wide for the line takes to cut when a long run of accents follows it
/// (they take no room, so the piece they're on never fills).
/// A long word is cut asking each character's width once: its piece was measured again
/// from its start at every character, so a run of characters that take no room (accents),
/// which never fills a piece, was measured as many times over as it is long.
#[test]
fn a_word_is_cut_measuring_each_character_once() {
    let asked = std::cell::Cell::new(0usize);
    // Accents take no room; a letter half the size.
    let advance = |c: char| {
        asked.set(asked.get() + 1);
        if c == '\u{301}' { 0.0 } else { 500.0 }
    };
    let word: String = "\u{301}".repeat(3000) + &"a".repeat(7) + &"\u{301}".repeat(2000) + "bc";
    let mut pieces: Vec<(String, f32)> = Vec::new();
    // Size 10, 20 wide: four letters a piece.
    let rest = cut_word(&word, 10.0, 20.0, advance, |piece, w| {
        pieces.push((piece, w));
        true
    });
    assert_eq!(asked.get(), word.chars().count(), "each character's width asked once");
    pieces.extend(rest);
    let letters: Vec<usize> = pieces.iter().map(|(p, _)| p.chars().filter(|c| *c != '\u{301}').count()).collect();
    assert_eq!(letters, [4, 4, 1]);
    let widths: Vec<f32> = pieces.iter().map(|(_, w)| *w).collect();
    assert_eq!(widths, [20.0, 20.0, 5.0]);
    assert_eq!(pieces.iter().map(|(p, _)| p.as_str()).collect::<String>(), word, "nothing lost");
    // Told to stop at the first full piece: no more asked than the characters up to it.
    asked.set(0);
    assert!(cut_word(&word, 10.0, 20.0, advance, |_, _| false).is_none());
    assert_eq!(asked.get(), 3005);
}

#[test]
#[ignore = "a measurement, with a system font"]
fn measure_cutting_a_word_of_accents() {
    let data = system_font().unwrap();
    let faces = [Face::parse(&data).unwrap(), Face::Helvetica { bold: true }];
    assert_eq!(faces[0].width('\u{308}'), 0.0, "the accent takes no room");
    let word = format!("{}{}", "x".repeat(200), "\u{308}".repeat(20_000));
    let start = std::time::Instant::now();
    let lines = wrap(&faces[0], &word, 11.0, 480.0);
    let plain = start.elapsed();
    let start = std::time::Instant::now();
    let mut rows = 0;
    rich::lay(&faces, &[vec![Run { text: word.clone(), ..Default::default() }]], 480.0, |_, _| {
        rows += 1;
        true
    });
    let formatted = start.elapsed();
    println!(
        "MEASURE cutting a word of 200 letters and 20,000 accents: wrap {plain:?} ({} lines), rich::lay {formatted:?} ({rows} rows)",
        lines.len()
    );
}

#[test]
fn embeds_a_system_font_when_there_is_one() {
    let Some(data) = system_font() else {
        // Windows and macOS always have one of them, so it's tested there; a Linux without
        // DejaVu has none to embed.
        if cfg!(any(windows, target_os = "macos")) {
            panic!("no system font to embed");
        }
        eprintln!("skipped: no system font to embed");
        return;
    };
    let p = pdf(&doc("Hello café — ünïcödé"), Some(Fonts { regular: &data, bold: None }));
    let text = String::from_utf8_lossy(&p);
    assert!(text.contains("/Subtype /CIDFontType2") && text.contains("/FontFile2") && text.contains("/ToUnicode"));
    assert!(p.len() < data.len(), "the font file is compressed");
    objects_are_where_the_xref_says(&p);
}
