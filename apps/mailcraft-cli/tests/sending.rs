//! Sending. `mailcraft-cli send` sends only from an account that can: with none, or with only an
//! Outlook data file, it fails and says why (it printed `{"sent": …, "local": true}` for a data
//! file, and nothing had gone). The font and size for new messages (issues #166 and #267), a
//! table pasted in (issue #268), and HTML for a message with nothing formatted when that's chosen
//! (issue #305), are in what a real server receives: sent through GreenMail on
//! this computer (never a real account), run by hand:
//!
//! ```text
//! docker run --rm -d --name mc-greenmail -p 127.0.0.1:3025:3025 -p 127.0.0.1:3143:3143 -p 127.0.0.1:3465:3465 \
//!   -e GREENMAIL_OPTS='-Dgreenmail.setup.test.all -Dgreenmail.hostname=0.0.0.0 -Dgreenmail.auth.disabled' \
//!   greenmail/standalone:2.1.3
//! MAILCRAFT_REAL_GREENMAIL="127.0.0.1:3143 127.0.0.1:3025 127.0.0.1:3465" \
//!   cargo test -p mailcraft-cli --test sending real_ -- --ignored --nocapture
//! ```

#[path = "support/servers.rs"]
mod servers;

use std::process::{Command, Output};
use std::time::{Duration, Instant};

use mailcraft_engine::model::{Event, FailureKind};
use serde_json::json;
use servers::{App, Imap};

fn cli(data: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mailcraft-cli")).arg("--data").arg(data).args(args).env_remove("MAILCRAFT_DATA").output().unwrap()
}

#[test]
fn send_fails_with_no_account_that_can_send() {
    let data = std::env::temp_dir().join(format!("mailcraft-cli-send-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&data);
    std::fs::create_dir_all(&data).unwrap();
    let send = ["send", "--to", "ann@example.org", "--subject", "Plans", "--body", "See you"];
    let out = cli(&data, &send);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("no accounts yet"), "{}", String::from_utf8_lossy(&out.stderr));
    // A data file opened (its import runs in the app; the account is there at once).
    let out = cli(&data, &["run", "account.importPst", r#"{"path": "Archive.pst"}"#]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let out = cli(&data, &send);
    let (stdout, stderr) = (String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(!out.status.success(), "{stdout}");
    assert!(stderr.contains("data file") && !stdout.contains("sent"), "{stdout} / {stderr}");
    let _ = std::fs::remove_dir_all(&data);
}

/// A new message goes in the font and size chosen for new messages: the HTML that arrives says
/// so, with text set to a size of its own in that size.
#[test]
#[ignore = "needs GreenMail in MAILCRAFT_REAL_GREENMAIL"]
fn real_new_messages_arrive_in_the_size_chosen() {
    let Ok(v) = std::env::var("MAILCRAFT_REAL_GREENMAIL") else {
        eprintln!("MAILCRAFT_REAL_GREENMAIL isn't set: skipped");
        return;
    };
    let [imap, smtp, ..] = v.split_whitespace().collect::<Vec<_>>()[..] else { panic!("MAILCRAFT_REAL_GREENMAIL: imap smtp") };
    let pid = std::process::id();
    let (me, to) = (format!("riley.{pid}@studio.example"), format!("kim.{pid}@studio.example"));
    let (mut app, id) = App::at(imap, smtp, &me, &me, "pw", "new-message-size");
    app.s.run("options.composeFont", &json!({"font": "Georgia"})).unwrap();
    app.s.run("options.composeSize", &json!({"size": 14})).unwrap();

    let subject = format!("Price list {pid}");
    app.s.run("mail.new", &json!({"account": id})).unwrap();
    app.s.run("compose.update", &json!({"to": to, "subject": subject, "body": "Hello Kim,\nThe price list is below."})).unwrap();
    // "Hello" in a size of its own: 11 points, written as such in a 14-point message.
    let c = app.s.compose.as_mut().unwrap();
    let body = c.body.clone();
    c.styles.restyle(&body, 0..5, |st| mailcraft_engine::mime::rich::Style { size: Some(11), ..st.clone() });
    app.s.run("compose.send", &json!({})).unwrap();
    app.pump(60, "sent", |_, e| matches!(e, Event::Sent { .. }));

    let mut kim = Imap::connect(imap, &to, "pw");
    let end = Instant::now() + Duration::from_secs(30);
    while !kim.subjects("INBOX").contains(&subject) {
        assert!(Instant::now() < end, "not delivered");
        std::thread::sleep(Duration::from_millis(500));
    }
    let text = kim.newest_text("INBOX").replace("=\r\n", "").replace("=3D", "=");
    assert!(text.contains("font-family: 'Georgia', Aptos") && text.contains("font-size: 14pt"), "{text}");
    assert!(text.contains("<span style=\"font-size:11pt\">Hello</span>"), "{text}");
    eprintln!("arrived in Georgia, 14 points, \"Hello\" in 11");
}

/// Compose messages in HTML (issue #305): a message with nothing formatted arrives as HTML with
/// its text version, as Outlook sends it, so a signature the server adds is added as HTML.
#[test]
#[ignore = "needs GreenMail in MAILCRAFT_REAL_GREENMAIL"]
fn real_a_plain_message_arrives_as_html_when_chosen() {
    let Ok(v) = std::env::var("MAILCRAFT_REAL_GREENMAIL") else {
        eprintln!("MAILCRAFT_REAL_GREENMAIL isn't set: skipped");
        return;
    };
    let [imap, smtp, ..] = v.split_whitespace().collect::<Vec<_>>()[..] else { panic!("MAILCRAFT_REAL_GREENMAIL: imap smtp") };
    let pid = std::process::id();
    let (me, to) = (format!("riley.{pid}@studio.example"), format!("lee.{pid}@studio.example"));
    let (mut app, id) = App::at(imap, smtp, &me, &me, "pw", "compose-html");
    app.s.run("options.composeHtml", &json!({"on": true})).unwrap();

    let subject = format!("Plain words {pid}");
    app.s.run("mail.new", &json!({"account": id})).unwrap();
    app.s.run("compose.update", &json!({"to": to, "subject": subject, "body": "Hello Lee,\nNothing formatted here."})).unwrap();
    app.s.run("compose.send", &json!({})).unwrap();
    app.pump(60, "sent", |_, e| matches!(e, Event::Sent { .. }));

    let mut lee = Imap::connect(imap, &to, "pw");
    let end = Instant::now() + Duration::from_secs(30);
    while !lee.subjects("INBOX").contains(&subject) {
        assert!(Instant::now() < end, "not delivered");
        std::thread::sleep(Duration::from_millis(500));
    }
    let text = lee.newest_text("INBOX").replace("=\r\n", "").replace("=3D", "=");
    // The body (its header, multipart/alternative, isn't in it): the text, then the HTML.
    assert!(text.contains("Content-Type: text/plain") && text.contains("Content-Type: text/html"), "{text}");
    assert!(text.contains("Nothing formatted here."), "{text}");
    eprintln!("arrived as HTML with its text version");
}

/// Sent over implicit TLS (GreenMail's SMTPS, its certificate made by itself): refused and asked
/// about first, then sent once that certificate is trusted, as MailCraft's own SMTP client checks
/// the server's certificate in the handshake.
#[test]
#[ignore = "needs GreenMail in MAILCRAFT_REAL_GREENMAIL"]
fn real_sent_over_tls_once_its_certificate_is_trusted() {
    let Ok(v) = std::env::var("MAILCRAFT_REAL_GREENMAIL") else {
        eprintln!("MAILCRAFT_REAL_GREENMAIL isn't set: skipped");
        return;
    };
    let [imap, _, smtps] = v.split_whitespace().collect::<Vec<_>>()[..] else { panic!("MAILCRAFT_REAL_GREENMAIL: imap smtp smtps") };
    let pid = std::process::id();
    let (me, to) = (format!("riley.{pid}@studio.example"), format!("mo.{pid}@studio.example"));
    let mut app = App::open("smtps", "pw");
    let (sh, sp) = smtps.rsplit_once(':').unwrap();
    let id = app.add_plain(
        imap,
        smtps,
        json!({"email": me, "password": "pw", "smtpHost": sh, "smtpPort": sp.parse::<u16>().unwrap(), "smtpSecurity": "tls"}),
    );
    let send = |app: &mut App, subject: &str| {
        app.s.run("mail.new", &json!({"account": id})).unwrap();
        app.s.run("compose.update", &json!({"to": to, "subject": subject, "body": "Over TLS."})).unwrap();
        app.s.run("compose.send", &json!({})).unwrap();
    };
    send(&mut app, &format!("Asked about {pid}"));
    let mut shown = None;
    app.pump(60, "refused", |_, e| match e {
        Event::Queued { kind: FailureKind::UntrustedCertificate(c), .. } => {
            shown = Some(c.fingerprint.clone());
            true
        }
        Event::Sent { .. } => panic!("sent to a certificate nobody trusted"),
        _ => false,
    });
    let print = shown.expect("its certificate shown");
    app.s.run("account.trustCertificate", &json!({"account": id, "fingerprint": print})).unwrap();
    let subject = format!("Once trusted {pid}");
    send(&mut app, &subject);
    app.pump(60, "sent", |_, e| matches!(e, Event::Sent { .. }));
    let mut mo = Imap::connect(imap, &to, "pw");
    let end = Instant::now() + Duration::from_secs(30);
    while !mo.subjects("INBOX").contains(&subject) {
        assert!(Instant::now() < end, "not delivered");
        std::thread::sleep(Duration::from_millis(500));
    }
    eprintln!("refused, asked about, then sent over TLS once trusted");
}

/// A table pasted into a message (issue #268), cells copied in Excel, arrives as a table: its
/// rows and cells with their lines in the HTML, its text a row a line in the plain text.
#[test]
#[ignore = "needs GreenMail in MAILCRAFT_REAL_GREENMAIL"]
fn real_a_pasted_table_arrives_as_a_table() {
    let Ok(v) = std::env::var("MAILCRAFT_REAL_GREENMAIL") else {
        eprintln!("MAILCRAFT_REAL_GREENMAIL isn't set: skipped");
        return;
    };
    let [imap, smtp, ..] = v.split_whitespace().collect::<Vec<_>>()[..] else { panic!("MAILCRAFT_REAL_GREENMAIL: imap smtp") };
    let pid = std::process::id();
    let (me, to) = (format!("riley.t{pid}@studio.example"), format!("kim.t{pid}@studio.example"));
    let (mut app, id) = App::at(imap, smtp, &me, &me, "pw", "pasted-table");
    let subject = format!("Prices {pid}");
    app.s.run("mail.new", &json!({"account": id})).unwrap();
    app.s.run("compose.update", &json!({"to": to, "subject": subject, "body": "Prices below"})).unwrap();
    let excel = "<html><head><style>.xl65 {border:.5pt solid windowtext; text-align:right}</style></head><body><table>\
        <tr><td class=xl65 style='text-align:left'>Paper</td><td class=xl65>12.50</td></tr>\
        <tr><td class=xl65 style='text-align:left'>Ink</td><td class=xl65>8.00</td></tr></table></body></html>";
    assert_eq!(app.s.run("compose.pasteHtml", &json!({"html": excel})).unwrap()["tables"], 1);
    app.s.run("compose.send", &json!({})).unwrap();
    app.pump(60, "sent", |_, e| matches!(e, Event::Sent { .. }));

    let mut kim = Imap::connect(imap, &to, "pw");
    let end = Instant::now() + Duration::from_secs(30);
    while !kim.subjects("INBOX").contains(&subject) {
        assert!(Instant::now() < end, "not delivered");
        std::thread::sleep(Duration::from_millis(500));
    }
    let text = kim.newest_text("INBOX").replace("=\r\n", "").replace("=3D", "=").replace("=09", "\t");
    assert!(text.contains("Paper\t12.50\r\nInk\t8.00") || text.contains("Paper\t12.50\nInk\t8.00"), "the plain text: {text}");
    assert!(text.contains("<table") && text.contains("border-top:1px solid #000000") && text.contains(" align=\"right\""), "{text}");
    assert!(!text.contains("xl65") && !text.contains("<style"), "nothing of Excel's own: {text}");
    eprintln!("arrived as a table, its text in the plain text");
}
