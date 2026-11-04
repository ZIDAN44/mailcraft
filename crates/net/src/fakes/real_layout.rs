//! Issue #41 through a real server, run by hand:
//!
//! ```text
//! MAILCRAFT_REAL_LAYOUT="127.0.0.1:3025 127.0.0.1:3143 ben@studio.test benpw" \
//!   cargo test -p mailcraft-net real_newsletter_layout -- --ignored --nocapture
//! ```
//!
//! A newsletter of blocks stacked one on another, as the tester's, sent over SMTP and read back
//! over IMAP the way the account's sync downloads it: its layout keeps each table's own room and
//! each block's padding and colour, and its text and preview read without rules or run-on words.

use mailcraft_model::{Security, ServerConfig};

fn at(addr: &str) -> ServerConfig {
    let (host, port) = addr.rsplit_once(':').unwrap_or((addr, "0"));
    ServerConfig { host: host.into(), port: port.parse().unwrap_or(0), security: Security::None }
}

#[test]
#[ignore]
fn real_newsletter_layout() {
    use mailcraft_mime::layout::Block;
    let Ok(v) = std::env::var("MAILCRAFT_REAL_LAYOUT") else {
        eprintln!("MAILCRAFT_REAL_LAYOUT isn't set: skipped");
        return;
    };
    let p: Vec<&str> = v.split_whitespace().collect();
    let [smtp, imap, user, password] = p.as_slice() else { panic!("MAILCRAFT_REAL_LAYOUT: smtp imap user password") };
    let id = std::process::id();
    let block = |text: &str| {
        format!(
            "<table width=\"600\" cellpadding=\"0\" cellspacing=\"0\" bgcolor=\"#fff8cc\"><tr><td style=\"padding:24px 20px\">{text}</td></tr></table>"
        )
    };
    let html = format!(
        "<table width=\"100%\" cellpadding=\"0\" cellspacing=\"0\" bgcolor=\"#f2f2f2\"><tr><td align=\"center\">{}{}{}</td></tr></table>",
        block("Banner"),
        block("First block, line one."),
        block("Centred block")
    );
    let raw = format!(
        "From: Studio News <news@studio.test>\r\nTo: {user}\r\nSubject: Stacked {id}\r\nMessage-ID: <stacked{id}@studio.test>\r\n\
         MIME-Version: 1.0\r\nContent-Type: text/html; charset=utf-8\r\n\r\n{html}\r\n"
    );
    crate::smtp::send(&at(smtp), user, password, "news@studio.test", &[user.to_string()], raw.as_bytes()).unwrap();
    let mut c = crate::imap::Client::connect(&at(imap)).unwrap();
    c.login(user, password).unwrap();
    c.select("INBOX", true).unwrap();
    let found = c.uid_search(&format!("SUBJECT \"Stacked {id}\"")).unwrap();
    let uid = *found.last().expect("delivered");
    let got = c.uid_fetch(&uid.to_string(), "UID BODY.PEEK[]", "").unwrap().into_iter().find_map(|f| f.body).unwrap();
    let p = mailcraft_mime::parse(&got).unwrap();
    let Some(Block::Table(outer)) = p.layout.first() else { panic!("no layout: {:?}", p.layout) };
    let blocks: Vec<_> = outer.rows[0][0].blocks.iter().filter_map(|b| if let Block::Table(t) = b { Some(t) } else { None }).collect();
    eprintln!("{} blocks; room {:?}", blocks.len(), blocks.iter().map(|t| (t.spacing, t.padding, t.rows[0][0].padding)).collect::<Vec<_>>());
    assert_eq!(blocks.len(), 3, "{:?}", outer.rows[0][0].blocks);
    assert!(
        blocks.iter().all(|t| t.spacing == Some(0) && t.rows[0][0].padding == Some([24, 20, 24, 20]) && t.background == Some([0xff, 0xf8, 0xcc]))
    );
    assert_eq!(outer.rows[0][0].blocks.len(), 3, "nothing between the blocks");
    assert!(!p.text.chars().any(|c| ('\u{2500}'..='\u{257F}').contains(&c)), "{}", p.text);
    let key = mailcraft_model::MessageKey { account: "r".into(), folder: "INBOX".into(), validity: 1, uid };
    let s = mailcraft_mime::summary_from_full(&got, key, mailcraft_model::Flags::default(), 0, 0);
    eprintln!("preview: {}", s.preview);
    assert!(s.preview.starts_with("Banner First block, line one. Centred block"), "{}", s.preview);
}
