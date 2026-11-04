//! How a copy on Linux or FreeBSD was installed, among the ways MailCraft ships there: the
//! Flatpak bundle, the AppImage, the `.deb` and `.rpm` packages, and the `.tar.gz` unpacked
//! somewhere. The ways each one is told apart follow craft-libs' updater (`craft-update`), which
//! the other Craft apps use: Flatpak's sandbox, `APPIMAGE`, the package tools' own records, and a
//! tree laid out as the tarball's.

use std::path::{Path, PathBuf};

use super::{Kind, Package};

/// MailCraft's id: its package's name is `mailcraft`; its Flatpak and desktop entry's this.
const APP_ID: &str = mailcraft_ui_egui::APP_ID;

/// What detection asks of the system, so tests can answer for it.
pub(super) struct Probe<'a> {
    /// An environment variable, if set.
    pub env: &'a dyn Fn(&str) -> Option<PathBuf>,
    /// Whether a file is there.
    pub file: &'a dyn Fn(&Path) -> bool,
    /// Whether this user may make files in a folder.
    pub writable: &'a dyn Fn(&Path) -> bool,
    /// Whether the package tool of a kind has MailCraft installed.
    pub package: &'a dyn Fn(Package) -> bool,
    /// FreeBSD, where only the tarball is shipped.
    pub freebsd: bool,
}

/// How the copy at `exe` was installed, if the updater can update it.
pub(super) fn unix_kind(exe: &Path, p: &Probe) -> Option<Kind> {
    if !p.freebsd {
        // In Flatpak's sandbox (MailCraft is shipped as a bundle only: a copy from a repository
        // would be that repository's to update, and MailCraft is in none).
        if (p.env)("FLATPAK_ID").is_some_and(|id| id.as_os_str() == APP_ID) || (p.file)(Path::new("/.flatpak-info")) {
            return Some(Kind::Flatpak);
        }
        // Replaced by a rename beside it: its folder must be ours to write (an AppImage kept in
        // /opt by root keeps File › About's Download link).
        if let Some(image) = (p.env)("APPIMAGE").filter(|i| (p.file)(i)) {
            if !image.parent().is_some_and(|d| (p.writable)(d)) {
                log::info!("updates: {} is in a folder this user can't write, so it isn't replaced", image.display());
                return None;
            }
            return Some(Kind::AppImage(image));
        }
        // The packages put it in /usr/bin; their tool says whether it's theirs.
        if exe == Path::new("/usr/bin/mailcraft") {
            return [Package::Deb, Package::Rpm].into_iter().find(|k| (p.package)(*k)).map(|k| Kind::Package(k, exe.to_path_buf()));
        }
    }
    // The tarball's tree: bin/mailcraft beside share/, with MailCraft's AppStream file in it.
    let bin = exe.parent()?;
    let root = bin.parent()?;
    let system = [Path::new("/usr"), Path::new("/usr/local"), Path::new("/")].contains(&root);
    if bin.file_name()? != "bin" || system || !(p.file)(&root.join("share/metainfo").join(format!("{APP_ID}.metainfo.xml"))) {
        return None;
    }
    if !(p.writable)(root) {
        log::info!("updates: {} is in a folder this user can't write, so it isn't replaced", root.display());
        return None;
    }
    Some(Kind::Tarball(root.to_path_buf()))
}

/// Whether the package tool of `kind` has MailCraft installed (`dpkg-query`, `rpm -q`).
pub(super) fn installed_package(kind: Package) -> bool {
    let out = match kind {
        Package::Deb => std::process::Command::new("dpkg-query").args(["-W", "-f", "${db:Status-Status}", "mailcraft"]).output(),
        Package::Rpm => std::process::Command::new("rpm").args(["-q", "mailcraft"]).output(),
    };
    out.is_ok_and(|o| o.status.success() && (kind == Package::Rpm || String::from_utf8_lossy(&o.stdout).trim() == "installed"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(exe: &str, env: &[(&str, &str)], files: &[&str], writable: bool, package: Option<Package>, freebsd: bool) -> Option<Kind> {
        let env = |k: &str| env.iter().find(|(n, _)| *n == k).map(|(_, v)| PathBuf::from(v));
        let file = |f: &Path| files.iter().any(|x| Path::new(x) == f);
        let writable = |_: &Path| writable;
        let package = |k: Package| package == Some(k);
        unix_kind(Path::new(exe), &Probe { env: &env, file: &file, writable: &writable, package: &package, freebsd })
    }

    /// Each way MailCraft ships on Linux and FreeBSD is told apart; one this user can't update,
    /// or that isn't MailCraft's own, isn't taken.
    #[test]
    fn the_install_kind_on_linux_and_freebsd() {
        let meta = "/home/zoe/Apps/mailcraft-0.33.0-linux-x86_64/share/metainfo/ai.storyteller.mailcraft.metainfo.xml";
        let tree = "/home/zoe/Apps/mailcraft-0.33.0-linux-x86_64/bin/mailcraft";
        // The Flatpak sandbox, whatever else is there.
        assert_eq!(kind("/app/bin/mailcraft", &[("FLATPAK_ID", "ai.storyteller.mailcraft")], &[], false, None, false), Some(Kind::Flatpak));
        assert_eq!(kind("/app/bin/mailcraft", &[], &["/.flatpak-info"], false, None, false), Some(Kind::Flatpak));
        // An AppImage, where it can be renamed over.
        let image = "/home/zoe/MailCraft.AppImage";
        assert_eq!(kind("/tmp/.mount_x/usr/bin/mailcraft", &[("APPIMAGE", image)], &[image], true, None, false), Some(Kind::AppImage(image.into())));
        assert_eq!(kind("/tmp/.mount_x/usr/bin/mailcraft", &[("APPIMAGE", image)], &[image], false, None, false), None);
        // The packages, by their tools' word.
        for p in [Package::Deb, Package::Rpm] {
            assert_eq!(kind("/usr/bin/mailcraft", &[], &[], false, Some(p), false), Some(Kind::Package(p, "/usr/bin/mailcraft".into())));
        }
        assert_eq!(kind("/usr/bin/mailcraft", &[], &[], false, None, false), None, "put there some other way");
        // The tarball's tree, where it can be written; on FreeBSD too, and only that.
        assert_eq!(kind(tree, &[], &[meta], true, None, false), Some(Kind::Tarball("/home/zoe/Apps/mailcraft-0.33.0-linux-x86_64".into())));
        assert_eq!(kind(tree, &[], &[meta], true, None, true), Some(Kind::Tarball("/home/zoe/Apps/mailcraft-0.33.0-linux-x86_64".into())));
        assert_eq!(kind(tree, &[], &[meta], false, None, false), None, "a folder this user can't write");
        assert_eq!(kind(tree, &[], &[], true, None, false), None, "not the tarball's tree");
        assert_eq!(kind("/home/zoe/mailcraft/target/release/mailcraft", &[], &[], true, None, false), None, "a developer's build");
        assert_eq!(
            kind("/usr/local/bin/mailcraft", &[], &["/usr/local/share/metainfo/ai.storyteller.mailcraft.metainfo.xml"], true, None, true),
            None
        );
        assert_eq!(kind("/app/bin/mailcraft", &[("FLATPAK_ID", "ai.storyteller.mailcraft")], &[], false, None, true), None, "no Flatpak on FreeBSD");
    }
}
