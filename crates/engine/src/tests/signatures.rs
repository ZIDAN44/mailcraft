//! Engine tests: several signatures per account, one for new messages and one (or none) for
//! replies and forwards, picked from the message's Signature menu, and following From to
//! another account (issue #11).

use super::*;
use mailcraft_mime as mime;

/// A signature with a picture in it, as the signature editor saves one.
fn with_picture(name: &str, text: &str, png: &[u8]) -> Value {
    let pic = mime::InlinePicture { cid: "logo@x".into(), name: "logo.png".into(), mime: "image/png".into(), data: png.to_vec() };
    let html = mime::embed_pictures(&format!("<p><b>{text}</b> <img src=\"cid:logo@x\" width=\"64\"></p>"), &[pic]);
    json!({"account": "sample", "name": name, "signature": text, "html": html})
}

fn body(s: &Session) -> String {
    s.compose.as_ref().map(|c| c.body.clone()).unwrap_or_default()
}

fn second_account(s: &mut Session) -> String {
    s.run("account.add", &json!({"email": "desk@studio.example", "password": "pw", "name": "Front desk"})).unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string()
}

/// The sample's one signature from older settings is "Signature", for both; a "Short" one for
/// replies goes into replies and forwards, "(none)" puts none in, and the list says so.
#[test]
fn new_messages_and_replies_have_signatures_of_their_own() {
    let mut s = sample_session("signatures-defaults");
    let a = &s.account("sample").unwrap().config;
    assert_eq!(a.signatures.len(), 1, "moved from the one signature it had");
    assert_eq!((a.signature_new.as_deref(), a.signature_reply.as_deref()), (Some("Signature"), Some("Signature")));
    s.run("account.setSignature", &json!({"account": "sample", "name": "Short", "signature": "Riley"})).unwrap();
    assert!(s.run("account.signatureDefaults", &json!({"account": "sample", "reply": "nope"})).is_err(), "no such one");
    s.run("account.signatureDefaults", &json!({"account": "sample", "reply": "short"})).unwrap();
    let list = s.run("account.signatures", &json!({"account": "sample"})).unwrap();
    assert_eq!(list["reply"], "Short", "by its own name: {list}");
    assert_eq!(list["signatures"].as_array().map(Vec::len), Some(2));

    s.run("mail.new", &json!({})).unwrap();
    assert!(body(&s).ends_with("-- \nRiley Moreno · Studio Lantern"), "{:?}", body(&s));
    assert_eq!(s.compose.as_ref().unwrap().signature_name, "Signature");
    assert!(!s.compose_written());
    s.run("compose.discard", &json!({})).ok();
    let k = key_of(&s, "Your proof for the festival poster is ready");
    s.run("list.open", &k).unwrap();
    for cmd in ["mail.reply", "mail.forward", "mail.forwardAsAttachment"] {
        s.run(cmd, &json!({})).unwrap();
        let b = body(&s);
        assert!(b.starts_with("\n\n-- \nRiley") && !b.contains("Studio Lantern\n\n"), "{cmd}: {b:?}");
        assert_eq!(s.compose.as_ref().unwrap().signature_name, "Short", "{cmd}");
        s.run("compose.discard", &json!({})).ok();
    }
    s.run("account.signatureDefaults", &json!({"account": "sample", "reply": null})).unwrap();
    s.run("mail.reply", &json!({})).unwrap();
    assert!(!body(&s).contains("-- \n"), "none in replies: {:?}", body(&s));
    // Insert signature still puts one in: the first, there being none for replies.
    s.run("compose.insertSignature", &json!({"at": 0})).unwrap();
    assert!(body(&s).starts_with("-- \nRiley Moreno"), "{:?}", body(&s));
    s.run("compose.discard", &json!({})).ok();

    // Renamed, the defaults follow it; removed, none goes in.
    s.run("account.setSignature", &json!({"account": "sample", "was": "Signature", "name": "Full", "signature": "Riley Moreno"})).unwrap();
    assert_eq!(s.account("sample").unwrap().config.signature_new.as_deref(), Some("Full"));
    s.run("account.removeSignature", &json!({"account": "sample", "name": "Full"})).unwrap();
    assert!(s.run("account.removeSignature", &json!({"account": "sample", "name": "Full"})).is_err());
    s.run("mail.new", &json!({})).unwrap();
    assert_eq!(body(&s), "");
    done(s);
}

/// Picked from the Signature menu, one replaces the other, its pictures and HTML code with it:
/// a picture under the same `cid:` is the new signature's, not the old one's.
#[test]
fn the_signature_menu_switches_signatures_while_writing() {
    let mut s = sample_session("signatures-menu");
    let png = crate::sample::sample_png();
    let mut other = png.clone();
    other.extend_from_slice(b"another picture");
    s.run("account.setSignature", &with_picture("Logo", "Riley", &png)).unwrap();
    s.run("account.setSignature", &with_picture("Other logo", "R.", &other)).unwrap();
    s.run(
        "account.setSignature",
        &json!({"account": "sample", "name": "Card", "signature": "", "code": "<table><tr><td><b>RILEY</b></td></tr></table>"}),
    )
    .unwrap();
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"body": format!("Hello{}", body(&s))})).unwrap();
    // `compose.update` with a body forgets the look: put in again, it's back.
    s.run("compose.insertSignature", &json!({"name": "Logo"})).unwrap();
    let pics = |s: &Session| s.compose.as_ref().unwrap().inline.iter().map(|p| (p.cid.clone(), p.data.clone())).collect::<Vec<_>>();
    assert_eq!(pics(&s).len(), 1);
    assert_eq!(pics(&s)[0].1, png);
    s.run("compose.insertSignature", &json!({"name": "other logo"})).unwrap();
    assert!(body(&s).starts_with("Hello"), "{:?}", body(&s));
    assert_eq!(body(&s).matches("-- \n").count(), 1, "one replaced the other: {:?}", body(&s));
    assert_eq!(pics(&s).len(), 1);
    assert_eq!(pics(&s)[0].1, other, "the new one's picture, not the old one's");
    let c = s.compose.clone().unwrap();
    let (_, html, pictures) = c.bodies_with(&c.signature_code);
    assert!(html.unwrap().contains(&pictures[0].cid));
    assert_eq!(pictures[0].data, other);

    s.run("compose.insertSignature", &json!({"name": "Card"})).unwrap();
    let c = s.compose.clone().unwrap();
    assert!(c.inline.is_empty(), "the old picture goes: {:?}", pics(&s));
    assert_eq!(c.signature_name, "Card");
    let html = c.bodies_with(&c.signature_code).1.unwrap_or_default();
    assert!(html.contains("<table") && html.contains("RILEY"), "the card as written: {html}");
    s.run("compose.insertSignature", &json!({"name": "Signature"})).unwrap();
    let c = s.compose.clone().unwrap();
    assert!(c.signature_code.is_empty() && c.bodies_with(&c.signature_code).1.is_none(), "plain text again");
    assert!(c.body.ends_with("-- \nRiley Moreno · Studio Lantern"));
    assert!(s.run("compose.insertSignature", &json!({"name": "Missing"})).is_err());
    done(s);
}

/// Issue #11: From changed to another account, its signature takes the old one's place; one
/// with none takes it out (and the blank line above it); one changed in the message stays.
#[test]
fn the_signature_follows_from() {
    let mut s = sample_session("signatures-from");
    let png = crate::sample::sample_png();
    s.run("account.setSignature", &with_picture("Signature", "Riley", &png)).unwrap();
    let desk = second_account(&mut s);
    s.run("account.setSignature", &json!({"account": desk, "name": "Desk", "signature": "Front desk\nStudio Lantern"})).unwrap();
    s.run("mail.new", &json!({"account": "sample"})).unwrap();
    s.compose.as_mut().unwrap().body.insert_str(0, "Hi Ann,");
    assert_eq!(s.compose.as_ref().unwrap().inline.len(), 1);
    s.run("compose.update", &json!({"account": desk})).unwrap();
    assert_eq!(body(&s), "Hi Ann,\n\n-- \nFront desk\nStudio Lantern");
    assert!(s.compose.as_ref().unwrap().inline.is_empty(), "the old one's picture goes");
    s.run("compose.update", &json!({"account": "sample"})).unwrap();
    assert!(body(&s).starts_with("Hi Ann,\n\n-- \nRiley"), "{:?}", body(&s));
    assert_eq!(s.compose.as_ref().unwrap().inline.len(), 1, "and its picture comes");
    // An account with none: out, with the blank line; back, in again where it was.
    s.run("account.removeSignature", &json!({"account": desk, "name": "Desk"})).unwrap();
    s.run("compose.update", &json!({"account": desk})).unwrap();
    assert_eq!(body(&s), "Hi Ann,");
    s.run("compose.update", &json!({"account": "sample"})).unwrap();
    assert!(body(&s).starts_with("Hi Ann,\n\n-- \nRiley"), "{:?}", body(&s));
    // Changed in the message: left as it is.
    s.run("account.setSignature", &json!({"account": desk, "name": "Desk", "signature": "Front desk"})).unwrap();
    let edited = body(&s).replace("Riley", "Riley (on leave)");
    s.compose.as_mut().unwrap().body = edited.clone();
    s.run("compose.update", &json!({"account": desk})).unwrap();
    assert_eq!(body(&s), edited);
    done(s);
}

/// In a reply, the new account's signature for replies goes above the quote, never into it,
/// even when the quote holds the same signature (an answer to one's own message).
#[test]
fn a_reply_takes_the_new_accounts_reply_signature_above_the_quote() {
    let mut s = sample_session("signatures-from-reply");
    let desk = second_account(&mut s);
    s.run("account.setSignature", &json!({"account": desk, "name": "Desk", "signature": "Front desk"})).unwrap();
    s.run("account.setSignature", &json!({"account": desk, "name": "Desk short", "signature": "FD"})).unwrap();
    s.run("account.signatureDefaults", &json!({"account": desk, "reply": "Desk short"})).unwrap();
    // A message of one's own, with the signature in it, answered.
    s.run("mail.new", &json!({"account": "sample"})).unwrap();
    s.run("compose.update", &json!({"to": "ann@x.example", "subject": "Own", "body": format!("Note{}", body(&s))})).unwrap();
    s.run("compose.send", &json!({})).unwrap();
    s.run("folder.open", &json!({"account": "sample", "folder": "Sent"})).unwrap();
    s.run("list.open", &key_of(&s, "Own")).unwrap();
    s.run("mail.reply", &json!({})).unwrap();
    let quote_sig = |b: &str| b.matches("Riley Moreno · Studio Lantern").count();
    assert_eq!(quote_sig(&body(&s)), 2, "its own, and the quote's: {:?}", body(&s));
    s.run("compose.update", &json!({"account": desk})).unwrap();
    let b = body(&s);
    assert!(b.starts_with("\n\n-- \nFD\n\n"), "{b:?}");
    assert_eq!(quote_sig(&b), 1, "the quote keeps its signature: {b:?}");
    rule_over_quote(&s);
    // To an account with no signature for replies and back: in again above the quote.
    s.run("account.signatureDefaults", &json!({"account": "sample", "reply": null})).unwrap();
    s.run("compose.update", &json!({"account": "sample"})).unwrap();
    assert!(body(&s).starts_with("\n\n\nFrom:"), "none, the quote where it was: {:?}", body(&s));
    assert!(!body(&s).contains("-- \nFD"));
    s.run("compose.update", &json!({"account": desk})).unwrap();
    assert!(body(&s).starts_with("\n\n-- \nFD\n\n"), "{:?}", body(&s));
    rule_over_quote(&s);
    // Its own taken out by hand, then From changed: the quote's, the same text, isn't touched.
    s.run("account.signatureDefaults", &json!({"account": "sample", "reply": "Signature"})).unwrap();
    s.run("compose.update", &json!({"account": "sample"})).unwrap();
    assert_eq!(quote_sig(&body(&s)), 2, "{:?}", body(&s));
    rule_over_quote(&s);
    let c = s.compose.as_mut().unwrap();
    let own = c.signature_at().unwrap();
    let old = c.body.clone();
    c.body.replace_range(own..own + c.signature.len(), "");
    c.styles.edit(&old, &c.body, &mime::rich::Style::default());
    let before = c.body.clone();
    s.run("compose.update", &json!({"account": desk})).unwrap();
    assert_eq!(body(&s), before, "left as the writer left it");
    assert_eq!(quote_sig(&body(&s)), 1);
    done(s);
}

/// The quote's line stays over it, wherever signatures went in and out above it.
fn rule_over_quote(s: &Session) {
    let c = s.compose.as_ref().unwrap();
    let rules: Vec<usize> = c.styles.paras.iter().enumerate().filter(|(_, p)| p.rule).map(|(i, _)| i).collect();
    let [rule] = rules.as_slice() else { panic!("one rule: {rules:?}") };
    assert_eq!(c.body.split('\n').nth(*rule), Some(""), "{:?}", c.body);
    assert!(c.body.split('\n').nth(rule + 1).is_some_and(|l| l.starts_with("From:")), "{:?}", c.body);
}

/// A reply whose blank lines above the quote were taken out: the signature goes in above it,
/// and the quote keeps its line.
#[test]
fn a_signature_goes_above_a_quote_that_starts_the_text() {
    let mut s = sample_session("signatures-quote-first");
    let desk = second_account(&mut s);
    s.run("account.setSignature", &json!({"account": desk, "name": "Desk", "signature": "FD"})).unwrap();
    s.run("account.signatureDefaults", &json!({"account": "sample", "reply": null})).unwrap();
    s.run("list.open", &key_of(&s, "Your proof for the festival poster is ready")).unwrap();
    s.run("mail.reply", &json!({})).unwrap();
    let c = s.compose.as_mut().unwrap();
    assert!(c.body.starts_with("\n\n"), "{:?}", c.body);
    c.body.replace_range(..2, "");
    c.styles.chars.drain(..2);
    c.styles.paras.drain(..2);
    assert_eq!(c.styles.paras.first().map(|p| p.rule), Some(true), "the quote starts it");
    s.run("compose.update", &json!({"account": desk})).unwrap();
    assert!(body(&s).starts_with("-- \nFD\n\n\nFrom:"), "{:?}", body(&s));
    rule_over_quote(&s);
    done(s);
}

/// What can't be: names on two lines, too long, taken already; junk for the defaults; an
/// empty signature removes the one named.
#[test]
fn signature_names_and_defaults_refuse_junk() {
    let mut s = sample_session("signatures-junk");
    for bad in [json!(""), json!("a\nb"), json!("x".repeat(65)), json!(" \t ")] {
        assert!(s.run("account.setSignature", &json!({"account": "sample", "name": bad, "signature": "x"})).is_err(), "{bad}");
    }
    s.run("account.setSignature", &json!({"account": "sample", "name": "Two", "signature": "2"})).unwrap();
    assert!(s.run("account.setSignature", &json!({"account": "sample", "was": "Two", "name": "signature", "signature": "2"})).is_err(), "taken");
    for bad in [json!(3), json!(["x"]), json!({"a": 1})] {
        assert!(s.run("account.signatureDefaults", &json!({"account": "sample", "new": bad})).is_err());
    }
    assert!(s.run("account.signatures", &json!({"account": "nobody"})).is_err());
    s.run("account.setSignature", &json!({"account": "sample", "name": "Two", "signature": "  ", "html": "<p> </p>"})).unwrap();
    assert!(s.account("sample").unwrap().config.signature_named("Two").is_none(), "emptied: removed");
    // A huge signature is cut, not refused.
    s.run("account.setSignature", &json!({"account": "sample", "name": "Big", "signature": "é".repeat(50_000)})).unwrap();
    assert_eq!(s.signature_of("sample", "big").unwrap().text.chars().count(), 10_000);
    // No message being written: Insert signature can't.
    assert!(s.run("compose.insertSignature", &json!({"name": "Big"})).is_err());
    done(s);
}
