//! Search's text of a message ([`index_doc`]): the text parts read, the files counted and named
//! but never decoded, and the count the same as the paperclip's ([`summary_from_header`]).

use mailcraft_model::search::IndexDoc;

use super::*;

const HEAD: &str = "From: Ana <ana@studio.example>\r\nTo: jun@studio.example\r\nSubject: Plans\r\nMIME-Version: 1.0\r\n";

/// A message of this type and body.
fn message(content_type: &str, body: &str) -> Vec<u8> {
    format!("{HEAD}Content-Type: {content_type}\r\n\r\n{body}").into_bytes()
}

/// A multipart body of these parts (each its headers, an empty line and its body).
fn parts(boundary: &str, parts: &[String]) -> String {
    let mut out = String::new();
    for p in parts {
        out.push_str(&format!("--{boundary}\r\n{p}\r\n"));
    }
    out.push_str(&format!("--{boundary}--\r\n"));
    out
}

fn text(t: &str) -> String {
    format!("Content-Type: text/plain; charset=utf-8\r\n\r\n{t}")
}

fn html(h: &str) -> String {
    format!("Content-Type: text/html; charset=utf-8\r\n\r\n{h}")
}

fn picture(cid: &str, name: &str, disposition: &str) -> String {
    format!(
        "Content-Type: image/png; name=\"{name}\"\r\nContent-ID: <{cid}>\r\nContent-Disposition: {disposition}; filename=\"{name}\"\r\nContent-Transfer-Encoding: base64\r\n\r\niVBORw0KGgo="
    )
}

fn pdf(name: &str, disposition: &str) -> String {
    format!(
        "Content-Type: application/pdf; name=\"{name}\"\r\nContent-Disposition: {disposition}; filename=\"{name}\"\r\nContent-Transfer-Encoding: base64\r\n\r\nJVBERi0xLjQ="
    )
}

/// `index_doc`, checked against the paperclip: both count the same files.
fn doc(raw: &[u8]) -> IndexDoc {
    let d = index_doc(raw);
    let s = summary_from_header(raw, key(), Flags::default(), 0, 0);
    assert_eq!(s.has_attachments, d.files > 0, "the paperclip and search agree: {d:?}");
    d
}

fn key() -> MessageKey {
    MessageKey { account: "a".into(), folder: "INBOX".into(), validity: 1, uid: 1 }
}

#[test]
fn an_alternative_gives_its_plain_text_once() {
    let raw = message(
        "multipart/alternative; boundary=\"alt\"",
        &parts("alt", &[text("The plan for Monday."), html("<p>The <b>plan</b> for Monday.</p>")]),
    );
    let d = doc(&raw);
    assert_eq!(d.text, "The plan for Monday.");
    assert!(d.has_text && !d.truncated);
    assert_eq!((d.files, d.names.len()), (0, 0));
}

/// Outlook's pictures in the body (`image001.png`, shown by `cid:`) aren't files, nor named;
/// one the HTML doesn't show is a file, named or not.
#[test]
fn pictures_the_html_shows_are_not_files() {
    let related = |pictures: &[String]| {
        let mut ps = vec![html("<p>Hi</p><img src=\"cid:image001.png@01DA\"><img src=\"CID:Logo@x\">")];
        ps.extend(pictures.iter().cloned());
        format!("Content-Type: multipart/related; boundary=\"rel\"\r\n\r\n{}", parts("rel", &ps))
    };
    let shown = [picture("image001.png@01DA", "image001.png", "inline"), picture("logo@x", "logo.png", "inline")];
    let raw = message("multipart/alternative; boundary=\"alt\"", &parts("alt", &[text("Hi"), related(&shown)]));
    let d = doc(&raw);
    assert_eq!((d.files, d.names.len(), d.text.as_str()), (0, 0, "Hi"));

    let not_shown = [picture("image001.png@01DA", "image001.png", "inline"), picture("other@x", "other.png", "inline")];
    let raw = message("multipart/alternative; boundary=\"alt\"", &parts("alt", &[text("Hi"), related(&not_shown)]));
    let d = doc(&raw);
    assert_eq!((d.files, d.names.clone()), (1, vec!["other.png".to_string()]));

    // Shown by its address (RFC 2557) rather than by `cid:`.
    let located =
        "Content-Type: image/png\r\nContent-Location: http://studio.example/chart.png\r\nContent-Transfer-Encoding: base64\r\n\r\niVBORw0KGgo=";
    let body = format!(
        "Content-Type: multipart/related; boundary=\"rel\"\r\n\r\n{}",
        parts("rel", &[html("<img src=\"http://studio.example/chart.png\">"), located.to_string()])
    );
    let d = doc(&message("multipart/mixed; boundary=\"mix\"", &parts("mix", &[body])));
    assert_eq!(d.files, 0);
}

#[test]
fn a_pdf_is_a_file_attached_or_inline() {
    let raw = message("multipart/mixed; boundary=\"mix\"", &parts("mix", &[text("See the report."), pdf("Report Q3.pdf", "attachment")]));
    let d = doc(&raw);
    assert_eq!((d.files, d.names.clone(), d.text.as_str()), (1, vec!["Report Q3.pdf".to_string()], "See the report."));
    // Apple Mail puts a PDF in the body, inline and with no Content-ID: still a file.
    let raw = message("multipart/mixed; boundary=\"mix\"", &parts("mix", &[text("Before"), pdf("inline.pdf", "inline"), text("After")]));
    let d = doc(&raw);
    assert_eq!((d.files, d.names.clone()), (1, vec!["inline.pdf".to_string()]));
    assert!(d.text.contains("Before") && d.text.contains("After"), "{d:?}");
    // A name in RFC 2231's form, decoded.
    let raw = message(
        "multipart/mixed; boundary=\"mix\"",
        &parts(
            "mix",
            &[
                text("x"),
                "Content-Type: application/octet-stream\r\nContent-Disposition: attachment; filename*=utf-8''%C3%A9t%C3%A9.txt\r\n\r\nabc".into(),
            ],
        ),
    );
    assert_eq!(doc(&raw).names, ["été.txt"]);
}

/// S/MIME's own parts are the message, not files in it.
#[test]
fn signed_and_encrypted_mail_has_no_files() {
    let sig = "Content-Type: application/pkcs7-signature; name=\"smime.p7s\"\r\nContent-Disposition: attachment; filename=\"smime.p7s\"\r\nContent-Transfer-Encoding: base64\r\n\r\nMIIB";
    let raw = message(
        "multipart/signed; protocol=\"application/pkcs7-signature\"; micalg=sha-256; boundary=\"sig\"",
        &parts("sig", &[text("Signed words."), sig.to_string()]),
    );
    let d = doc(&raw);
    assert_eq!((d.files, d.names.len(), d.text.as_str()), (0, 0, "Signed words."));
    let raw = format!(
        "{HEAD}Content-Type: application/pkcs7-mime; smime-type=enveloped-data; name=\"smime.p7m\"\r\nContent-Disposition: attachment; filename=\"smime.p7m\"\r\nContent-Transfer-Encoding: base64\r\n\r\nMIIB\r\n"
    );
    let d = doc(raw.as_bytes());
    assert_eq!((d.files, d.has_text), (0, false));
}

/// An attached message: a file named by its subject, its text found with the message's.
#[test]
fn an_attached_message_is_read_and_named() {
    let inner =
        "Content-Type: message/rfc822\r\n\r\nSubject: Notes from Monday\r\nFrom: zoe@studio.example\r\nContent-Type: text/plain\r\n\r\nThe minutes."
            .to_string();
    let raw = message("multipart/mixed; boundary=\"mix\"", &parts("mix", &[text("Forwarding."), inner]));
    let d = doc(&raw);
    assert_eq!(d.files, 1);
    assert_eq!(d.names, ["Notes from Monday.eml"]);
    assert!(d.text.contains("Forwarding.") && d.text.contains("Notes from Monday") && d.text.contains("The minutes."), "{d:?}");
}

#[test]
fn encoded_text_is_decoded() {
    let b64 = "Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\nQ2Fmw6kgYXQgbm9vbg==";
    let qp = "Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\nNa=C3=AFve =\r\nsoft break";
    let latin2 =
        "Content-Type: text/plain; charset=iso-8859-2\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\nZa=BF=F3=B3=E6 g=EA=B6l=B1 ja=BC=F1";
    let raw = message("multipart/mixed; boundary=\"mix\"", &parts("mix", &[b64.into(), qp.into(), latin2.into()]));
    let d = doc(&raw);
    assert!(d.text.contains("Café at noon"), "{d:?}");
    assert!(d.text.contains("Naïve soft break"), "{d:?}");
    assert!(d.text.contains("Zażółć gęślą jaźń"), "{d:?}");
    assert_eq!(d.files, 0);
}

/// Past the old 50,000-character cut a word is found; past a million characters the text stops
/// there, flagged.
#[test]
fn long_bodies_are_kept_to_a_million_characters() {
    let body = format!("{} needle", "word ".repeat(24_000));
    let d = index_doc(&message("text/plain; charset=utf-8", &body));
    assert!(d.text.ends_with("needle") && !d.truncated);
    assert!(search_text(&message("text/plain", &body)).contains("needle"));
    let body = "é".repeat(1_500_000);
    let d = index_doc(&message("text/plain; charset=utf-8", &body));
    assert_eq!(d.text.chars().count(), mailcraft_model::search::MAX_INDEX_CHARS);
    assert!(d.truncated);
}

/// A part a hundred multiparts deep is cut off at the depth limit, without a panic or an
/// overflow; a body never closed runs to its end.
#[test]
fn hostile_structures_are_bounded() {
    let mut body = text("deepest");
    for i in 0..100 {
        body = format!("Content-Type: multipart/mixed; boundary=\"b{i}\"\r\n\r\n{}", parts(&format!("b{i}"), &[body]));
    }
    let d = index_doc(format!("{HEAD}{body}").as_bytes());
    assert!(d.truncated && !d.text.contains("deepest"), "{d:?}");

    let raw = message("multipart/mixed; boundary=\"mix\"", &format!("--mix\r\n{}\r\n--mix\r\n{}", text("first"), pdf("unclosed.pdf", "attachment")));
    let d = doc(&raw);
    assert_eq!((d.text.as_str(), d.files), ("first", 1));

    let many: Vec<String> = (0..2000).map(|i| pdf(&format!("f{i}.pdf"), "attachment")).collect();
    let d = index_doc(&message("multipart/mixed; boundary=\"mix\"", &parts("mix", &many)));
    assert!(d.truncated && d.files < 1000, "{} files", d.files);

    for junk in [
        &b""[..],
        b"\r\n\r\n",
        b"Content-Type: multipart/mixed; boundary=\r\n\r\n--\r\n--",
        b"Content-Type: multipart/mixed; boundary=b\r\n\r\n--b",
        b"--b--",
    ] {
        let _ = index_doc(junk);
    }
}

/// The first delimiter at the body's very start (no preamble), and an inner boundary that starts
/// with the outer one: neither confuses the walk.
#[test]
fn boundaries_are_found_exactly() {
    let raw = message("multipart/mixed; boundary=\"b\"", &parts("b", &[text("at offset zero"), pdf("a.pdf", "attachment")]));
    assert!(raw.ends_with(b"--b--\r\n"));
    let d = doc(&raw);
    assert_eq!((d.text.as_str(), d.files), ("at offset zero", 1));

    let inner =
        format!("Content-Type: multipart/alternative; boundary=\"b-1\"\r\n\r\n{}", parts("b-1", &[text("inner plain"), html("<p>inner html</p>")]));
    let raw = message("multipart/mixed; boundary=\"b\"", &parts("b", &[inner, pdf("b.pdf", "attachment")]));
    let d = doc(&raw);
    assert_eq!((d.text.as_str(), d.files, d.names.clone()), ("inner plain", 1, vec!["b.pdf".to_string()]));

    // `--b` inside a line, or followed by other text, isn't a delimiter.
    let raw = message("multipart/mixed; boundary=\"b\"", "--b\r\nContent-Type: text/plain\r\n\r\nsee --b here\r\n--bx\r\nstill text\r\n--b--\r\n");
    let d = index_doc(&raw);
    assert!(d.text.contains("see --b here") && d.text.contains("--bx") && d.text.contains("still text"), "{d:?}");
}

/// A 20 MB attachment that isn't even valid base64: named and counted, never decoded (what goes
/// to a decoder stays under 64 KiB).
#[test]
fn a_big_attachment_is_never_decoded() {
    let junk = "!!!! not base64 ~~~~\r\n".repeat((20 << 20) / 22);
    let big = format!(
        "Content-Type: application/octet-stream; name=\"big.bin\"\r\nContent-Disposition: attachment; filename=\"big.bin\"\r\nContent-Transfer-Encoding: base64\r\n\r\n{junk}"
    );
    let raw = message("multipart/mixed; boundary=\"mix\"", &parts("mix", &[text("The big one."), big]));
    assert!(raw.len() > 20 << 20);
    let (d, stats) = index_doc_stats(&raw);
    assert_eq!((d.files, d.names.clone(), d.text.as_str()), (1, vec!["big.bin".to_string()], "The big one."));
    assert!(stats.decoded_bytes <= 64 << 10, "{} bytes decoded", stats.decoded_bytes);
}

/// An alternative with no plain text gives its HTML's words, not those of a calendar or
/// enriched text sent after it (a meeting invite: HTML, then `text/calendar`).
#[test]
fn an_alternative_without_plain_text_gives_its_html() {
    let calendar = "Content-Type: text/calendar; method=REQUEST; charset=utf-8\r\n\r\nBEGIN:VCALENDAR\r\nEND:VCALENDAR".to_string();
    let raw = message("multipart/alternative; boundary=\"alt\"", &parts("alt", &[html("<p>meeting agenda words</p>"), calendar]));
    let d = doc(&raw);
    assert_eq!(d.text, "meeting agenda words");
    let enriched = "Content-Type: text/enriched\r\n\r\n<bold>other</bold>".to_string();
    let raw = message("multipart/alternative; boundary=\"alt\"", &parts("alt", &[html("<p>html words</p>"), enriched]));
    assert_eq!(doc(&raw).text, "html words");
    // A related HTML body chosen over a later part that gives no text.
    let related = format!("Content-Type: multipart/related; boundary=\"rel\"\r\n\r\n{}", parts("rel", &[html("<p>related words</p>")]));
    let raw = message("multipart/alternative; boundary=\"alt\"", &parts("alt", &[related, "Content-Type: text/calendar\r\n\r\nX".into()]));
    assert_eq!(doc(&raw).text, "related words");
}

/// A text file sent inline or with no disposition (Apple Mail sends small .txt and .html files
/// so) is a file by its name, as mail-parser and the paperclip take it, not more body text.
#[test]
fn a_named_text_part_is_a_file() {
    for disposition in ["Content-Disposition: inline; filename=\"notes.txt\"\r\n", ""] {
        let named = format!("Content-Type: text/plain; charset=utf-8; name=\"notes.txt\"\r\n{disposition}\r\nfile words");
        let d = doc(&message("multipart/mixed; boundary=\"mix\"", &parts("mix", &[text("body words"), named])));
        assert_eq!((d.files, d.names.clone(), d.text.as_str()), (1, vec!["notes.txt".to_string()], "body words"));
    }
    let page = "Content-Type: text/html; name=\"page.html\"\r\nContent-Disposition: inline; filename=\"page.html\"\r\n\r\n<p>page</p>".to_string();
    let d = doc(&message("multipart/mixed; boundary=\"mix\"", &parts("mix", &[text("body words"), page])));
    assert_eq!((d.files, d.names.clone()), (1, vec!["page.html".to_string()]));
    // The body itself may carry a name, and a filename only in the disposition isn't one.
    let first = "Content-Type: text/plain; name=\"body.txt\"\r\n\r\nfirst words".to_string();
    let only_filename = "Content-Type: text/plain\r\nContent-Disposition: inline; filename=\"x.txt\"\r\n\r\nmore words".to_string();
    let d = doc(&message("multipart/mixed; boundary=\"mix\"", &parts("mix", &[first, only_filename])));
    assert_eq!((d.files, d.text.as_str()), (0, "first words\nmore words"));
}

/// The HTML shows a part only by a whole `cid:` or an address it refers to: not `cid:image10`
/// for `image1`, nor a file's name in its words.
#[test]
fn a_reference_must_be_whole() {
    let pictures = [picture("image1", "a.png", "inline"), picture("image10", "b.png", "inline")];
    let mut ps = vec![html("<img src=\"cid:image10\">")];
    ps.extend(pictures.iter().cloned());
    let body = format!("Content-Type: multipart/related; boundary=\"rel\"\r\n\r\n{}", parts("rel", &ps));
    let d = doc(&message("multipart/mixed; boundary=\"mix\"", &parts("mix", &[body])));
    assert_eq!((d.files, d.names.clone()), (1, vec!["a.png".to_string()]));

    let located = "Content-Type: application/pdf\r\nContent-Location: report.pdf\r\nContent-Transfer-Encoding: base64\r\n\r\nJVBERi0xLjQ=";
    let raw = message("multipart/mixed; boundary=\"mix\"", &parts("mix", &[html("<p>The report.pdf is attached.</p>"), located.to_string()]));
    assert_eq!(doc(&raw).files, 1);
    let by_id = "Content-Type: application/pdf\r\nContent-ID: <part1>\r\nContent-Transfer-Encoding: base64\r\n\r\nJVBERi0xLjQ=";
    let raw = message("multipart/mixed; boundary=\"mix\"", &parts("mix", &[html("<img src=\"cid:part12\">"), by_id.to_string()]));
    assert_eq!(doc(&raw).files, 1);
    // By its last segment, as the reading pane finds it.
    let located = "Content-Type: image/png\r\nContent-Location: http://studio.example/a/chart.png\r\n\r\nx";
    let body =
        format!("Content-Type: multipart/related; boundary=\"rel\"\r\n\r\n{}", parts("rel", &[html("<img src='chart.png'>"), located.to_string()]));
    assert_eq!(doc(&message("multipart/mixed; boundary=\"mix\"", &parts("mix", &[body]))).files, 0);
}

/// A long boundary is followed; one past the limit is flagged; a multipart whose boundary never
/// comes is one file, as mail-parser lists it.
#[test]
fn unusual_boundaries_keep_the_count() {
    let long = "b".repeat(210);
    let raw = message(&format!("multipart/mixed; boundary=\"{long}\""), &parts(&long, &[text("long words"), pdf("a.pdf", "attachment")]));
    let d = doc(&raw);
    assert_eq!((d.text.as_str(), d.files, d.truncated), ("long words", 1, false));

    let huge = "b".repeat(5000);
    let raw = message(&format!("multipart/mixed; boundary=\"{huge}\""), &parts(&huge, &[text("x")]));
    assert!(index_doc(&raw).truncated);

    let raw = message("multipart/mixed; boundary=\"never\"", "just some words\r\n");
    let d = doc(&raw);
    assert_eq!(d.files, 1);
}
