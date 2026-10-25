//! Test builds keep themselves up to date.
//!
//! Only a build made by the unsigned test-build workflow has this (it sets
//! `MAILCRAFT_UPDATE_CHANNEL=test` when compiling); signed releases are left to how they were
//! installed, and a developer's build never changes itself. `MAILCRAFT_NO_AUTO_UPDATE=1` turns
//! it off for a run.
//!
//! The steps, the way browsers and code editors update themselves (fetch quietly, say when it's
//! ready, install on restart or as the app closes):
//!
//! 1. **Check** a minute after start (ten seconds when a download was left part way) and then
//!    about hourly while MailCraft stays open, with jitter; asking GitHub with the last `ETag`
//!    so an unchanged list costs nothing against its anonymous quota. A check that fails is
//!    tried again sooner: 5 minutes, doubling up to the hour.
//! 2. **Pick** the newest test build newer than this one ([`newer_release`]: never an older or
//!    the same one), and its installer of the kind this copy was installed from ([`Kind`]).
//! 3. **Download** it into `<data>/updates`, resumably (`mailcraft_net::updates::download`): the
//!    bytes go to a partial file as they come, a download cut off carries on from there with a
//!    range request (in this run, or the next one when MailCraft was closed first), and the file
//!    is kept only if its SHA-256 matches GitHub's record of it (or the release's
//!    `SHA256SUMS.txt`) and its size the release's.
//! 4. **Say so**: the window shows a bar with Restart now, and the system a notification when
//!    MailCraft isn't in front (`crates/ui-egui/src/app/updates.rs`). Closing asks first what
//!    becomes of it (installed as MailCraft closes, or kept for the next time), and says when a
//!    download is still going (it carries on next time).
//! 5. **Install** as MailCraft closes, or at once with Restart now: a small detached helper waits
//!    for this process to end, puts the new version in place the way this copy was installed,
//!    and (when restarting) opens it (`self_update/install.rs`).
//!
//! A marker remembers the version just installed for a day, so a release whose binary reports
//! something else can't make it install itself over and over.
//!
//! To try it against the newest test build, run a test build with
//! `MAILCRAFT_UPDATE_PRETEND_VERSION=0.20.0` (it counts as that version, for this run only, and
//! the sample mailbox's run updates too) and `MAILCRAFT_UPDATE_FIRST_CHECK=5` (seconds before
//! the first check).
//!
//! | Installed from | Installed by the helper |
//! |---|---|
//! | Windows `.msi` (Program Files) | `msiexec /i … /passive` (Windows asks for an administrator) |
//! | Windows portable `.zip` | unzipped over the app's folder; `MailCraftData` stays |
//! | macOS `.dmg` | `MailCraft.app` copied out of it in place of this one, where this user may write (`/Applications` for an administrator's account, `~/Applications`) |
//! | Linux `.AppImage` | the new file renamed over this one |
//!
//! Anything else (a `.deb`, `.rpm`, Flatpak, FreeBSD, an app macOS runs from a read-only
//! translocated copy) keeps File › About's Download link.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::Duration;

use mailcraft_engine::model::job::Release;
use mailcraft_engine::newer_release;
use mailcraft_net::updates::{self, Listing, Progress};
use mailcraft_ui_egui::UpdateState;

mod install;
#[cfg(test)]
mod tests;

/// Set at compile time by the test-build workflow.
const CHANNEL: Option<&str> = option_env!("MAILCRAFT_UPDATE_CHANNEL");
/// The first check, after start; sooner when a download waits to be carried on.
const FIRST_CHECK: Duration = Duration::from_secs(60);
const FIRST_RESUME: Duration = Duration::from_secs(10);
/// Between checks (give or take [`JITTER_SECS`]).
const EVERY: Duration = Duration::from_secs(60 * 60);
const JITTER_SECS: u64 = 10 * 60;
/// After a check fails: this, doubling each time, up to [`EVERY`].
const RETRY: Duration = Duration::from_secs(5 * 60);
/// How long the version just installed isn't offered again.
const JUST_INSTALLED_FOR: u64 = 24 * 3600;
/// The marker recording it, in the data folder.
const MARKER: &str = "update-installed.txt";
/// Where downloads wait, in the data folder.
const DOWNLOADS: &str = "updates";

/// Whether this build updates itself.
pub fn enabled() -> bool {
    CHANNEL == Some("test") && std::env::var_os("MAILCRAFT_NO_AUTO_UPDATE").is_none_or(|v| v.is_empty() || v == "0")
}

/// Whether this run tries the updater out (`MAILCRAFT_UPDATE_PRETEND_VERSION`): the sample
/// mailbox's run then updates too.
pub fn trying_out() -> bool {
    std::env::var_os("MAILCRAFT_UPDATE_PRETEND_VERSION").is_some_and(|v| !v.is_empty())
}

/// How this copy was installed, among the kinds the updater can replace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The Windows Installer package, in Program Files: the exe.
    Msi(PathBuf),
    /// The Windows portable zip: the exe, with `portable.txt` beside it.
    Portable(PathBuf),
    /// `MailCraft.app`, somewhere this user may replace it.
    MacApp(PathBuf),
    /// The AppImage file.
    AppImage(PathBuf),
}

impl Kind {
    /// How the release names this kind's file for this machine (the end of its name).
    pub fn suffix(&self) -> String {
        let arch = if cfg!(target_arch = "aarch64") {
            "arm64"
        } else if cfg!(target_arch = "x86") {
            "x86"
        } else {
            "x64"
        };
        match self {
            Kind::Msi(_) => format!("-windows-{arch}.msi"),
            Kind::Portable(_) => format!("-windows-{arch}-portable.zip"),
            Kind::MacApp(_) => "-macos-universal.dmg".into(),
            Kind::AppImage(_) => format!("-linux-{}.AppImage", if cfg!(target_arch = "aarch64") { "aarch64" } else { "x86_64" }),
        }
    }
}

/// How the running copy was installed, if the updater can replace it.
pub fn detect() -> Option<Kind> {
    let exe = std::env::current_exe().ok()?;
    let exe = plain(exe.canonicalize().unwrap_or(exe));
    detect_at(&exe, &|k| std::env::var_os(k).map(PathBuf::from))
}

/// A path without Windows' `\\?\` prefix, which `canonicalize` adds and PowerShell's archive
/// commands (and many programs) don't take.
fn plain(p: PathBuf) -> PathBuf {
    let s = p.to_string_lossy();
    if let Some(unc) = s.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{unc}"));
    }
    match s.strip_prefix(r"\\?\") {
        Some(rest) => PathBuf::from(rest),
        None => p,
    }
}

/// [`detect`] for an exe at `exe`, with environment variables from `env`.
fn detect_at(exe: &Path, env: &dyn Fn(&str) -> Option<PathBuf>) -> Option<Kind> {
    let dir = exe.parent()?;
    if cfg!(windows) {
        if crate::portable::MARKERS.iter().any(|m| dir.join(m).is_file()) {
            return Some(Kind::Portable(exe.to_path_buf()));
        }
        // The MSI installs into Program Files\MailCraft (packaging/windows/mailcraft.wxs).
        let same = |a: &Path, b: &Path| a.to_string_lossy().eq_ignore_ascii_case(&b.to_string_lossy());
        let installed = ["ProgramW6432", "ProgramFiles", "ProgramFiles(x86)"].iter().filter_map(|k| env(k)).any(|p| same(dir, &p.join("MailCraft")));
        return installed.then(|| Kind::Msi(exe.to_path_buf()));
    }
    if cfg!(target_os = "macos") {
        let bundle = exe.ancestors().find(|p| p.extension().is_some_and(|e| e == "app"))?;
        // Run from where it was downloaded, macOS runs a read-only copy of it elsewhere.
        if bundle.to_string_lossy().contains("/AppTranslocation/") {
            log::info!("updates: macOS runs this MailCraft from a read-only copy; moved to Applications, it updates itself");
            return None;
        }
        // Replacing the bundle needs its folder to be ours to write.
        let parent = bundle.parent()?;
        return writable(parent).then(|| Kind::MacApp(bundle.to_path_buf()));
    }
    if cfg!(target_os = "linux") {
        return env("APPIMAGE").filter(|p| p.is_file()).map(Kind::AppImage);
    }
    None
}

/// Whether a file can be made in `dir` (made and removed again to find out).
fn writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".mailcraft-update-probe-{}", std::process::id()));
    let ok = std::fs::write(&probe, b"").is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

/// Where the updater is.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Status {
    #[default]
    Waiting,
    /// A newer version is downloading: its tag.
    Downloading { tag: String },
    /// A newer version is downloaded and checked: its tag and file.
    Ready { tag: String, file: PathBuf },
}

/// What the window and the updater's thread share.
#[derive(Default)]
struct Shared {
    status: Mutex<Status>,
    progress: Progress,
    /// The helper has been started (once per run).
    started: AtomicBool,
    /// Restart now was chosen: the new version opens once installed.
    restart: AtomicBool,
    /// Close without updating was chosen: not installed as MailCraft closes this time.
    later: AtomicBool,
    /// MailCraft is closing: a download stops between reads (what came is kept).
    stop: AtomicBool,
    /// Wakes the thread from its wait between checks as MailCraft closes.
    asleep: Mutex<()>,
    wake: Condvar,
    /// Wakes the window when where the updater is changes: an idle window draws no frames, and a
    /// minimized one only looks (`eframe::App::logic`) when woken.
    waker: Mutex<Option<Waker>>,
}

/// What wakes the window.
pub type Waker = Arc<dyn Fn() + Send + Sync>;

/// The updater, shared by the window (to show and start it) and its own thread.
#[derive(Clone)]
pub struct Updater {
    kind: Kind,
    data: PathBuf,
    shared: Arc<Shared>,
}

impl Updater {
    /// Start checking in the background, when this build updates itself and knows how this copy
    /// was installed. `data` is MailCraft's data folder; `build_sha` the commit it was built from.
    pub fn start(data: PathBuf, build_sha: Option<String>) -> Option<Updater> {
        if !enabled() {
            return None;
        }
        let Some(kind) = detect() else {
            log::info!("updates: this copy wasn't installed in a way MailCraft can update by itself");
            return None;
        };
        let u = Updater { kind, data, shared: Arc::default() };
        let worker = u.clone();
        let spawned = std::thread::Builder::new().name("updates".into()).spawn(move || worker.run(build_sha.as_deref()));
        if let Err(e) = spawned {
            log::warn!("updates: {e}");
            return None;
        }
        Some(u)
    }

    fn status(&self) -> Status {
        self.shared.status.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    fn set_status(&self, status: Status) {
        *self.shared.status.lock().unwrap_or_else(PoisonError::into_inner) = status;
        let waker = self.shared.waker.lock().unwrap_or_else(PoisonError::into_inner).clone();
        if let Some(wake) = waker {
            wake();
        }
    }

    /// What wakes the window when the update starts downloading, is ready, or goes.
    pub fn set_waker(&self, waker: Waker) {
        *self.shared.waker.lock().unwrap_or_else(PoisonError::into_inner) = Some(waker);
    }

    /// Where it is, for the window.
    pub fn state(&self) -> UpdateState {
        let version = |tag: &str| tag.trim_start_matches('v').to_string();
        match self.status() {
            Status::Waiting => UpdateState::None,
            Status::Downloading { tag } => {
                let (done, total) = self.shared.progress.get();
                UpdateState::Downloading { version: version(&tag), done, total }
            }
            // The MSI installs for everyone: Windows asks for an administrator.
            Status::Ready { tag, .. } => UpdateState::Ready { version: version(&tag), admin: matches!(self.kind, Kind::Msi(_)) },
        }
    }

    /// Restart now: the waiting version is installed as MailCraft closes and then opened. False
    /// when nothing is waiting.
    pub fn restart_after_close(&self) -> bool {
        let ready = matches!(self.status(), Status::Ready { .. });
        if ready {
            self.shared.later.store(false, Ordering::SeqCst);
            self.shared.restart.store(true, Ordering::SeqCst);
        }
        ready
    }

    /// Close without updating: not installed as MailCraft closes this time (the next time is
    /// asked again).
    pub fn keep_for_later(&self) {
        self.shared.later.store(true, Ordering::SeqCst);
    }

    /// MailCraft is closing: install what's waiting (unless kept for later), and stop a download
    /// (what came of it is carried on from next time).
    pub fn closing(&self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        self.shared.wake.notify_all();
        if self.shared.later.load(Ordering::SeqCst) {
            log::info!("updates: the update waits for the next time MailCraft closes");
            return;
        }
        if let Err(e) = self.install(self.shared.restart.load(Ordering::SeqCst)) {
            log::warn!("updates: {e}");
        }
    }

    fn run(&self, build_sha: Option<&str>) {
        let dir = self.data.join(DOWNLOADS);
        let first = std::env::var("MAILCRAFT_UPDATE_FIRST_CHECK").ok().and_then(|v| v.trim().parse::<u64>().ok()).map(Duration::from_secs);
        let first = first.unwrap_or(if has_part(&dir) { FIRST_RESUME } else { FIRST_CHECK });
        if !self.sleep(first) {
            return;
        }
        let mut etag: Option<String> = None;
        let mut failed = 0u32;
        loop {
            match self.check(build_sha, &mut etag) {
                Ok(()) => failed = 0,
                Err(e) => {
                    log::info!("updates: {e}");
                    failed = failed.saturating_add(1);
                }
            }
            let wait = if failed > 0 { retry_wait(failed) } else { next_wait(seed()) };
            if !self.sleep(wait) {
                return;
            }
        }
    }

    /// Wait `d`: false when MailCraft is closing.
    fn sleep(&self, d: Duration) -> bool {
        let deadline = std::time::Instant::now() + d;
        let mut asleep = self.shared.asleep.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            if self.shared.stop.load(Ordering::SeqCst) {
                return false;
            }
            let now = std::time::Instant::now();
            if now >= deadline {
                return true;
            }
            asleep = match self.shared.wake.wait_timeout(asleep, deadline - now) {
                Ok((g, _)) => g,
                Err(e) => e.into_inner().0,
            };
        }
    }

    /// One look at the releases, and the download of a newer one.
    fn check(&self, build_sha: Option<&str>, etag: &mut Option<String>) -> Result<(), String> {
        let releases = match updates::releases_since(Some(&self.kind.suffix()), etag.as_deref()).map_err(|e| e.to_string())? {
            Listing::Unchanged => return Ok(()),
            Listing::Changed(list, tag) => {
                *etag = tag;
                list
            }
        };
        let Some(release) = newer_release(&current_version(), build_sha, &releases) else { return Ok(()) };
        if release.download.is_empty() {
            return Err(format!("{} has no {} to update with", release.tag, self.kind.suffix().trim_start_matches('-')));
        }
        if just_installed(&self.data, &release.tag, now()) {
            return Ok(());
        }
        if let Status::Ready { tag, .. } = self.status()
            && tag == release.tag
        {
            return Ok(());
        }
        self.download(release)
    }

    /// Fetch and check `release`'s installer into the downloads folder, carrying on from what an
    /// earlier try left there.
    fn download(&self, release: &Release) -> Result<(), String> {
        let dir = self.data.join(DOWNLOADS);
        let expected = updates::expected_sum(release).map_err(|e| e.to_string())?;
        // An older version ready to install stays so while a newer one downloads.
        let ready = self.status();
        if !matches!(ready, Status::Ready { .. }) {
            self.set_status(Status::Downloading { tag: release.tag.clone() });
        }
        let fetched = updates::download_installer(release, &expected, &dir, &self.shared.progress, &self.shared.stop);
        match fetched {
            Ok(Some(file)) => {
                // Anything older waiting there goes.
                if let Ok(list) = std::fs::read_dir(&dir) {
                    for e in list.flatten().filter(|e| e.path() != file) {
                        let _ = std::fs::remove_file(e.path());
                    }
                }
                log::info!("updates: {} is downloaded and checked; it installs when MailCraft closes", release.tag);
                self.set_status(Status::Ready { tag: release.tag.clone(), file });
                Ok(())
            }
            // MailCraft is closing: carried on from next time.
            Ok(None) => Ok(()),
            Err(e) => {
                // Whatever was ready before still is.
                self.set_status(if matches!(ready, Status::Ready { .. }) { ready } else { Status::Waiting });
                Err(e.to_string())
            }
        }
    }

    /// Install what's waiting once this process has ended; `restart` opens the new version then.
    /// Nothing waiting, or started already: nothing to do.
    pub fn install(&self, restart: bool) -> Result<bool, String> {
        let Status::Ready { tag, file } = self.status() else { return Ok(false) };
        if self.shared.started.swap(true, Ordering::SeqCst) {
            return Ok(false);
        }
        if let Err(e) = mailcraft_engine::store::write_atomic(&self.data.join(MARKER), format!("{tag}\n{}\n", now()).as_bytes()) {
            log::warn!("updates: couldn't note the update: {e}");
        }
        let log = self.data.join(DOWNLOADS).join("install.log");
        let result = install::helper(&self.kind, &file, std::process::id(), restart, &log);
        if result.is_err() {
            self.shared.started.store(false, Ordering::SeqCst);
        }
        result.map(|()| {
            log::info!("updates: installing {tag}{}", if restart { " and restarting" } else { " as MailCraft closes" });
            true
        })
    }
}

/// The version this build counts as: its own, or `MAILCRAFT_UPDATE_PRETEND_VERSION` (to try the
/// updater against the newest release; the restarted copy reports its own again).
fn current_version() -> String {
    std::env::var("MAILCRAFT_UPDATE_PRETEND_VERSION")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| v.split('.').count() == 3 && v.split('.').all(|p| !p.is_empty() && p.len() < 7 && p.bytes().all(|b| b.is_ascii_digit())))
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string())
}

/// Whether a download was left part way in `dir`.
fn has_part(dir: &Path) -> bool {
    std::fs::read_dir(dir).is_ok_and(|l| l.flatten().any(|e| e.file_name().to_string_lossy().ends_with(".part")))
}

/// Whether `tag` was installed by the updater within [`JUST_INSTALLED_FOR`] of `now`.
fn just_installed(data: &Path, tag: &str, now: u64) -> bool {
    let Ok(text) = std::fs::read_to_string(data.join(MARKER)) else { return false };
    let mut lines = text.lines();
    let (Some(done), Some(at)) = (lines.next(), lines.next().and_then(|a| a.trim().parse::<u64>().ok())) else { return false };
    done.trim() == tag && now.checked_sub(at).is_some_and(|age| age < JUST_INSTALLED_FOR)
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Something different each time, to spread checks out.
fn seed() -> u64 {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
    u64::from(t) ^ u64::from(std::process::id()).rotate_left(17)
}

/// The wait before the next check: [`EVERY`], give or take [`JITTER_SECS`], from `seed`.
fn next_wait(seed: u64) -> Duration {
    let spread = seed % (2 * JITTER_SECS + 1);
    Duration::from_secs((EVERY.as_secs() + spread).saturating_sub(JITTER_SECS))
}

/// The wait after `failed` checks in a row failed: [`RETRY`], doubling, at most [`EVERY`].
fn retry_wait(failed: u32) -> Duration {
    RETRY.saturating_mul(1 << failed.saturating_sub(1).min(6)).min(EVERY)
}
