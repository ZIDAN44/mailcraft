//! POP3 (RFC 1939): mail is downloaded from the server's single maildrop into the account's
//! Inbox on this computer; every other folder lives here only, as in Outlook's POP accounts.
//!
//! A session is short: connect (TLS on 995, or `STLS` (RFC 2595) on 110), sign in (`USER`/`PASS`,
//! or `AUTH XOAUTH2` for OAuth accounts), list the messages' unique ids (`UIDL`, RFC 1939 §7),
//! fetch those not seen before (`RETR`), and leave them on the server unless the account says to
//! remove them (`DELE`), then `QUIT`. The ids already downloaded are kept with the account.
//!
//! Answers are hostile input: line lengths, message sizes and counts are capped.

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Read, Write};
use std::time::{Duration, Instant};

use mailcraft_mime as mime;
use mailcraft_model::{AccountConfig, Flags, Folder, FolderRole, MessageKey, Security, ServerConfig, Summary, sort_newest_first};
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
/// New mail is saved (its sources, the Inbox's list and the record of what came) once this many
/// messages, or this many bytes of them, have come, or this long after the last save, and at the
/// end: saved after each message, a first download of N messages wrote N ever longer lists.
const SAVE_EVERY: usize = 500;
const SAVE_BYTES: usize = 16 << 20;
const SAVE_AFTER: Duration = Duration::from_secs(5);
/// The ids of downloaded messages, kept with the account.
/// Before: the ids downloaded, a list (still read whenever no state is kept, [`STATE_DOC`]).
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
    /// Recorded as it was saved, the Inbox's list perhaps not yet naming it: a download stopped
    /// there is settled the next time (`settle`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pending: bool,
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
    let ids: HashSet<String> = keys.iter().map(MessageKey::to_id).collect();
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
    // A new key is in the folder moved to, never one moved from: none moves twice.
    let to: HashMap<String, String> = pairs.iter().map(|(old, new)| (old.to_id(), new.to_id())).collect();
    for k in state.values_mut() {
        if let Some(new) = k.key.as_ref().and_then(|id| to.get(id)) {
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

/// What [`Client::piece`] read.
enum Piece {
    Line(Vec<u8>),
    /// The answer's end (a line of just `.`).
    End,
    /// A line longer than it had room for.
    TooLong,
}

fn proto(m: impl Into<String>) -> NetError {
    NetError::Protocol(m.into())
}

impl Client<Stream> {
    /// Connect as the server settings say (no sign-in yet): `trusted` is what the account trusts
    /// besides the usual authorities ([`stream::tls_trusting`]).
    pub fn connect_trusting(cfg: &ServerConfig, trusted: stream::Trusted<'_>) -> Result<Client<Stream>> {
        let sock = stream::tcp(&cfg.host, cfg.port)?;
        match cfg.security {
            Security::Tls => Client::greet(stream::tls_trusting(sock, &cfg.host, trusted)?),
            Security::None => Client::greet(Stream::Plain(sock)),
            Security::StartTls => {
                let mut c = Client::greet(Stream::Plain(sock))?;
                c.ok("STLS").map_err(|_| NetError::Tls("the server does not offer STLS".into()))?;
                let unread = c.r.buffer().len();
                Ok(Client { r: BufReader::new(stream::starttls(c.r.into_inner(), unread, &cfg.host, trusted)?) })
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
        let mut buf = match super::capped_line(&mut self.r, MAX_LINE)? {
            super::Capped::Line(buf) => buf,
            super::Capped::Closed => return Err(NetError::Closed("the server closed the connection".into())),
            super::Capped::TooLong => return Err(proto("a line too long")),
        };
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

    /// The next line of a multi-line answer, dot-stuffing undone, read whole when it takes at
    /// most `room` bytes (its line break included).
    fn piece(&mut self, room: usize) -> Result<Piece> {
        // A whole line, however long, only the room capping it: read in pieces of a short line's
        // length, each was taken for a line (a break put inside it), and a piece that was just
        // `.` for the end.
        let mut buf = Vec::new();
        let n = (&mut self.r).take(u64::try_from(room).unwrap_or(u64::MAX)).read_until(b'\n', &mut buf)?;
        if !buf.ends_with(b"\n") {
            return if n == room { Ok(Piece::TooLong) } else { Err(NetError::Closed("the server closed the connection".into())) };
        }
        let line = buf.strip_suffix(b"\r\n").or_else(|| buf.strip_suffix(b"\n")).unwrap_or(&buf);
        if line == b"." {
            return Ok(Piece::End);
        }
        let line = line.strip_prefix(b".").filter(|_| line.starts_with(b"..")).unwrap_or(line);
        Ok(Piece::Line(line.to_vec()))
    }

    /// A multi-line answer's body (dot-stuffing undone), bytes at most `max`.
    fn multi(&mut self, max: usize) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        let too_big = || proto(format!("larger than {} MB", max >> 20));
        loop {
            match self.piece(max.saturating_sub(out.len()).saturating_add(3))? {
                Piece::End => return Ok(out),
                Piece::TooLong => return Err(too_big()),
                Piece::Line(line) => {
                    if out.len() + line.len() + 2 > max {
                        return Err(too_big());
                    }
                    out.extend_from_slice(&line);
                    out.extend_from_slice(b"\r\n");
                }
            }
        }
    }

    /// Sign in with a password.
    pub fn login(&mut self, user: &str, password: &str) -> Result<()> {
        self.ok(&format!("USER {user}")).map_err(|e| auth(&e))?;
        self.ok(&format!("PASS {password}")).map_err(|e| auth(&e))?;
        Ok(())
    }

    /// Sign in with the password as `method` says (issue #123): NTLM when asked for (Outlook's
    /// Secure Password Authentication), else `USER` and `PASS`, as left to MailCraft too.
    pub fn login_as(&mut self, user: &str, password: &str, method: mailcraft_model::SignInMethod) -> Result<()> {
        match method {
            mailcraft_model::SignInMethod::Ntlm => self.login_ntlm(user, password),
            _ => self.login(user, password),
        }
    }

    /// NTLM's messages through RFC 5034's `AUTH` exchange (as [MS-POP3] has it): the server's
    /// challenge answered, the password never sent.
    fn login_ntlm(&mut self, user: &str, password: &str) -> Result<()> {
        // As a password's refusal is read: a mailbox in use, or signed in to too soon again, waits.
        let refused = |l: &str| auth(&NetError::No(l.trim_start_matches("-ERR").trim().to_string()));
        // `+ <data>` goes on (its data), `+OK` and `-ERR` end it.
        let goes_on = |l: &str| l.strip_prefix('+').filter(|_| !l.starts_with("+OK")).map(|d| d.trim().to_string());
        self.send("AUTH NTLM")?;
        let l = self.line()?;
        if goes_on(&l).is_none() {
            return Err(refused(&l));
        }
        self.send(&mailcraft_model::base64::encode(&crate::ntlm::negotiate()))?;
        let l = self.line()?;
        let Some(challenge) = goes_on(&l) else { return Err(refused(&l)) };
        let answer =
            crate::ews::unb64(&challenge).ok_or_else(|| proto("bad NTLM challenge")).and_then(|c| crate::ntlm::answer(&c, user, password, None));
        let answer = match answer {
            Ok(a) => a,
            Err(e) => {
                // Cancelled, as RFC 5034 cancels: the server's `-ERR` read, the reason said.
                self.send("*")?;
                let _ = self.line();
                return Err(e);
            }
        };
        self.send(&mailcraft_model::base64::encode(&answer))?;
        let l = self.line()?;
        if l.starts_with("+OK") { Ok(()) } else { Err(refused(&l)) }
    }

    /// Sign in with an OAuth token (`AUTH XOAUTH2`).
    pub fn login_xoauth2(&mut self, user: &str, token: &str) -> Result<()> {
        self.ok(&format!("AUTH XOAUTH2 {}", crate::oauth::xoauth2(user, token))).map(drop).map_err(|e| auth(&e))
    }

    /// Message numbers and unique ids. Read a line at a time, the cap on their count: capped at
    /// 4 MB as a whole, ordinary ids passed it near 130,000 messages. More than the cap is an
    /// error rather than a list cut short, whose missing ids were forgotten as gone.
    pub fn uidl(&mut self) -> Result<Vec<(u32, String)>> {
        self.ok("UIDL")?;
        let mut out = Vec::new();
        let mut lines = 0usize;
        loop {
            let line = match self.piece(MAX_LINE)? {
                Piece::End => return Ok(out),
                Piece::TooLong => return Err(proto("a UIDL line too long")),
                Piece::Line(line) => line,
            };
            lines += 1;
            if lines > MAX_MESSAGES {
                return Err(proto(format!("the server lists more than {MAX_MESSAGES} messages")));
            }
            let line = String::from_utf8_lossy(&line);
            if let Some((n, id)) = line.trim().split_once(' ')
                && let Ok(n) = n.parse()
            {
                out.push((n, id.trim().chars().take(70).collect()));
            }
        }
    }

    /// Message numbers and their sizes in bytes (`LIST`), as the server reckons them: for showing
    /// how much is to come. Read as [`Client::uidl`] reads its list.
    pub fn list(&mut self) -> Result<Vec<(u32, u64)>> {
        self.ok("LIST")?;
        let mut out = Vec::new();
        loop {
            let line = match self.piece(MAX_LINE)? {
                Piece::End => return Ok(out),
                Piece::TooLong => return Err(proto("a LIST line too long")),
                Piece::Line(line) => line,
            };
            if out.len() >= MAX_MESSAGES {
                return Err(proto(format!("the server lists more than {MAX_MESSAGES} messages")));
            }
            let line = String::from_utf8_lossy(&line);
            let mut words = line.split_whitespace();
            if let (Some(Ok(n)), Some(Ok(size))) = (words.next().map(str::parse), words.next().map(str::parse)) {
                out.push((n, size));
            }
        }
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
    let user = acc.login_name();
    let mut c = Client::connect_trusting(cfg, stream::Trusted::of(acc))?;
    if acc.oauth.is_some() {
        c.login_xoauth2(user, secret)?;
    } else {
        c.login_as(user, secret, acc.sign_in)?;
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

/// [`fetch_new_as`] for an account as it is now (the tests').
#[cfg(test)]
pub fn fetch_new<S: Read + Write>(c: &mut Client<S>, store: &Store, account: &str, inbox: &str, keep: &Keep, now: i64) -> Result<SyncReport> {
    fetch_new_as(c, store, account, inbox, keep, now, store.removals(account), &mut |_| true)
}

/// How a download is going, as Send / Receive shows it: messages and their bytes done of those
/// new on the server (bytes as the server's `LIST` reckons them; none when it didn't say), and
/// how many are in the Inbox's list already (saved a batch at a time: the window shows them).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Fetching {
    pub done: u32,
    pub total: u32,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub saved: u32,
}

/// Download what's new into the Inbox, and remove from the server what the account says should go
/// (`keep`): at once, so many days after downloading, or once deleted here for good. For a worker
/// that started when its account had been removed `seen` times: a removal since has it write
/// nothing of the account again, nor remove anything from the server. `watch` is told how it's
/// going before each new message and at the end; answering `false`, the download stops there
/// (Cancel), what came before saved, as at the end.
#[expect(clippy::too_many_arguments, reason = "the download's account, place, rules and watcher, each its own")]
pub fn fetch_new_as<S: Read + Write>(
    c: &mut Client<S>,
    store: &Store,
    account: &str,
    inbox: &str,
    keep: &Keep,
    now: i64,
    seen: u64,
    watch: &mut dyn FnMut(&Fetching) -> bool,
) -> Result<SyncReport> {
    let mut state = load_state(store, account, now)?;
    let listed = c.uidl()?;
    let mut fs = store.load_folder(account, inbox)?;
    if fs.validity == 0 {
        fs.validity = 1;
        fs.uid_next = fs.uid_next.max(1);
    }
    settle(store, account, inbox, seen, &mut state, &fs)?;
    // The numbers the new mail takes are set aside first, the list saved so: a message recorded
    // but not yet listed (stopped part-way) keeps its number from any other filed here meanwhile.
    let mut new: HashSet<&str> = listed.iter().map(|(_, id)| id.as_str()).filter(|id| !state.contains_key(*id)).collect();
    // What's to come, and its size: the server's sizes asked for only when something is new (a
    // server that won't say is downloaded all the same, its size not shown).
    let sizes: HashMap<u32, u64> = if new.is_empty() { HashMap::new() } else { c.list().unwrap_or_default().into_iter().collect() };
    let mut going = Fetching { total: u32::try_from(new.len()).unwrap_or(u32::MAX), ..Fetching::default() };
    going.bytes_total = listed.iter().filter(|(_, id)| new.contains(id.as_str())).filter_map(|(n, _)| sizes.get(n)).sum();
    let next = fs.free_uid(account, inbox)?;
    if !new.is_empty() {
        let mut end = next;
        for _ in 0..new.len() {
            FolderState::take_uid(&mut end, account, inbox)?;
        }
        fs.uid_next = end;
        if_kept(store, account, seen, || store.save_folder(account, inbox, &fs))?;
    }
    let mut d = Download { store, account, inbox, seen, now, state, fs, next, came: Vec::new(), bytes: 0, since: Instant::now(), saved: 0 };
    let mut told = Told { going, sizes: &sizes, watch };
    if let Err(e) = d.fetch(c, &listed, &mut new, keep, &mut told) {
        // What came before the failure is kept; the failure is what's told.
        let _ = d.save();
        return Err(e);
    }
    for n in d.save()? {
        if keep.remove_now {
            c.dele(n)?;
        }
    }
    // Forget the ids the server no longer has (one removed now is still listed until QUIT, so it
    // goes from here next time).
    let on_server: HashSet<&str> = listed.iter().map(|(_, id)| id.as_str()).collect();
    d.state.retain(|id, _| on_server.contains(id.as_str()));
    if_kept(store, account, seen, || store.save_account_doc(account, STATE_DOC, &d.state))?;
    told.going.saved = u32::try_from(d.saved).unwrap_or(u32::MAX);
    (told.watch)(&told.going);
    Ok(SyncReport { new: d.saved, total: u32::try_from(d.fs.messages.len()).unwrap_or(u32::MAX), unread: d.fs.unread(), ..Default::default() })
}

/// A download stopped part-way (an error, or the app closed) after recording messages and before
/// the Inbox's list named them ([`Download::save`]): each recorded so that the list doesn't name
/// is forgotten, to come again, its source (if written) removed; one the list names, or moved or
/// deleted for good since, stays.
fn settle(store: &Store, account: &str, inbox: &str, seen: u64, state: &mut State, fs: &FolderState) -> Result<()> {
    if !state.values().any(|k| k.pending) {
        return Ok(());
    }
    let listed: HashSet<u32> = fs.messages.iter().filter(|m| m.key.validity == fs.validity).map(|m| m.key.uid).collect();
    let mut lost = Vec::new();
    state.retain(|_, k| {
        if !std::mem::take(&mut k.pending) || k.gone {
            return true;
        }
        match k.key.as_deref().and_then(MessageKey::from_id) {
            Some(key) if key.account == account && key.folder == inbox && key.validity == fs.validity && !listed.contains(&key.uid) => {
                lost.push(key);
                false
            }
            _ => true,
        }
    });
    if_kept(store, account, seen, || store.save_account_doc(account, STATE_DOC, state))?;
    store.while_kept(account, seen, || lost.iter().for_each(|key| store.remove_body(key)));
    Ok(())
}

/// The watcher of a download ([`fetch_new_as`]), and what it's told.
struct Told<'a> {
    going: Fetching,
    sizes: &'a HashMap<u32, u64>,
    watch: &'a mut dyn FnMut(&Fetching) -> bool,
}

/// A message downloaded and not yet saved.
struct Came {
    /// Its number on the server, this session.
    n: u32,
    id: String,
    summary: Summary,
    raw: Vec<u8>,
}

/// New mail on its way into the Inbox, saved a batch at a time ([`Download::save`]).
struct Download<'a> {
    store: &'a Store,
    account: &'a str,
    inbox: &'a str,
    seen: u64,
    now: i64,
    state: State,
    fs: FolderState,
    /// The next of the numbers set aside.
    next: u32,
    came: Vec<Came>,
    bytes: usize,
    since: Instant,
    saved: usize,
}

impl Download<'_> {
    /// Download what `new` names (each once), saving as it goes, and remove from the server what
    /// the account says should go; stopped before a message when `told`'s watcher says so.
    fn fetch<S: Read + Write>(
        &mut self,
        c: &mut Client<S>,
        listed: &[(u32, String)],
        new: &mut HashSet<&str>,
        keep: &Keep,
        told: &mut Told<'_>,
    ) -> Result<()> {
        for (n, id) in listed {
            if !new.remove(id.as_str()) {
                if let Some(k) = self.state.get(id)
                    && keep.removes(k, self.now)
                {
                    c.dele(*n)?;
                }
                continue;
            }
            told.going.saved = u32::try_from(self.saved).unwrap_or(u32::MAX);
            if !(told.watch)(&told.going) {
                return Ok(());
            }
            let raw = c.retr(*n)?;
            told.going.done = told.going.done.saturating_add(1);
            // As the server reckoned it, so it adds up to what was said was to come.
            let said = told.sizes.get(n).copied().unwrap_or(raw.len() as u64);
            told.going.bytes_done = told.going.bytes_done.saturating_add(said);
            let uid = FolderState::take_uid(&mut self.next, self.account, self.inbox)?;
            let key = MessageKey { account: self.account.to_string(), folder: self.inbox.to_string(), validity: self.fs.validity, uid };
            let size = u32::try_from(raw.len()).unwrap_or(u32::MAX);
            let summary = mime::summary_from_full(&raw, key, Flags::default(), size, 0);
            self.bytes = self.bytes.saturating_add(raw.len());
            self.came.push(Came { n: *n, id: id.clone(), summary, raw });
            if self.came.len() >= SAVE_EVERY || self.bytes >= SAVE_BYTES || self.since.elapsed() >= SAVE_AFTER {
                for n in self.save()? {
                    if keep.remove_now {
                        c.dele(n)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Save what came: its record first, each marked pending; then the sources and the list
    /// naming them; then the record again, no longer pending. Stopped anywhere between (an error,
    /// or the app closed), nothing is lost or listed twice: one recorded that the list doesn't
    /// name comes again ([`settle`]), its number given to no other. Returns the server's numbers
    /// of the messages saved (to remove there only once saved here).
    fn save(&mut self) -> Result<Vec<u32>> {
        self.since = Instant::now();
        if self.came.is_empty() {
            return Ok(Vec::new());
        }
        let came = std::mem::take(&mut self.came);
        self.bytes = 0;
        let (store, account, seen) = (self.store, self.account, self.seen);
        for c in &came {
            self.state.insert(c.id.clone(), Kept { at: self.now, key: Some(c.summary.key.to_id()), gone: false, pending: true });
        }
        if_kept(store, account, seen, || store.save_account_doc(account, STATE_DOC, &self.state))?;
        let packs = store.packs(account);
        let mut numbers = Vec::with_capacity(came.len());
        let mut ids = Vec::with_capacity(came.len());
        let mut sources = Vec::with_capacity(came.len());
        for c in came {
            numbers.push(c.n);
            ids.push(c.id);
            sources.push((c.summary.key.clone(), Store::prepare(packs, c.raw)));
            self.fs.messages.push(c.summary.into());
        }
        if_kept(store, account, seen, || store.write_prepared(sources, true))?;
        sort_newest_first(&mut self.fs.messages);
        if_kept(store, account, seen, || store.save_folder(account, self.inbox, &self.fs))?;
        for id in &ids {
            if let Some(k) = self.state.get_mut(id) {
                k.pending = false;
            }
        }
        if_kept(store, account, seen, || store.save_account_doc(account, STATE_DOC, &self.state))?;
        self.saved += numbers.len();
        Ok(numbers)
    }
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
        let uids: HashSet<u32> = keys.iter().filter(|k| k.folder == *folder).map(|k| k.uid).collect();
        let (going, staying): (Vec<_>, Vec<_>) = std::mem::take(&mut src.messages).into_iter().partition(|m| uids.contains(&m.key.uid));
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
pub(crate) mod fake;

#[cfg(test)]
mod tests;
