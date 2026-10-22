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
//! any failure falls back to the old behaviour: a window of its own. The exchange is one line each
//! way, `show` and `ok`. On Windows the socket comes from `uds_windows` (std has no Unix sockets
//! there, and Winsock directly would need `unsafe`).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};
#[cfg(windows)]
use uds_windows::{UnixListener, UnixStream};

/// What a launch asks: show the window.
const SHOW: &str = "show\n";
/// What the running MailCraft answers.
const OK: &str = "ok\n";
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
    wake: Mutex<Option<egui::Context>>,
}

/// The first MailCraft's end of the socket: a thread answers launches as long as it runs.
pub struct Server {
    asks: Arc<Asks>,
    path: PathBuf,
}

impl Server {
    /// Wake this window when a launch asks (one that asked before the window existed is shown
    /// on its first frames).
    pub fn connect(&self, ctx: &egui::Context) {
        *self.asks.wake.lock().unwrap_or_else(PoisonError::into_inner) = Some(ctx.clone());
    }

    /// Whether a launch asked for the window since the last call.
    pub fn take_show(&self) -> bool {
        self.asks.show.swap(false, Ordering::AcqRel)
    }

    fn ask(&self) {
        self.asks.show.store(true, Ordering::Release);
        if let Some(ctx) = self.asks.wake.lock().unwrap_or_else(PoisonError::into_inner).as_ref() {
            ctx.request_repaint();
        }
    }

    /// Stop answering as MailCraft closes: the socket goes (a portable copy leaves no trace).
    pub fn close(&self) {
        let _ = std::fs::remove_file(&self.path);
        let _ = std::fs::remove_file(self.path.with_extension("lock"));
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

/// Decide what a launch on the data folder `data` does.
pub fn claim(data: &Path) -> Claim {
    match socket_path(data) {
        Some(path) => claim_at(&path),
        None => Claim::Alone,
    }
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
fn claim_at(path: &Path) -> Claim {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // Released as this returns: by then this launch listens, or has been answered.
    let _election = election(path);
    // A third round for a socket that goes in between.
    for _ in 0..3 {
        match UnixStream::connect(path) {
            Ok(stream) => {
                return match ask_to_show(&stream) {
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
        match UnixListener::bind(path) {
            Ok(listener) => return serve(listener, path),
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

/// Ask the MailCraft at the other end to show its window, and wait for its `ok`.
fn ask_to_show(stream: &UnixStream) -> std::io::Result<()> {
    use std::io::{BufRead, Read, Write};
    stream.set_read_timeout(Some(TIMEOUT))?;
    stream.set_write_timeout(Some(TIMEOUT))?;
    let mut writer = stream;
    writer.write_all(SHOW.as_bytes())?;
    let mut reply = String::new();
    std::io::BufReader::new(stream.take(16)).read_line(&mut reply)?;
    if reply == OK { Ok(()) } else { Err(std::io::Error::other("no ok")) }
}

/// Answer launches on a thread of its own for as long as the process runs.
fn serve(listener: UnixListener, path: &Path) -> Claim {
    let server = Server { asks: Arc::default(), path: path.to_path_buf() };
    let answering = Server { asks: Arc::clone(&server.asks), path: path.to_path_buf() };
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

/// Read one launch's ask (anything else is refused), note it and confirm.
fn answer(stream: &UnixStream, server: &Server) -> std::io::Result<()> {
    use std::io::{BufRead, Read, Write};
    stream.set_read_timeout(Some(TIMEOUT))?;
    stream.set_write_timeout(Some(TIMEOUT))?;
    let mut line = String::new();
    std::io::BufReader::new(stream.take(64)).read_line(&mut line)?;
    if line != SHOW {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "not a show request"));
    }
    server.ask();
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
        for p in [app, socket_at("together"), socket_at("stale"), socket_at("silent"), socket_at("show")] {
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
        let Claim::Primary(first) = claim_at(&path) else { panic!("the first launch listens") };
        assert!(!first.take_show());
        assert!(matches!(claim_at(&path), Claim::Shown));
        assert!(matches!(claim_at(&path), Claim::Shown));
        assert!(first.take_show(), "asked");
        assert!(!first.take_show(), "once");
        // Woken when the window exists.
        let ctx = egui::Context::default();
        first.connect(&ctx);
        assert!(matches!(claim_at(&path), Claim::Shown));
        assert!(first.take_show());
        first.close();
        assert!(std::fs::symlink_metadata(&path).is_err(), "the socket goes");
        assert!(matches!(claim_at(&path), Claim::Primary(_)), "no one listens: this one is first");
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
                    claim_at(&path)
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
        let Claim::Primary(first) = claim_at(&path) else { panic!("replaced") };
        {
            use std::io::{Read, Write};
            let mut s = UnixStream::connect(&path).unwrap();
            s.write_all(b"open C:/x\n").unwrap();
            let mut reply = Vec::new();
            let _ = s.read_to_end(&mut reply);
            assert!(reply.is_empty(), "no ok for junk");
        }
        assert!(!first.take_show());
        first.close();

        // A listener that never answers: a window of its own after the wait.
        let silent = socket("silent");
        std::fs::create_dir_all(silent.parent().unwrap()).unwrap();
        let _listener = UnixListener::bind(&silent).unwrap();
        assert!(matches!(claim_at(&silent), Claim::Alone));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
        let _ = std::fs::remove_dir_all(silent.parent().unwrap());
    }
}
