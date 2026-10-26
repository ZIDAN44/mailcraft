//! Putting a downloaded version in place: a small detached helper waits for MailCraft to end,
//! installs it the way this copy was installed, and (when restarting) opens it.

use std::path::Path;

use super::Kind;

/// Start the helper that puts `file` in place for `kind` once process `pid` has ended; what it
/// does is written to `log`.
pub(super) fn helper(kind: &Kind, file: &Path, pid: u32, restart: bool, log: &Path) -> Result<(), String> {
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
pub(super) fn msi_step(file: &Path) -> String {
    format!(
        "$p = Start-Process -FilePath (Join-Path $env:SystemRoot 'System32\\msiexec.exe') -ArgumentList '/i',{},'/passive','/norestart' -Wait -PassThru; \
         if ($p.ExitCode -eq 0) {{ Remove-Item -LiteralPath {} -ErrorAction SilentlyContinue }}",
        ps_quote(&format!("\"{}\"", file.display())),
        ps_quote(&file.display().to_string()),
    )
}

/// The PowerShell step unzipping the portable zip over `dir` (its one top folder's contents).
pub(super) fn portable_step(zip: &Path, dir: &Path) -> String {
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

/// The shell step copying `MailCraft.app` out of the DMG in place of `bundle`. Only a whole copy
/// takes its place (a failed or partial one, or one left by an earlier try, never does), and if
/// the new one can't be moved in, the old one goes back. A disk image the system is still busy
/// with (checking it, indexing it) is attached again for about a minute before the update gives
/// up (craft-libs' updater found that one refusal threw the download away).
pub(super) fn mac_step(dmg: &Path, bundle: &Path) -> String {
    let (b, d) = (sh_quote(bundle), sh_quote(dmg));
    format!(
        "rm -rf {b}.new; att() {{ for w in 1 2 4 8 16 32 0; do hdiutil attach -nobrowse -readonly -mountpoint \"$1\" {d} >/dev/null && return 0; \
         [ $w -eq 0 ] && return 1; sleep $w; done; }}; m=$(mktemp -d) && att \"$m\" && \
         {{ ditto \"$m/MailCraft.app\" {b}.new; ok=$?; hdiutil detach \"$m\" >/dev/null 2>&1; \
         [ $ok -eq 0 ] && rm -rf {b}.old && mv {b} {b}.old && {{ mv {b}.new {b} || mv {b}.old {b}; }}; }}; \
         rm -rf {b}.new {b}.old; rm -f {d}"
    )
}

/// A PowerShell single-quoted string. PowerShell ends one at a typographic quote too (‘ ’ ‚ ‛),
/// so those are doubled as `'` is (a folder named `Zoë’s` is common).
pub(super) fn ps_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        if matches!(c, '\'' | '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}') {
            out.push(c);
        }
        out.push(c);
    }
    out.push('\'');
    out
}

/// A POSIX shell single-quoted word.
pub(super) fn sh_quote(p: impl AsRef<std::ffi::OsStr>) -> String {
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
