//! UI tests: the message form in each of its states (new, suggesting, a reply with everything
//! shown, attachments, narrow, dark, an alert over it, two being written, signed, in a window
//! of its own with a delay, from an account that can't send), each drawn with what it shows.

use super::*;

/// The texts a frame of this size draws.
fn drawn(ctx: &egui::Context, a: &mut MailApp, size: egui::Vec2) -> Vec<String> {
    drawn_strings(ctx, a, size)
}

/// Each state of the form: its name, and what it draws (settled over a few frames).
fn states() -> Vec<(&'static str, Vec<String>)> {
    let (mut a, _) = app("form-states");
    a.session.prefs.new_mail_window = false;
    let ctx = egui::Context::default();
    let wide = egui::vec2(1280.0, 800.0);
    let mut out = Vec::new();
    let mut settle = |a: &mut MailApp, name: &'static str, size: egui::Vec2| {
        for _ in 0..3 {
            drawn(&ctx, a, size);
        }
        out.push((name, drawn(&ctx, a, size)));
    };
    // An account with a server too: From offers both, and Delay delivery shows.
    a.run("account.add", json!({"email": "zoe@example.org", "password": "pw"}));
    a.session.take_jobs();
    let zoe = a.session.accounts.iter().find(|x| x.config.email == "zoe@example.org").map(|x| x.config.id.clone()).unwrap();
    frame(&ctx, &mut a, vec![]);
    // A new message: To has the focus.
    a.run("mail.new", json!({}));
    settle(&mut a, "new", wide);
    // Typing in To: the people suggested.
    frame(&ctx, &mut a, vec![Event::Text("zo".into())]);
    settle(&mut a, "suggesting", wide);
    a.run("compose.discard", json!({}));
    // A reply, with Bcc, replies to, high importance, a read receipt, plain text, and an error.
    let k = a.session.list.first().map(|m| m.key.clone()).unwrap();
    a.run("list.open", json!({"key": k.to_id()}));
    a.run("mail.reply", json!({}));
    if let Some(c) = a.session.compose.as_mut() {
        c.show_bcc = true;
        c.show_reply_to = true;
        c.reply_to = "desk@example.org".into();
        c.importance = 1;
        c.read_receipt = true;
        c.plain = true;
        c.cc = "ann@".into();
        c.error = Some("the server said no".into());
    }
    settle(&mut a, "reply", wide);
    // Attachments, one a photo that says where it was taken, and a delay.
    a.run("compose.attach", json!({"name": "notes.txt", "data": [1, 2, 3]}));
    a.run("compose.attach", json!({"name": "report.pdf", "data": [4, 5]}));
    a.run("compose.update", json!({"sendAfter": NOW + 3600}));
    settle(&mut a, "attachments", wide);
    // Narrow: the option rows one under the other.
    settle(&mut a, "narrow", egui::vec2(640.0, 800.0));
    // Dark.
    a.ui.dark = Some(true);
    settle(&mut a, "dark", wide);
    a.ui.dark = Some(false);
    // An alert over it: to no one, then in the engine's words.
    a.ui.compose_alert = Some(crate::reading::ComposeAlert::NoRecipient);
    settle(&mut a, "alert", wide);
    a.ui.compose_alert = Some(crate::reading::ComposeAlert::Other("the subject is empty".into()));
    settle(&mut a, "alert-other", wide);
    a.ui.compose_alert = None;
    a.run("compose.discard", json!({}));
    // Two being written: the other a click away above it (no windows of their own here).
    a.run("mail.new", json!({}));
    a.run("compose.update", json!({"subject": "First thoughts"}));
    a.run("mail.new", json!({}));
    settle(&mut a, "two", wide);
    a.run("compose.discard", json!({}));
    a.run("compose.discard", json!({}));
    // With an OpenPGP key of its own: Sign and Encrypt, and who can read it encrypted.
    let secret = std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../pgp/testdata/riley-secret.asc")).unwrap();
    let k = mailcraft_engine::pgp::read_secret_key(&secret, "").unwrap();
    let event = MailEvent::PgpKeys { account: "sample".into(), keys: vec![k.public], imported: Some(k.info.fingerprint), error: None };
    a.session.apply(event);
    a.run("mail.new", json!({}));
    a.run("compose.update", json!({"to": "ana@studio.example", "subject": "Signed"}));
    settle(&mut a, "signed", wide);
    a.run("compose.discard", json!({}));
    // In a window of its own.
    a.session.prefs.new_mail_window = true;
    a.run("mail.new", json!({"account": zoe}));
    a.run("compose.update", json!({"to": "ann@example.org", "subject": "Window", "sendAfter": NOW + 3600}));
    settle(&mut a, "window", wide);
    a.run("compose.discard", json!({}));
    a.session.prefs.new_mail_window = false;
    // From a data file, with no account that can send: the bar, and the alert offering one.
    let file = a.session.run("account.importPst", &json!({"path": r"C:\Mail\Archive.pst"})).unwrap()["account"].as_str().unwrap().to_string();
    a.session.accounts.retain(|x| x.config.id == file);
    a.run("mail.new", json!({"account": file}));
    a.run("compose.update", json!({"to": "ann@example.org", "subject": "Plans"}));
    settle(&mut a, "blocked", wide);
    a.ui.compose_alert = Some(crate::reading::ComposeAlert::Blocked(mailcraft_engine::SendBlock::DataFile));
    settle(&mut a, "blocked-alert", wide);
    out
}

#[test]
fn the_message_form_draws_in_each_state() {
    let all = states();
    let get = |name: &str| all.iter().find(|(n, ..)| *n == name).map(|(_, texts)| texts.clone()).unwrap_or_default();
    let has = |name: &str, what: &str| get(name).iter().any(|t| t == what);
    for name in ["new", "reply", "attachments", "dark", "window", "blocked"] {
        for what in ["Send", "From", "To", "Cc", "Subject", "Save draft", "Discard", "Plain text", "Read receipt", "Replies to"] {
            assert!(has(name, what), "{name}: {what}: {:?}", get(name));
        }
    }
    assert!(has("suggesting", "Zoe Park") && has("suggesting", "zoe@studio.example"), "{:?}", get("suggesting"));
    assert!(has("reply", "Bcc") && has("reply", "Hide Bcc") && has("reply", "desk@example.org"), "{:?}", get("reply"));
    assert!(has("reply", "Not sent: the server said no") && has("reply", "Not an email address: ann@"), "{:?}", get("reply"));
    assert!(get("attachments").iter().any(|t| t.starts_with("notes.txt (")), "{:?}", get("attachments"));
    assert!(get("window").iter().any(|t| t.starts_with("Sends ")), "{:?}", get("window"));
    for what in ["Send", "From", "To", "Save draft", "Discard", "notes.txt (3 B)"] {
        assert!(has("narrow", what), "narrow: {what}: {:?}", get("narrow"));
    }
    assert!(has("alert", "Add at least one name in To, Cc or Bcc before sending.") && has("alert", "OK"));
    assert!(has("alert-other", "The subject is empty.") && !has("alert-other", "Add an account"));
    assert!(has("two", "Also being written:") && has("two", "First thoughts"), "{:?}", get("two"));
    assert!(has("window", "Window") || has("window", "ann@example.org"), "{:?}", get("window"));
    assert!(has("signed", "Sign") && has("signed", "Encrypt"), "{:?}", get("signed"));
    let why = crate::reading::send_block_text(mailcraft_engine::SendBlock::DataFile);
    assert!(has("blocked", &why) && has("blocked", "Add an account"), "{:?}", get("blocked"));
    assert_eq!(get("blocked-alert").iter().filter(|t| *t == "Add an account").count(), 2, "in the bar and in the alert");
}
