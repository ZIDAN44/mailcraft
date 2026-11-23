//! Starting MailCraft as the user signs in (issue #87), the way each system has for an app of the
//! user's own, none needing an administrator: on Windows, a value under the user's
//! `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`; on Linux and the BSDs, an XDG autostart
//! entry in `~/.config/autostart`; on a Mac, a launch agent in `~/Library/LaunchAgents`. Each
//! names this copy of MailCraft (an AppImage's file, not the folder it unpacks to), and says it's
//! on only when it names this copy: one left by another copy reads as off, and turning it on
//! names this one instead.
//!
//! Not offered where it would outlive the copy or can't be written: a portable copy (it leaves
//! nothing on the computer, and its drive letter changes with the USB stick it's on), Flatpak and
//! Snap (their sandboxes start an app at sign-in through a portal of their own).
//!
//! With "Start MailCraft in the notification area" on, a start at sign-in keeps its window put
//! away, as every start does (`desktop/launch.rs`).

use std::path::{Path, PathBuf};

/// Whether starting at sign-in is offered: not in a portable copy, nor in a sandbox.
pub(crate) fn offered(portable: bool) -> bool {
    !portable && !sandboxed() && program().is_some()
}

/// Whether MailCraft starts as the user signs in: the system's entry names this copy.
pub(crate) fn on() -> bool {
    program().is_some_and(|p| system::on(&p))
}

/// MailCraft made to start as the user signs in, or not.
pub(crate) fn set(on: bool) -> Result<(), String> {
    let p = program().ok_or_else(|| "MailCraft's own program file can't be found".to_string())?;
    system::set(&p, on)
}

/// Flatpak's or Snap's sandbox, where an app is started at sign-in by the sandbox's portal.
fn sandboxed() -> bool {
    cfg!(all(unix, not(target_os = "macos")))
        && (std::env::var_os("FLATPAK_ID").is_some() || Path::new("/.flatpak-info").exists() || std::env::var_os("SNAP").is_some())
}

/// The program the entry starts: this one, or the AppImage it runs from.
fn program() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let var = |name: &str| std::env::var_os(name).map(PathBuf::from).filter(|p| p.is_absolute());
    Some(program_of(exe, var("APPIMAGE").filter(|i| i.is_file()), var("APPDIR")))
}

/// An AppImage unpacks to a folder of its own each time it runs (`APPDIR`), gone when it ends:
/// the entry names the image (`APPIMAGE`), as long as this program runs from inside it. Another
/// AppImage that starts MailCraft (a browser opening a `mailto:` link) passes its own `APPIMAGE`
/// on, as the updater found (`self_update/detect.rs`).
fn program_of(exe: PathBuf, appimage: Option<PathBuf>, appdir: Option<PathBuf>) -> PathBuf {
    match (appimage, appdir) {
        (Some(image), Some(dir)) if exe.starts_with(&dir) => image,
        _ => exe,
    }
}

#[cfg(windows)]
mod system {
    use std::path::Path;

    /// Where Windows keeps what starts as the user signs in, per user.
    const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    /// Where Settings › Apps › Startup and Task Manager keep a `Run` value turned off there (the
    /// value stays in `Run`): a value of the same name whose first byte is odd.
    const APPROVED: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
    /// MailCraft's value there.
    const NAME: &str = "MailCraft";

    pub(super) fn on(program: &Path) -> bool {
        on_as(NAME, program)
    }

    pub(super) fn set(program: &Path, on: bool) -> Result<(), String> {
        set_as(NAME, program, on)
    }

    /// The command the value holds: the program, quoted (a path with spaces is otherwise read
    /// up to its first space).
    pub(super) fn command(program: &Path) -> String {
        format!("\"{}\"", program.display())
    }

    /// Whether the value `name` starts this program, and Windows' own startup settings haven't
    /// turned it off.
    pub(super) fn on_as(name: &str, program: &Path) -> bool {
        windows_registry::CURRENT_USER.open(RUN).and_then(|k| k.get_string(name)).is_ok_and(|v| v.trim().eq_ignore_ascii_case(&command(program)))
            && !turned_off(name)
    }

    /// Whether Windows' startup settings have the value `name` turned off.
    fn turned_off(name: &str) -> bool {
        windows_registry::CURRENT_USER.open(APPROVED).and_then(|k| k.get_value(name)).is_ok_and(|v| v.first().is_some_and(|b| b % 2 == 1))
    }

    /// The value `name` written to start this program, or taken away (none there is fine). Turned
    /// on here, it's on in Windows' startup settings too: what they had for it is taken away.
    pub(super) fn set_as(name: &str, program: &Path, on: bool) -> Result<(), String> {
        let key = windows_registry::CURRENT_USER.create(RUN).map_err(|e| e.to_string())?;
        if on {
            key.set_string(name, command(program)).map_err(|e| e.to_string())?;
            return remove(APPROVED, name);
        }
        remove(RUN, name)?;
        remove(APPROVED, name)
    }

    /// The value `name` under `HKCU\<path>` taken away: none there (nor the key) is fine.
    fn remove(path: &str, name: &str) -> Result<(), String> {
        let Ok(key) = windows_registry::CURRENT_USER.options().read().write().open(path) else { return Ok(()) };
        match key.remove_value(name) {
            Err(_) if key.get_value(name).is_err() => Ok(()),
            r => r.map_err(|e| e.to_string()),
        }
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod system {
    use std::path::{Path, PathBuf};

    /// The autostart entry's file name: MailCraft's id, as its desktop entry's.
    const FILE: &str = "io.github.zidan44.MailCraft.desktop";

    /// The user's settings folder (`XDG_CONFIG_HOME`, else `~/.config`).
    fn config() -> Option<PathBuf> {
        let var = |name: &str| std::env::var_os(name).map(PathBuf::from).filter(|p| p.is_absolute());
        var("XDG_CONFIG_HOME").or_else(|| var("HOME").map(|h| h.join(".config")))
    }

    pub(super) fn on(program: &Path) -> bool {
        config().is_some_and(|c| on_in(&c, program))
    }

    pub(super) fn set(program: &Path, on: bool) -> Result<(), String> {
        let config = config().ok_or_else(|| "there's no settings folder (HOME isn't set)".to_string())?;
        set_in(&config, program, on)
    }

    /// Whether the autostart entry in the settings folder `config` starts this program (and
    /// isn't switched off there: `Hidden=true`, or GNOME's own switch).
    pub(super) fn on_in(config: &Path, program: &Path) -> bool {
        let Some(exec) = exec(program) else { return false };
        std::fs::read_to_string(config.join("autostart").join(FILE)).is_ok_and(|e| {
            let lines: Vec<&str> = e.lines().map(str::trim).collect();
            lines.contains(&exec.as_str()) && !lines.contains(&"Hidden=true") && !lines.contains(&"X-GNOME-Autostart-enabled=false")
        })
    }

    /// The entry written in `config`'s autostart folder, or taken away (none there is fine).
    pub(super) fn set_in(config: &Path, program: &Path, on: bool) -> Result<(), String> {
        let path = config.join("autostart").join(FILE);
        if !on {
            return match std::fs::remove_file(&path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("{}: {e}", path.display())),
                _ => Ok(()),
            };
        }
        let entry = entry(program).ok_or_else(|| format!("{} can't be named in an autostart entry", program.display()))?;
        let dir = config.join("autostart");
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        std::fs::write(&path, entry).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// The autostart entry (the Desktop Entry Specification's) that starts this program.
    pub(super) fn entry(program: &Path) -> Option<String> {
        Some(format!(
            "[Desktop Entry]\nType=Application\nName=MailCraft\nComment=Started as you sign in, from MailCraft's options\n{}\nIcon=io.github.zidan44.MailCraft\nTerminal=false\nX-GNOME-Autostart-enabled=true\n",
            exec(program)?
        ))
    }

    /// The entry's `Exec` line: the program's path quoted as the specification asks, its `"`,
    /// `` ` ``, `$` and `\` escaped and the escapes' own `\` doubled again (the line is a string
    /// value), `%` written `%%`. `None` for a path that isn't text or has a line break in it.
    pub(super) fn exec(program: &Path) -> Option<String> {
        let path = program.to_str().filter(|p| !p.contains(['\n', '\r']))?;
        let mut quoted = String::new();
        for c in path.chars() {
            match c {
                '"' | '`' | '$' => {
                    quoted.push_str("\\\\");
                    quoted.push(c);
                }
                '\\' => quoted.push_str("\\\\\\\\"),
                '%' => quoted.push_str("%%"),
                c => quoted.push(c),
            }
        }
        Some(format!("Exec=\"{quoted}\""))
    }
}

#[cfg(target_os = "macos")]
mod system {
    use std::path::{Path, PathBuf};

    /// The launch agent's label and file name: MailCraft's bundle id.
    const LABEL: &str = "io.github.zidan44.MailCraft";

    /// The user's own launch agents' folder.
    fn agents() -> Option<PathBuf> {
        std::env::var_os("HOME").map(PathBuf::from).filter(|h| h.is_absolute()).map(|h| h.join("Library").join("LaunchAgents"))
    }

    pub(super) fn on(program: &Path) -> bool {
        agents().is_some_and(|a| on_in(&a, program))
    }

    pub(super) fn set(program: &Path, on: bool) -> Result<(), String> {
        let agents = agents().ok_or_else(|| "there's no home folder (HOME isn't set)".to_string())?;
        set_in(&agents, program, on)
    }

    /// Whether the launch agent in the folder `agents` starts this program.
    pub(super) fn on_in(agents: &Path, program: &Path) -> bool {
        std::fs::read_to_string(agents.join(format!("{LABEL}.plist"))).is_ok_and(|p| p.contains(&format!("<string>{}</string>", xml(program))))
    }

    /// The launch agent written in `agents`, or taken away (none there is fine). macOS loads it
    /// at the next sign-in (and says in a notification that MailCraft added a login item).
    pub(super) fn set_in(agents: &Path, program: &Path, on: bool) -> Result<(), String> {
        let path = agents.join(format!("{LABEL}.plist"));
        if !on {
            return match std::fs::remove_file(&path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("{}: {e}", path.display())),
                _ => Ok(()),
            };
        }
        std::fs::create_dir_all(agents).map_err(|e| format!("{}: {e}", agents.display()))?;
        std::fs::write(&path, agent(program)).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// The launch agent: this program, started once as the user signs in.
    pub(super) fn agent(program: &Path) -> String {
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n\t<key>Label</key>\n\t<string>{LABEL}</string>\n\t<key>ProgramArguments</key>\n\t<array>\n\t\t<string>{}</string>\n\t</array>\n\t<key>RunAtLoad</key>\n\t<true/>\n\t<key>ProcessType</key>\n\t<string>Interactive</string>\n</dict>\n</plist>\n",
            xml(program)
        )
    }

    /// A path as XML text.
    fn xml(program: &Path) -> String {
        program.to_string_lossy().replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
    }
}

#[cfg(test)]
mod tests;
