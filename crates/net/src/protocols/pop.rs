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
use mailcraft_store::{Filing, FolderState, Store};

use crate::stream::{self, Stream};
use crate::sync::{SyncReport, if_kept};
use crate::{NetError, Result};

/// Longest line read.
const MAX_LINE: usize = 64 << 10;
/// Largest message downloaded.
const MAX_MESSAGE: usize = 64 << 20;
/// Messages listed at most.
const MAX_MESSAGES: usize = 200_000;
/// The ids of downloaded messages, kept with the account.
/// Before: the ids downloaded, a list (still read, once).
const SEEN_DOC: &str = "pop-seen.json";
/// What's known of each message on the server (by its `UIDL` id): see [`Kept`].
const STATE_DOC: &str = "pop-state.json";

/// A message downloaded from the server: when, which message it is here, and whether it's been
/// deleted here for good.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Kept {
    pub at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub gone: bool,
}

/// When the server's copy goes, as the account says.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Keep {
    /// Removed as soon as it's downloaded.
    pub remove_now: bool,
    /// Kept this many days after it's downloaded.
    pub days: Option<u32>,
    /// Removed once it's deleted here for good.
    pub with_trash: bool,
}

impl Keep {
    pub fn of(acc: &AccountConfig) -> Keep {
        Keep { remove_now: acc.pop_delete, days: acc.pop_keep_days.filter(|d| *d > 0), with_trash: acc.pop_delete_with_trash }
    }

    fn removes(&self, k: &Kept, now: i64) -> bool {
        self.remove_now || self.days.is_some_and(|d| now.saturating_sub(k.at) >= i64::from(d).saturating_mul(86_400)) || (self.with_trash && k.gone)
    }
}

type State = std::collections::BTreeMap<String, Kept>;

/// What was downloaded. One that can't be read stops the download: read as empty, everything on
/// the server came again, and the record of it was written over.
fn load_state(store: &Store, account: &str, now: i64) -> Result<State> {
    let s: State = store.load_account_doc(account, STATE_DOC)?;
    if !s.is_empty() {
        return Ok(s);
    }
    // From before: the ids only, as if downloaded now.
    let seen: Vec<String> = store.load_account_doc(account, SEEN_DOC)?;
    Ok(seen.into_iter().map(|id| (id, Kept { at: now, ..Default::default() })).collect())
}

/// Note that messages here were deleted for good (their server copies can go, when the account
/// says so).
pub fn deleted_for_good(store: &Store, account: &str, keys: &[MessageKey]) -> Result<()> {
    if keys.is_empty() {
        return Ok(());
    }
    let ids: Vec<String> = keys.iter().map(MessageKey::to_id).collect();
    let mut state: State = store.load_account_doc(account, STATE_DOC)?;
    let mut changed = false;
    for k in state.values_mut().filter(|k| k.key.as_ref().is_some_and(|id| ids.contains(id))) {
        k.gone = true;
        changed = true;
    }
    if changed {
        store.save_account_doc(account, STATE_DOC, &state)?;
    }
    Ok(())
}

/// A message moved here: the server's record follows it.
fn moved(store: &Store, account: &str, pairs: &[(MessageKey, MessageKey)]) -> Result<()> {
    let mut state: State = store.load_account_doc(account, STATE_DOC)?;
    let mut changed = false;
    for (old, new) in pairs {
        let (old, new) = (old.to_id(), new.to_id());
        for k in state.values_mut().filter(|k| k.key.as_deref() == Some(old.as_str())) {
            k.key = Some(new.clone());
            changed = true;
        }
    }
    if changed {
        store.save_account_doc(account, STATE_DOC, &state)?;
    }
    Ok(())
}
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
        Self::connect_trusting(cfg, &[])
    }

    /// [`Self::connect`] for an account's server: `trusted` are the certificates the user chose
    /// to trust for one on this computer ([`stream::tls_trusting`]).
    pub fn connect_trusting(cfg: &ServerConfig, trusted: &[String]) -> Result<Client<Stream>> {
        let sock = stream::tcp(&cfg.host, cfg.port)?;
        match cfg.security {
            Security::Tls => Client::greet(stream::tls_trusting(sock, &cfg.host, trusted)?),
            Security::None => Client::greet(Stream::Plain(sock)),
            Security::StartTls => {
                let mut c = Client::greet(Stream::Plain(sock))?;
                c.ok("STLS").map_err(|_| NetError::Tls("the server does not offer STLS".into()))?;
                if !c.r.buffer().is_empty() {
                    return Err(NetError::Tls("unexpected data before TLS negotiation".into()));
                }
                let Stream::Plain(sock) = c.r.into_inner() else { return Err(NetError::Tls("connection already encrypted".into())) };
                Ok(Client { r: BufReader::new(stream::tls_trusting(sock, &cfg.host, trusted)?) })
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
        let too_big = || proto(format!("larger than {} MB", max >> 20));
        loop {
            // A whole line, however long, only the answer's room capping it: read in pieces of
            // a short line's length, each was taken for a line (a break put inside it), and a
            // piece that was just `.` for the end.
            let room = max.saturating_sub(out.len()).saturating_add(3);
            let mut buf = Vec::new();
            let n = (&mut self.r).take(u64::try_from(room).unwrap_or(u64::MAX)).read_until(b'\n', &mut buf)?;
            if !buf.ends_with(b"\n") {
                return Err(if n == room { too_big() } else { NetError::Closed("the server closed the connection".into()) });
            }
            let line = buf.strip_suffix(b"\r\n").or_else(|| buf.strip_suffix(b"\n")).unwrap_or(&buf);
            if line == b"." {
                return Ok(out);
            }
            let line = line.strip_prefix(b".").filter(|_| line.starts_with(b"..")).unwrap_or(line);
            if out.len() + line.len() + 2 > max {
                return Err(too_big());
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
        // The mailbox is in use, or the server asks to wait (RFC 2449, 3206): not the password,
        // which was asked for again, the account checking no more until it was typed.
        NetError::No(m) if ["[in-use]", "[login-delay]", "[sys/temp]"].iter().any(|code| m.to_ascii_lowercase().contains(code)) => {
            NetError::Later(m.clone())
        }
        NetError::No(m) => NetError::Auth(if m.is_empty() { "the server refused the user name or password".into() } else { m.clone() }),
        e => NetError::Protocol(e.to_string()),
    }
}

/// Sign in to an account's POP server.
pub fn open(acc: &AccountConfig, secret: &str) -> Result<Client<Stream>> {
    let cfg = acc.pop.as_ref().ok_or_else(|| proto("not a POP account"))?;
    let user = if acc.username.is_empty() { acc.email.as_str() } else { acc.username.as_str() };
    let mut c = Client::connect_trusting(cfg, &acc.trusted_certificates)?;
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
            list.push(Folder { account: account.to_string(), path: path.into(), delimiter: Some('/'), role, selectable: true, ..Default::default() });
            added = true;
        }
    }
    if added {
        store.save_folders(account, &list)?;
    }
    Ok(list)
}

/// Download what's new into the Inbox, and remove from the server what the account says should go
/// (`keep`): at once, so many days after downloading, or once deleted here for good.
pub fn fetch_new<S: Read + Write>(c: &mut Client<S>, store: &Store, account: &str, inbox: &str, keep: &Keep, now: i64) -> Result<SyncReport> {
    fetch_new_as(c, store, account, inbox, keep, now, store.removals(account))
}

/// [`fetch_new`] for a worker that started when its account had been removed `seen` times: a
/// removal since has it write nothing of the account again, nor remove anything from the server.
pub fn fetch_new_as<S: Read + Write>(
    c: &mut Client<S>,
    store: &Store,
    account: &str,
    inbox: &str,
    keep: &Keep,
    now: i64,
    seen: u64,
) -> Result<SyncReport> {
    let mut state = load_state(store, account, now)?;
    let listed = c.uidl()?;
    let mut fs = store.load_folder(account, inbox)?;
    if fs.validity == 0 {
        fs.validity = 1;
        fs.uid_next = fs.uid_next.max(1);
    }
    let mut report = SyncReport::default();
    for (n, id) in &listed {
        if let Some(k) = state.get(id) {
            if keep.removes(k, now) {
                c.dele(*n)?;
            }
            continue;
        }
        let raw = c.retr(*n)?;
        let mut next = fs.free_uid(account, inbox)?;
        let uid = FolderState::take_uid(&mut next, account, inbox)?;
        let key = MessageKey { account: account.to_string(), folder: inbox.to_string(), validity: fs.validity, uid };
        if_kept(store, account, seen, || store.write_body(&key, &raw))?;
        let size = u32::try_from(raw.len()).unwrap_or(u32::MAX);
        state.insert(id.clone(), Kept { at: now, key: Some(key.to_id()), gone: false });
        fs.messages.push(mime::summary_from_full(&raw, key, Flags::default(), size, 0).into());
        fs.uid_next = next;
        report.new += 1;
        // Saved as it goes: a dropped connection loses nothing already downloaded.
        sort_newest_first(&mut fs.messages);
        if_kept(store, account, seen, || {
            store.save_folder(account, inbox, &fs)?;
            store.save_account_doc(account, STATE_DOC, &state)
        })?;
        if keep.remove_now {
            c.dele(*n)?;
        }
    }
    // Forget the ids the server no longer has (one removed now is still listed until QUIT, so it
    // goes from here next time).
    state.retain(|id, _| listed.iter().any(|(_, l)| l == id));
    if_kept(store, account, seen, || store.save_account_doc(account, STATE_DOC, &state))?;
    report.total = u32::try_from(fs.messages.len()).unwrap_or(u32::MAX);
    report.unread = fs.unread();
    Ok(report)
}

/// A folder kept on this computer: its counts (an error rather than none when its list can't be
/// read: they're shown and kept).
pub fn local_counts(store: &Store, account: &str, folder: &str) -> Result<SyncReport> {
    let fs: FolderState = store.load_folder(account, folder)?;
    Ok(SyncReport { total: u32::try_from(fs.messages.len()).unwrap_or(u32::MAX), unread: fs.unread(), ..Default::default() })
}

/// Put a message in a folder on this computer (a draft, or the copy of one sent); its key.
pub fn file_locally(store: &Store, account: &str, folder: &str, raw: &[u8], flags: Flags, now: i64) -> Result<MessageKey> {
    let mut filing = Filing::new(store, account, folder)?;
    let key = filing.key()?;
    let size = u32::try_from(raw.len()).unwrap_or(u32::MAX);
    filing.add(mime::summary_from_full(raw, key.clone(), flags, size, now), Some(raw.to_vec()))?;
    filing.finish()?;
    Ok(key)
}

/// Move messages between folders on this computer (a POP account's folders are only here): each
/// keeps its flags and body under a new key in `to`. Returns the folders changed. Every list is
/// read and the messages filed in `to` before a source folder is saved without them or an old
/// source removed: a store error on the way leaves them where they were, not in neither folder.
pub fn move_locally(store: &Store, account: &str, keys: &[MessageKey], to: &str) -> Result<Vec<String>> {
    let mut dest = store.load_folder(account, to)?;
    if dest.validity == 0 {
        dest.validity = 1;
    }
    let mut sources: Vec<&str> = keys.iter().map(|k| k.folder.as_str()).filter(|f| *f != to).collect();
    sources.sort_unstable();
    sources.dedup();
    let mut lists = Vec::new();
    for folder in sources {
        lists.push((folder, store.load_folder(account, folder)?));
    }
    let mut uid = dest.free_uid(account, to)?;
    let mut pairs = Vec::new();
    let mut changed = Vec::new();
    for (folder, src) in &mut lists {
        let (going, staying): (Vec<_>, Vec<_>) =
            std::mem::take(&mut src.messages).into_iter().partition(|m| keys.iter().any(|k| k.folder == *folder && k.uid == m.key.uid));
        src.messages = staying;
        if going.is_empty() {
            continue;
        }
        for mut m in going {
            let old = m.key.clone();
            let key = MessageKey {
                account: account.to_string(),
                folder: to.to_string(),
                validity: dest.validity,
                uid: FolderState::take_uid(&mut uid, account, to)?,
            };
            // One whose source isn't here moves without it.
            if let Some(raw) = store.load_body(&old)? {
                store.write_body(&key, &raw)?;
            }
            pairs.push((old, key.clone()));
            std::sync::Arc::make_mut(&mut m).key = key;
            dest.messages.push(m);
        }
        changed.push(folder.to_string());
    }
    if changed.is_empty() {
        return Ok(changed);
    }
    dest.uid_next = uid;
    sort_newest_first(&mut dest.messages);
    store.save_folder(account, to, &dest)?;
    for (folder, src) in &lists {
        if changed.iter().any(|c| c == folder) {
            store.save_folder(account, folder, src)?;
        }
    }
    for (old, _) in &pairs {
        store.remove_body(old);
    }
    changed.push(to.to_string());
    moved(store, account, &pairs)?;
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
mod tests;
