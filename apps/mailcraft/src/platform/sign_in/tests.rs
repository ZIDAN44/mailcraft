use super::*;

/// An AppImage's entry names the image, not the folder it's unpacked to for this run (gone when
/// it ends); a program elsewhere, though `APPIMAGE` is set (another AppImage started it), is
/// itself.
#[test]
fn an_appimage_is_started_by_its_file() {
    let image = PathBuf::from("/home/ann/Apps/MailCraft.AppImage");
    let dir = PathBuf::from("/tmp/.mount_MailCrXYZ");
    assert_eq!(program_of(dir.join("usr/bin/mailcraft"), Some(image.clone()), Some(dir.clone())), image);
    assert_eq!(program_of(PathBuf::from("/usr/bin/mailcraft"), Some(image.clone()), Some(dir)), PathBuf::from("/usr/bin/mailcraft"));
    assert_eq!(program_of(PathBuf::from("/usr/bin/mailcraft"), None, None), PathBuf::from("/usr/bin/mailcraft"));
}

/// Issue #87: the user's Run value starts this program, quoted; it reads as on only while it
/// names this one, and goes when turned off. Under a name of the test's own, not MailCraft's.
#[cfg(windows)]
#[test]
fn windows_starts_it_from_the_users_run_key() {
    let name = format!("MailCraft-test-{}", std::process::id());
    let me = Path::new(r"C:\Program Files\MailCraft\mailcraft.exe");
    assert_eq!(system::command(me), r#""C:\Program Files\MailCraft\mailcraft.exe""#);
    assert!(!system::on_as(&name, me));
    system::set_as(&name, me, true).unwrap();
    let on = system::on_as(&name, me);
    let other = system::on_as(&name, Path::new(r"D:\MailCraft\mailcraft.exe"));
    system::set_as(&name, me, false).unwrap();
    assert!(on && !other, "on: {on}, for another copy: {other}");
    assert!(!system::on_as(&name, me));
    // Turned off when it isn't there: nothing to do.
    system::set_as(&name, me, false).unwrap();

    // Turned off in Settings › Apps › Startup (or Task Manager), which leaves the Run value: off
    // here too, and on again when turned on here.
    let approved = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
    system::set_as(&name, me, true).unwrap();
    let key = windows_registry::CURRENT_USER.create(approved).unwrap();
    key.set_bytes(&name, windows_registry::Type::Bytes, &[3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]).unwrap();
    let off_there = system::on_as(&name, me);
    system::set_as(&name, me, true).unwrap();
    let on_again = system::on_as(&name, me);
    let approval_gone = key.get_value(&name).is_err();
    key.set_bytes(&name, windows_registry::Type::Bytes, &[2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]).unwrap();
    let on_there = system::on_as(&name, me);
    system::set_as(&name, me, false).unwrap();
    assert!(
        !off_there && on_again && approval_gone && on_there,
        "off there: {off_there}, on again: {on_again}, {approval_gone}, on there: {on_there}"
    );
    assert!(key.get_value(&name).is_err(), "Windows' setting for it goes with it");
}

/// Issue #87: the autostart entry names this program as the Desktop Entry Specification quotes
/// it, reads as on only while it's this one's and not switched off there, and goes when turned
/// off.
#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn linux_starts_it_from_an_autostart_entry() {
    let config = std::env::temp_dir().join(format!("mailcraft-sign-in-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&config);
    let me = Path::new("/home/ann/Apps/MailCraft.AppImage");
    assert!(!system::on_in(&config, me));
    system::set_in(&config, me, true).unwrap();
    assert!(system::on_in(&config, me));
    assert!(!system::on_in(&config, Path::new("/usr/bin/mailcraft")), "another copy's");
    let file = config.join("autostart/io.github.zidan44.MailCraft.desktop");
    let entry = std::fs::read_to_string(&file).unwrap();
    assert!(entry.starts_with("[Desktop Entry]\n") && entry.contains("\nExec=\"/home/ann/Apps/MailCraft.AppImage\"\n"), "{entry}");
    // Switched off by the desktop's own settings.
    std::fs::write(&file, format!("{entry}Hidden=true\n")).unwrap();
    assert!(!system::on_in(&config, me));
    system::set_in(&config, me, false).unwrap();
    assert!(!file.exists());
    system::set_in(&config, me, false).unwrap();
    let _ = std::fs::remove_dir_all(&config);
    // What the quoting rules escape, and a path that can't be one line.
    assert_eq!(system::exec(Path::new("/opt/My Mail/mailcraft")).as_deref(), Some("Exec=\"/opt/My Mail/mailcraft\""));
    assert_eq!(system::exec(Path::new("/opt/a\"b`c$d\\e%f")).as_deref(), Some(r#"Exec="/opt/a\\"b\\`c\\$d\\\\e%%f""#));
    assert_eq!(system::exec(Path::new("/opt/two\nlines")), None);
}

/// Issue #87: the launch agent starts this program at sign-in, reads as on only while it's this
/// one's, and goes when turned off.
#[cfg(target_os = "macos")]
#[test]
fn a_mac_starts_it_from_a_launch_agent() {
    let agents = std::env::temp_dir().join(format!("mailcraft-sign-in-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&agents);
    let me = Path::new("/Applications/Mail & More/MailCraft.app/Contents/MacOS/mailcraft");
    assert!(!system::on_in(&agents, me));
    system::set_in(&agents, me, true).unwrap();
    assert!(system::on_in(&agents, me));
    assert!(!system::on_in(&agents, Path::new("/Applications/MailCraft.app/Contents/MacOS/mailcraft")), "another copy's");
    let agent = std::fs::read_to_string(agents.join("io.github.zidan44.MailCraft.plist")).unwrap();
    assert!(agent.contains("<string>/Applications/Mail &amp; More/MailCraft.app/Contents/MacOS/mailcraft</string>"), "{agent}");
    assert!(agent.contains("<key>RunAtLoad</key>\n\t<true/>"), "{agent}");
    system::set_in(&agents, me, false).unwrap();
    assert!(!system::on_in(&agents, me));
    system::set_in(&agents, me, false).unwrap();
    let _ = std::fs::remove_dir_all(&agents);
}
