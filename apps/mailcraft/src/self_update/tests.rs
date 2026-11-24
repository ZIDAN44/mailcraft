use super::install::*;
use super::*;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mailcraft-update-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn verbatim_paths_are_made_plain() {
    assert_eq!(plain(PathBuf::from(r"\\?\C:\Tools\MailCraft\mailcraft.exe")), PathBuf::from(r"C:\Tools\MailCraft\mailcraft.exe"));
    assert_eq!(plain(PathBuf::from(r"\\?\UNC\server\share\m.exe")), PathBuf::from(r"\\server\share\m.exe"));
    assert_eq!(plain(PathBuf::from("/Applications/MailCraft.app")), PathBuf::from("/Applications/MailCraft.app"));
}

/// A new AppImage takes the old one's place, runnable, and leaves nothing beside it; one left at
/// the name a copy used to be made under (a link, say) is never written through.
#[test]
fn an_appimage_is_replaced_through_a_file_of_its_own() {
    let dir = scratch("appimage-replace");
    let (new, image, other) = (dir.join("download.AppImage"), dir.join("MailCraft.AppImage"), dir.join("someone-elses"));
    std::fs::write(&new, b"the new version").unwrap();
    std::fs::write(&image, b"the old version").unwrap();
    std::fs::write(&other, b"not MailCraft's").unwrap();
    // The name the copy was made under before: a link there led the update elsewhere.
    #[cfg(unix)]
    std::os::unix::fs::symlink(&other, dir.join(".MailCraft.AppImage.new")).unwrap();
    #[cfg(not(unix))]
    std::fs::write(dir.join(".MailCraft.AppImage.new"), b"left over").unwrap();
    replace_appimage(&new, &image).unwrap();
    assert_eq!(std::fs::read(&image).unwrap(), b"the new version");
    assert_eq!(std::fs::read(&other).unwrap(), b"not MailCraft's");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&image).unwrap().permissions().mode() & 0o777, 0o755);
    }
    let mut left: Vec<String> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    left.sort();
    assert_eq!(left, [".MailCraft.AppImage.new", "MailCraft.AppImage", "download.AppImage", "someone-elses"]);
    // A download that can't be read changes nothing, and leaves nothing.
    assert!(replace_appimage(&dir.join("gone.AppImage"), &image).is_err());
    assert_eq!(std::fs::read(&image).unwrap(), b"the new version");
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 4);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn checks_spread_around_the_hour() {
    let waits: Vec<u64> = [0, 1, 599, 600, 1200, u64::MAX].iter().map(|s| next_wait(*s).as_secs()).collect();
    assert!(waits.iter().all(|w| (50 * 60..=70 * 60).contains(w)), "{waits:?}");
    assert_ne!(waits[0], waits[2]);
}

/// A check that fails (no connection, GitHub's quota used up) is tried again sooner than the
/// hour, then less and less often.
#[test]
fn failed_checks_back_off_up_to_the_hour() {
    let waits: Vec<u64> = (1..=8).map(|n| retry_wait(n).as_secs() / 60).collect();
    assert_eq!(waits, [5, 10, 20, 40, 60, 60, 60, 60]);
    assert_eq!(retry_wait(u32::MAX), EVERY);
}

#[test]
fn the_version_just_installed_isnt_offered_again_for_a_day() {
    let dir = scratch("marker");
    let (old, new) = ("0.21.0 1111111", "0.21.0 2222222");
    assert!(!just_installed(&dir, "v0.21.0-test.19", 1_000_000, new));
    // Written by the build that installed it; read by the new one.
    std::fs::write(dir.join(MARKER), format!("v0.21.0-test.19\n1000000\n{old}\n")).unwrap();
    assert!(just_installed(&dir, "v0.21.0-test.19", 1_000_000 + 3600, new));
    assert!(!just_installed(&dir, "v0.21.0-test.20", 1_000_000 + 3600, new), "another version is offered");
    assert!(!just_installed(&dir, "v0.21.0-test.19", 1_000_000 + JUST_INSTALLED_FOR, new), "a day later it may be again");
    assert!(!just_installed(&dir, "v0.21.0-test.19", 999_000, new), "a clock gone back proves nothing");
    // Read by the build that wrote it, still running: the install didn't take (refused, failed,
    // or put off while MailCraft still ran), and the version is offered again.
    assert!(!just_installed(&dir, "v0.21.0-test.19", 1_000_000 + 3600, old));
    // A marker from before it named its build holds as it did.
    std::fs::write(dir.join(MARKER), "v0.21.0-test.19\n1000000\n").unwrap();
    assert!(just_installed(&dir, "v0.21.0-test.19", 1_000_000 + 3600, old));
    std::fs::write(dir.join(MARKER), "junk").unwrap();
    assert!(!just_installed(&dir, "junk", 1_000_000, new));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A download left part way is carried on soon after start.
#[test]
fn a_partial_download_is_noticed() {
    let dir = scratch("part");
    assert!(!has_part(&dir) && !has_part(&dir.join("missing")));
    std::fs::write(dir.join("mailcraft-0.25.0-windows-x64.msi"), b"").unwrap();
    assert!(!has_part(&dir));
    std::fs::write(dir.join("mailcraft-0.25.0-windows-x64.msi.42.part"), b"").unwrap();
    assert!(has_part(&dir));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Paths with quotes and spaces stay one word in the helper's scripts.
#[test]
fn helper_scripts_quote_their_paths() {
    let odd = Path::new(r"C:\Users\O'Brien\My Apps\mailcraft-0.21.0-windows-x64.msi");
    let msi = msi_step(odd);
    assert!(msi.contains(r#"'"C:\Users\O''Brien\My Apps\mailcraft-0.21.0-windows-x64.msi"'"#), "{msi}");
    let zip = portable_step(Path::new(r"C:\Tools\MailCraft\x.zip"), Path::new(r"C:\Tools\Mail'Craft"));
    assert!(zip.contains(r"-Destination 'C:\Tools\Mail''Craft'"), "{zip}");
    // PowerShell ends a quoted string at a typographic quote as well: doubled too.
    assert_eq!(ps_quote("D:\\Zoë’s ‘mail’ ‚x‛"), "'D:\\Zoë’’s ‘‘mail’’ ‚‚x‛‛'");
    assert_eq!(sh_quote("/Applications/Mail'Craft.app"), r"'/Applications/Mail'\''Craft.app'");
    let mac = mac_step(Path::new("/tmp/m.dmg"), Path::new("/Applications/MailCraft.app"));
    assert!(mac.contains("ditto \"$m/MailCraft.app\" '/Applications/MailCraft.app'.new"), "{mac}");
    // Only a whole copy takes the app's place, and a failed move puts the old one back.
    assert!(mac.starts_with("rm -rf '/Applications/MailCraft.app'.new;"), "a copy left by an earlier try goes first");
    assert!(mac.contains("[ $ok -eq 0 ] &&") && mac.contains("|| mv '/Applications/MailCraft.app'.old '/Applications/MailCraft.app'"), "{mac}");
}

/// The helper installs nothing while MailCraft still runs (it waited and MailCraft didn't end):
/// copied over a running copy, some files would be new and some old. Once MailCraft has ended,
/// the step runs. The helper's own shell runs the script, with the wait cut to a second.
#[test]
fn nothing_is_installed_while_mailcraft_still_runs() {
    let dir = scratch("still-running");
    let done = dir.join("installed");
    let log = dir.join("install.log");
    // A process that has ended, kept (its `Child`) until the checks are done: on Windows its id
    // can't be given to another process while it's held (on a busy computer one would otherwise
    // get it, and the update would wait on that one).
    let mut gone = if cfg!(windows) {
        std::process::Command::new("cmd").args(["/C", "exit"]).spawn().unwrap()
    } else {
        std::process::Command::new("true").spawn().unwrap()
    };
    gone.wait().unwrap();
    let ended = gone.id();
    for (pid, installs) in [(std::process::id(), false), (ended, true)] {
        let _ = std::fs::remove_file(&done);
        let status = if cfg!(windows) {
            let step = format!("Set-Content -LiteralPath {} -Value yes", ps_quote(&done.display().to_string()));
            let script = windows_script(pid, &step, Path::new(r"C:\nowhere\mailcraft.exe"), false, &log, 1);
            std::process::Command::new("powershell.exe").args(["-NoProfile", "-NonInteractive", "-Command", &script]).status().unwrap()
        } else {
            let step = format!("echo yes > {}", sh_quote(&done));
            std::process::Command::new("sh").arg("-c").arg(unix_script(pid, &step, None, &log, 1)).status().unwrap()
        };
        assert!(status.success());
        assert_eq!(done.exists(), installs, "pid {pid}: {}", std::fs::read_to_string(&log).unwrap_or_default());
        if !installs {
            assert!(std::fs::read_to_string(&log).unwrap_or_default().contains("still running"));
        }
    }
    drop(gone);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The Mac step run by a shell, `hdiutil` and `ditto` stood in for: a disk image refused twice
/// (the system still busy with it) is attached the third time and the app replaced; one refused
/// every time leaves the app as it was.
#[cfg(unix)]
#[test]
fn a_busy_disk_image_is_attached_again() {
    use std::os::unix::fs::PermissionsExt;
    let dir = scratch("busy-dmg");
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let tries = dir.join("tries");
    let tool = |name: &str, body: &str| {
        let p = bin.join(name);
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    };
    // attach: refused while `tries` has no more than $REFUSE lines, then "mounts" the new app.
    let t = tries.display();
    tool(
        "hdiutil",
        &format!(
            "[ \"$1\" = attach ] || exit 0; echo x >> '{t}'; [ $(wc -l < '{t}') -gt $REFUSE ] || exit 1; mkdir -p \"$5/MailCraft.app\" && echo new > \"$5/MailCraft.app/v\""
        ),
    );
    tool("ditto", "cp -R \"$1\" \"$2\"");
    // The waits are cut short.
    tool("sleep", "exit 0");
    for (refuse, want) in [("2", "new"), ("99", "old")] {
        let _ = std::fs::remove_file(&tries);
        let app = dir.join("MailCraft.app");
        let _ = std::fs::remove_dir_all(&app);
        std::fs::create_dir_all(&app).unwrap();
        std::fs::write(app.join("v"), "old").unwrap();
        let dmg = dir.join("m.dmg");
        std::fs::write(&dmg, "").unwrap();
        let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default());
        std::process::Command::new("sh").arg("-c").arg(mac_step(&dmg, &app)).env("PATH", path).env("REFUSE", refuse).status().unwrap();
        assert_eq!(std::fs::read_to_string(app.join("v")).unwrap().trim(), want, "refused {refuse} times");
        assert_eq!(dmg.exists(), want == "old", "refused {refuse} times: the download is kept only when it wasn't used");
        let attempts = std::fs::read_to_string(&tries).unwrap().lines().count();
        assert_eq!(attempts, if refuse == "2" { 3 } else { 7 }, "refused {refuse} times");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// The kind of copy is told from where the exe is (Windows: beside a portable marker, or in
/// Program Files\MailCraft); the names pick the matching file of a release.
#[test]
fn the_install_kind_follows_where_mailcraft_is() {
    if cfg!(windows) {
        let dir = scratch("kind");
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
    // An AppImage is replaced only where this user may write beside it (/proc: nobody may, not
    // even root, as in CI's containers).
    if cfg!(target_os = "linux") {
        let dir = scratch("appimage");
        let image = dir.join("MailCraft-x86_64.AppImage");
        std::fs::write(&image, b"").unwrap();
        // Where the image is mounted, as the AppImage runtime says (`APPDIR`).
        let mount = dir.join("mount");
        std::fs::create_dir_all(mount.join("usr").join("bin")).unwrap();
        // As `detect` sees the running copy: its own path.
        let mount = mount.canonicalize().unwrap();
        let exe = mount.join("usr").join("bin").join("mailcraft");
        let found = image.clone();
        let appdir = mount.clone();
        let env = move |k: &str| match k {
            "APPIMAGE" => Some(found.clone()),
            "APPDIR" => Some(appdir.clone()),
            _ => None,
        };
        assert_eq!(detect_at(&exe, &env), Some(Kind::AppImage(image.clone())));
        let elsewhere = dir.join("elsewhere").join("mailcraft");
        assert_eq!(detect_at(&elsewhere, &env), None, "another program's AppImage, passed on");
        let appdir = mount.clone();
        let env = move |k: &str| match k {
            "APPIMAGE" => Some(PathBuf::from("/proc/version")),
            "APPDIR" => Some(appdir.clone()),
            _ => None,
        };
        assert_eq!(detect_at(&exe, &env), None, "a folder it can't write");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// The window's view of it: what's said, and that Close without updating keeps it for later.
#[test]
fn the_window_sees_where_the_update_is() {
    let dir = scratch("state");
    let u = Updater::new(
        Kind::Msi(PathBuf::from(r"C:\Program Files\MailCraft\mailcraft.exe")),
        dir.clone(),
        mailcraft_net::updates::Feed::github(),
        "0.24.0".into(),
        false,
    );
    assert_eq!(u.state(), UpdateState::None);
    assert!(!u.restart_after_close(), "nothing to restart into");
    // Each change wakes the window, which an idle or minimized one needs to see it.
    let woken = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let w = woken.clone();
    u.set_waker(Arc::new(move || {
        w.fetch_add(1, Ordering::SeqCst);
    }));
    u.set_status(Status::Downloading { tag: "v0.25.0-test.22".into() });
    assert_eq!(woken.load(Ordering::SeqCst), 1);
    u.shared.progress.done.store(10, Ordering::Relaxed);
    u.shared.progress.total.store(40, Ordering::Relaxed);
    assert_eq!(u.state(), UpdateState::Downloading { version: "0.25.0-test.22".into(), done: 10, total: 40 });
    u.set_status(Status::Ready { tag: "v0.25.0-test.22".into(), file: dir.join("x.msi") });
    assert_eq!(
        u.state(),
        UpdateState::Ready { version: "0.25.0-test.22".into(), asks: InstallAsks::Administrator },
        "the MSI asks for an administrator"
    );
    let portable = Updater::new(Kind::Portable(PathBuf::from("m.exe")), dir.clone(), mailcraft_net::updates::Feed::github(), "0.24.0".into(), false);
    portable.set_status(Status::Ready { tag: "v0.25.0-test.22".into(), file: dir.join("x.zip") });
    assert_eq!(portable.state(), UpdateState::Ready { version: "0.25.0-test.22".into(), asks: InstallAsks::Nothing });
    // Closing without updating: nothing is started.
    u.keep_for_later();
    u.closing();
    assert!(!u.shared.started.load(Ordering::SeqCst) && u.shared.stop.load(Ordering::SeqCst));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A portable zip that can't be unpacked changes nothing and stays, to be tried again from (the
/// check takes a file already there and right as it is, rather than downloading it again); one
/// that unpacks is put in place and goes.
#[cfg(windows)]
#[test]
fn a_damaged_portable_zip_is_kept() {
    let dir = scratch("zip-damaged");
    let app = dir.join("MailCraft");
    std::fs::create_dir_all(&app).unwrap();
    std::fs::write(app.join("mailcraft.exe"), b"old").unwrap();
    let ps =
        |script: &str| std::process::Command::new("powershell.exe").args(["-NoProfile", "-NonInteractive", "-Command", script]).status().unwrap();
    let bad = dir.join("bad.zip");
    std::fs::write(&bad, b"not a zip").unwrap();
    ps(&portable_step(&bad, &app));
    assert_eq!(std::fs::read(app.join("mailcraft.exe")).unwrap(), b"old");
    assert!(bad.is_file(), "kept, to be tried again");
    assert!(!app.join(".mailcraft-update").exists());
    let top = dir.join("pack").join("mailcraft-0.40.0-windows-x64");
    std::fs::create_dir_all(&top).unwrap();
    std::fs::write(top.join("mailcraft.exe"), b"new").unwrap();
    let good = dir.join("good.zip");
    let q = |p: &Path| ps_quote(&p.display().to_string());
    assert!(ps(&format!("Compress-Archive -Path {} -DestinationPath {}", q(&top), q(&good))).success());
    ps(&portable_step(&good, &app));
    assert_eq!(std::fs::read(app.join("mailcraft.exe")).unwrap(), b"new");
    assert!(!good.exists(), "installed: the download goes");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Updates turned off in the settings (issue #207, or by an organisation, #245): a check asks
/// nothing, and an update downloaded before isn't installed as MailCraft closes. Before the
/// settings are read, nothing is asked either. On, the same check asks the feed (here one
/// nothing answers at).
#[test]
fn updates_turned_off_ask_nothing() {
    let dir = scratch("choice");
    let feed = mailcraft_net::updates::Feed::local("http://127.0.0.1:9/").unwrap();
    let u = Updater::new(Kind::Msi(PathBuf::from(r"C:\Program Files\MailCraft\mailcraft.exe")), dir.clone(), feed, "0.24.0".into(), false);
    assert_eq!(u.check(None, &mut None), Ok(()), "the settings not read yet");
    u.follow_choice(mailcraft_engine::policy::Updates::On);
    assert!(u.check(None, &mut None).is_err(), "on: the feed is asked");
    u.follow_choice(mailcraft_engine::policy::Updates::Off);
    assert_eq!(u.check(None, &mut None), Ok(()));
    u.follow_choice(mailcraft_engine::policy::Updates::Stable);
    assert!(u.check(None, &mut None).is_err());
    u.follow_choice(mailcraft_engine::policy::Updates::Off);
    u.set_status(Status::Ready { tag: "v0.25.0".into(), file: dir.join("x.msi") });
    u.closing();
    assert!(!u.shared.started.load(Ordering::SeqCst), "not installed");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The first start after an update says how it ended, once: this is the version installed, or
/// the one that didn't install is offered again; nothing while it may still be installing, a
/// day on, or with no marker (as EffectCraft's updater says at its first start after an update).
#[test]
fn the_first_start_after_an_update_says_how_it_ended() {
    let dir = scratch("outcome");
    let (old, new) = ("0.42.0 aaa", "0.43.0 bbb");
    let mark = |by: &str| std::fs::write(dir.join(MARKER), format!("v0.43.0\n1000000\n{by}\n")).unwrap();
    mark(old);
    assert_eq!(outcome(&dir, "0.43.0", new, 1_000_060), Some(UpdateOutcome::Updated { version: "0.43.0".into() }));
    assert_eq!(outcome(&dir, "0.43.0", new, 1_000_120), None, "said once");
    assert!(just_installed(&dir, "v0.43.0", 1_000_120, new), "the marker still holds the version back");
    mark(old);
    assert_eq!(outcome(&dir, "0.42.0", old, 1_000_010), None, "still installing");
    assert_eq!(outcome(&dir, "0.42.0", old, 1_000_000 + INSTALL_TIME), Some(UpdateOutcome::NotInstalled { version: "0.43.0".into() }));
    assert!(!just_installed(&dir, "v0.43.0", 1_000_000 + INSTALL_TIME, old), "offered again");
    mark(old);
    assert_eq!(outcome(&dir, "0.43.0", new, 1_000_000 + JUST_INSTALLED_FOR), None, "a day later");
    std::fs::remove_file(dir.join(MARKER)).unwrap();
    assert_eq!(outcome(&dir, "0.43.0", new, 1_000_060), None, "no marker");
}
