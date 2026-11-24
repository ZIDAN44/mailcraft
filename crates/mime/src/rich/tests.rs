use super::*;

fn bold() -> Style {
    Style { bold: true, ..Default::default() }
}

#[test]
fn typing_follows_the_text() {
    let mut s = Styles::plain("Hello world");
    s.restyle("Hello world", 0..5, |st| Style { bold: true, ..st.clone() });
    // Typed inside the bold word: bold; deleted across: gone.
    s.edit("Hello world", "Helllo world", &bold());
    assert_eq!(s.chars.len(), 12);
    assert!(s.style(3).bold && !s.style(7).bold);
    s.edit("Helllo world", "Hd", &Style::default());
    assert_eq!((s.chars.len(), s.style(0).bold, s.style(1).bold), (2, true, false));
    // A new paragraph takes the layout of the one it split.
    let mut s = Styles::plain("one\ntwo");
    s.relayout("one\ntwo", 0..1, |p| p.list = List::Bullet);
    // Enter at the end of "one", then "new": typed where the cursor is.
    s.edit_at("one\ntwo", "one\nnew\ntwo", &Style::default(), Some(7));
    assert_eq!(s.paras.iter().map(|p| p.list).collect::<Vec<_>>(), [List::Bullet, List::Bullet, List::None]);
    s.edit("one\nnew\ntwo", "one two", &Style::default());
    assert_eq!(s.paras.len(), 1);
    // A cursor past the end of the text (egui's, after Arabic text) is not trusted.
    for cursor in [9, 40, usize::MAX] {
        let mut s = Styles::plain("ab");
        s.edit_at("ab", "abcd", &Style::default(), Some(cursor));
        assert_eq!(s.chars.len(), 4, "cursor {cursor}");
    }
    // Junk lengths are fixed rather than trusted.
    let mut j = Styles { chars: vec![9, 9, 9, 9, 9, 9], palette: vec![], paras: vec![] };
    j.fit("ab");
    assert_eq!((j.chars.as_slice(), j.paras.len()), (&[0u16, 0][..], 1));
}

#[test]
fn html_and_text() {
    let text = "Hi team\nfirst\nsecond\n\nsee site\nx\u{FFFC}";
    let mut s = Styles::plain(text);
    s.restyle(text, 0..2, |st| Style { bold: true, color: Some([200, 0, 0]), ..st.clone() });
    s.relayout(text, 8..10, |p| p.list = List::Number);
    s.relayout(text, 14..16, |p| p.list = List::Number);
    s.restyle(text, 26..30, |st| Style { link: Some("https://a.example/".into()), ..st.clone() });
    s.relayout(text, 22..23, |p| p.align = Align::Center);
    s.restyle(text, 32..33, |st| Style { picture: Some("pic@x".into()), ..st.clone() });
    s.restyle(text, 3..7, |st| Style { size: Some(18), highlight: Some([255, 255, 0]), italic: true, underline: true, ..st.clone() });
    let html = to_html(text, &s);
    assert!(html.contains("<span style=\"color:#c80000\"><b>Hi</b></span>"), "{html}");
    assert!(html.contains("<span style=\"font-size:18pt; background-color:#ffff00\"><i><u>team</u></i></span>"), "{html}");
    assert!(html.contains("<ol>\n<li>first</li>\n<li>second</li>\n</ol>"), "{html}");
    assert!(html.contains("<div style=\"text-align:center\">see <a href=\"https://a.example/\">site</a></div>"), "{html}");
    assert!(html.contains("<div><br></div>") && html.contains("x<img src=\"cid:pic@x\" style=\"max-width:100%\">"), "{html}");
    // A resized picture says how wide it was made; typing after it doesn't carry that on.
    let mut resized = s.clone();
    resized.restyle(text, 32..33, |st| Style { picture_width: Some(240), ..st.clone() });
    assert!(to_html(text, &resized).contains("x<img src=\"cid:pic@x\" width=\"240\" style=\"max-width:100%\">"));
    assert_eq!(resized.style(32).for_typing().picture_width, None);
    let t = to_text(text, &s);
    assert_eq!(t, "Hi team\n1. first\n2. second\n\nsee site <https://a.example/>\nx[picture]");
    // Nothing typed can become markup.
    let evil = "<script>x</script>";
    assert!(!to_html(evil, &Styles::plain(evil)).contains("<script>"));
    let mut s = Styles::plain("a");
    s.restyle("a", 0..1, |st| Style { link: Some("javascript:alert(1)".into()), ..st.clone() });
    assert!(!to_html("a", &s).contains("javascript"));
}

#[test]
fn rules_received_formatting_and_appending() {
    let mut text = "Thanks".to_string();
    let mut styles = Styles::plain(&text);
    let (orig, ost) = from_rich(&vec![
        vec![Span { text: "Big".into(), heading: 1, ..Default::default() }],
        vec![Span { text: "plain ".into(), ..Default::default() }, Span { text: "red".into(), color: Some([255, 0, 0]), ..Default::default() }],
    ]);
    assert_eq!(orig, "Big\nplain red");
    assert!(ost.style(0).bold && ost.style(0).size == Some(20) && ost.style(12).color == Some([255, 0, 0]));
    let mut rule = Styles::plain("\n\n");
    rule.paras[1].rule = true;
    append(&mut text, &mut styles, "\n\n", &rule);
    append(&mut text, &mut styles, &orig, &ost);
    assert_eq!(text, "Thanks\n\nBig\nplain red");
    assert!(styles.paras[1].rule && !styles.paras[2].rule);
    assert!(styles.style(8).bold);
    assert!(to_html(&text, &styles).contains("<hr"));
    assert_eq!(to_text(&text, &styles).lines().nth(1), Some(crate::markup::RULE));
    assert_eq!((step_size(11, true), step_size(11, false), step_size(36, true)), (12, 10, 36));
    assert_eq!(lines_of("ab\ncd\nef", 1..4), (0, 1));
    assert!(all("abc", &styles_bold("abc"), 0..3, |s| s.bold) && !all("abc", &styles_bold("abc"), 0..0, |s| s.bold));
}

fn styles_bold(t: &str) -> Styles {
    let mut s = Styles::plain(t);
    s.restyle(t, 0..t.len(), |_| bold());
    s
}

#[test]
#[ignore = "a measurement: cargo test --release -p mailcraft-mime --lib rich_cost -- --ignored --nocapture"]
fn rich_cost() {
    // 100,000 characters in 1,000 paragraphs, every word linked: Select All, Bold, then send.
    let text: String = (0..1000).map(|_| "word ".repeat(20)).collect::<Vec<_>>().join("\n");
    let n = text.chars().count();
    let mut s = Styles::plain(&text);
    s.restyle(&text, 0..n, |st| Style { link: Some("https://studio.example/a/long/address".into()), font: Some("Calibri".into()), ..st.clone() });
    let t = std::time::Instant::now();
    s.restyle(&text, 0..n, |st| Style { bold: true, ..st.clone() });
    let restyle = t.elapsed();
    let t = std::time::Instant::now();
    let toggled = all(&text, &s, 0..n, |st| st.bold);
    let all_time = t.elapsed();
    let t = std::time::Instant::now();
    let html = to_html(&text, &s);
    let html_time = t.elapsed();
    let t = std::time::Instant::now();
    let plain = to_text(&text, &s);
    let text_time = t.elapsed();
    println!(
        "MEASURE {n} linked characters: restyle {} ms, all {} ms, to_html {} ms, to_text {} ms",
        restyle.as_millis(),
        all_time.as_millis(),
        html_time.as_millis(),
        text_time.as_millis()
    );
    assert!(toggled && html.len() > n && plain.len() >= n);
}

/// A message's own typeface (issue #166) is first in its font list, before MailCraft's; a name
/// can't end the quotes or the attribute, and one with nothing left of it leaves MailCraft's.
#[test]
fn a_message_in_a_typeface_of_its_own() {
    let s = Styles::plain("Hi");
    assert!(to_html_in("Hi", &s, Some("Georgia"), None).starts_with("<div style=\"font-family: 'Georgia', Aptos, Calibri"));
    assert_eq!(to_html_in("Hi", &s, None, None), to_html("Hi", &s));
    let evil = to_html_in("Hi", &s, Some("x'; color: red\"><script>alert(1)</script>"), None);
    assert!(!evil.contains("<script>") && !evil.contains("color: red\""), "{evil}");
    assert!(evil.starts_with("<div style=\"font-family: 'x color red"), "{evil}");
    assert_eq!(to_html_in("Hi", &s, Some("'\";<>"), None), to_html("Hi", &s));
    let long = "A".repeat(500);
    assert_eq!(font_name(&long).map(|f| f.chars().count()), Some(FONT_NAME_MAX));
    assert_eq!(font_name("  Segoe UI  ").as_deref(), Some("Segoe UI"));
}

/// The size for new messages (issue #267) is the message's own: the text with none set on it
/// is in it, a size set on text is written where it differs from it, and only a size the size
/// box offers is taken.
#[test]
fn a_message_is_written_in_its_own_size() {
    let text = "Big small same";
    let mut s = Styles::plain(text);
    s.restyle(text, 4..9, |st| Style { size: Some(11), ..st.clone() });
    s.restyle(text, 10..14, |st| Style { size: Some(14), ..st.clone() });
    let html = to_html_in(text, &s, None, Some(14));
    assert!(html.starts_with("<div style=\"font-family: Aptos, Calibri, 'Segoe UI', Arial, sans-serif; font-size: 14pt\">"), "{html}");
    assert!(html.contains("<span style=\"font-size:11pt\">small</span>"), "{html}");
    assert!(!html.contains("font-size:14pt"), "the message's own size isn't written again: {html}");
    // MailCraft's own, as before.
    let own = to_html_in(text, &s, None, None);
    assert!(own.contains("font-size: 11pt\">") && own.contains("font-size:14pt") && !own.contains("font-size:11pt"), "{own}");
    // A size the box doesn't offer is MailCraft's own.
    assert_eq!(to_html_in(text, &s, None, Some(13)), own);
    assert_eq!(to_html_in(text, &s, None, Some(0)), own);
    assert_eq!((font_size(Some(36)), font_size(Some(37)), font_size(None)), (Some(36), None, None));
}
