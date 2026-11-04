use super::*;

/// A signature laid out in nested tables, as business mail writes one: a column of icons beside
/// the numbers and addresses, a logo on the right, a coloured warning line, then a centred logo
/// under it all.
const CARD: &str = r##"<html><body>
<p>Dear Sam,</p><p>We will share the updated invoice soon.</p>
<table width="640" cellpadding="0" cellspacing="0"><tr>
  <td width="40"><table><tr><td><img src="cid:phone@x" width="16" alt="phone"></td></tr><tr><td><img src="cid:web@x" width="16" alt="web"></td></tr></table></td>
  <td width="420"><table>
    <tr><td>+00 1234 5678</td></tr>
    <tr><td><a href="https://example.com/">example.com</a> | <b style="color:#ff0000">A warning in red</b></td></tr>
    <tr><td><b>Office</b>: 1 Example Road, Example Town</td></tr>
  </table></td>
  <td width="180" align="right"><img src="cid:logo@x" width="120" alt="logo"></td>
</tr></table>
<table width="640"><tr><td align="center"><img src="cid:logo2@x" width="90" alt="mill"></td></tr></table>
<hr><p><b>From:</b> Someone</p>
</body></html>"##;

fn card() -> Parsed {
    let raw = format!(
        "From: Morgan <morgan@mill.example>\r\nTo: sam@studio.example\r\nSubject: Invoice\r\nMIME-Version: 1.0\r\nContent-Type: text/html; charset=utf-8\r\n\r\n{CARD}\r\n"
    );
    parse(raw.as_bytes()).unwrap()
}

/// The owner's report: Reply turned a signature in layout tables into `│` and `├──` lines with
/// stray spaces. The original goes as its cells one after another, keeping its pictures, link,
/// colour and alignment.
#[test]
fn a_reply_quotes_a_table_layout_cleanly() {
    let p = card();
    assert!(!p.layout.is_empty(), "the card has columns");
    for mode in [ReplyMode::Reply, ReplyMode::ReplyAll, ReplyMode::Forward] {
        let (d, styles) = reply_doc(&p, mode, &[], &Clock::default(), "");
        let boxy = |c: char| ('\u{2500}'..='\u{257F}').contains(&c);
        assert!(!d.body.chars().any(boxy), "{mode:?}: {}", d.body);
        // No line padded out to a column.
        assert!(d.body.lines().all(|l| !l.starts_with("    ")), "{}", d.body);
        for want in ["Dear Sam,", "+00 1234 5678", "A warning in red", "Office: 1 Example Road, Example Town"] {
            assert!(d.body.lines().any(|l| l.trim() == want || l.contains(want)), "{want} in {}", d.body);
        }
        let pictures = rich::pictures(&d.body, &styles);
        assert_eq!(pictures, ["phone@x", "web@x", "logo@x", "logo2@x"], "{}", d.body);
        let html = rich::to_html(&d.body, &styles);
        assert!(html.contains("<span style=\"color:#ff0000\"><b>A warning in red</b></span>"), "{html}");
        assert!(html.contains("<a href=\"https://example.com/\">example.com</a>"), "{html}");
        assert!(html.contains("<img src=\"cid:logo@x\" width=\"120\""), "{html}");
        // The right-hand and centred cells keep their places.
        assert!(html.contains("<div style=\"text-align:right\"><img src=\"cid:logo@x\""), "{html}");
        assert!(html.contains("<div style=\"text-align:center\"><img src=\"cid:logo2@x\""), "{html}");
        assert!(!html.contains("&#x2502;") && !html.contains('│'), "{html}");
    }
}

/// Pictures a reply carries on are found in the layout too.
#[test]
fn quoted_pictures_reach_the_layout() {
    let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
    let html = CARD.replace("cid:logo@x", "logo.png");
    let raw = format!(
        "From: Morgan <morgan@mill.example>\r\nTo: sam@studio.example\r\nSubject: Invoice\r\nMIME-Version: 1.0\r\nContent-Type: multipart/related; boundary=\"R\"\r\n\r\n\
--R\r\nContent-Type: text/html; charset=utf-8\r\n\r\n{html}\r\n\
--R\r\nContent-Type: image/png\r\nContent-Location: logo.png\r\nContent-Transfer-Encoding: base64\r\n\r\n{png}\r\n--R--\r\n"
    );
    let mut p = parse(raw.as_bytes()).unwrap();
    assert!(!p.layout.is_empty());
    let (pictures, _) = quoted_pictures(&mut p, raw.as_bytes());
    let cid = pictures.first().map(|q| q.cid.clone()).unwrap_or_default();
    assert!(!cid.is_empty(), "{pictures:?}");
    let (d, styles) = reply_doc(&p, ReplyMode::Forward, &[], &Clock::default(), "");
    assert!(rich::pictures(&d.body, &styles).contains(&cid), "{}", d.body);
}

/// A layout written out as HTML again (the original under a reply, changed in place) reads back
/// as the same layout: its blocks of text in the same order, its widths, its label over the
/// picture.
#[test]
fn a_layout_written_out_reads_back_the_same() {
    let html = r##"<table width="600" align="center"><tr><td bgcolor="#2e7d6b" style="color:#ffffff"><h2>Open call</h2></td></tr>
<tr><td><table width="100%"><tr><td width="60%"><p>Dear artists,</p><p>Up to <b>three works</b>.</p></td>
<td width="40%" align="center"><div style="position:relative;width:180px"><img src="cid:poster@x" alt="Poster" width="160">
<span style="position:absolute;top:6px;right:6px;background-color:#c62828;color:#ffffff"><b>NEW</b></span></div>
<p><span style="color:#c62828">Deadline</span></p></td></tr></table></td></tr></table>"##;
    let first = layout::html_layout(html);
    assert!(!first.is_empty());
    let texts = |b: &[layout::Block]| -> Vec<String> {
        layout::text_blocks(b).into_iter().map(|(l, _)| rich::from_rich(l).0).filter(|t| !t.trim().is_empty()).collect()
    };
    let cells = layout::text_blocks(&first);
    let out = layout::to_html(&first, &mut |n| {
        cells
            .get(n)
            .map(|(l, _)| {
                let (t, s) = rich::from_rich(l);
                rich::to_html(&t, &s)
            })
            .unwrap_or_default()
    });
    let again = layout::html_layout(&out);
    assert_eq!(texts(&again), texts(&first), "{out}");
    fn overlays(b: &[layout::Block]) -> usize {
        b.iter()
            .map(|b| match b {
                layout::Block::Table(t) => t.rows.iter().flatten().map(|c| c.overlays.len() + overlays(&c.blocks)).sum(),
                layout::Block::Text(_) => 0,
            })
            .sum()
    }
    assert_eq!(overlays(&again), 1, "the label over the picture: {out}");
    assert!(out.contains("width=\"600\"") && out.contains("bgcolor=\"#2e7d6b\"") && out.contains("width=\"60%\""), "{out}");
}
