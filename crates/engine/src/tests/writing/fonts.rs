//! Engine tests: the font new messages are written in (issue #166), as Outlook's "New mail
//! messages" font: kept with the preferences, and sent as the message's own typeface.

use super::*;

#[test]
fn the_font_for_new_messages_is_kept_and_sent() {
    let mut s = sample_session("compose-font");
    assert_eq!(s.prefs.compose_font, None, "MailCraft's own unless chosen");
    assert_eq!(s.run("options.composeFont", &json!({"font": " Georgia "})).unwrap()["font"], "Georgia");
    // Not a name a style can carry: refused, the font kept.
    for bad in [json!({"font": ""}), json!({"font": "'\";<>"}), json!({"font": 7})] {
        assert!(s.run("options.composeFont", &bad).is_err(), "{bad}");
    }
    assert_eq!(s.prefs.compose_font.as_deref(), Some("Georgia"));

    // A new message is written in it: sent as HTML in it, though nothing in it is formatted.
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "ann@x.example", "subject": "Plan", "body": "Hello"})).unwrap();
    let c = s.compose.clone().unwrap();
    assert_eq!(c.font.as_deref(), Some("Georgia"));
    let (text, html, _) = c.bodies();
    assert!(text.contains("Hello"));
    assert!(html.as_deref().is_some_and(|h| h.contains("font-family: 'Georgia', Aptos")), "{html:?}");
    // Plain text stays plain text.
    let mut plain = c.clone();
    plain.plain = true;
    assert_eq!(plain.bodies().1, None);
    s.run("compose.discard", &json!({})).unwrap();

    // A reply too.
    let key = s.list.first().map(|m| m.key.clone()).unwrap();
    s.run("list.open", &json!({"key": key.to_id()})).unwrap();
    s.run("mail.reply", &json!({})).unwrap();
    let r = s.compose.clone().unwrap();
    assert_eq!(r.font.as_deref(), Some("Georgia"));
    assert!(r.bodies().1.is_some_and(|h| h.contains("'Georgia'")));
    s.run("compose.discard", &json!({})).unwrap();

    // Back to MailCraft's own: a new message with nothing formatted goes as text again.
    assert_eq!(s.run("options.composeFont", &json!({"font": null})).unwrap()["font"], json!(null));
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "ann@x.example", "subject": "Plan", "body": "Hello"})).unwrap();
    let c = s.compose.clone().unwrap();
    assert_eq!(c.font, None);
    assert_eq!(c.bodies().1, None);

    // Kept with the preferences; read back from disk only as a name a style can carry.
    s.run("options.composeFont", &json!({"font": "Segoe UI"})).unwrap();
    let saved = serde_json::to_value(&s.prefs).unwrap();
    assert_eq!(saved["composeFont"], "Segoe UI");
    let mut read = saved.clone();
    read["composeFont"] = json!("x\"; color: red");
    let p: crate::Prefs = serde_json::from_value(read).unwrap();
    assert_eq!(p.compose_font.as_deref(), Some("x color red"));
    let mut none = saved;
    none["composeFont"] = json!("'\";");
    assert_eq!(serde_json::from_value::<crate::Prefs>(none).unwrap().compose_font, None);
}

/// The size new messages are written in (issue #267), with the font: kept with the
/// preferences, and sent as the message's own size.
#[test]
fn the_size_for_new_messages_is_kept_and_sent() {
    let mut s = sample_session("compose-size");
    assert_eq!(s.prefs.compose_size, None, "MailCraft's own unless chosen");
    assert_eq!(s.run("options.composeSize", &json!({"size": 14})).unwrap()["size"], 14);
    // Not one the size box offers: refused, the size kept.
    for bad in [json!({"size": 13}), json!({"size": 0}), json!({"size": 300}), json!({"size": -1}), json!({"size": "14"}), json!({"size": 14.5})] {
        assert!(s.run("options.composeSize", &bad).is_err(), "{bad}");
    }
    assert_eq!(s.prefs.compose_size, Some(14));

    // A new message is written in it: sent as HTML in it, though nothing in it is formatted.
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "ann@x.example", "subject": "Plan", "body": "Hello"})).unwrap();
    let c = s.compose.clone().unwrap();
    assert_eq!((c.size, c.base_size()), (Some(14), 14));
    let html = c.bodies().1.unwrap_or_default();
    assert!(html.contains("font-size: 14pt\">"), "{html}");
    let mut plain = c.clone();
    plain.plain = true;
    assert_eq!(plain.bodies().1, None, "plain text stays plain");
    s.run("compose.discard", &json!({})).unwrap();

    // A reply too.
    let key = s.list.first().map(|m| m.key.clone()).unwrap();
    s.run("list.open", &json!({"key": key.to_id()})).unwrap();
    s.run("mail.reply", &json!({})).unwrap();
    let r = s.compose.clone().unwrap();
    assert_eq!(r.size, Some(14));
    assert!(r.bodies().1.is_some_and(|h| h.contains("font-size: 14pt")));
    s.run("compose.discard", &json!({})).unwrap();

    // MailCraft's own size, chosen, is no size of its own: a message with nothing formatted
    // goes as text again.
    assert_eq!(s.run("options.composeSize", &json!({"size": 11})).unwrap()["size"], json!(null));
    s.run("options.composeSize", &json!({"size": 18})).unwrap();
    assert_eq!(s.run("options.composeSize", &json!({"size": null})).unwrap()["size"], json!(null));
    s.run("mail.new", &json!({})).unwrap();
    s.run("compose.update", &json!({"to": "ann@x.example", "subject": "Plan", "body": "Hello"})).unwrap();
    let c = s.compose.clone().unwrap();
    assert_eq!((c.size, c.bodies().1), (None, None));

    // Kept with the preferences; read back from disk only as a size the box offers.
    s.run("options.composeSize", &json!({"size": 9})).unwrap();
    let saved = serde_json::to_value(&s.prefs).unwrap();
    assert_eq!(saved["composeSize"], 9);
    for (on_disk, read) in [(json!(9), Some(9)), (json!(13), None), (json!(999), None), (json!("big"), None), (json!(-4), None)] {
        let mut p = saved.clone();
        p["composeSize"] = on_disk.clone();
        assert_eq!(serde_json::from_value::<crate::Prefs>(p).unwrap().compose_size, read, "{on_disk}");
    }
}
