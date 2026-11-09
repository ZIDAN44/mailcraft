//! Issue #57: what's filed in a POP account's folders shows at once. Those folders are only on
//! this computer, so nothing reads them again by itself.

use std::time::{Duration, Instant};

use mailcraft_model::job::Secret;
use mailcraft_model::{AccountConfig, Flags, Security, ServerConfig, SyncStatus};

use super::*;

const MAIL: &[u8] = b"From: me@pop.example\r\nTo: ana@pop.example\r\nSubject: Hi\r\nMessage-ID: <s@x>\r\n\r\nhi\r\n";

fn until(pool: &Pool, what: &str, f: impl Fn(&Event) -> bool) {
    let end = Instant::now() + Duration::from_secs(20);
    let mut seen = Vec::new();
    while Instant::now() < end {
        if let Some(e) = pool.wait(Duration::from_millis(100)) {
            if f(&e) {
                return;
            }
            seen.push(e);
        }
    }
    panic!("timed out waiting for {what}; saw {seen:?}");
}

fn changed(e: &Event, name: &str) -> bool {
    matches!(e, Event::FolderChanged { folder, .. } if folder == name)
}

/// A folder that has mail already is the case that went wrong: the first message in an empty one
/// always showed.
#[test]
fn a_copy_filed_in_a_folder_with_mail_shows_at_once() {
    let server = crate::pop::fake::serve(Vec::new());
    let (smtp_port, smtp) = crate::smtp::tests::serve_once();
    let store = crate::fake::store("pop-filed");
    crate::fakes::fake_dav::looked_for_today(&store, "p1");
    for folder in ["Sent Items", "Drafts"] {
        crate::pop::file_locally(&store, "p1", folder, MAIL, Flags { seen: true, ..Default::default() }, 1).unwrap();
    }
    let mut pool = Pool::new(store.clone(), std::sync::Arc::new(MemoryCredentials::default()));
    let account = AccountConfig {
        id: "p1".into(),
        email: "me@pop.example".into(),
        username: "me@pop.example".into(),
        pop: Some(ServerConfig { host: "127.0.0.1".into(), port: server.port, security: Security::None }),
        smtp: ServerConfig { host: "127.0.0.1".into(), port: smtp_port, security: Security::None },
        ..Default::default()
    };
    pool.submit(Job::Start { account, password: Some(Secret("pw".into())) });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));

    let recipients = vec!["ana@pop.example".to_string()];
    pool.submit(Job::Send {
        account: "p1".into(),
        id: 1,
        raw: MAIL.to_vec(),
        from: "me@pop.example".into(),
        recipients,
        sent_folder: Some("Sent Items".into()),
    });
    until(&pool, "Sent Items said to have changed", |e| changed(e, "Sent Items"));
    assert!(smtp.join().unwrap().contains("RCPT TO:<ana@pop.example>"));
    assert_eq!(store.load_folder("p1", "Sent Items").unwrap().messages.len(), 2, "on disk before it's said");

    pool.submit(Job::SaveDraft { account: "p1".into(), raw: MAIL.to_vec(), folder: "Drafts".into(), replaces: None });
    until(&pool, "Drafts said to have changed", |e| changed(e, "Drafts"));
    assert_eq!(store.load_folder("p1", "Drafts").unwrap().messages.len(), 2, "on disk before it's said");
    pool.submit(Job::Remove { account: "p1".into() });
}
