//! Windows' own programs MailCraft starts (Explorer to open a folder, rundll32 to open a file or a
//! link, PowerShell to install an update or make a video's frame), by their full path. Started by
//! its bare name, a program is looked for in MailCraft's own folder first, where a portable copy's
//! (in Downloads, on a USB stick) could hold anyone's `explorer.exe`, as GridCraft found.

use std::path::{Path, PathBuf};

/// One of Windows' own programs (`rundll32.exe`, `WindowsPowerShell\v1.0\powershell.exe`), by
/// its full path in Windows' system folder as Windows itself answers it (`GetSystemDirectory`, not
/// `%SystemRoot%`, which whoever starts MailCraft may set); `explorer.exe` is in the Windows folder
/// above it.
pub(crate) fn path(name: &str) -> PathBuf {
    #[cfg(windows)]
    let answered = winsafe::GetSystemDirectory().ok().map(PathBuf::from);
    #[cfg(not(windows))]
    let answered: Option<PathBuf> = None;
    let system = answered.filter(|d| d.is_absolute()).unwrap_or_else(|| PathBuf::from(r"C:\Windows\System32"));
    if name.eq_ignore_ascii_case("explorer.exe") {
        return system.parent().map_or_else(|| PathBuf::from(r"C:\Windows"), Path::to_path_buf).join(name);
    }
    system.join(name)
}

// Windows alone: elsewhere there are no Windows programs to find.
#[cfg(all(test, windows))]
mod tests {
    use super::*;

    /// Windows' programs MailCraft starts (to open a folder, a file or a link, and to install an
    /// update) are Windows' own, by their full path where Windows says its folders are. Started by
    /// name, an `explorer.exe` beside a portable MailCraft ran in its place.
    #[test]
    fn windows_programs_are_started_by_their_full_path() {
        let system = PathBuf::from(winsafe::GetSystemDirectory().unwrap());
        for name in ["rundll32.exe", "explorer.exe", r"WindowsPowerShell\v1.0\powershell.exe"] {
            let p = path(name);
            assert!(p.is_absolute() && p.is_file(), "{}", p.display());
        }
        assert_eq!(path("rundll32.exe"), system.join("rundll32.exe"));
        assert_eq!(path("explorer.exe"), system.parent().unwrap().join("explorer.exe"));
    }
}
