//! MailCraft's id was `ai.storyteller.mailcraft` until 0.36, a name of the ArtCraft suite's; it's
//! [`mailcraft_ui_egui::APP_ID`] now. An update puts the new id's files in place, and the
//! packages (`.msi`, `.deb`, `.rpm`) take their old ones away; what else the old id left on this
//! computer is tidied here at start, wherever it's found, and a Flatpak's mail is brought across
//! (a new Flatpak id starts with an empty sandbox).
//!
//! | System | What the old id left | Here |
//! |---|---|---|
//! | Windows | Its notification name (`HKCU\Software\Classes\AppUserModelId\<id>`) | removed |
//! | macOS | Its preferences, saved window state, caches and web storage in `~/Library` | removed |
//! | Linux, FreeBSD | A default-app choice in `mimeapps.list` naming its desktop entry | pointed at the new entry |
//! | `.tar.gz` | Its desktop entry, AppStream file and icons, beside the new ones | removed |
//! | Flatpak | Its sandbox's data (`~/.var/app/<id>/data/mailcraft`) | copied into this one's, once |
//!
//! Only what's named for the old id is touched. Everything here is best effort: what can't be
//! done is logged, and MailCraft starts as usual.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// MailCraft's id until 0.36.
pub const OLD_ID: &str = "ai.storyteller.mailcraft";

/// Set when this start brought a Flatpak's mail across from the old id's sandbox.
static CARRIED_OVER: AtomicBool = AtomicBool::new(false);

/// Whether this start brought the old Flatpak's mail across (a bar above the panes then says
/// how to remove the old one).
pub fn carried_over() -> bool {
    CARRIED_OVER.load(Ordering::Relaxed)
}

/// In MailCraft's Flatpak, before anything opens its app folder `base`: the old id's sandbox
/// data copied in when this sandbox has none yet. The old copy stays (its sandbox is the old
/// Flatpak's, which only Flatpak removes).
pub fn carry_over_flatpak(base: &Path) {
    if std::env::var_os("FLATPAK_ID").is_none_or(|id| id != mailcraft_ui_egui::APP_ID) {
        return;
    }
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from).filter(|h| h.is_absolute()) else { return };
    let old = home.join(".var/app").join(OLD_ID).join("data/mailcraft");
    match carry_over(&old, base) {
        Ok(true) => CARRIED_OVER.store(true, Ordering::Relaxed),
        Ok(false) => {}
        // Logged once the log is open (it lives in `base`).
        Err(e) => remember(format!("the earlier Flatpak's mail ({}) couldn't be brought over: {e}", old.display())),
    }
}

/// What happened before the log was open, said once it is ([`tidy`]).
static EARLIER: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

fn remember(line: String) {
    if let Ok(mut l) = EARLIER.lock() {
        l.push(line);
    }
}

/// `old`'s files copied to `new` when `new` has nothing yet: into a folder beside `new` first,
/// renamed into place once all of it came, so a copy cut short leaves `new` as it was. `false`
/// when there was nothing to do.
pub(super) fn carry_over(old: &Path, new: &Path) -> Result<bool, String> {
    if !old.is_dir() || std::fs::read_dir(new).is_ok_and(|mut d| d.next().is_some()) {
        return Ok(false);
    }
    let Some(name) = new.file_name() else { return Err("no folder to copy into".into()) };
    let part = new.with_file_name(format!("{}.carrying", name.to_string_lossy()));
    let _ = std::fs::remove_dir_all(&part);
    let copied = copy_tree(old, &part, 0).and_then(|()| {
        // An empty folder made for it already (by an earlier start's lock) goes first.
        let _ = std::fs::remove_dir(new);
        std::fs::rename(&part, new).map_err(|e| e.to_string())
    });
    if copied.is_err() {
        let _ = std::fs::remove_dir_all(&part);
    }
    copied.map(|()| true)
}

/// How deep a folder is copied (MailCraft's own are a few levels).
const MAX_DEPTH: u32 = 16;

/// The folders and files under `from` copied to `to`; links and anything else left out.
fn copy_tree(from: &Path, to: &Path, depth: u32) -> Result<(), String> {
    if depth > MAX_DEPTH {
        return Err(format!("{} is nested too deep", from.display()));
    }
    std::fs::create_dir_all(to).map_err(|e| format!("{}: {e}", to.display()))?;
    for entry in std::fs::read_dir(from).map_err(|e| format!("{}: {e}", from.display()))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        let (src, dst) = (entry.path(), to.join(entry.file_name()));
        if kind.is_dir() {
            copy_tree(&src, &dst, depth.saturating_add(1))?;
        } else if kind.is_file() {
            std::fs::copy(&src, &dst).map_err(|e| format!("{}: {e}", src.display()))?;
        }
    }
    Ok(())
}

/// After start, once the log is open: what the old id left on this system, tidied.
pub fn tidy() {
    if let Ok(mut l) = EARLIER.lock() {
        for line in l.drain(..) {
            log::warn!("{line}");
        }
    }
    if carried_over() {
        log::info!("renamed: the earlier Flatpak's mail ({OLD_ID}) was brought over; that Flatpak stays installed until it's removed");
    }
    #[cfg(windows)]
    {
        let key = format!(r"Software\Classes\AppUserModelId\{OLD_ID}");
        if windows_registry::CURRENT_USER.open(&key).is_ok() {
            match windows_registry::CURRENT_USER.remove_tree(&key) {
                Ok(()) => log::info!("renamed: removed the old id's notification name"),
                Err(e) => log::warn!("renamed: the old id's notification name stays: {e}"),
            }
        }
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).filter(|h| h.is_absolute());
    if cfg!(target_os = "macos")
        && let Some(home) = &home
    {
        report("macOS leftovers", remove_all(&mac_leftovers(home)));
    }
    if cfg!(any(target_os = "linux", target_os = "freebsd")) {
        let flatpak = std::env::var_os("FLATPAK_ID").is_some() || Path::new("/.flatpak-info").exists();
        // In the Flatpak, the host's settings can't be written (nor are they the sandbox's).
        if !flatpak && let Some(home) = &home {
            let var = |k: &str| std::env::var_os(k).map(PathBuf::from).filter(|p| p.is_absolute());
            let config = var("XDG_CONFIG_HOME").unwrap_or_else(|| home.join(".config"));
            let data = var("XDG_DATA_HOME").unwrap_or_else(|| home.join(".local/share"));
            let desktops = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
            for list in mimeapps_lists(&config, &data, &desktops) {
                match repoint_mimeapps(&list) {
                    Ok(true) => log::info!("renamed: {} now names MailCraft's new desktop entry", list.display()),
                    Ok(false) => {}
                    Err(e) => log::warn!("renamed: {}: {e}", list.display()),
                }
            }
        }
        if let Some(super::Kind::Tarball(root)) = super::detect() {
            report("the old id's files in the unpacked folder", remove_all(&tarball_leftovers(&root)));
        }
    }
}

fn report(what: &str, (removed, failed): (usize, Vec<String>)) {
    if removed > 0 {
        log::info!("renamed: removed {removed} of {what}");
    }
    for f in failed {
        log::warn!("renamed: {f}");
    }
}

/// What macOS kept under the old bundle id, in `home`'s `Library`.
pub(super) fn mac_leftovers(home: &Path) -> Vec<PathBuf> {
    let lib = home.join("Library");
    vec![
        lib.join("Preferences").join(format!("{OLD_ID}.plist")),
        lib.join("Saved Application State").join(format!("{OLD_ID}.savedState")),
        lib.join("Caches").join(OLD_ID),
        lib.join("HTTPStorages").join(OLD_ID),
        lib.join("HTTPStorages").join(format!("{OLD_ID}.binarycookies")),
    ]
}

/// The old id's desktop entry, AppStream file and icons in a folder the `.tar.gz` was unpacked
/// into (`root`): an update copies the new ones in beside them.
pub(super) fn tarball_leftovers(root: &Path) -> Vec<PathBuf> {
    let share = root.join("share");
    let mut out = vec![share.join("applications").join(format!("{OLD_ID}.desktop")), share.join("metainfo").join(format!("{OLD_ID}.metainfo.xml"))];
    if let Ok(sizes) = std::fs::read_dir(share.join("icons/hicolor")) {
        for size in sizes.flatten().take(64) {
            for ext in ["png", "svg"] {
                out.push(size.path().join("apps").join(format!("{OLD_ID}.{ext}")));
            }
        }
    }
    out
}

/// Each of `paths` that's there removed (a folder with what's in it; a link itself, never
/// what it points to): how many went, and why the others didn't.
fn remove_all(paths: &[PathBuf]) -> (usize, Vec<String>) {
    let (mut removed, mut failed) = (0, Vec::new());
    for p in paths {
        let Ok(meta) = std::fs::symlink_metadata(p) else { continue };
        let gone = if meta.is_dir() { std::fs::remove_dir_all(p) } else { std::fs::remove_file(p) };
        match gone {
            Ok(()) => removed += 1,
            Err(e) => failed.push(format!("{}: {e}", p.display())),
        }
    }
    (removed, failed)
}

/// The `mimeapps.list` files where a default app is chosen (the XDG spec's, for this user):
/// the desktops' own (`gnome-mimeapps.list`), the general one, and the old place in
/// `applications/`.
pub(super) fn mimeapps_lists(config: &Path, data: &Path, desktops: &str) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = desktops
        .split(':')
        .map(|d| d.trim().to_ascii_lowercase())
        .filter(|d| !d.is_empty() && d.len() < 64 && d.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'))
        .take(8)
        .map(|d| config.join(format!("{d}-mimeapps.list")))
        .collect();
    out.push(config.join("mimeapps.list"));
    out.push(data.join("applications").join("mimeapps.list"));
    out
}

/// The largest `mimeapps.list` read.
const MAX_LIST: u64 = 1 << 20;

/// `list` with the old desktop entry's name changed to the new one's, if it names it: written
/// to a file beside it, then renamed over it. `false` when it doesn't name it (or isn't there).
pub(super) fn repoint_mimeapps(list: &Path) -> Result<bool, String> {
    let Ok(meta) = std::fs::symlink_metadata(list) else { return Ok(false) };
    if !meta.is_file() || meta.len() > MAX_LIST {
        return Ok(false);
    }
    let text = std::fs::read(list).map_err(|e| e.to_string())?;
    let Some(new) = repointed(&text) else { return Ok(false) };
    let Some(name) = list.file_name() else { return Ok(false) };
    let tmp = list.with_file_name(format!(".{}.mailcraft", name.to_string_lossy()));
    std::fs::write(&tmp, new).and_then(|()| std::fs::rename(&tmp, list)).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })?;
    Ok(true)
}

/// `text` with every `<old id>.desktop` made `<new id>.desktop`, or `None` when it names none
/// (or isn't UTF-8, as the spec has it: it's left as it is rather than changed byte by byte).
pub(super) fn repointed(text: &[u8]) -> Option<Vec<u8>> {
    let old = format!("{OLD_ID}.desktop");
    let new = format!("{}.desktop", mailcraft_ui_egui::APP_ID);
    let s = std::str::from_utf8(text).ok()?;
    s.contains(&old).then(|| s.replace(&old, &new).into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mailcraft-renamed-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The old Flatpak's mail comes across whole, once, into a sandbox that has none; a copy cut
    /// short leaves nothing half there; the old copy stays.
    #[test]
    fn a_flatpaks_mail_comes_across_once() {
        let t = scratch("carry");
        let old = t.join("old/data/mailcraft");
        std::fs::create_dir_all(old.join("blobs/ab")).unwrap();
        std::fs::write(old.join("accounts.json"), b"[{\"email\":\"zoe@studio.example\"}]").unwrap();
        std::fs::write(old.join("blobs/ab/cdef"), b"hello").unwrap();
        let new = t.join("new/data/mailcraft");
        assert_eq!(carry_over(&old, &new), Ok(true));
        assert_eq!(std::fs::read(new.join("accounts.json")).unwrap(), b"[{\"email\":\"zoe@studio.example\"}]");
        assert_eq!(std::fs::read(new.join("blobs/ab/cdef")).unwrap(), b"hello");
        assert!(old.join("accounts.json").is_file(), "the old copy stays");
        assert!(!t.join("new/data/mailcraft.carrying").exists());
        // This sandbox has mail now: nothing more comes, even when the old one changes.
        std::fs::write(old.join("accounts.json"), b"[]").unwrap();
        assert_eq!(carry_over(&old, &new), Ok(false));
        assert_eq!(std::fs::read(new.join("accounts.json")).unwrap(), b"[{\"email\":\"zoe@studio.example\"}]");
        // An empty folder made for this one already is filled; no old sandbox, nothing to do.
        let fresh = t.join("fresh");
        std::fs::create_dir_all(&fresh).unwrap();
        assert_eq!(carry_over(&old, &fresh), Ok(true));
        assert!(fresh.join("blobs/ab/cdef").is_file());
        assert_eq!(carry_over(&t.join("none"), &t.join("other")), Ok(false));
        assert!(!t.join("other").exists());
        // Nested past reason, it's refused and nothing is left half copied.
        let deep = (0..=MAX_DEPTH + 1).fold(t.join("deep"), |p, i| p.join(format!("d{i}")));
        std::fs::create_dir_all(&deep).unwrap();
        assert!(carry_over(&t.join("deep"), &t.join("deep-copy")).is_err());
        assert!(!t.join("deep-copy").exists() && !t.join("deep-copy.carrying").exists());
        let _ = std::fs::remove_dir_all(&t);
    }

    /// A default-app choice naming the old desktop entry names the new one; anything else, or a
    /// file that isn't UTF-8, is left as it is.
    #[test]
    fn the_default_mail_app_follows_the_new_desktop_entry() {
        let t = scratch("mimeapps");
        let list = t.join("mimeapps.list");
        std::fs::write(
            &list,
            "[Default Applications]\nx-scheme-handler/mailto=ai.storyteller.mailcraft.desktop;\ntext/html=firefox.desktop\n\n[Added Associations]\nmessage/rfc822=org.gnome.Evolution.desktop;ai.storyteller.mailcraft.desktop;\n",
        )
        .unwrap();
        assert_eq!(repoint_mimeapps(&list), Ok(true));
        let now = std::fs::read_to_string(&list).unwrap();
        assert_eq!(
            now,
            "[Default Applications]\nx-scheme-handler/mailto=io.github.zidan44.MailCraft.desktop;\ntext/html=firefox.desktop\n\n[Added Associations]\nmessage/rfc822=org.gnome.Evolution.desktop;io.github.zidan44.MailCraft.desktop;\n"
        );
        assert_eq!(repoint_mimeapps(&list), Ok(false), "once");
        assert!(std::fs::read_dir(&t).unwrap().count() == 1, "no file left beside it");
        assert_eq!(repointed(b"\xff ai.storyteller.mailcraft.desktop"), None);
        assert_eq!(repoint_mimeapps(&t.join("missing.list")), Ok(false));
        // The desktops' own lists come first, then the general one and the old place; a name that
        // isn't a desktop's is no file name.
        let lists = mimeapps_lists(Path::new("/c"), Path::new("/d"), "ubuntu:GNOME:../x");
        assert_eq!(
            lists,
            [
                PathBuf::from("/c/ubuntu-mimeapps.list"),
                PathBuf::from("/c/gnome-mimeapps.list"),
                PathBuf::from("/c/mimeapps.list"),
                PathBuf::from("/d/applications/mimeapps.list")
            ]
        );
        let _ = std::fs::remove_dir_all(&t);
    }

    /// In a folder the `.tar.gz` was unpacked into, the old id's files go and the new ones and
    /// everything else stay.
    #[test]
    fn an_unpacked_folder_loses_the_old_ids_files_only() {
        let t = scratch("tarball");
        let share = t.join("share");
        let mut keep = Vec::new();
        for (dir, old, new) in [
            ("applications", format!("{OLD_ID}.desktop"), "io.github.zidan44.MailCraft.desktop".to_string()),
            ("metainfo", format!("{OLD_ID}.metainfo.xml"), "io.github.zidan44.MailCraft.metainfo.xml".into()),
            ("icons/hicolor/64x64/apps", format!("{OLD_ID}.png"), "io.github.zidan44.MailCraft.png".into()),
            ("icons/hicolor/scalable/apps", format!("{OLD_ID}.svg"), "io.github.zidan44.MailCraft.svg".into()),
        ] {
            std::fs::create_dir_all(share.join(dir)).unwrap();
            std::fs::write(share.join(dir).join(old), b"old").unwrap();
            std::fs::write(share.join(dir).join(&new), b"new").unwrap();
            keep.push(share.join(dir).join(new));
        }
        std::fs::create_dir_all(t.join("bin")).unwrap();
        std::fs::write(t.join("bin/mailcraft"), b"exe").unwrap();
        let (removed, failed) = remove_all(&tarball_leftovers(&t));
        assert_eq!((removed, failed.len()), (4, 0));
        assert!(keep.iter().all(|k| k.is_file()) && t.join("bin/mailcraft").is_file());
        assert_eq!(remove_all(&tarball_leftovers(&t)), (0, Vec::new()), "nothing left to do");
        let _ = std::fs::remove_dir_all(&t);
    }

    /// What macOS kept under the old bundle id: its files and folders go, a link goes without
    /// what it points to.
    #[test]
    fn macos_leftovers_go() {
        let t = scratch("mac");
        let paths = mac_leftovers(&t);
        assert!(paths.iter().all(|p| p.starts_with(t.join("Library")) && p.to_string_lossy().contains(OLD_ID)));
        let prefs = t.join("Library/Preferences");
        std::fs::create_dir_all(&prefs).unwrap();
        std::fs::write(prefs.join(format!("{OLD_ID}.plist")), b"plist").unwrap();
        std::fs::write(prefs.join("com.example.other.plist"), b"other").unwrap();
        let saved = t.join("Library/Saved Application State").join(format!("{OLD_ID}.savedState"));
        std::fs::create_dir_all(&saved).unwrap();
        std::fs::write(saved.join("windows.plist"), b"w").unwrap();
        assert_eq!(remove_all(&paths), (2, Vec::new()));
        assert!(prefs.join("com.example.other.plist").is_file() && !saved.exists());
        let _ = std::fs::remove_dir_all(&t);
    }
}
