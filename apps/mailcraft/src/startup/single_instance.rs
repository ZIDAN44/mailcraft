//! One MailCraft per data folder: a second launch brings the running one forward and exits.
//!
//! Started again (a second click on the taskbar or Start, a `mailto:` handler, a desktop
//! shortcut), MailCraft opened a second window on the same data. Its account data files were
//! open in the first ("open in another copy of MailCraft"), so it showed errors or nothing, and
//! its log replaced the running one's. The first MailCraft now listens on a Unix domain socket;
//! a later launch on the same data folder asks it to show its window and leaves. Another data
//! folder (`--data`, `--sample`, a portable copy) is another MailCraft and opens its own window.
//!
//! As PdfCraft does (`apps/pdfcraft/src/single_instance.rs`): the socket is in a folder only the
//! user can reach (`store::scratch_dir`: the user's temporary folder on Windows and macOS,
//! `$XDG_RUNTIME_DIR` on Linux and the BSDs), so other accounts can't listen in MailCraft's
//! place; its name has the data folder's hash and the Windows session or the display, so a
//! launch never reaches a window on another screen. Launches started together take turns through
//! a lock file beside it, a socket left by a MailCraft that didn't exit cleanly is replaced, and
//! any failure falls back to the old behaviour: a window of its own. On Windows the socket comes
//! from `uds_windows` (std has no Unix sockets there, and Winsock directly would need `unsafe`).
//!
//! A launch also hands over what it was started with: `mailto:` links and mail files, from
//! MailCraft being the system's mail app or Open with, which the running window opens, and files
//! to send (`--attach`, `args.rs`), which it attaches to a new message. The exchange is one line
//! each way: a JSON request, `{"key": …, "open": […], "attach": […]}`, and `ok`.
//! The key is drawn afresh by each first MailCraft from the system's secure random source and
//! kept beside the socket in a file only the user can read (`.key`); a request without it is
//! refused, so another program can't start messages or add events through the socket (one that
//! can read the user's private files can already do far more).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use super::args::{Handed, MAX_ATTACHED};

#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};
#[cfg(windows)]
use uds_windows::{UnixListener, UnixStream};

/// What the running MailCraft answers.
const OK: &str = "ok\n";
/// Longest request read: links and paths, far more than a launch hands over.
const MAX_REQUEST: u64 = 1 << 20;
/// Most links and files one request hands over; more are left out.
const MAX_OPEN: usize = 64;
/// Most links and files waiting for the window.
const MAX_WAITING: usize = 256;
/// Most launches' files to send waiting for the window (a new message each).
const MAX_WAITING_SENDS: usize = 16;
/// Longest key file read.
const MAX_KEY: u64 = 256;
/// How long either side waits for the other.
const TIMEOUT: Duration = Duration::from_secs(5);
/// How often a launch asks again when the running MailCraft's key isn't in place, and how long
/// it waits before each.
const KEY_TRIES: usize = 5;
const KEY_WAIT: Duration = Duration::from_millis(40);

/// What this launch does.
pub enum Claim {
    /// The running MailCraft is showing its window: exit without one.
    Shown,
    /// The first MailCraft on this data: open the window and answer later launches.
    Primary(Server),
    /// A window of its own, answering nothing (as before).
    Alone,
}

/// Later launches' asks, and how to wake the window for them.
#[derive(Default)]
struct Asks {
    show: AtomicBool,
    handed: Mutex<Handed>,
    wake: Mutex<Option<egui::Context>>,
}

/// The first MailCraft's end of the socket: a thread answers launches as long as it runs.
pub struct Server {
    asks: Arc<Asks>,
    path: PathBuf,
    /// What every request must carry.
    key: String,
}

impl Server {
    /// Wake this window when a launch asks (one that asked before the window existed is shown
    /// on its first frames).
    pub fn connect(&self, ctx: &egui::Context) {
        *self.asks.wake.lock().unwrap_or_else(PoisonError::into_inner) = Some(ctx.clone());
    }

    /// What launches handed over since the last call (links and files, in order), or `None`
    /// when no launch asked: the window comes forward either way.
    pub fn take(&self) -> Option<Handed> {
        let handed = std::mem::take(&mut *self.asks.handed.lock().unwrap_or_else(PoisonError::into_inner));
        let show = self.asks.show.swap(false, Ordering::AcqRel);
        (show || !handed.is_empty()).then_some(handed)
    }

    fn ask(&self, handed: Handed) {
        {
            let mut waiting = self.asks.handed.lock().unwrap_or_else(PoisonError::into_inner);
            let room = MAX_WAITING.saturating_sub(waiting.open.len());
            waiting.open.extend(handed.open.into_iter().take(room));
            let room = MAX_WAITING_SENDS.saturating_sub(waiting.attach.len());
            waiting.attach.extend(handed.attach.into_iter().take(room));
        }
        self.asks.show.store(true, Ordering::Release);
        if let Some(ctx) = self.asks.wake.lock().unwrap_or_else(PoisonError::into_inner).as_ref() {
            ctx.request_repaint();
        }
    }

    /// Stop answering as MailCraft closes: the socket goes (a portable copy leaves no trace).
    pub fn close(&self) {
        let _ = std::fs::remove_file(&self.path);
        let _ = std::fs::remove_file(self.path.with_extension("lock"));
        let _ = std::fs::remove_file(self.path.with_extension("key"));
    }
}

/// Bring the window forward: out of the taskbar, in front. A Wayland window can't put itself in
/// front (winit ignores focus there), so it asks for the user's attention instead.
pub fn show(ctx: &egui::Context, native_wayland: bool) {
    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    if native_wayland {
        ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(egui::UserAttentionType::Informational));
    }
}

/// Decide what a launch on the data folder `data` does; `handed` is what it hands over to a
/// running MailCraft (links, and files with absolute paths).
pub fn claim(data: &Path, handed: &Handed) -> Claim {
    match socket_path(data) {
        Some(path) => claim_at(&path, handed),
        None => Claim::Alone,
    }
}

/// The request a launch sends: the key, what it hands over, and a newline. A launch has one group
/// of files to send at most (its `--attach`); a MailCraft from before 0.39 reads no `attach`.
fn request(key: &str, handed: &Handed) -> String {
    let open: Vec<&String> = handed.open.iter().take(MAX_OPEN).collect();
    let attach: Vec<&String> = handed.attach.iter().flatten().take(MAX_ATTACHED).collect();
    let mut line = serde_json::json!({ "key": key, "open": open, "attach": attach }).to_string();
    line.push('\n');
    line
}

/// What a request hands over, or `None` when it isn't one with `key`: anything but one JSON
/// object with that key, a list of strings to open and, if there is one, a list of strings to
/// send (empty ones, and those past [`MAX_OPEN`] or [`MAX_ATTACHED`], left out).
fn parse_request(line: &str, key: &str) -> Option<Handed> {
    let value: serde_json::Value = serde_json::from_str(line.trim_end()).ok()?;
    let sent = value.get("key")?.as_str()?;
    if key.is_empty() || !keys_match(sent.as_bytes(), key.as_bytes()) {
        return None;
    }
    let strings = |list: &serde_json::Value, most: usize| -> Option<Vec<String>> {
        let list = list.as_array()?.iter().map(|v| v.as_str().map(str::to_string)).collect::<Option<Vec<String>>>()?;
        Some(list.into_iter().filter(|s| !s.trim().is_empty()).take(most).collect())
    };
    let open = strings(value.get("open")?, MAX_OPEN)?;
    let attach = match value.get("attach") {
        Some(list) => strings(list, MAX_ATTACHED)?,
        None => Vec::new(),
    };
    Some(Handed::new(open, attach))
}

/// Compares a sent key with ours without stopping at the first wrong byte, so a caller timing
/// the answer learns nothing about how long a correct prefix it guessed.
fn keys_match(sent: &[u8], ours: &[u8]) -> bool {
    if sent.len() != ours.len() {
        return false;
    }
    let mut differing = 0u8;
    for (s, o) in sent.iter().zip(ours) {
        differing |= s ^ o;
    }
    differing == 0
}

/// Stores `key` in the file `key_file`, made afresh.
fn write_key(key_file: &Path, key: &str) -> std::io::Result<()> {
    // A key from a run that crashed would make the exclusive create below fail.
    let _ = std::fs::remove_file(key_file);
    let mut file = owner_only_file(key_file)?;
    std::io::Write::write_all(&mut file, key.as_bytes())
}

/// Where a launch keeps its key until it listens: a name of its own beside the socket (each
/// launch, threads in tests too, has its own).
fn pending_key(socket: &Path) -> PathBuf {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    socket.with_extension(format!("key-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)))
}

/// Listen at `socket` with a new key: `Ok(None)` when another launch listens there first. The
/// key is written under a name of this launch's own, and takes the `.key` name only once this
/// launch listens: written there first, a launch that lost the socket to another replaced the
/// key that one's later launches are checked against, and each of them opened a window of its
/// own. (A launch that connects between the bind and the rename, which the election lock keeps
/// out when it's had, finds no key or the one of before: it asks again, [`hand_over`].)
fn listen(socket: &Path) -> Result<Option<(UnixListener, String)>, String> {
    let pending = pending_key(socket);
    let key = new_key(&pending).map_err(|e| format!("no key ({e})"))?;
    let listener = match UnixListener::bind(socket) {
        Ok(listener) => listener,
        Err(e) => {
            let _ = std::fs::remove_file(&pending);
            return match e.kind() {
                std::io::ErrorKind::AddrInUse => Ok(None),
                // Older Windows, a path too long for a socket, a file system without sockets.
                _ => Err(format!("can't listen at {} ({e})", socket.display())),
            };
        }
    };
    let key_file = socket.with_extension("key");
    // Windows replaces a file by renaming only when nothing has it open (a launch reading it).
    let placed = std::fs::rename(&pending, &key_file).or_else(|_| {
        let _ = std::fs::remove_file(&key_file);
        std::fs::rename(&pending, &key_file)
    });
    if let Err(e) = placed {
        drop(listener);
        let _ = std::fs::remove_file(socket);
        let _ = std::fs::remove_file(&pending);
        return Err(format!("the key wasn't put in place ({e})"));
    }
    Ok(Some((listener, key)))
}

/// Creates `path`, failing if something is already there; on Unix with mode 0600, so other
/// users can't read it (on Windows the per-user scratch folder already keeps them out).
fn owner_only_file(path: &Path) -> std::io::Result<std::fs::File> {
    let mut how = std::fs::OpenOptions::new();
    how.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        how.mode(0o600);
    }
    how.open(path)
}

/// The running MailCraft's key, from beside its socket.
fn read_key(socket: &Path) -> std::io::Result<String> {
    use std::io::Read;
    let mut key = String::new();
    std::fs::File::open(socket.with_extension("key"))?.take(MAX_KEY).read_to_string(&mut key)?;
    Ok(key.trim().to_string())
}

/// The socket for `data` in this user's private scratch folder.
fn socket_path(data: &Path) -> Option<PathBuf> {
    let dir = mailcraft_engine::store::scratch_dir("instance")?;
    Some(dir.join(socket_name(data, &screen())))
}

/// The Windows session, or the display on Linux and the BSDs (`WAYLAND_DISPLAY`, else
/// `DISPLAY`): a launch reaches only a window on its own screen.
fn screen() -> String {
    let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    if cfg!(windows) {
        windows_session(session_id(), var("SESSIONNAME").as_deref())
    } else if cfg!(target_os = "macos") {
        String::new()
    } else {
        var("WAYLAND_DISPLAY").or_else(|| var("DISPLAY")).unwrap_or_default()
    }
}

/// The Windows session this process runs in, from its access token: the console, each Remote
/// Desktop sign-in and session 0 (services) each have their own. `None` if Windows won't say.
#[cfg(windows)]
fn session_id() -> Option<u32> {
    use winsafe::{HACCESSTOKEN, TokenInfo, co::TOKEN_INFORMATION_CLASS};
    match HACCESSTOKEN::GetCurrentProcessToken().GetTokenInformation(TOKEN_INFORMATION_CLASS::SessionId) {
        Ok(TokenInfo::SessionId(id)) => Some(*id),
        _ => None,
    }
}

#[cfg(not(windows))]
fn session_id() -> Option<u32> {
    None
}

/// The Windows session as the socket names it: its id (`session`). A launch from Task Scheduler,
/// a service or a remote shell has no `SESSIONNAME` but still runs in a session, so it finds that
/// session's window instead of opening a second MailCraft on the same data; and a window in
/// another session (the same user at the console and over Remote Desktop) is never reached. Only
/// when the id can't be read does `SESSIONNAME` (`Console`, `RDP-Tcp#3`…) name it (as PdfCraft's
/// single instance).
fn windows_session(session: Option<u32>, session_name: Option<&str>) -> String {
    match session {
        Some(id) => format!("s{id}"),
        None => session_name.unwrap_or_default().to_string(),
    }
}

/// The socket's file name: the data folder's hash (its path, case folded on Windows and macOS,
/// whose file systems mostly are) and the screen, letters, digits, `-` and `_` only.
fn socket_name(data: &Path, screen: &str) -> String {
    let data = std::path::absolute(data).unwrap_or_else(|_| data.to_path_buf());
    let mut key = data.to_string_lossy().trim_end_matches(['/', '\\']).to_string();
    if cfg!(any(windows, target_os = "macos")) {
        key = key.to_lowercase();
    }
    let screen: String = screen.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_')).collect();
    let screen: String = screen.get(screen.len().saturating_sub(24)..).unwrap_or_default().to_string();
    let hash = mailcraft_engine::store::fnv64(&key);
    if screen.is_empty() { format!("{hash:016x}.sock") } else { format!("{hash:016x}-{screen}.sock") }
}

/// Held while a launch finds out whether it's the first, so launches started together take
/// turns: the first listens before the next looks. Released by the system however the process
/// ends. Best effort: without it the launch goes on.
fn election(socket: &Path) -> Option<std::fs::File> {
    let file = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(socket.with_extension("lock")).ok()?;
    let deadline = std::time::Instant::now() + TIMEOUT;
    loop {
        match file.try_lock() {
            Ok(()) => return Some(file),
            Err(std::fs::TryLockError::WouldBlock) if std::time::Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            Err(e) => {
                log::info!("single instance: no election lock ({e})");
                return None;
            }
        }
    }
}

/// [`claim`] with the socket at `path`.
fn claim_at(path: &Path, handed: &Handed) -> Claim {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // Released as this returns: by then this launch listens, or has been answered.
    let _election = election(path);
    // A third round for a socket that goes in between.
    for _ in 0..3 {
        match UnixStream::connect(path) {
            Ok(stream) => {
                return match hand_over(stream, path, handed, || std::thread::sleep(KEY_WAIT)) {
                    Ok(()) => Claim::Shown,
                    Err(e) => {
                        log::warn!("the running MailCraft didn't answer ({e}); opening a window");
                        Claim::Alone
                    }
                };
            }
            // Nobody listens: no socket yet, or one left by a MailCraft that didn't exit cleanly.
            // Removed without asking `exists()` first: on Windows that follows the socket's
            // reparse point, which can fail.
            Err(_) => match std::fs::remove_file(path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                // Another launch bound it and is about to listen: try it again.
                Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                    std::thread::sleep(Duration::from_millis(50));
                    continue;
                }
                Err(e) => {
                    log::info!("single instance off: can't clear {} ({e})", path.display());
                    return Claim::Alone;
                }
            },
        }
        match listen(path) {
            Ok(Some((listener, key))) => return serve(listener, path, key),
            // Another launch listens there now: asked in the next round.
            Ok(None) => continue,
            Err(e) => {
                log::info!("single instance off: {e}");
                return Claim::Alone;
            }
        }
    }
    Claim::Alone
}

/// A new key for this run, written to `key_file`.
fn new_key(key_file: &Path) -> std::io::Result<String> {
    let key = mailcraft_net::oauth::secret_token().map_err(|e| std::io::Error::other(e.to_string()))?;
    write_key(key_file, &key)?;
    Ok(key)
}

/// Ask the MailCraft at the other end of `stream` to show its window and take `handed`, again
/// (after `wait`, a few times) while its key isn't in place: it listens before its key has its
/// name ([`listen`]), so a launch that reaches it in between, without the election lock, finds
/// no key or the one of the run before, was refused and opened a second window. One that doesn't
/// answer in time isn't asked again.
fn hand_over(stream: UnixStream, socket: &Path, handed: &Handed, mut wait: impl FnMut()) -> std::io::Result<()> {
    let mut stream = stream;
    for _ in 1..KEY_TRIES {
        match hand_over_once(&stream, socket, handed) {
            Err(e) if !matches!(e.kind(), std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock) => {
                log::info!("single instance: not answered ({e}); asking again");
                drop(stream);
                wait();
                stream = UnixStream::connect(socket)?;
            }
            done => return done,
        }
    }
    hand_over_once(&stream, socket, handed)
}

/// How long one read on the socket waits before it's made again, until [`TIMEOUT`]. Windows can
/// miss waking a read for a close that comes just as the read starts (about one in four hundred),
/// as a launch that asks again closes a connection it sent nothing on; data always wakes it. The
/// read then waited out all of [`TIMEOUT`], and the ask behind it timed out and opened a second
/// window. A read made again sees the close at once.
const READ_SLICE: Duration = Duration::from_millis(50);

/// Reads `stream` until `TIMEOUT` from when it's made, [`READ_SLICE`] at a time.
struct Patient<'a> {
    stream: &'a UnixStream,
    until: std::time::Instant,
}

impl<'a> Patient<'a> {
    fn new(stream: &'a UnixStream) -> Self {
        Patient { stream, until: std::time::Instant::now() + TIMEOUT }
    }

    /// What can be read without waiting: never a wait with no timeout set.
    fn read_now(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.stream.set_nonblocking(true)?;
        let read = std::io::Read::read(&mut self.stream, buf);
        self.stream.set_nonblocking(false)?;
        read
    }
}

impl std::io::Read for Patient<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        use std::io::ErrorKind::{Interrupted, TimedOut, WouldBlock};
        loop {
            let left = self.until.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() {
                return Err(TimedOut.into());
            }
            // A timeout of zero is refused: at least a millisecond.
            if let Err(refused) = self.stream.set_read_timeout(Some(left.min(READ_SLICE).max(Duration::from_millis(1)))) {
                // macOS refuses a timeout on a socket whose other end has closed ("invalid
                // argument"): what's there is read without waiting, the close as the end it is.
                return self.read_now(buf).map_err(|_| refused);
            }
            match std::io::Read::read(&mut self.stream, buf) {
                Err(e) if matches!(e.kind(), TimedOut | WouldBlock | Interrupted) => {}
                read => return read,
            }
        }
    }
}

/// Ask once, with the key beside the socket, and wait for the `ok`.
fn hand_over_once(stream: &UnixStream, socket: &Path, handed: &Handed) -> std::io::Result<()> {
    use std::io::{BufRead, Read, Write};
    stream.set_write_timeout(Some(TIMEOUT))?;
    let key = read_key(socket)?;
    let mut writer = stream;
    writer.write_all(request(&key, handed).as_bytes())?;
    let mut reply = String::new();
    std::io::BufReader::new(Patient::new(stream).take(16)).read_line(&mut reply)?;
    if reply == OK { Ok(()) } else { Err(std::io::Error::other("no ok")) }
}

/// Answer launches on a thread of its own for as long as the process runs.
fn serve(listener: UnixListener, path: &Path, key: String) -> Claim {
    let server = Server { asks: Arc::default(), path: path.to_path_buf(), key: key.clone() };
    let answering = Server { asks: Arc::clone(&server.asks), path: path.to_path_buf(), key };
    let started = std::thread::Builder::new().name("single-instance".into()).spawn(move || {
        for stream in listener.incoming() {
            if let Err(e) = stream.and_then(|s| answer(&s, &answering)) {
                log::warn!("single instance: a launch's ask failed ({e})");
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    });
    match started {
        Ok(_) => Claim::Primary(server),
        Err(e) => {
            log::warn!("single instance off: no thread to answer ({e})");
            Claim::Alone
        }
    }
}

/// Read one launch's request (anything without the key is refused), note it and confirm.
fn answer(stream: &UnixStream, server: &Server) -> std::io::Result<()> {
    use std::io::{BufRead, Read, Write};
    stream.set_write_timeout(Some(TIMEOUT))?;
    let mut line = String::new();
    std::io::BufReader::new(Patient::new(stream).take(MAX_REQUEST)).read_line(&mut line)?;
    let handed =
        parse_request(&line, &server.key).ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "not a request with the key"))?;
    server.ask(handed);
    let mut writer = stream;
    writer.write_all(OK.as_bytes())
}

#[cfg(test)]
mod tests;
