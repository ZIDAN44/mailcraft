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
//! MailCraft being the system's mail app or Open with (`args.rs`), which the running window
//! opens. The exchange is one line each way: a JSON request, `{"key": …, "open": […]}`, and `ok`.
//! The key is drawn afresh by each first MailCraft from the system's secure random source and
//! kept beside the socket in a file only the user can read (`.key`); a request without it is
//! refused, so another program can't start messages or add events through the socket (one that
//! can read the user's private files can already do far more).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

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
/// Longest key file read.
const MAX_KEY: u64 = 256;
/// How long either side waits for the other.
const TIMEOUT: Duration = Duration::from_secs(5);

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
    open: Mutex<Vec<String>>,
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
    pub fn take(&self) -> Option<Vec<String>> {
        let open = std::mem::take(&mut *self.asks.open.lock().unwrap_or_else(PoisonError::into_inner));
        let show = self.asks.show.swap(false, Ordering::AcqRel);
        (show || !open.is_empty()).then_some(open)
    }

    fn ask(&self, open: Vec<String>) {
        {
            let mut waiting = self.asks.open.lock().unwrap_or_else(PoisonError::into_inner);
            let room = MAX_WAITING.saturating_sub(waiting.len());
            waiting.extend(open.into_iter().take(room));
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

/// Decide what a launch on the data folder `data` does; `open` is what it hands over to a
/// running MailCraft (links, and files with absolute paths).
pub fn claim(data: &Path, open: &[String]) -> Claim {
    match socket_path(data) {
        Some(path) => claim_at(&path, open),
        None => Claim::Alone,
    }
}

/// The request a launch sends: the key, what it hands over, and a newline.
fn request(key: &str, open: &[String]) -> String {
    let open: Vec<&String> = open.iter().take(MAX_OPEN).collect();
    let mut line = serde_json::json!({ "key": key, "open": open }).to_string();
    line.push('\n');
    line
}

/// What a request hands over, or `None` when it isn't one with `key`: anything but one JSON
/// object with that key and a list of strings (empty ones and those past [`MAX_OPEN`] left out).
fn parse_request(line: &str, key: &str) -> Option<Vec<String>> {
    let value: serde_json::Value = serde_json::from_str(line.trim_end()).ok()?;
    let sent = value.get("key")?.as_str()?;
    if key.is_empty() || !keys_match(sent.as_bytes(), key.as_bytes()) {
        return None;
    }
    let open = value.get("open")?.as_array()?;
    let open = open.iter().map(|v| v.as_str().map(str::to_string)).collect::<Option<Vec<String>>>()?;
    Some(open.into_iter().filter(|s| !s.trim().is_empty()).take(MAX_OPEN).collect())
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

/// Stores `key` in the `.key` file next to the socket. Runs before the socket is bound, so by
/// the time a launch can connect, the key is there for it to read.
fn write_key(socket: &Path, key: &str) -> std::io::Result<()> {
    let key_file = socket.with_extension("key");
    // A key from a run that crashed would make the exclusive create below fail.
    let _ = std::fs::remove_file(&key_file);
    let mut file = owner_only_file(&key_file)?;
    std::io::Write::write_all(&mut file, key.as_bytes())
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

/// The Windows session (`SESSIONNAME`: `Console`, `RDP-Tcp#3`), or the display on Linux and the
/// BSDs (`WAYLAND_DISPLAY`, else `DISPLAY`): a launch reaches only a window on its own screen.
fn screen() -> String {
    let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    if cfg!(windows) {
        var("SESSIONNAME").unwrap_or_default()
    } else if cfg!(target_os = "macos") {
        String::new()
    } else {
        var("WAYLAND_DISPLAY").or_else(|| var("DISPLAY")).unwrap_or_default()
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
fn claim_at(path: &Path, open: &[String]) -> Claim {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // Released as this returns: by then this launch listens, or has been answered.
    let _election = election(path);
    // A third round for a socket that goes in between.
    for _ in 0..3 {
        match UnixStream::connect(path) {
            Ok(stream) => {
                return match hand_over(&stream, path, open) {
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
        // The key first: a launch that finds the socket finds its key too.
        let key = match new_key(path) {
            Ok(key) => key,
            Err(e) => {
                log::info!("single instance off: no key ({e})");
                return Claim::Alone;
            }
        };
        match UnixListener::bind(path) {
            Ok(listener) => return serve(listener, path, key),
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => continue,
            // Older Windows, a path too long for a socket, a file system without sockets.
            Err(e) => {
                log::info!("single instance off: can't listen at {} ({e})", path.display());
                return Claim::Alone;
            }
        }
    }
    Claim::Alone
}

/// A new key for this run, kept beside the socket.
fn new_key(socket: &Path) -> std::io::Result<String> {
    let key = mailcraft_net::oauth::secret_token().map_err(|e| std::io::Error::other(e.to_string()))?;
    write_key(socket, &key)?;
    Ok(key)
}

/// Ask the MailCraft at the other end to show its window and open `open`, and wait for its `ok`.
fn hand_over(stream: &UnixStream, socket: &Path, open: &[String]) -> std::io::Result<()> {
    use std::io::{BufRead, Read, Write};
    stream.set_read_timeout(Some(TIMEOUT))?;
    stream.set_write_timeout(Some(TIMEOUT))?;
    let key = read_key(socket)?;
    let mut writer = stream;
    writer.write_all(request(&key, open).as_bytes())?;
    let mut reply = String::new();
    std::io::BufReader::new(stream.take(16)).read_line(&mut reply)?;
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
    stream.set_read_timeout(Some(TIMEOUT))?;
    stream.set_write_timeout(Some(TIMEOUT))?;
    let mut line = String::new();
    std::io::BufReader::new(stream.take(MAX_REQUEST)).read_line(&mut line)?;
    let open = parse_request(&line, &server.key).ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "not a request with the key"))?;
    server.ask(open);
    let mut writer = stream;
    writer.write_all(OK.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Where a test's socket goes, in a folder of its own, its name kept short: a socket's whole
    /// path must fit its address (103 bytes on macOS, whose temporary folder alone takes about 50).
    fn socket_at(test: &str) -> PathBuf {
        std::env::temp_dir().join(format!("mci-{:x}-{test}", std::process::id())).join(socket_name(Path::new("C:/data"), ""))
    }

    /// That socket's folder, emptied.
    fn socket(test: &str) -> PathBuf {
        let path = socket_at(test);
        if let Some(dir) = path.parent() {
            let _ = std::fs::remove_dir_all(dir);
        }
        path
    }

    /// Every socket fits a Unix socket's address (`sun_path`: 104 bytes on macOS, 108 elsewhere,
    /// the NUL included). One that doesn't can't be listened at, and every launch opened a window
    /// of its own: the tests' did on the macOS CI runner, with a five-digit process id.
    #[test]
    fn socket_paths_fit_a_socket_address() {
        let app = socket_path(Path::new("/home/zoe/.local/share/mailcraft")).unwrap_or_default();
        for p in [app, socket_at("together"), socket_at("stale"), socket_at("silent"), socket_at("show"), socket_at("keyed"), socket_at("handed")] {
            let len = p.as_os_str().len();
            assert!(len < 104, "{len} bytes: {}", p.display());
        }
    }

    /// One data folder, one name; another folder (`--data`, `--sample`, a portable copy) or
    /// another screen, another name; never a path.
    #[test]
    fn each_data_folder_and_screen_has_its_own_socket() {
        let a = socket_name(Path::new("/home/zoe/.local/share/mailcraft"), "wayland-0");
        assert_eq!(a, socket_name(Path::new("/home/zoe/.local/share/mailcraft/"), "wayland-0"));
        assert_ne!(a, socket_name(Path::new("/home/zoe/.local/share/mailcraft/sample"), "wayland-0"));
        assert_ne!(a, socket_name(Path::new("/media/usb/MailCraftData"), "wayland-0"));
        assert_ne!(a, socket_name(Path::new("/home/zoe/.local/share/mailcraft"), "wayland-1"));
        assert_ne!(socket_name(Path::new("/d"), "RDP-Tcp#3"), socket_name(Path::new("/d"), "Console"));
        for odd in ["../../x/y", "RDP-Tcp#3", &"z".repeat(200), ""] {
            let n = socket_name(Path::new("/d"), odd);
            assert!(n.ends_with(".sock") && n.matches('.').count() == 1 && !n.contains(['/', '\\', '#']), "{n}");
            assert!(n.len() <= 16 + 1 + 24 + 5, "{n}");
        }
        #[cfg(windows)]
        assert_eq!(
            socket_name(Path::new(r"C:\Users\Zoe\AppData\Roaming\MailCraft"), ""),
            socket_name(Path::new(r"c:\users\zoe\appdata\roaming\mailcraft"), "")
        );
    }

    /// The first launch listens; a second asks it to show its window and exits, and the first
    /// sees the ask once; closing removes the socket, and the next launch is the first again.
    #[test]
    fn a_second_launch_shows_the_first() {
        let path = socket("show");
        let Claim::Primary(first) = claim_at(&path, &[]) else { panic!("the first launch listens") };
        assert!(first.take().is_none());
        assert!(matches!(claim_at(&path, &[]), Claim::Shown));
        assert!(matches!(claim_at(&path, &[]), Claim::Shown));
        assert_eq!(first.take(), Some(Vec::new()), "asked");
        assert!(first.take().is_none(), "once");
        // Woken when the window exists.
        let ctx = egui::Context::default();
        first.connect(&ctx);
        assert!(matches!(claim_at(&path, &[]), Claim::Shown));
        assert!(first.take().is_some());
        first.close();
        assert!(std::fs::symlink_metadata(&path).is_err(), "the socket goes");
        assert!(matches!(claim_at(&path, &[]), Claim::Primary(_)), "no one listens: this one is first");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// Several launches at once (a double click and then some): one window.
    #[test]
    fn launches_started_together_open_one_window() {
        let path = socket("together");
        let barrier = Arc::new(std::sync::Barrier::new(8));
        let launches: Vec<_> = (0..8)
            .map(|_| {
                let (path, barrier) = (path.clone(), Arc::clone(&barrier));
                std::thread::spawn(move || {
                    barrier.wait();
                    claim_at(&path, &[])
                })
            })
            .collect();
        let claims: Vec<Claim> = launches.into_iter().map(|t| t.join().unwrap()).collect();
        assert_eq!(claims.iter().filter(|c| matches!(c, Claim::Primary(_))).count(), 1, "one window");
        assert_eq!(claims.iter().filter(|c| matches!(c, Claim::Shown)).count(), 7, "the rest show it");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// A socket left by a MailCraft that didn't exit cleanly is replaced; something that isn't
    /// MailCraft at the other end, or junk sent to it, opens a window of its own.
    #[test]
    fn a_stale_socket_is_replaced_and_junk_is_refused() {
        let path = socket("stale");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        drop(UnixListener::bind(&path).unwrap());
        assert!(std::fs::symlink_metadata(&path).is_ok(), "left behind");
        let Claim::Primary(first) = claim_at(&path, &[]) else { panic!("replaced") };
        {
            use std::io::{Read, Write};
            let mut s = UnixStream::connect(&path).unwrap();
            s.write_all(b"open C:/x\n").unwrap();
            let mut reply = Vec::new();
            let _ = s.read_to_end(&mut reply);
            assert!(reply.is_empty(), "no ok for junk");
        }
        assert!(first.take().is_none());
        first.close();

        // A listener that never answers: a window of its own after the wait.
        let silent = socket("silent");
        std::fs::create_dir_all(silent.parent().unwrap()).unwrap();
        let _listener = UnixListener::bind(&silent).unwrap();
        assert!(matches!(claim_at(&silent, &[]), Claim::Alone));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
        let _ = std::fs::remove_dir_all(silent.parent().unwrap());
    }

    /// What a launch hands over reaches the running window, in order, once; the request carries
    /// the run's key, which is in a file beside the socket and new each run.
    #[test]
    fn a_launch_hands_over_its_links_and_files() {
        let path = socket("handed");
        let Claim::Primary(first) = claim_at(&path, &[]) else { panic!("the first launch listens") };
        let key = read_key(&path).unwrap();
        assert_eq!(key, first.key);
        assert!(key.len() >= 40, "{key}");
        let open = vec!["mailto:ana@x.example?subject=Hi".to_string(), "C:/Users/zoe/Downloads/Invite.ics".to_string()];
        assert!(matches!(claim_at(&path, &open), Claim::Shown));
        assert!(matches!(claim_at(&path, &["/home/zoe/x.eml".to_string()]), Claim::Shown));
        let mut all = open.clone();
        all.push("/home/zoe/x.eml".into());
        assert_eq!(first.take(), Some(all));
        assert!(first.take().is_none());
        first.close();
        assert!(std::fs::symlink_metadata(path.with_extension("key")).is_err(), "the key goes with the socket");
        let Claim::Primary(second) = claim_at(&path, &[]) else { panic!("first again") };
        assert_ne!(second.key, key, "a new key each run");
        second.close();
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// Another program on the computer can't open anything through the socket: a request without
    /// the key, with another key, or in the old form gets no `ok` and hands nothing over.
    #[test]
    fn requests_without_the_key_are_refused() {
        use std::io::{Read, Write};
        let path = socket("keyed");
        let Claim::Primary(first) = claim_at(&path, &[]) else { panic!("the first launch listens") };
        let guesses = [
            "show\n".to_string(),
            "{\"open\":[\"mailto:a@x.example\"]}\n".into(),
            request("", &["mailto:a@x.example".into()]),
            request("not-the-key", &["mailto:a@x.example".into()]),
            request(&format!("{}x", first.key), &["mailto:a@x.example".into()]),
            request(first.key.get(1..).unwrap(), &["mailto:a@x.example".into()]),
            format!("{{\"key\":\"{}\",\"open\":\"mailto:a@x.example\"}}\n", first.key),
            format!("{{\"key\":\"{}\",\"open\":[1]}}\n", first.key),
        ];
        for g in &guesses {
            let mut s = UnixStream::connect(&path).unwrap();
            s.write_all(g.as_bytes()).unwrap();
            let mut reply = Vec::new();
            let _ = s.read_to_end(&mut reply);
            assert!(reply.is_empty(), "no ok for {g:?}");
        }
        assert!(first.take().is_none(), "nothing handed over");
        // The right key: accepted.
        let mut s = UnixStream::connect(&path).unwrap();
        s.write_all(request(&first.key, &["mailto:a@x.example".into()]).as_bytes()).unwrap();
        let mut reply = String::new();
        let _ = s.read_to_string(&mut reply);
        assert_eq!(reply, OK);
        assert_eq!(first.take(), Some(vec!["mailto:a@x.example".to_string()]));
        first.close();
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// The request's parser: the key exactly, a list of strings, empties and extras left out.
    #[test]
    fn requests_are_read_strictly() {
        let open: Vec<String> = (0..100).map(|i| format!("/m/{i}.eml")).collect();
        let line = request("k3y", &open);
        assert!(line.ends_with('\n') && !line.trim_end().contains('\n'), "one line");
        assert_eq!(parse_request(&line, "k3y").map(|v| v.len()), Some(MAX_OPEN));
        assert_eq!(parse_request(&line, "k3Y"), None);
        assert_eq!(parse_request(&line, ""), None, "no key, no requests");
        assert_eq!(parse_request(&request("", &[]), ""), None);
        assert_eq!(parse_request("{\"key\":\"k\",\"open\":[\"\",\" \",\"a\"]}", "k"), Some(vec!["a".to_string()]));
        for junk in ["", "{}", "[]", "null", "{\"key\":\"k\"}", "{\"key\":5,\"open\":[]}", "\u{0}\u{ff}"] {
            assert_eq!(parse_request(junk, "k"), None, "{junk:?}");
        }
        assert!(keys_match(b"abc", b"abc") && !keys_match(b"abc", b"abd") && !keys_match(b"abc", b"ab"));
    }
}
