//! POP3 (RFC 1939): mail is downloaded from the server's single maildrop into the account's
//! Inbox on this computer; every other folder lives here only, as in Outlook's POP accounts.
//!
//! A session is short: connect (TLS on 995, or `STLS` (RFC 2595) on 110), sign in (`USER`/`PASS`,
//! or `AUTH XOAUTH2` for OAuth accounts), list the messages' unique ids (`UIDL`, RFC 1939 §7),
//! fetch those not seen before (`RETR`), and leave them on the server unless the account says to
//! remove them (`DELE`), then `QUIT`. The ids already downloaded are kept with the account.
//!
//! Answers are hostile input: line lengths, message sizes and counts are capped.

use std::io::{BufRead, BufReader, Read, Write};

use mailcraft_mime as mime;
use mailcraft_model::{AccountConfig, Flags, Folder, FolderRole, MessageKey, Security, ServerConfig, sort_newest_first};
use mailcraft_store::{FolderState, Store};

use crate::stream::{self, Stream};
use crate::sync::SyncReport;
use crate::{NetError, Result};

/// Longest line read.
const MAX_LINE: usize = 64 << 10;
/// Largest message downloaded.
const MAX_MESSAGE: usize = 64 << 20;
/// Messages listed at most.
const MAX_MESSAGES: usize = 200_000;
/// The ids of downloaded messages, kept with the account.
const SEEN_DOC: &str = "pop-seen.json";
/// The folders a POP account has on this computer.
pub const FOLDERS: [(&str, FolderRole); 5] = [
    ("INBOX", FolderRole::Inbox),
    ("Drafts", FolderRole::Drafts),
    ("Sent Items", FolderRole::Sent),
    ("Deleted Items", FolderRole::Trash),
    ("Junk Email", FolderRole::Junk),
];

/// A POP3 session.
pub struct Client<S: Read + Write> {
    r: BufReader<S>,
}

fn proto(m: impl Into<String>) -> NetError {
    NetError::Protocol(m.into())
}

impl Client<Stream> {
    /// Connect as the server settings say (no sign-in yet).
    pub fn connect(cfg: &ServerConfig) -> Result<Client<Stream>> {
        let sock = stream::tcp(&cfg.host, cfg.port)?;
        match cfg.security {
            Security::Tls => Client::greet(stream::tls(sock, &cfg.host)?),
            Security::None => Client::greet(Stream::Plain(sock)),
            Security::StartTls => {
                let mut c = Client::greet(Stream::Plain(sock))?;
                c.ok("STLS").map_err(|_| NetError::Tls("the server does not offer STLS".into()))?;
                if !c.r.buffer().is_empty() {
                    return Err(NetError::Tls("unexpected data before TLS negotiation".into()));
                }
                let Stream::Plain(sock) = c.r.into_inner() else { return Err(NetError::Tls("connection already encrypted".into())) };
                Ok(Client { r: BufReader::new(stream::tls(sock, &cfg.host)?) })
            }
        }
    }
}

impl<S: Read + Write> Client<S> {
    /// Wrap a connected stream and read the greeting.
    pub fn greet(s: S) -> Result<Client<S>> {
        let mut c = Client { r: BufReader::new(s) };
        let g = c.line()?;
        if !g.starts_with("+OK") {
            return Err(proto(format!("not a POP3 server: {}", g.chars().take(80).collect::<String>())));
        }
        Ok(c)
    }

    fn line(&mut self) -> Result<String> {
        let mut buf = Vec::new();
        let n = (&mut self.r).take(MAX_LINE as u64).read_until(b'\n', &mut buf)?;
        if n == 0 {
            return Err(NetError::Closed("the server closed the connection".into()));
        }
        if !buf.ends_with(b"\n") {
            return Err(proto("a line too long"));
        }
        while buf.last().is_some_and(|b| *b == b'\n' || *b == b'\r') {
            buf.pop();
        }
        Ok(String::from_utf8_lossy(&buf).into_owned())
    }

    fn send(&mut self, cmd: &str) -> Result<()> {
        if cmd.contains(['\r', '\n']) {
            return Err(proto("a command with a line break"));
        }
        let s = self.r.get_mut();
        s.write_all(cmd.as_bytes())?;
        s.write_all(b"\r\n")?;
        s.flush()?;
        Ok(())
    }

    /// A command answered `+OK`: the rest of that line.
    fn ok(&mut self, cmd: &str) -> Result<String> {
        self.send(cmd)?;
        let l = self.line()?;
        match l.strip_prefix("+OK") {
            Some(rest) => Ok(rest.trim().to_string()),
            None => Err(NetError::No(l.trim_start_matches("-ERR").trim().to_string())),
        }
    }

    /// A multi-line answer's body (dot-stuffing undone), bytes at most `max`.
    fn multi(&mut self, max: usize) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        loop {
            let mut buf = Vec::new();
            let n = (&mut self.r).take(MAX_LINE as u64 + 2).read_until(b'\n', &mut buf)?;
            if n == 0 {
                return Err(NetError::Closed("the server closed the connection".into()));
            }
            let line = buf.strip_suffix(b"\r\n").or_else(|| buf.strip_suffix(b"\n")).unwrap_or(&buf);
            if line == b"." {
                return Ok(out);
            }
            let line = line.strip_prefix(b".").filter(|_| line.starts_with(b"..")).unwrap_or(line);
            if out.len() + line.len() + 2 > max {
                return Err(proto(format!("larger than {} MB", max >> 20)));
            }
            out.extend_from_slice(line);
            out.extend_from_slice(b"\r\n");
        }
    }

    /// Sign in with a password.
    pub fn login(&mut self, user: &str, password: &str) -> Result<()> {
        self.ok(&format!("USER {user}")).map_err(|e| auth(&e))?;
        self.ok(&format!("PASS {password}")).map_err(|e| auth(&e))?;
        Ok(())
    }

    /// Sign in with an OAuth token (`AUTH XOAUTH2`).
    pub fn login_xoauth2(&mut self, user: &str, token: &str) -> Result<()> {
        let blob = crate::imap::base64(format!("user={user}\x01auth=Bearer {token}\x01\x01").as_bytes());
        self.ok(&format!("AUTH XOAUTH2 {blob}")).map(drop).map_err(|e| auth(&e))
    }

    /// Message numbers and unique ids.
    pub fn uidl(&mut self) -> Result<Vec<(u32, String)>> {
        self.ok("UIDL")?;
        let body = self.multi(MAX_LINE * 64)?;
        Ok(String::from_utf8_lossy(&body)
            .lines()
            .take(MAX_MESSAGES)
            .filter_map(|l| {
                let (n, id) = l.trim().split_once(' ')?;
                Some((n.parse().ok()?, id.trim().chars().take(70).collect()))
            })
            .collect())
    }

    pub fn retr(&mut self, n: u32) -> Result<Vec<u8>> {
        self.ok(&format!("RETR {n}"))?;
        self.multi(MAX_MESSAGE)
    }

    pub fn dele(&mut self, n: u32) -> Result<()> {
        self.ok(&format!("DELE {n}")).map(drop)
    }

    pub fn quit(mut self) -> Result<()> {
        self.ok("QUIT").map(drop)
    }
}

fn auth(e: &NetError) -> NetError {
    match e {
        NetError::No(m) => NetError::Auth(if m.is_empty() { "the server refused the user name or password".into() } else { m.clone() }),
        e => NetError::Protocol(e.to_string()),
    }
}

/// Sign in to an account's POP server.
pub fn open(acc: &AccountConfig, secret: &str) -> Result<Client<Stream>> {
    let cfg = acc.pop.as_ref().ok_or_else(|| proto("not a POP account"))?;
    let user = if acc.username.is_empty() { acc.email.as_str() } else { acc.username.as_str() };
    let mut c = Client::connect(cfg)?;
    if acc.oauth.is_some() {
        c.login_xoauth2(user, secret)?;
    } else {
        c.login(user, secret)?;
    }
    Ok(c)
}

/// The account's folders on this computer, made the first time.
pub fn folders(store: &Store, account: &str) -> Result<Vec<Folder>> {
    let mut list = store.load_folders(account)?;
    let mut added = false;
    for (path, role) in FOLDERS {
        if !list.iter().any(|f| f.path == path || f.role == role) {
            list.push(Folder { account: account.to_string(), path: path.into(), delimiter: Some('/'), role, selectable: true, unread: 0, total: 0 });
            added = true;
        }
    }
    if added {
        store.save_folders(account, &list)?;
    }
    Ok(list)
}

/// Download what's new into the Inbox (removing it from the server when the account says so).
pub fn fetch_new<S: Read + Write>(c: &mut Client<S>, store: &Store, account: &str, inbox: &str, delete: bool) -> Result<SyncReport> {
    let mut seen: Vec<String> = store.load_account_doc(account, SEEN_DOC).unwrap_or_default();
    let listed = c.uidl()?;
    let mut fs = store.load_folder(account, inbox)?;
    if fs.validity == 0 {
        fs.validity = 1;
        fs.uid_next = fs.uid_next.max(1);
    }
    let mut report = SyncReport::default();
    for (n, id) in &listed {
        if seen.contains(id) {
            if delete {
                c.dele(*n)?;
            }
            continue;
        }
        let raw = c.retr(*n)?;
        let uid = fs.uid_next.max(1);
        let key = MessageKey { account: account.to_string(), folder: inbox.to_string(), validity: fs.validity, uid };
        store.write_body(&key, &raw)?;
        let size = u32::try_from(raw.len()).unwrap_or(u32::MAX);
        fs.messages.push(mime::summary_from_full(&raw, key, Flags::default(), size, 0));
        fs.uid_next = uid.saturating_add(1);
        seen.push(id.clone());
        report.new += 1;
        // Saved as it goes: a dropped connection loses nothing already downloaded.
        sort_newest_first(&mut fs.messages);
        store.save_folder(account, inbox, &fs)?;
        store.save_account_doc(account, SEEN_DOC, &seen)?;
        if delete {
            c.dele(*n)?;
        }
    }
    // Forget the ids the server no longer has (unless removing: then they're all gone anyway).
    if !delete {
        seen.retain(|s| listed.iter().any(|(_, id)| id == s));
        store.save_account_doc(account, SEEN_DOC, &seen)?;
    }
    report.total = u32::try_from(fs.messages.len()).unwrap_or(u32::MAX);
    report.unread = fs.unread();
    Ok(report)
}

/// A folder kept on this computer: its counts.
pub fn local_counts(store: &Store, account: &str, folder: &str) -> SyncReport {
    let fs: FolderState = store.load_folder(account, folder).unwrap_or_default();
    SyncReport { total: u32::try_from(fs.messages.len()).unwrap_or(u32::MAX), unread: fs.unread(), ..Default::default() }
}

/// Put a message in a folder on this computer (a draft, or the copy of one sent); its key.
pub fn file_locally(store: &Store, account: &str, folder: &str, raw: &[u8], flags: Flags, now: i64) -> Result<MessageKey> {
    let mut fs = store.load_folder(account, folder)?;
    if fs.validity == 0 {
        fs.validity = 1;
    }
    let uid = fs.uid_next.max(1);
    let key = MessageKey { account: account.to_string(), folder: folder.to_string(), validity: fs.validity, uid };
    store.write_body(&key, raw)?;
    let size = u32::try_from(raw.len()).unwrap_or(u32::MAX);
    fs.messages.push(mime::summary_from_full(raw, key.clone(), flags, size, now));
    fs.uid_next = uid.saturating_add(1);
    sort_newest_first(&mut fs.messages);
    store.save_folder(account, folder, &fs)?;
    Ok(key)
}

/// Move messages between folders on this computer (a POP account's folders are only here): each
/// keeps its flags and body under a new key in `to`. Returns the folders changed.
pub fn move_locally(store: &Store, account: &str, keys: &[MessageKey], to: &str) -> Result<Vec<String>> {
    let mut dest = store.load_folder(account, to)?;
    if dest.validity == 0 {
        dest.validity = 1;
    }
    let mut changed = Vec::new();
    let mut sources: Vec<&str> = keys.iter().map(|k| k.folder.as_str()).filter(|f| *f != to).collect();
    sources.sort_unstable();
    sources.dedup();
    for folder in sources {
        let mut src = store.load_folder(account, folder)?;
        let (going, staying): (Vec<_>, Vec<_>) =
            src.messages.into_iter().partition(|m| keys.iter().any(|k| k.folder == folder && k.uid == m.key.uid));
        src.messages = staying;
        if going.is_empty() {
            continue;
        }
        for mut m in going {
            let old = m.key.clone();
            let key = MessageKey { account: account.to_string(), folder: to.to_string(), validity: dest.validity, uid: dest.uid_next.max(1) };
            if let Some(raw) = store.read_body(&old) {
                store.write_body(&key, &raw)?;
            }
            store.remove_body(&old);
            dest.uid_next = key.uid.saturating_add(1);
            m.key = key;
            dest.messages.push(m);
        }
        store.save_folder(account, folder, &src)?;
        changed.push(folder.to_string());
    }
    if !changed.is_empty() {
        sort_newest_first(&mut dest.messages);
        store.save_folder(account, to, &dest)?;
        changed.push(to.to_string());
    }
    Ok(changed)
}

#[cfg(test)]
pub(crate) mod fake {
    //! A POP3 server on localhost for tests: one maildrop, `USER`/`PASS`, `UIDL`, `RETR`, `DELE`.
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    pub struct Server {
        pub port: u16,
        /// (unique id, message); deleted ones go at `QUIT`.
        pub drop: Arc<Mutex<Vec<(String, Vec<u8>)>>>,
    }

    pub fn serve(messages: Vec<(String, Vec<u8>)>) -> Server {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let drop = Arc::new(Mutex::new(messages));
        let shared = drop.clone();
        std::thread::spawn(move || {
            for conn in l.incoming() {
                let Ok(mut s) = conn else { continue };
                let shared = shared.clone();
                std::thread::spawn(move || {
                    let mut r = BufReader::new(s.try_clone().unwrap());
                    let _ = s.write_all(b"+OK test POP3 ready\r\n");
                    let (mut user, mut authed, mut deleted) = (String::new(), false, Vec::<usize>::new());
                    loop {
                        let mut line = String::new();
                        if r.read_line(&mut line).unwrap_or(0) == 0 {
                            return;
                        }
                        let line = line.trim_end().to_string();
                        let (cmd, arg) = line.split_once(' ').unwrap_or((line.as_str(), ""));
                        let d = shared.lock().unwrap().clone();
                        let out = match cmd.to_ascii_uppercase().as_str() {
                            "USER" => {
                                user = arg.to_string();
                                "+OK\r\n".to_string()
                            }
                            "PASS" if user == "me@pop.example" && arg == "pw" => {
                                authed = true;
                                "+OK signed in\r\n".to_string()
                            }
                            "PASS" => "-ERR [AUTH] wrong password\r\n".to_string(),
                            _ if !authed => "-ERR sign in first\r\n".to_string(),
                            "UIDL" => {
                                let mut o = "+OK\r\n".to_string();
                                for (i, (id, _)) in d.iter().enumerate().filter(|(i, _)| !deleted.contains(i)) {
                                    o.push_str(&format!("{} {id}\r\n", i + 1));
                                }
                                o + ".\r\n"
                            }
                            "RETR" => match arg.parse::<usize>().ok().and_then(|n| d.get(n.wrapping_sub(1))) {
                                Some((_, m)) => {
                                    let body: String = String::from_utf8_lossy(m)
                                        .split("\r\n")
                                        .map(|l| if l.starts_with('.') { format!(".{l}\r\n") } else { format!("{l}\r\n") })
                                        .collect();
                                    format!("+OK\r\n{body}.\r\n")
                                }
                                None => "-ERR no such message\r\n".to_string(),
                            },
                            "DELE" => {
                                if let Ok(n) = arg.parse::<usize>() {
                                    deleted.push(n - 1);
                                }
                                "+OK\r\n".to_string()
                            }
                            "QUIT" => {
                                let mut all = shared.lock().unwrap();
                                let mut i = 0;
                                all.retain(|_| {
                                    i += 1;
                                    !deleted.contains(&(i - 1))
                                });
                                let _ = s.write_all(b"+OK bye\r\n");
                                return;
                            }
                            _ => "-ERR unknown\r\n".to_string(),
                        };
                        let _ = s.write_all(out.as_bytes());
                    }
                });
            }
        });
        Server { port, drop }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mail(subject: &str, extra: &str) -> Vec<u8> {
        format!("From: Ana <ana@pop.example>\r\nTo: me@pop.example\r\nSubject: {subject}\r\nMessage-ID: <{subject}@x>\r\n\r\nHello.\r\n{extra}")
            .into_bytes()
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
        let r = fetch_new(&mut c, &store, "p1", "INBOX", false).unwrap();
        c.quit().unwrap();
        assert_eq!((r.new, r.total, r.unread), (2, 2, 2));
        let st = store.load_folder("p1", "INBOX").unwrap();
        let two = st.messages.iter().find(|m| m.subject == "Two").unwrap();
        let raw = String::from_utf8(store.read_body(&two.key).unwrap()).unwrap();
        assert!(raw.contains("\r\n.a line starting with a dot\r\n"), "dot-stuffing undone: {raw}");
        // Again: nothing new; the server still has both.
        let mut c = open(&acc, "pw").unwrap();
        assert_eq!(fetch_new(&mut c, &store, "p1", "INBOX", false).unwrap().new, 0);
        c.quit().unwrap();
        assert_eq!(server.drop.lock().unwrap().len(), 2);
        // New mail, and removing from the server.
        server.drop.lock().unwrap().push(("u3".into(), mail("Three", "")));
        let mut c = open(&acc, "pw").unwrap();
        assert_eq!(fetch_new(&mut c, &store, "p1", "INBOX", true).unwrap().new, 1);
        c.quit().unwrap();
        assert!(server.drop.lock().unwrap().is_empty(), "removed after downloading");
        assert_eq!(store.load_folder("p1", "INBOX").unwrap().messages.len(), 3);
        // A copy filed here.
        let k = file_locally(&store, "p1", "Sent Items", &mail("Sent", ""), Flags { seen: true, ..Default::default() }, 1).unwrap();
        assert!(store.has_body(&k));
        let _ = std::fs::remove_dir_all(dir);
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
        use std::time::{Duration, Instant};
        let server = fake::serve(vec![("u1".into(), mail("Welcome", "")), ("u2".into(), mail("Invoice", ""))]);
        let (smtp_port, smtp) = crate::smtp::tests::serve_once();
        let dir = std::env::temp_dir().join(format!("mailcraft-pop-pool-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store::open(&dir).unwrap();
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
        let until = |pool: &crate::worker::Pool, what: &str, f: &dyn Fn(&Event) -> bool| {
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
        };
        pool.submit(Job::Start { account: acc.clone(), password: Some(Secret("wrong".into())) });
        until(&pool, "sign in again", &|e| matches!(e, Event::Status { status: SyncStatus::NeedsPassword { .. }, .. }));
        pool.submit(Job::Start { account: acc.clone(), password: Some(Secret("pw".into())) });
        let seen = until(&pool, "online", &|e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }));
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
            let end = Instant::now() + Duration::from_secs(10);
            while store.load_folder("p1", folder).map_or(0, |f| f.messages.len()) < n && Instant::now() < end {
                std::thread::sleep(Duration::from_millis(50));
            }
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
        until(&pool, "sent", &|e| matches!(e, Event::Sent { id: 1, .. }));
        assert!(smtp.join().unwrap().contains("RCPT TO:<ana@pop.example>"));
        assert_eq!(here("Sent Items", 1).messages.len(), 1);
        pool.submit(Job::Remove { account: "p1".into() });
        let _ = std::fs::remove_dir_all(dir);
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
}
