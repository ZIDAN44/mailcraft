//! AI agents in the open window (issue #255): while the user lets them in (File › Options › AI
//! agents), MailCraft serves the Craft apps' control channel on loopback (`craft-control`), behind
//! a token made afresh each time in an owner-only file, and publishes where in its data folder
//! (`agent-connection.json`: the port, its process id and the token file, no secret), where
//! `mailcraft-cli mcp` finds it before each tool call. Turned off, or as MailCraft closes, it stops
//! listening and the file goes.
//!
//! Each request is handed to the window ([`MailApp::agent_request`]): reading answers at once; a
//! command waits for the user's Allow or Deny, its reply sent once they answered; a picture of
//! the window is taken over the next frames. MailCraft's own switch decides, not the ArtCraft
//! Launcher's list of Craft apps (MailCraft isn't one of them).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use craft_control::agents::{
    CONNECTION_FILE, Connection, PROBE_TIMEOUT, TOKEN_FILE, probe, read_connection, remove_connection, remove_stale_connection, write_connection,
};
use craft_control::server::{Hub, Protocol, Stop, Summary, Waits};
use craft_control::{ControlRequest, Effect};
use mailcraft_ui_egui::{AgentAnswer, MailApp};
use serde_json::{Value, json};

/// Longest request line taken (a command's params).
const MAX_LINE: usize = 4 << 20;
/// The most pictures of the window waiting at once.
const SHOTS_MAX: usize = 4;
/// How long a command waits for the user's answer: then it's refused, nothing run.
const ASK_WAIT: Duration = Duration::from_secs(600);

/// The data folder the token file goes in, for `craft-control`'s [`craft_control::auth::App`].
static DATA: OnceLock<PathBuf> = OnceLock::new();

fn app() -> craft_control::auth::App {
    craft_control::auth::App {
        name: "MailCraft",
        data_dir: || DATA.get().cloned(),
        env: |name| std::env::var_os(format!("MAILCRAFT_{name}")),
        utc: utc_text,
        default_port: 0,
    }
}

/// A time in seconds since the Unix epoch as UTC (`2026-11-23T09:30:00Z`), as the channel's
/// messages give times.
fn utc_text(secs: u64) -> String {
    let secs = i64::try_from(secs).unwrap_or(i64::MAX);
    let (y, m, d) = mailcraft_engine::model::dates::civil_from_days(secs.div_euclid(86_400));
    let t = secs.rem_euclid(86_400);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", t / 3600, t % 3600 / 60, t % 60)
}

/// What MailCraft's requests mean to the channel.
struct Mail;

impl Protocol for Mail {
    fn summary(&self, method: &str, params: &Value) -> Summary {
        let command = params.get("command").and_then(Value::as_str).unwrap_or_default();
        let what = if method == "engine.execute" { format!("`{method}` ({command})") } else { format!("`{method}`") };
        let effect = if method == "engine.execute" { Effect::Changes } else { Effect::ReadsOnly };
        Summary { what, effect, command: None, steps: None, long: method == "engine.execute" }
    }
    fn document(&self) -> &'static str {
        "mailbox"
    }
    fn change_event(&self) -> &'static str {
        "mailbox.changed"
    }
    fn revision_method(&self) -> &'static str {
        "mailbox.revision"
    }
    fn docs(&self) -> &'static str {
        "docs/mcp.md"
    }
    fn capabilities(&self) -> Value {
        json!({"methods": ["agents.read", "engine.commands", "engine.execute", "ui.inspect", "ui.screenshot"]})
    }
}

/// The channel while it listens.
struct Listening {
    stop: Stop,
    requests: Receiver<ControlRequest>,
    token_file: PathBuf,
    token: String,
}

/// The control channel, and the requests waiting for the user or for a picture.
pub(crate) struct Channel {
    dir: Option<PathBuf>,
    listening: Option<Listening>,
    /// Commands waiting for the user's answer, by the window's ticket, and since when.
    asking: Vec<(u64, ControlRequest, Instant)>,
    /// Pictures of the window asked for and not taken yet.
    shots: Arc<AtomicUsize>,
    /// It couldn't listen: not tried again until the setting changes.
    failed: bool,
}

impl Channel {
    /// The channel for the data folder `dir` (none: never listens); the files a MailCraft that
    /// didn't close cleanly left there go: its connection file, and its token with no connection
    /// beside it (a live one, another window's, keeps its own).
    pub(crate) fn new(dir: Option<PathBuf>) -> Channel {
        if let Some(d) = &dir {
            let _ = DATA.set(d.clone());
            remove_stale_connection(d);
            if !d.join(CONNECTION_FILE).exists() {
                let token_file = d.join(TOKEN_FILE);
                let small = std::fs::metadata(&token_file).is_ok_and(|m| m.len() <= 4096);
                if let Some(token) = small.then(|| std::fs::read_to_string(&token_file).ok()).flatten() {
                    craft_control::auth::remove_token_file(&token_file, &token);
                }
            }
        }
        Channel { dir, listening: None, asking: Vec::new(), shots: Arc::default(), failed: false }
    }

    /// Each frame: listening as the setting says, requests handed to the window, answers sent.
    pub(crate) fn follow(&mut self, app: &mut MailApp, ctx: &egui::Context) {
        let wanted = app.session.prefs.agents;
        match (&self.listening, wanted) {
            (None, true) if !self.failed => self.start(ctx),
            (Some(_), false) => {
                self.stop();
                app.agents_refuse_all(mailcraft_ui_egui::AGENTS_OFF);
            }
            (None, false) => self.failed = false,
            _ => {}
        }
        self.answer_decided(app);
        self.withdraw_stale(app);
        let Some(l) = &self.listening else { return };
        let requests: Vec<ControlRequest> = l.requests.try_iter().take(64).collect();
        for req in requests {
            // One the agent cancelled on its way here isn't run (the channel answered it).
            if req.ticket.is_cancelled() {
                continue;
            }
            req.started.store(true, Ordering::Release);
            if req.method == "ui.screenshot" {
                if self.shots.load(Ordering::Acquire) >= SHOTS_MAX {
                    reply(&req, Err("too many pictures of the window are waiting".into()));
                } else if let Err(e) = app.agent_may_picture() {
                    reply(&req, Err(e));
                } else {
                    // Taken once the next frame is drawn: sent from there.
                    self.shots.fetch_add(1, Ordering::AcqRel);
                    let shots = Arc::clone(&self.shots);
                    ctx.request_screenshot(move |image| {
                        shots.fetch_sub(1, Ordering::AcqRel);
                        reply(&req, png(&image).map(|(data, w, h)| json!({"png": base64(&data), "width": w, "height": h})));
                    });
                    ctx.request_repaint();
                }
                continue;
            }
            match app.agent_request(&req.method, &req.params) {
                AgentAnswer::Now(r) => reply(&req, r),
                // Asked about (the window brings itself forward and says so, `agents_asking_window`),
                // or a server's answer awaited: answered once decided.
                AgentAnswer::Asking(ticket) => self.asking.push((ticket, req, Instant::now())),
            }
        }
        if !self.asking.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }
    }

    fn answer_decided(&mut self, app: &mut MailApp) {
        for (ticket, answer) in app.agent_decisions() {
            if let Some(i) = self.asking.iter().position(|(t, ..)| *t == ticket) {
                let (_, req, _) = self.asking.remove(i);
                reply(&req, answer);
            }
        }
    }

    /// The commands asked about that may no longer run are taken back from the window: those
    /// the agent cancelled (`control.cancel`), and those unanswered for [`ASK_WAIT`].
    fn withdraw_stale(&mut self, app: &mut MailApp) {
        let mut kept = Vec::with_capacity(self.asking.len());
        for (ticket, req, since) in self.asking.drain(..) {
            if req.ticket.is_cancelled() {
                app.agent_withdraw(ticket);
            } else if since.elapsed() >= ASK_WAIT {
                app.agent_withdraw(ticket);
                reply(&req, Err("the user didn't answer within ten minutes: nothing ran".into()));
            } else {
                kept.push((ticket, req, since));
            }
        }
        self.asking = kept;
    }

    fn start(&mut self, ctx: &egui::Context) {
        let Some(dir) = self.dir.clone() else {
            self.failed = true;
            return;
        };
        match listen(&dir, ctx) {
            Ok(l) => {
                log::info!("AI agents: listening on 127.0.0.1:{}", l.stop.port());
                self.listening = Some(l);
            }
            Err(e) => {
                log::warn!("AI agents: can't listen ({e})");
                self.failed = true;
            }
        }
    }

    /// Stop listening: the connection file and the token go, and every request still waiting is
    /// refused.
    pub(crate) fn stop(&mut self) {
        let Some(l) = self.listening.take() else { return };
        // Answered first: stopping ends the connections they'd be answered on (once the port is
        // closed, which gives the answers time to go out).
        for (_, req, _) in self.asking.drain(..) {
            reply(&req, Err(mailcraft_ui_egui::AGENTS_OFF.into()));
        }
        if let Some(dir) = &self.dir {
            remove_connection(dir, std::process::id());
        }
        craft_control::auth::remove_token_file(&l.token_file, &l.token);
        l.stop.stop();
    }
}

/// Listen on a free loopback port with a new token, the token in an owner-only file and the
/// connection file published in `dir`.
fn listen(dir: &Path, ctx: &egui::Context) -> Result<Listening, String> {
    // Another MailCraft on this data folder that lets agents in already keeps them: its token
    // and connection file aren't written over (a second window, where the single-instance check
    // can't keep one out, may be running without the mail).
    if let Ok(Some(c)) = read_connection(dir)
        && c.pid != std::process::id()
        && probe(c.port, PROBE_TIMEOUT)
    {
        return Err(format!("another MailCraft (process {}) lets AI agents in on this data folder", c.pid));
    }
    let token = craft_control::server::new_token().ok_or("no token: the system's random source didn't answer")?;
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let token_file = dir.join(TOKEN_FILE);
    app().write_token_file(&token_file, &token).map_err(|e| e.to_string())?;
    let wake = ctx.clone();
    // Patient: a command waits for the user's answer, which comes within [`ASK_WAIT`] (refused
    // then), so the channel waits a little longer than that rather than its usual hour.
    let waits = Waits { long: ASK_WAIT + Duration::from_secs(60), ..Waits::default() };
    let hub = Arc::new(Hub::new(Mail, move || wake.request_repaint(), MAX_LINE).with_waits(waits));
    let (tx, requests) = channel();
    let stop = hub.listen_stoppable(listener, token.clone(), tx, true).map_err(|e| e.to_string())?;
    let version = format!("MailCraft {}", env!("CARGO_PKG_VERSION"));
    if let Err(e) = write_connection(dir, &Connection::new(port, token_file.clone(), version)) {
        stop.stop();
        craft_control::auth::remove_token_file(&token_file, &token);
        return Err(e.to_string());
    }
    Ok(Listening { stop, requests, token_file, token })
}

fn reply(req: &ControlRequest, r: Result<Value, String>) {
    let v = match r {
        Ok(result) => json!({"ok": true, "result": result}),
        Err(error) => json!({"ok": false, "error": error}),
    };
    let _ = req.reply.send(v);
}

/// A picture of the window as PNG, and its size.
fn png(image: &egui::ColorImage) -> Result<(Vec<u8>, usize, usize), String> {
    let [w, h] = image.size;
    let rgba: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_srgba_unmultiplied()).collect();
    let buffer = image::RgbaImage::from_raw(u32::try_from(w).map_err(|e| e.to_string())?, u32::try_from(h).map_err(|e| e.to_string())?, rgba)
        .ok_or("the picture's size doesn't match its pixels")?;
    let mut out = std::io::Cursor::new(Vec::new());
    buffer.write_to(&mut out, image::ImageFormat::Png).map_err(|e| e.to_string())?;
    Ok((out.into_inner(), w, h))
}

/// Standard base64 with padding (RFC 4648), as MCP's image content carries a picture.
fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk.first().copied().unwrap_or(0), chunk.get(1).copied().unwrap_or(0), chunk.get(2).copied().unwrap_or(0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for (i, shift) in [18u32, 12, 6, 0].into_iter().enumerate() {
            let c = T.get(((n >> shift) & 63) as usize).copied().unwrap_or(b'=');
            out.push(if i <= chunk.len() { char::from(c) } else { '=' });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_as_rfc_4648_says() {
        for (data, want) in [
            (&b""[..], ""),
            (b"f", "Zg=="),
            (b"fo", "Zm8="),
            (b"foo", "Zm9v"),
            (b"foob", "Zm9vYg=="),
            (b"fooba", "Zm9vYmE="),
            (b"foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(data), want);
        }
    }

    /// Turned off, nothing of the channel stays in the data folder: no connection file, no token,
    /// and no process file beside the token. Nor does what a MailCraft that crashed left.
    #[test]
    fn the_channel_leaves_no_files_behind() {
        let dir = std::env::temp_dir().join(format!("mailcraft-agents-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let left = |dir: &Path| {
            let mut names: Vec<String> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
            names.sort();
            names
        };
        let ctx = egui::Context::default();
        let mut c = Channel::new(Some(dir.clone()));
        c.start(&ctx);
        assert!(c.listening.is_some() && !c.failed);
        assert_eq!(left(&dir), ["agent-connection.json", "agent-control.pid", "agent-control.token"]);
        c.stop();
        assert_eq!(left(&dir), Vec::<String>::new());
        // Crashed while listening: the connection file names a port no one listens on any more.
        c.start(&ctx);
        if let Some(l) = c.listening.take() {
            l.stop.stop();
        }
        assert_eq!(left(&dir).len(), 3);
        let _ = Channel::new(Some(dir.clone()));
        assert_eq!(left(&dir), Vec::<String>::new());

        // Another MailCraft on the folder lets agents in already (it listens): its files stay as
        // they are, and this one doesn't listen.
        let theirs = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = theirs.local_addr().unwrap().port();
        let mine = Connection::new(port, dir.join(TOKEN_FILE), "MailCraft");
        write_connection(&dir, &Connection { pid: mine.pid.wrapping_add(1), ..mine }).unwrap();
        std::fs::write(dir.join(TOKEN_FILE), "theirs").unwrap();
        let mut second = Channel::new(Some(dir.clone()));
        second.start(&ctx);
        assert!(second.listening.is_none() && second.failed);
        assert_eq!(std::fs::read_to_string(dir.join(TOKEN_FILE)).unwrap(), "theirs");
        assert_eq!(read_connection(&dir).unwrap().map(|c| c.port), Some(port));
        drop(theirs);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
