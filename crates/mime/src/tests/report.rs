use super::*;

pub(crate) fn report() -> Vec<u8> {
    let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
    format!("From: Reports <reports@fw.example>\r\nTo: me@studio.example\r\nSubject: Reports: Daily\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"M\"\r\n\r\n\
--M\r\nContent-Type: multipart/alternative; boundary=\"A\"\r\n\r\n\
--A\r\nContent-Type: text/plain\r\n\r\nHTML Report enclosed.\r\n\
--A\r\nContent-Type: text/html; charset=utf-8\r\n\r\n\
<html><head><style>.title {{ background-color: #888888; color: #ffffff; font-size: 20px }} td.odd {{ background-color: #f0f0f0 }} .bar {{ background-color:#333333; color:#fff }}</style></head><body>\
<table width=\"100%\"><tr><td><img src=\"cid:logo@x\"></td><td><h2>Reports: Daily</h2></td></tr></table>\
<table width=\"100%\"><tr><td class=\"bar\"><img src=\"icon_config.png\"> Config</td></tr></table>\
<table width=\"100%\"><tr><td class=\"title\" colspan=\"2\">Summary</td></tr><tr><td>Hostname</td><td>untangle</td></tr><tr><td class=\"odd\">Version</td><td class=\"odd\">17.4.1</td></tr></table>\
<div style=\"text-align:center\"><img src=\"cid:chart%201.png\"><img src=\"cid:host.png\"></div></body></html>\r\n\
--A--\r\n\
--M\r\nContent-Type: image/png\r\nContent-ID: <logo@x>\r\nContent-Disposition: inline; filename=\"BrandingLogo.png\"\r\nContent-Transfer-Encoding: base64\r\n\r\n{png}\r\n\
--M\r\nContent-Type: image/png\r\nContent-Location: icon_config.png\r\nContent-Transfer-Encoding: base64\r\n\r\n{png}\r\n\
--M\r\nContent-Type: image/png\r\nContent-ID: <chart 1.png>\r\nContent-Disposition: inline; filename=\"chart 1.png\"\r\nContent-Transfer-Encoding: base64\r\n\r\n{png}\r\n\
--M\r\nContent-Type: image/png\r\nContent-Disposition: attachment; filename=\"host.png\"\r\nContent-Transfer-Encoding: base64\r\n\r\n{png}\r\n\
--M--\r\n").into_bytes()
}

/// Issue #1: a firewall's daily report as Outlook shows it. Its logo, icon and charts have
/// no description and are found by `cid:`, by Content-Location and by file name; its grey
/// bars and striped rows come from a style sheet's classes, and its titles are sized ones.
#[test]
fn a_generated_report_keeps_its_pictures_colours_and_titles() {
    let p = parse(&report()).unwrap();
    let found: Vec<Option<usize>> = p.images.iter().map(|i| picture_attachment(&p, i)).collect();
    assert_eq!(found, [Some(0), Some(1), Some(2), Some(3)], "{:?}", p.images);
    assert_eq!(picture_attachment(&p, "https://fw.example/logo.png"), None, "the web is never an attachment");
    assert_eq!(picture_attachment(&p, "cid:nothing@x"), None);
    assert_eq!(picture_attachment(&p, "images/host.png?v=2"), Some(3), "a path's file name");
    // Every picture has its place, in the flowing text and in the layout.
    let pictures = |r: &RichText| r.iter().flatten().filter(|s| s.image.is_some()).count();
    assert_eq!(pictures(&p.rich), 4);
    let mut cells = Vec::new();
    fn walk<'a>(b: &'a [layout::Block], out: &mut Vec<&'a layout::Cell>) {
        for x in b {
            if let layout::Block::Table(t) = x {
                for c in t.rows.iter().flatten() {
                    out.push(c);
                    walk(&c.blocks, out);
                }
            }
        }
    }
    walk(&p.layout, &mut cells);
    let text = |c: &layout::Cell| -> String {
        c.blocks
            .iter()
            .map(|b| if let layout::Block::Text(r) = b { r.iter().flatten().map(|s| s.text.as_str()).collect::<String>() } else { String::new() })
            .collect()
    };
    let cell = |t: &str| cells.iter().find(|c| text(c).contains(t)).copied().unwrap();
    assert_eq!(cell("Config").background, Some([0x33, 0x33, 0x33]), "a class's background");
    let summary = cell("Summary");
    assert_eq!((summary.background, summary.span), (Some([0x88, 0x88, 0x88]), 2));
    assert!(
        matches!(&summary.blocks[0], layout::Block::Text(r) if r[0][0].heading == 3 && r[0][0].color == Some([255, 255, 255])),
        "sized as a title, in the class's colour"
    );
    assert_eq!(cell("17.4.1").background, Some([0xf0, 0xf0, 0xf0]), "td.odd");
    assert_eq!(cell("untangle").background, None);
    let logo = cells
        .iter()
        .find(|c| matches!(c.blocks.first(), Some(layout::Block::Text(r)) if r.iter().flatten().any(|s| s.image.as_deref() == Some("cid:logo@x"))));
    assert!(logo.is_some(), "the logo's cell stays");
    let charts = cells
        .iter()
        .find(|c| matches!(c.blocks.first(), Some(layout::Block::Text(r)) if r.iter().flatten().any(|s| s.image.as_deref() == Some("cid:host.png"))));
    assert_eq!(charts.map(|c| c.align), Some(layout::Align::Center), "centred charts stay centred");
}

/// Tracking pixels stay out; a picture's own description is kept; `<font size>` titles.
#[test]
fn pictures_without_a_description_keep_their_place() {
    let r = html_to_rich(
        r#"<p><img src="cid:a@x"><img src="https://t.example/p.gif" width="1" height="1"><img src="cid:b@x" alt="Chart"><img alt="" src='cid:c@x'></p>"#,
    );
    let imgs: Vec<(&str, &str)> = r.iter().flatten().filter_map(|s| Some((s.image.as_deref()?, s.text.as_str()))).collect();
    assert_eq!(imgs, [("cid:a@x", ""), ("cid:b@x", "Chart"), ("cid:c@x", "")]);
    assert!(r.iter().flatten().all(|s| !s.text.contains(PICTURE_MARK)));
    let p = layout::inline_css(r#"<div><font size="6">Big</font></div><div style="font-size:11pt">Body</div>"#);
    let r = html_to_rich(&p);
    let level = |t: &str| r.iter().flatten().find(|s| s.text.contains(t)).map(|s| s.heading);
    assert_eq!((level("Big"), level("Body")), (Some(1), Some(0)), "{p}");
}
