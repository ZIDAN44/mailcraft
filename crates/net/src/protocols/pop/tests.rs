//! POP3 tests: the client against canned answers and the fake server, and the account's sync.

use super::*;

mod measure;
mod stopped;

fn mail(subject: &str, extra: &str) -> Vec<u8> {
    format!("From: Ana <ana@pop.example>\r\nTo: me@pop.example\r\nSubject: {subject}\r\nMessage-ID: <{subject}@x>\r\n\r\nHello.\r\n{extra}")
        .into_bytes()
}

/// Found in review: a mailbox another session holds, or a server that asks to wait, was
/// taken for a refused password, which was then asked for and the account checked no more.
#[test]
fn a_busy_mailbox_isnt_a_wrong_password() {
    for busy in ["[IN-USE] mailbox locked", "[in-use] Do not hammer", "[LOGIN-DELAY] wait 30 seconds", "[SYS/TEMP] try later"] {
        assert!(matches!(auth(&NetError::No(busy.into())), NetError::Later(m) if m == busy), "{busy}");
    }
    assert!(matches!(auth(&NetError::No("[AUTH] wrong password".into())), NetError::Auth(_)));
    assert!(matches!(auth(&NetError::No(String::new())), NetError::Auth(_)));
}

#[test]
fn new_mail_comes_down_once_and_stays_on_the_server_unless_asked() {
    let server = fake::serve(vec![("u1".into(), mail("One", "")), ("u2".into(), mail("Two", ".a line starting with a dot\r\n"))]);
    let dir = std::env::temp_dir().join(format!("mailcraft-pop-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    let cfg = ServerConfig { host: "127.0.0.1".into(), port: server.port, security: Security::None };
    let acc = AccountConfig { id: "p1".into(), email: "me@pop.example".into(), pop: Some(cfg), ..Default::default() };
    assert!(matches!(open(&acc, "wrong"), Err(NetError::Auth(_))));
    let list = folders(&store, "p1").unwrap();
    assert_eq!(list.iter().filter(|f| f.role == FolderRole::Inbox).count(), 1);
    let mut c = open(&acc, "pw").unwrap();
    let keep = Keep::default();
    let r = fetch_new(&mut c, &store, "p1", "INBOX", &keep, 1_000).unwrap();
    c.quit().unwrap();
    assert_eq!((r.new, r.total, r.unread), (2, 2, 2));
    let st = store.load_folder("p1", "INBOX").unwrap();
    let two = st.messages.iter().find(|m| m.subject == "Two").unwrap();
    let raw = String::from_utf8(store.read_body(&two.key).unwrap()).unwrap();
    assert!(raw.contains("\r\n.a line starting with a dot\r\n"), "dot-stuffing undone: {raw}");
    // Again: nothing new; the server still has both.
    let mut c = open(&acc, "pw").unwrap();
    assert_eq!(fetch_new(&mut c, &store, "p1", "INBOX", &keep, 1_000).unwrap().new, 0);
    c.quit().unwrap();
    assert_eq!(server.drop.lock().unwrap().len(), 2);
    // New mail, and removing from the server.
    server.drop.lock().unwrap().push(("u3".into(), mail("Three", "")));
    let mut c = open(&acc, "pw").unwrap();
    assert_eq!(fetch_new(&mut c, &store, "p1", "INBOX", &Keep { remove_now: true, ..keep }, 1_000).unwrap().new, 1);
    c.quit().unwrap();
    assert!(server.drop.lock().unwrap().is_empty(), "removed after downloading");
    assert_eq!(store.load_folder("p1", "INBOX").unwrap().messages.len(), 3);
    // A copy filed here.
    let k = file_locally(&store, "p1", "Sent Items", &mail("Sent", ""), Flags { seen: true, ..Default::default() }, 1).unwrap();
    assert!(store.has_body(&k));
    let _ = std::fs::remove_dir_all(dir);
}

/// As Outlook's POP settings: a copy stays on the server for so many days, or until it's
/// deleted here for good.
#[test]
fn server_copies_go_after_days_or_once_deleted_for_good() {
    let day = 86_400;
    let server = fake::serve(vec![("u1".into(), mail("One", "")), ("u2".into(), mail("Two", "")), ("u3".into(), mail("Three", ""))]);
    let dir = std::env::temp_dir().join(format!("mailcraft-pop-keep-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    let cfg = ServerConfig { host: "127.0.0.1".into(), port: server.port, security: Security::None };
    let acc = AccountConfig { id: "p1".into(), email: "me@pop.example".into(), pop: Some(cfg), ..Default::default() };
    folders(&store, "p1").unwrap();
    let keep = Keep { remove_now: false, days: Some(14), with_trash: true };
    let sync = |now: i64| {
        let mut c = open(&acc, "pw").unwrap();
        let r = fetch_new(&mut c, &store, "p1", "INBOX", &keep, now).unwrap();
        c.quit().unwrap();
        r
    };
    assert_eq!(sync(0).new, 3);
    let on_server = || server.drop.lock().unwrap().iter().map(|(id, _)| id.clone()).collect::<Vec<_>>();
    // One moved to Deleted Items and deleted there for good: its server copy goes.
    let inbox = store.load_folder("p1", "INBOX").unwrap();
    let one = inbox.messages.iter().find(|m| m.subject == "One").unwrap().key.clone();
    move_locally(&store, "p1", &[one], "Deleted Items").unwrap();
    let moved = store.load_folder("p1", "Deleted Items").unwrap().messages[0].key.clone();
    deleted_for_good(&store, "p1", &[moved]).unwrap();
    assert_eq!(sync(day).new, 0);
    assert_eq!(on_server(), ["u2", "u3"]);
    // Thirteen days on, both stay; fourteen, they go (and aren't downloaded again).
    sync(13 * day);
    assert_eq!(on_server().len(), 2);
    server.drop.lock().unwrap().push(("u4".into(), mail("Four", "")));
    assert_eq!(sync(14 * day).new, 1);
    assert_eq!(on_server(), ["u4"], "the new one stays its fourteen days");
    assert_eq!(store.load_folder("p1", "INBOX").unwrap().messages.len(), 3, "this computer keeps them all");
    // From before: the seen list carries over, nothing downloaded twice.
    store.save_account_doc("p1", STATE_DOC, &State::new()).unwrap();
    store.save_account_doc("p1", SEEN_DOC, &vec!["u4".to_string()]).unwrap();
    assert_eq!(sync(15 * day).new, 0);
    let _ = std::fs::remove_dir_all(dir);
}

/// A move the cache can't read or write leaves the mail where it was, its source with it;
/// working again, it moves.
#[test]
fn a_move_that_cant_read_or_write_keeps_the_mail() {
    let store = Store::failing();
    folders(&store, "p1").unwrap();
    let one = file_locally(&store, "p1", "INBOX", &mail("One", ""), Flags::default(), 1).unwrap();
    let two = file_locally(&store, "p1", "INBOX", &mail("Two", ""), Flags::default(), 2).unwrap();
    let kept = file_locally(&store, "p1", "Archive", &mail("Kept", ""), Flags::default(), 3).unwrap();
    let path = store.body_path(&kept);
    let dir = path.parent().and_then(std::path::Path::parent).and_then(std::path::Path::file_name).unwrap().to_string_lossy();
    let archive_list = format!("{dir}/state.json");
    let count = |f: &str| store.load_folder("p1", f).unwrap().messages.len();
    let whole = |f: &str| store.load_folder("p1", f).unwrap().messages.iter().all(|m| store.read_body(&m.key).is_some());
    for (reads, writes) in [(Some(".eml"), None), (None, Some(".eml")), (None, Some(archive_list.as_str())), (Some("state.json"), None)] {
        store.fail(reads, writes);
        let r = move_locally(&store, "p1", &[one.clone(), two.clone()], "Archive");
        store.fail(None, None);
        assert!(r.is_err(), "{reads:?} {writes:?}: {r:?}");
        assert_eq!((count("INBOX"), count("Archive")), (2, 1), "{reads:?} {writes:?}");
        assert!(whole("INBOX") && whole("Archive"), "{reads:?} {writes:?}");
    }
    assert_eq!(move_locally(&store, "p1", &[one, two], "Archive").unwrap(), ["INBOX", "Archive"]);
    assert_eq!((count("INBOX"), count("Archive")), (0, 3));
    assert!(whole("Archive"));
    // A list at the last number takes no more, rather than filing two under one.
    let mut full = store.load_folder("p1", "Archive").unwrap();
    full.uid_next = u32::MAX;
    store.save_folder("p1", "Archive", &full).unwrap();
    let moving: Vec<_> = full.messages.iter().map(|m| m.key.clone()).take(2).collect();
    assert!(move_locally(&store, "p1", &moving, "INBOX").is_ok(), "the Inbox has numbers");
    let back = store.load_folder("p1", "INBOX").unwrap().messages.iter().map(|m| m.key.clone()).collect::<Vec<_>>();
    assert!(move_locally(&store, "p1", &back, "Archive").is_err());
    assert!(file_locally(&store, "p1", "Archive", &mail("Late", ""), Flags::default(), 4).is_err());
    assert_eq!((count("INBOX"), count("Archive")), (2, 1));
    assert!(whole("INBOX") && whole("Archive"));
}

/// Answers to read, and what's written thrown away.
struct Canned(std::io::Cursor<Vec<u8>>);
impl Read for Canned {
    fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(b)
    }
}
impl Write for Canned {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn canned(s: &[u8]) -> Canned {
    Canned(std::io::Cursor::new(s.to_vec()))
}

#[test]
fn a_pop_account_works_through_the_pool() {
    use mailcraft_model::job::Secret;
    use mailcraft_model::{Event, Flag, Job, Op, SyncStatus};

    use crate::waiting::{eventually, until};
    let server = fake::serve(vec![("u1".into(), mail("Welcome", "")), ("u2".into(), mail("Invoice", ""))]);
    let (smtp_port, smtp) = crate::smtp::tests::serve_once();
    let dir = std::env::temp_dir().join(format!("mailcraft-pop-pool-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    crate::fakes::fake_dav::looked_for_today(&store, "p1");
    let mut pool = crate::worker::Pool::new(store.clone(), std::sync::Arc::new(crate::worker::MemoryCredentials::default()));
    let acc = AccountConfig {
        id: "p1".into(),
        name: "Me".into(),
        email: "me@pop.example".into(),
        username: "me@pop.example".into(),
        pop: Some(ServerConfig { host: "127.0.0.1".into(), port: server.port, security: Security::None }),
        smtp: ServerConfig { host: "127.0.0.1".into(), port: smtp_port, security: Security::None },
        ..Default::default()
    };
    assert!(!acc.is_local() && !acc.is_api());
    pool.submit(Job::Start { account: acc.clone(), password: Some(Secret("wrong".into())) });
    until(&pool, "sign in again", |e| matches!(e, Event::Status { status: SyncStatus::NeedsPassword { .. }, .. }));
    pool.submit(Job::Start { account: acc.clone(), password: Some(Secret("pw".into())) });
    let seen = until(&pool, "online", |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
    let folders = seen.iter().rev().find_map(|e| if let Event::Folders { folders, .. } = e { Some(folders.clone()) } else { None }).unwrap();
    let mut paths: Vec<&str> = folders.iter().map(|f| f.path.as_str()).collect();
    paths.sort_unstable();
    assert_eq!(paths, ["Deleted Items", "Drafts", "INBOX", "Junk Email", "Sent Items"]);
    let inbox = store.load_folder("p1", "INBOX").unwrap();
    assert_eq!(inbox.messages.len(), 2);
    // Changes stay here.
    let k = inbox.messages[0].key.clone();
    pool.submit(Job::Apply { account: "p1".into(), op: Op::SetFlag { keys: vec![k.clone()], flag: Flag::Seen, on: true } });
    pool.submit(Job::Apply { account: "p1".into(), op: Op::Move { keys: vec![k], to: "Deleted Items".into() } });
    let here = |folder: &str, n: usize| {
        eventually(&format!("{n} in {folder}"), || store.load_folder("p1", folder).map_or(0, |f| f.messages.len()) >= n);
        store.load_folder("p1", folder).unwrap()
    };
    assert_eq!(here("Deleted Items", 1).messages.len(), 1);
    assert_eq!(server.drop.lock().unwrap().len(), 2, "the server keeps its copies");
    // A draft and a sent copy are filed here.
    pool.submit(Job::SaveDraft { account: "p1".into(), raw: mail("Draft", ""), folder: "Drafts".into(), replaces: None });
    assert!(here("Drafts", 1).messages.iter().any(|m| m.flags.draft));
    let raw = b"From: me@pop.example\r\nTo: ana@pop.example\r\nSubject: Hi\r\nMessage-ID: <s@x>\r\n\r\nhi\r\n".to_vec();
    pool.submit(Job::Send {
        account: "p1".into(),
        id: 1,
        raw,
        from: "me@pop.example".into(),
        recipients: vec!["ana@pop.example".into()],
        sent_folder: Some("Sent Items".into()),
    });
    until(&pool, "sent", |e| matches!(e, Event::Sent { id: 1, .. }));
    assert!(smtp.join().unwrap().contains("RCPT TO:<ana@pop.example>"));
    assert_eq!(here("Sent Items", 1).messages.len(), 1);
    // The server keeps no automatic replies, nor mail to search that isn't here.
    pool.submit(Job::GetVacation { account: "p1".into() });
    until(
        &pool,
        "no automatic replies",
        |e| matches!(e, Event::Vacation { vacation: None, error: Some(m), .. } if m == "POP accounts have no automatic replies on the server"),
    );
    pool.submit(Job::SearchServer { account: "p1".into(), folders: vec!["INBOX".into()], query: "invoice".into(), search: 1 });
    until(
        &pool,
        "no search of the server",
        |e| matches!(e, Event::ServerSearched { done: true, keys, error: Some(m), .. } if keys.is_empty() && m == "a POP account keeps all its mail here"),
    );
    pool.submit(Job::Remove { account: "p1".into() });
    let _ = std::fs::remove_dir_all(dir);
}

/// What was downloaded can't be read: nothing comes down again (it all did, as new), and the
/// record isn't written over; a folder that can't be read has no counts rather than 0.
#[test]
fn a_store_that_cant_be_read_downloads_nothing_twice() {
    let server = fake::serve(vec![("u1".into(), mail("One", "")), ("u2".into(), mail("Two", ""))]);
    let store = Store::failing();
    let cfg = ServerConfig { host: "127.0.0.1".into(), port: server.port, security: Security::None };
    let acc = AccountConfig { id: "p1".into(), email: "me@pop.example".into(), pop: Some(cfg), ..Default::default() };
    folders(&store, "p1").unwrap();
    let keep = Keep { remove_now: false, days: None, with_trash: false };
    let sync = || {
        let mut c = open(&acc, "pw").unwrap();
        let r = fetch_new(&mut c, &store, "p1", "INBOX", &keep, 0);
        let _ = c.quit();
        r
    };
    assert_eq!(sync().unwrap().new, 2);
    store.fail(Some(STATE_DOC), None);
    assert!(sync().is_err());
    store.fail(Some("state.json"), None);
    assert!(local_counts(&store, "p1", "Deleted Items").is_err());
    store.fail(None, None);
    assert_eq!(sync().unwrap().new, 0);
    assert_eq!(store.load_folder("p1", "INBOX").unwrap().messages.len(), 2);
    assert_eq!(local_counts(&store, "p1", "INBOX").unwrap().total, 2);
}

#[test]
fn hostile_answers_are_refused() {
    assert!(Client::greet(canned(b"* IMAP\r\n")).is_err());
    let long = format!("+OK hi\r\n{}", "x".repeat(MAX_LINE + 10));
    let mut c = Client::greet(canned(long.as_bytes())).unwrap();
    assert!(c.line().is_err());
    let mut c = Client::greet(canned(b"+OK\r\n+OK\r\n1 a\r\n")).unwrap();
    assert!(c.uidl().is_err(), "an answer that never ends");
    let mut c = Client::greet(canned(b"+OK\r\n-ERR [AUTH] no\r\n")).unwrap();
    assert!(matches!(c.login("u", "p"), Err(NetError::Auth(_))));
    assert!(Client::greet(canned(b"+OK\r\n")).unwrap().send("USER a\r\nPASS b").is_err(), "no command injection");
}

/// A message with a line longer than 64 KB comes whole. It was read in pieces, each taken for a
/// line of its own (a line break put inside it), and a piece that was just `.` ended the message
/// there, the rest left to be read as the next answers. One line past the answer's cap is
/// refused, without being held whole.
#[test]
fn a_line_over_64_kb_comes_whole() {
    let long = format!("{}.", "x".repeat(MAX_LINE + 2));
    let stuffed = format!("..{}", "y".repeat(MAX_LINE * 2));
    let answer = format!("+OK hi\r\n+OK message follows\r\n{long}\r\n{stuffed}\r\nafter\r\n.\r\n");
    let mut c = Client::greet(canned(answer.as_bytes())).unwrap();
    let got = c.retr(1).unwrap();
    assert!(got == format!("{long}\r\n{}\r\nafter\r\n", &stuffed[1..]).as_bytes(), "{} bytes", got.len());
    let huge = format!("+OK hi\r\n+OK\r\n1 {}\r\n.\r\n", "z".repeat(MAX_LINE * 64 + 10));
    let mut c = Client::greet(canned(huge.as_bytes())).unwrap();
    assert!(c.uidl().is_err(), "past the cap");
}

/// A `UIDL` answer of `count` lines, each id 24 characters (as many servers' are).
fn listing(count: usize) -> Vec<u8> {
    let mut a = String::from("+OK hi\r\n+OK\r\n");
    for n in 1..=count {
        a.push_str(&format!("{n} {n:0>12}{:0>12x}\r\n", n * 7919));
    }
    a.push_str(".\r\n");
    a.into_bytes()
}

/// Found in the audit: the listing was capped at 4 MB, which ordinary ids pass near 130,000
/// messages, so a maildrop that size failed every check though up to 200,000 were promised;
/// and one past 200,000 had its tail dropped without a word (those ids then forgotten as if
/// gone from the server). Now every message up to the limit is listed, and more is an error.
#[test]
fn a_listing_of_every_message_up_to_the_limit_comes_whole() {
    let mut c = Client::greet(canned(&listing(150_000))).unwrap();
    let got = c.uidl().unwrap();
    assert_eq!(got.len(), 150_000);
    assert_eq!(got.last().map(|(n, id)| (*n, id.len())), Some((150_000, 24)));
    let mut c = Client::greet(canned(&listing(MAX_MESSAGES))).unwrap();
    assert_eq!(c.uidl().unwrap().len(), MAX_MESSAGES);
    let mut c = Client::greet(canned(&listing(MAX_MESSAGES + 1))).unwrap();
    let e = c.uidl().unwrap_err();
    assert!(e.to_string().contains("more than 200000 messages"), "{e}");
}

/// A canned server that keeps what it's sent.
struct Talk {
    input: std::io::Cursor<Vec<u8>>,
    written: Vec<u8>,
}
impl Read for Talk {
    fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
        self.input.read(b)
    }
}
impl Write for Talk {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.written.extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Issue #123: NTLM (Outlook's Secure Password Authentication) over POP, when chosen: the
/// server's challenge answered, the password never sent. Left to MailCraft, `USER` and `PASS`.
#[test]
fn ntlm_signs_in_to_pop_when_chosen() {
    use mailcraft_model::SignInMethod;
    let challenge = crate::ntlm::make_challenge([7; 8], &[]);
    let server = format!("+OK ready\r\n+ \r\n+ {}\r\n+OK signed in\r\n", mailcraft_model::base64::encode(&challenge));
    let mut c = Client::greet(Talk { input: std::io::Cursor::new(server.into_bytes()), written: Vec::new() }).unwrap();
    c.login_as("CORP\\riley", "my pw!", SignInMethod::Ntlm).unwrap();
    let written = String::from_utf8(c.r.into_inner().written).unwrap();
    let lines: Vec<&str> = written.split("\r\n").collect();
    assert_eq!(lines[0], "AUTH NTLM");
    assert!(lines[1].starts_with("TlRMTVNTUAAB"), "{written}");
    let challenge = crate::ntlm::challenge(&challenge).unwrap();
    assert!(crate::ntlm::verify(&challenge, &crate::ews::unb64(lines[2]).unwrap(), "riley", "CORP", "my pw!"));
    assert!(!written.contains("my pw!"));
    // Refused at the end, in the server's words; or at once, by a server without NTLM.
    for (server, said) in [
        (
            format!(
                "+OK ready\r\n+ \r\n+ {}\r\n-ERR [AUTH] wrong password\r\n",
                mailcraft_model::base64::encode(&crate::ntlm::make_challenge([7; 8], &[]))
            ),
            "[AUTH] wrong password",
        ),
        ("+OK ready\r\n-ERR unknown command\r\n".to_string(), "unknown command"),
    ] {
        let mut c = Client::greet(canned(server.as_bytes())).unwrap();
        let e = c.login_as("riley", "pw", SignInMethod::Ntlm).unwrap_err();
        assert!(matches!(&e, NetError::Auth(m) if m == said), "{e:?}");
    }
    // A mailbox signed in to too soon again, as Cyrus refuses it: said to wait, not taken for a
    // wrong password (found against Cyrus 3.6, which allows one sign-in a minute).
    let challenge = mailcraft_model::base64::encode(&crate::ntlm::make_challenge([7; 8], &[]));
    let soon = format!("+OK ready\r\n+ \r\n+ {challenge}\r\n-ERR [LOGIN-DELAY] Logins must be at least 1 minute apart\r\n");
    let mut c = Client::greet(canned(soon.as_bytes())).unwrap();
    assert!(matches!(c.login_as("riley", "pw", SignInMethod::Ntlm), Err(NetError::Later(_))));
    let mut c = Client::greet(Talk { input: std::io::Cursor::new(b"+OK ready\r\n+OK\r\n+OK\r\n".to_vec()), written: Vec::new() }).unwrap();
    c.login_as("riley", "pw", SignInMethod::Automatic).unwrap();
    assert_eq!(c.r.into_inner().written, b"USER riley\r\nPASS pw\r\n");
}
