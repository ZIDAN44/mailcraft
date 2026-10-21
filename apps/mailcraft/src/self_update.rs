//! Test builds keep themselves up to date.
//!
//! Only a build made by the unsigned test-build workflow has this (it sets
//! `MAILCRAFT_UPDATE_CHANNEL=test` when compiling); signed releases are left to how they were
//! installed, and a developer's build never changes itself. `MAILCRAFT_NO_AUTO_UPDATE=1` turns
//! it off for a run.
//!
//! The steps, in the spirit of Office's own updates (fetch quietly, install when the app is
//! closed, or now when asked):
//!
//! 1. **Check** a minute after start and then about hourly (with jitter), asking GitHub with the
//!    last `ETag` so an unchanged list costs nothing against its anonymous quota.
//! 2. **Pick** the newest test build newer than this one ([`newer_release`]: never an older or
//!    the same one), and its installer of the kind this copy was installed from ([`Kind`]).
//! 3. **Download** it into `<data>/updates`, refusing it unless its SHA-256 matches GitHub's record
//!    of the file (or the release's `SHA256SUMS.txt`) and its size matches.
//! 4. **Install** when MailCraft closes, or at once with Restart to update: a small detached
//!    helper waits for this process to end, puts the new version in place the way this copy was
//!    installed, and (when restarting) opens it.
//!
//! A marker remembers the version just installed for a day, so a release whose binary reports
//! something else can't make it install itself over and over.
//!
//! To try it against the newest test build, run a test build with
//! `MAILCRAFT_UPDATE_PRETEND_VERSION=0.20.0` (it counts as that version, for this run only) and
//! `MAILCRAFT_UPDATE_FIRST_CHECK=5` (seconds before the first check).
//!
//! | Installed from | Installed by the helper |
//! |---|---|
//! | Windows `.msi` (Program Files) | `msiexec /i … /passive` (Windows asks for an administrator) |
//! | Windows portable `.zip` | unzipped over the app's folder; `MailCraftData` stays |
//! | macOS `.dmg` | `MailCraft.app` copied out of it in place of this one |
//! | Linux `.AppImage` | the new file renamed over this one |
//!
//! Anything else (a `.deb`, `.rpm`, Flatpak, FreeBSD) keeps File › About's Download link.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mailcraft_engine::model::job::Release;
use mailcraft_engine::newer_release;
use mailcraft_net::updates::{self, Listing};

/// Set at compile time by the test-build workflow.
const CHANNEL: Option<&str> = option_env!("MAILCRAFT_UPDATE_CHANNEL");
/// The first check, after start.
const FIRST_CHECK: Duration = Duration::from_secs(60);
/// Between checks (give or take [`JITTER_SECS`]).
const EVERY: Duration = Duration::from_secs(60 * 60);
const JITTER_SECS: u64 = 10 * 60;
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
    /// A newer version is downloaded and checked: its tag and file.
    Ready { tag: String, file: PathBuf },
}

/// The updater, shared by the window (to show and start it) and its own thread.
#[derive(Clone)]
pub struct Updater {
    kind: Kind,
    data: PathBuf,
    status: Arc<Mutex<Status>>,
    /// The helper has been started (once per run).
    started: Arc<AtomicBool>,
    /// Restart to update was chosen: the new version opens once installed.
    restart: Arc<AtomicBool>,
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
        let u = Updater { kind, data, status: Arc::default(), started: Arc::default(), restart: Arc::default() };
        let worker = u.clone();
        let spawned = std::thread::Builder::new().name("updates".into()).spawn(move || worker.run(build_sha.as_deref()));
        if let Err(e) = spawned {
            log::warn!("updates: {e}");
            return None;
        }
        Some(u)
    }

    /// The version waiting to be installed (its tag, as `v0.21.0-test.19`).
    pub fn ready(&self) -> Option<String> {
        match &*self.status.lock().unwrap_or_else(std::sync::PoisonError::into_inner) {
            Status::Ready { tag, .. } => Some(tag.clone()),
            Status::Waiting => None,
        }
    }

    /// Restart to update: the waiting version is installed as MailCraft closes and then opened.
    /// False when nothing is waiting.
    pub fn restart_after_close(&self) -> bool {
        let ready = self.ready().is_some();
        if ready {
            self.restart.store(true, Ordering::SeqCst);
        }
        ready
    }

    /// Whether the new version is to open after installing.
    pub fn restart_wanted(&self) -> bool {
        self.restart.load(Ordering::SeqCst)
    }

    fn run(&self, build_sha: Option<&str>) {
        let first = std::env::var("MAILCRAFT_UPDATE_FIRST_CHECK").ok().and_then(|v| v.trim().parse::<u64>().ok()).map(Duration::from_secs);
        std::thread::sleep(first.unwrap_or(FIRST_CHECK));
        let mut etag: Option<String> = None;
        loop {
            match self.check(build_sha, &mut etag) {
                Ok(Some(status)) => {
                    if let Status::Ready { tag, .. } = &status {
                        log::info!("updates: {tag} is downloaded and checked; it installs when MailCraft closes");
                    }
                    *self.status.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = status;
                }
                Ok(None) => {}
                Err(e) => log::info!("updates: {e}"),
            }
            std::thread::sleep(next_wait(seed()));
        }
    }

    /// One look at the releases: the status to show when there's a newer one to install.
    fn check(&self, build_sha: Option<&str>, etag: &mut Option<String>) -> Result<Option<Status>, String> {
        let releases = match updates::releases_since(Some(&self.kind.suffix()), etag.as_deref()).map_err(|e| e.to_string())? {
            Listing::Unchanged => return Ok(None),
            Listing::Changed(list, tag) => {
                *etag = tag;
                list
            }
        };
        let Some(release) = newer_release(&current_version(), build_sha, &releases) else { return Ok(None) };
        if release.download.is_empty() {
            return Err(format!("{} has no {} to update with", release.tag, self.kind.suffix().trim_start_matches('-')));
        }
        if just_installed(&self.data, &release.tag, now()) {
            return Ok(None);
        }
        if let Some(tag) = self.ready()
            && tag == release.tag
        {
            return Ok(None);
        }
        self.download(release).map(Some)
    }

    /// Fetch and check `release`'s installer into the downloads folder.
    fn download(&self, release: &Release) -> Result<Status, String> {
        let name = file_name(&release.download).ok_or("the installer has no usable file name")?;
        let dir = self.data.join(DOWNLOADS);
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let file = dir.join(name);
        let expected = updates::expected_sum(release).map_err(|e| e.to_string())?;
        // Already here from an earlier run, and still the right file.
        let have = std::fs::read(&file).ok().is_some_and(|b| updates::sha256_hex(&b) == expected);
        if !have {
            let bytes = updates::fetch_installer(release, &expected).map_err(|e| e.to_string())?;
            mailcraft_engine::store::write_atomic(&file, &bytes).map_err(|e| e.to_string())?;
        }
        // Anything older waiting there goes.
        if let Ok(list) = std::fs::read_dir(&dir) {
            for e in list.flatten().filter(|e| e.path() != file) {
                let _ = std::fs::remove_file(e.path());
            }
        }
        Ok(Status::Ready { tag: release.tag.clone(), file })
    }

    /// Install what's waiting once this process has ended; `restart` opens the new version then.
    /// Nothing waiting, or started already: nothing to do.
    pub fn install(&self, restart: bool) -> Result<bool, String> {
        let Status::Ready { tag, file } = self.status.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone() else { return Ok(false) };
        if self.started.swap(true, Ordering::SeqCst) {
            return Ok(false);
        }
        if let Err(e) = mailcraft_engine::store::write_atomic(&self.data.join(MARKER), format!("{tag}\n{}\n", now()).as_bytes()) {
            log::warn!("updates: couldn't note the update: {e}");
        }
        let log = self.data.join(DOWNLOADS).join("install.log");
        let result = helper(&self.kind, &file, std::process::id(), restart, &log);
        if result.is_err() {
            self.started.store(false, Ordering::SeqCst);
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

/// The last part of a download address, if it's a plain file name.
fn file_name(url: &str) -> Option<&str> {
    let name = url.rsplit('/').next()?;
    (!name.is_empty()
        && name.len() < 200
        && !name.starts_with('.')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_')))
    .then_some(name)
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

/// Start the helper that puts `file` in place for `kind` once process `pid` has ended; what it
/// does is written to `log`.
fn helper(kind: &Kind, file: &Path, pid: u32, restart: bool, log: &Path) -> Result<(), String> {
    match kind {
        Kind::Msi(exe) => windows_helper(pid, &msi_step(file), exe, restart, log),
        Kind::Portable(exe) => {
            let dir = exe.parent().ok_or("the app isn't in a folder")?;
            windows_helper(pid, &portable_step(file, dir), exe, restart, log)
        }
        Kind::MacApp(bundle) => unix_helper(pid, &mac_step(file, bundle), restart.then(|| format!("open {}", sh_quote(bundle))).as_deref(), log),
        Kind::AppImage(path) => {
            // A running AppImage keeps its mounted (now unlinked) file: replace it at once.
            replace_appimage(file, path)?;
            let _ = std::fs::remove_file(file);
            if restart { unix_helper(pid, "", Some(&sh_quote(path)), log) } else { Ok(()) }
        }
    }
}

/// The PowerShell step installing an MSI (Windows asks for an administrator: it's per machine).
fn msi_step(file: &Path) -> String {
    format!(
        "$p = Start-Process -FilePath (Join-Path $env:SystemRoot 'System32\\msiexec.exe') -ArgumentList '/i',{},'/passive','/norestart' -Wait -PassThru; \
         if ($p.ExitCode -eq 0) {{ Remove-Item -LiteralPath {} -ErrorAction SilentlyContinue }}",
        ps_quote(&format!("\"{}\"", file.display())),
        ps_quote(&file.display().to_string()),
    )
}

/// The PowerShell step unzipping the portable zip over `dir` (its one top folder's contents).
fn portable_step(zip: &Path, dir: &Path) -> String {
    let tmp = dir.join(".mailcraft-update");
    format!(
        "$t = {t}; Remove-Item -LiteralPath $t -Recurse -Force -ErrorAction SilentlyContinue; \
         Expand-Archive -LiteralPath {z} -DestinationPath $t -Force; \
         $top = Get-ChildItem -LiteralPath $t -Directory | Select-Object -First 1; \
         if ($top) {{ Copy-Item -Path (Join-Path $top.FullName '*') -Destination {d} -Recurse -Force }}; \
         Remove-Item -LiteralPath $t -Recurse -Force -ErrorAction SilentlyContinue; Remove-Item -LiteralPath {z} -ErrorAction SilentlyContinue",
        t = ps_quote(&tmp.display().to_string()),
        z = ps_quote(&zip.display().to_string()),
        d = ps_quote(&dir.display().to_string()),
    )
}

/// The shell step copying `MailCraft.app` out of the DMG in place of `bundle`.
fn mac_step(dmg: &Path, bundle: &Path) -> String {
    let (b, d) = (sh_quote(bundle), sh_quote(dmg));
    format!(
        "m=$(mktemp -d) && hdiutil attach -nobrowse -readonly -mountpoint \"$m\" {d} >/dev/null && \
         rm -rf {b}.new && ditto \"$m/MailCraft.app\" {b}.new; hdiutil detach \"$m\" >/dev/null 2>&1; \
         if [ -d {b}.new ]; then rm -rf {b}.old && mv {b} {b}.old && mv {b}.new {b} && rm -rf {b}.old; fi; rm -f {d}"
    )
}

/// A PowerShell single-quoted string.
fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// A POSIX shell single-quoted word.
fn sh_quote(p: impl AsRef<std::ffi::OsStr>) -> String {
    format!("'{}'", p.as_ref().to_string_lossy().replace('\'', "'\\''"))
}

/// A hidden PowerShell that waits for `pid`, runs `step`, and (when restarting) starts `exe`,
/// writing a transcript to `log`.
fn windows_helper(pid: u32, step: &str, exe: &Path, restart: bool, log: &Path) -> Result<(), String> {
    let start = if restart { format!("; Start-Process -FilePath {}", ps_quote(&exe.display().to_string())) } else { String::new() };
    let script = format!(
        "Start-Transcript -LiteralPath {} -Force | Out-Null; Wait-Process -Id {pid} -Timeout 120 -ErrorAction SilentlyContinue; {step}{start}; Stop-Transcript | Out-Null",
        ps_quote(&log.display().to_string())
    );
    let mut cmd = std::process::Command::new("powershell.exe");
    cmd.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-WindowStyle", "Hidden", "-Command", &script]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW: a console of its own, never shown. (Without any console, as with
        // DETACHED_PROCESS, PowerShell can end before running anything.)
        cmd.creation_flags(0x0800_0000);
    }
    spawn(cmd)
}

/// `sh` waiting for `pid` to end, then running `step` and `then` (each may be empty), its output
/// going to `log`.
fn unix_helper(pid: u32, step: &str, then: Option<&str>, log: &Path) -> Result<(), String> {
    let mut script =
        format!("exec >{} 2>&1; i=0; while kill -0 {pid} 2>/dev/null && [ $i -lt 1200 ]; do sleep 0.1; i=$((i+1)); done; ", sh_quote(log));
    if !step.is_empty() {
        script.push_str(step);
        script.push_str("; ");
    }
    if let Some(t) = then {
        script.push_str("exec ");
        script.push_str(t);
    }
    let mut cmd = std::process::Command::new("/bin/sh");
    cmd.args(["-c", &script]);
    spawn(cmd)
}

fn spawn(mut cmd: std::process::Command) -> Result<(), String> {
    // The new version reports itself as it is: the trying-out settings were for this run.
    cmd.env_remove("MAILCRAFT_UPDATE_PRETEND_VERSION").env_remove("MAILCRAFT_UPDATE_FIRST_CHECK");
    cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    cmd.spawn().map(|_| ()).map_err(|e| format!("couldn't start the installer ({e})"))
}

/// Put the new AppImage in place of `path`: copied beside it, made runnable, renamed over it.
fn replace_appimage(new: &Path, path: &Path) -> Result<(), String> {
    let dir = path.parent().ok_or("the AppImage isn't in a folder")?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let staged = dir.join(format!(".{name}.new"));
    std::fs::copy(new, &staged).map_err(|e| format!("couldn't write beside {} ({e})", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    }
    std::fs::rename(&staged, path).map_err(|e| {
        let _ = std::fs::remove_file(&staged);
        format!("couldn't replace {} ({e})", path.display())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verbatim_paths_are_made_plain() {
        assert_eq!(plain(PathBuf::from(r"\\?\C:\Tools\MailCraft\mailcraft.exe")), PathBuf::from(r"C:\Tools\MailCraft\mailcraft.exe"));
        assert_eq!(plain(PathBuf::from(r"\\?\UNC\server\share\m.exe")), PathBuf::from(r"\\server\share\m.exe"));
        assert_eq!(plain(PathBuf::from("/Applications/MailCraft.app")), PathBuf::from("/Applications/MailCraft.app"));
    }

    #[test]
    fn checks_spread_around_the_hour() {
        let waits: Vec<u64> = [0, 1, 599, 600, 1200, u64::MAX].iter().map(|s| next_wait(*s).as_secs()).collect();
        assert!(waits.iter().all(|w| (50 * 60..=70 * 60).contains(w)), "{waits:?}");
        assert_ne!(waits[0], waits[2]);
    }

    #[test]
    fn the_version_just_installed_isnt_offered_again_for_a_day() {
        let dir = std::env::temp_dir().join(format!("mailcraft-update-marker-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!just_installed(&dir, "v0.21.0-test.19", 1_000_000));
        std::fs::write(dir.join(MARKER), "v0.21.0-test.19\n1000000\n").unwrap();
        assert!(just_installed(&dir, "v0.21.0-test.19", 1_000_000 + 3600));
        assert!(!just_installed(&dir, "v0.21.0-test.20", 1_000_000 + 3600), "another version is offered");
        assert!(!just_installed(&dir, "v0.21.0-test.19", 1_000_000 + JUST_INSTALLED_FOR), "a day later it may be again");
        assert!(!just_installed(&dir, "v0.21.0-test.19", 999_000), "a clock gone back proves nothing");
        std::fs::write(dir.join(MARKER), "junk").unwrap();
        assert!(!just_installed(&dir, "junk", 1_000_000));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_plain_file_names_are_downloaded_to() {
        let base = "https://github.com/ZIDAN44/mailcraft/releases/download/v0.21.0-test.19/";
        assert_eq!(file_name(&format!("{base}mailcraft-0.21.0-windows-x64.msi")), Some("mailcraft-0.21.0-windows-x64.msi"));
        for bad in ["", "..", ".hidden.msi", "a%2F..%2Fb.msi", "x?y=1", "spaces in.msi"] {
            assert_eq!(file_name(&format!("{base}{bad}")), None, "{bad}");
        }
    }

    /// Paths with quotes and spaces stay one word in the helper's scripts.
    #[test]
    fn helper_scripts_quote_their_paths() {
        let odd = Path::new(r"C:\Users\O'Brien\My Apps\mailcraft-0.21.0-windows-x64.msi");
        let msi = msi_step(odd);
        assert!(msi.contains(r#"'"C:\Users\O''Brien\My Apps\mailcraft-0.21.0-windows-x64.msi"'"#), "{msi}");
        let zip = portable_step(Path::new(r"C:\Tools\MailCraft\x.zip"), Path::new(r"C:\Tools\Mail'Craft"));
        assert!(zip.contains(r"-Destination 'C:\Tools\Mail''Craft'"), "{zip}");
        assert_eq!(sh_quote("/Applications/Mail'Craft.app"), r"'/Applications/Mail'\''Craft.app'");
        let mac = mac_step(Path::new("/tmp/m.dmg"), Path::new("/Applications/MailCraft.app"));
        assert!(mac.contains("ditto \"$m/MailCraft.app\" '/Applications/MailCraft.app'.new"), "{mac}");
    }

    /// The kind of copy is told from where the exe is (Windows: beside a portable marker, or in
    /// Program Files\MailCraft); the names pick the matching file of a release.
    #[test]
    fn the_install_kind_follows_where_mailcraft_is() {
        if cfg!(windows) {
            let dir = std::env::temp_dir().join(format!("mailcraft-update-kind-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(dir.join("MailCraft")).unwrap();
            let exe = dir.join("MailCraft").join("mailcraft.exe");
            let pf = dir.clone();
            let env = move |k: &str| (k == "ProgramFiles").then(|| pf.clone());
            assert_eq!(detect_at(&exe, &env), Some(Kind::Msi(exe.clone())));
            std::fs::write(dir.join("MailCraft").join("portable.txt"), "").unwrap();
            assert_eq!(detect_at(&exe, &env), Some(Kind::Portable(exe.clone())), "the marker wins");
            let elsewhere = dir.join("Downloads").join("mailcraft.exe");
            assert_eq!(detect_at(&elsewhere, &env), None, "a copy run from elsewhere isn't replaced");
            let _ = std::fs::remove_dir_all(&dir);
            assert!(Kind::Msi(exe.clone()).suffix().ends_with(".msi") && Kind::Portable(exe).suffix().ends_with("-portable.zip"));
        }
        assert_eq!(Kind::MacApp(PathBuf::from("/Applications/MailCraft.app")).suffix(), "-macos-universal.dmg");
        assert!(Kind::AppImage(PathBuf::from("/x")).suffix().ends_with(".AppImage"));
    }
}
