//! Issue #38: message files dropped on a folder go into it.

use std::time::{Duration, Instant};

use mailcraft_model::job::Secret;
use mailcraft_model::{AccountConfig, Security, ServerConfig, SyncStatus};

use super::*;
use crate::fake::{State, serve_tcp};

fn until(pool: &Pool, what: &str, mut f: impl FnMut(&Event) -> bool) -> Vec<Event> {
    let end = Instant::now() + Duration::from_secs(20);
    let mut seen = Vec::new();
    while Instant::now() < end {
        if let Some(e) = pool.wait(Duration::from_millis(100)) {
            let hit = f(&e);
            seen.push(e);
            if hit {
                return seen;
            }
        }
    }
    panic!("timed out waiting for {what}; saw {seen:?}");
}

/// The files to drop: an Outlook `.msg`, an `.eml`, and a picture that isn't a message.
fn files(name: &str) -> (std::path::PathBuf, Vec<String>) {
    let dir = std::env::temp_dir().join(format!("mailcraft-filing-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let msg = dir.join("Quarterly figures.msg");
    std::fs::write(&msg, crate::data_files::msg::tests::sample()).unwrap();
    let eml = dir.join("hello.eml");
    std::fs::write(
        &eml,
        b"From: Zoe <zoe@studio.test>\r\nTo: me@studio.test\r\nSubject: Saved hello\r\nMessage-ID: <hello@studio.test>\r\n\r\nHi.\r\n",
    )
    .unwrap();
    let png = dir.join("photo.png");
    std::fs::write(&png, b"\x89PNG\r\n\x1a\nnot a message").unwrap();
    let paths = [msg, eml, png].iter().map(|p| p.to_string_lossy().into_owned()).collect();
    (dir, paths)
}

/// On an IMAP account each message goes to the server's folder as read mail, the folder is read
/// again, and what isn't a message is said.
#[test]
fn message_files_go_into_a_server_folder() {
    let st = State::new();
    let store = crate::fake::store("filing-imap");
    crate::fakes::fake_dav::looked_for_today(&store, "a1");
    let mut pool = Pool::new(store.clone(), std::sync::Arc::new(MemoryCredentials::default()));
    pool.opts.prefetch = 0;
    let account = AccountConfig {
        id: "a1".into(),
        email: "me@studio.test".into(),
        username: "me@studio.test".into(),
        imap: ServerConfig { host: "127.0.0.1".into(), port: serve_tcp(&st), security: Security::None },
        smtp: ServerConfig { host: "127.0.0.1".into(), port: 1, security: Security::None },
        ..Default::default()
    };
    pool.submit(Job::Start { account, password: Some(Secret("pw".into())) });
    until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    let (dir, paths) = files("imap");
    pool.submit(Job::FileMessages { account: "a1".into(), folder: "Archive".into(), paths });
    let seen = until(&pool, "filed", |e| matches!(e, Event::MessagesFiled { .. }));
    let Some(Event::MessagesFiled { added, failed, folder, .. }) = seen.last() else { panic!() };
    assert_eq!((*added, folder.as_str()), (2, "Archive"));
    assert_eq!(failed.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), ["photo.png"]);
    {
        let s = st.lock().unwrap();
        let archive = s.boxes.iter().find(|b| b.name == "Archive").unwrap();
        assert_eq!(archive.msgs.len(), 2);
        assert!(archive.msgs.iter().all(|m| m.flags.iter().any(|f| f == "\\Seen")), "as read mail");
        let subjects: Vec<String> = archive.msgs.iter().filter_map(|m| mailcraft_mime::parse(&m.raw)).map(|p| p.subject).collect();
        assert!(subjects.contains(&"Quarterly figures — räder".to_string()) && subjects.contains(&"Saved hello".to_string()), "{subjects:?}");
    }
    let kept = store.load_folder("a1", "Archive").unwrap();
    assert_eq!(kept.messages.len(), 2, "the folder read again");
    let _ = std::fs::remove_dir_all(dir);
}

/// On an account on this computer (no server) they're filed in its own folder, with the other
/// changes to its mail; on a server account that isn't connected, none go in.
#[test]
fn message_files_go_into_a_folder_here() {
    let store = crate::fake::store("filing-local");
    let mut pool = Pool::new(store.clone(), std::sync::Arc::new(MemoryCredentials::default()));
    let (dir, paths) = files("local");
    pool.submit(Job::FileMessages { account: "local".into(), folder: "Inbox".into(), paths: paths.clone() });
    let seen = until(&pool, "filed", |e| matches!(e, Event::MessagesFiled { .. }));
    assert!(seen.iter().any(|e| matches!(e, Event::FolderChanged { account, folder } if account == "local" && folder == "Inbox")));
    let Some(Event::MessagesFiled { added, failed, .. }) = seen.last() else { panic!() };
    assert_eq!((*added, failed.len()), (2, 1));
    let kept = store.load_folder("local", "Inbox").unwrap();
    assert_eq!(kept.messages.len(), 2);
    assert!(kept.messages.iter().all(|m| m.flags.seen));
    assert!(kept.messages.iter().any(|m| m.subject == "Quarterly figures — räder"));
    assert!(kept.messages.iter().all(|m| store.has_body(&m.key)));
    // A .msg read to be shown.
    pool.submit(Job::ReadMessageFile { path: paths[0].clone() });
    let seen = until(&pool, "read", |e| matches!(e, Event::MessageFileRead { .. }));
    let Some(Event::MessageFileRead { raw: Ok(raw), .. }) = seen.last() else { panic!("{seen:?}") };
    assert_eq!(mailcraft_mime::parse(raw).unwrap().subject, "Quarterly figures — räder");
    pool.submit(Job::ReadMessageFile { path: paths[2].clone() });
    let seen = until(&pool, "read", |e| matches!(e, Event::MessageFileRead { .. }));
    assert!(matches!(seen.last(), Some(Event::MessageFileRead { raw: Err(_), .. })));
    let _ = std::fs::remove_dir_all(dir);
}
