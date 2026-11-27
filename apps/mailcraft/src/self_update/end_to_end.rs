//! The updater from end to end, against releases served on this computer: it lists them, picks
//! the one this build follows, downloads and checks its file, and its helper installs it over a
//! copy laid out as one installed from the portable zip (Windows) or the tarball (Linux, FreeBSD),
//! once the process it waits for has ended. And the shell steps for the packages, run with the
//! system's tools stood in for.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::install::*;
use super::*;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mailcraft-e2e-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// GitHub's answer for a release `tag` with one file, as the feed at `base` serves it.
fn release_json(base: &str, tag: &str, name: &str, body: &[u8], prerelease: bool) -> String {
    format!(
        r#"{{"tag_name":"{tag}","draft":false,"prerelease":{prerelease},"target_commitish":"0123456789abcdef0123456789abcdef01234567",
           "html_url":"{base}tag/{tag}","assets":[{{"name":"{name}","size":{size},"digest":"sha256:{sum}",
           "browser_download_url":"{base}download/{tag}/{name}"}}]}}"#,
        size = body.len(),
        sum = mailcraft_net::updates::sha256_hex(body),
    )
}

/// A process that has ended, for the helper to wait for.
fn ended() -> u32 {
    let mut child = if cfg!(windows) {
        std::process::Command::new("cmd").args(["/C", "exit"]).spawn().unwrap()
    } else {
        std::process::Command::new("true").spawn().unwrap()
    };
    let pid = child.id();
    child.wait().unwrap();
    pid
}

fn wait_for(what: &str, secs: u64, mut done: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(secs);
    while !done() {
        assert!(Instant::now() < end, "{what}: not in {secs} s");
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// A build of the 0.32 release follows releases only: offered 0.33.0 and not the later test
/// build of 0.34, it downloads and checks it, and installed after this "MailCraft" ended, the new
/// version is where the old one was, its data kept.
#[test]
fn a_release_build_updates_to_the_next_release_from_end_to_end() {
    let dir = scratch("release");
    let app = dir.join("MailCraft");
    let (new, old) = (b"the new MailCraft".to_vec(), b"the old MailCraft".to_vec());
    // The copy installed, and the new version's file as its release has it.
    let (kind, name, archive, installed) = if cfg!(windows) {
        std::fs::create_dir_all(app.join("MailCraftData")).unwrap();
        std::fs::write(app.join("MailCraftData").join("accounts.json"), b"[]").unwrap();
        std::fs::write(app.join("portable.txt"), b"").unwrap();
        std::fs::write(app.join("mailcraft.exe"), &old).unwrap();
        let top = dir.join("pack").join("mailcraft-0.33.0-windows-x64");
        std::fs::create_dir_all(&top).unwrap();
        std::fs::write(top.join("mailcraft.exe"), &new).unwrap();
        std::fs::write(top.join("portable.txt"), b"").unwrap();
        let zip = dir.join("new.zip");
        let made = std::process::Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                &format!("Compress-Archive -Path {} -DestinationPath {}", ps_quote(&top.display().to_string()), ps_quote(&zip.display().to_string())),
            ])
            .status()
            .unwrap();
        assert!(made.success());
        (
            Kind::Portable(app.join("mailcraft.exe")),
            "mailcraft-0.33.0-windows-x64-portable.zip".to_string(),
            std::fs::read(&zip).unwrap(),
            app.join("mailcraft.exe"),
        )
    } else {
        for d in ["bin", "share/metainfo", "data"] {
            std::fs::create_dir_all(app.join(d)).unwrap();
        }
        std::fs::write(app.join("data").join("accounts.json"), b"[]").unwrap();
        std::fs::write(app.join("bin").join("mailcraft"), &old).unwrap();
        let top = dir.join("pack").join("mailcraft-0.33.0-linux-x86_64");
        std::fs::create_dir_all(top.join("bin")).unwrap();
        std::fs::write(top.join("bin").join("mailcraft"), &new).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(top.join("bin").join("mailcraft"), std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let tgz = dir.join("new.tar.gz");
        let made = std::process::Command::new("tar")
            .arg("-czf")
            .arg(&tgz)
            .arg("-C")
            .arg(dir.join("pack"))
            .arg("mailcraft-0.33.0-linux-x86_64")
            .status()
            .unwrap();
        assert!(made.success());
        let kind = Kind::Tarball(app.clone());
        let name = format!("mailcraft-0.33.0{}", kind.suffix());
        (kind, name, std::fs::read(&tgz).unwrap(), app.join("bin").join("mailcraft"))
    };
    let later_name = name.replace("0.33.0", "0.34.0");
    let mut files = Vec::new();
    // The list names the files' addresses, so the port is taken before what's served is made;
    // it stays this test's (freed and bound again, another test's server could take it between).
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://127.0.0.1:{}/", listener.local_addr().unwrap().port());
    let list = format!(
        "[{},{}]",
        release_json(&base, "v0.34.0-test.40", &later_name, b"a test build", true),
        release_json(&base, "v0.33.0", &name, &archive, false)
    );
    files.push(("/releases".to_string(), list.into_bytes()));
    files.push((format!("/download/v0.33.0/{name}"), archive.clone()));
    serve(listener, files);
    let feed = mailcraft_net::updates::Feed::local(&base).unwrap();
    let data = dir.join("data-folder");
    std::fs::create_dir_all(&data).unwrap();
    let mut u = Updater::new(kind, data.clone(), feed, "0.32.0".into(), true);
    // How an update installs, as a build without a key takes it (a release unsigned): the
    // signature is held to in its own test, `an_update_is_taken_only_with_its_checksums_signed`.
    u.keys = &[];
    // The settings read: updating as installed.
    u.follow_choice(mailcraft_engine::policy::Updates::On);
    let u = u.launch(None, Some(Duration::ZERO)).unwrap();
    wait_for("the download", 60, || matches!(u.status(), Status::Ready { .. }));
    assert_eq!(u.state(), UpdateState::Ready { version: "0.33.0".into(), asks: InstallAsks::Nothing }, "the release, not the test build");
    assert!(u.install_after(false, ended()).unwrap());
    wait_for("the install", 90, || std::fs::read(&installed).is_ok_and(|b| b == new));
    wait_for("the download to go once installed", 30, || !data.join(DOWNLOADS).join(&name).exists());
    // The new build isn't offered it again for a while; the one that installed it would be.
    assert!(just_installed(&data, "v0.33.0", now(), "0.33.0 -"));
    assert!(!just_installed(&data, "v0.33.0", now(), &u.build()));
    let kept = if cfg!(windows) { app.join("MailCraftData").join("accounts.json") } else { app.join("data").join("accounts.json") };
    assert_eq!(std::fs::read(&kept).unwrap(), b"[]", "what MailCraft keeps beside it stays");
    u.closing();
    let _ = std::fs::remove_dir_all(&dir);
}

/// Base64, as minisign writes its keys and signatures.
fn base64(bytes: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |n, (i, b)| n | u32::from(*b) << (16 - 8 * i));
        for i in 0..4 {
            out.push(if i <= chunk.len() { char::from(ABC[(n >> (18 - 6 * i) & 63) as usize]) } else { '=' });
        }
    }
    out
}

/// `data` signed as minisign signs it (Ed25519 over its BLAKE2b-512 hash, `comment` signed with
/// the signature) by the key made from `seed`: that key's public line (`RW…`), and the
/// `.minisig` file.
fn minisign(seed: u8, data: &[u8], comment: &str) -> (String, String) {
    use blake2::Digest;
    use ring::signature::KeyPair;
    let pair = ring::signature::Ed25519KeyPair::from_seed_unchecked(&[seed; 32]).unwrap();
    let id = [seed; 8];
    let public = [b"Ed".as_slice(), &id, pair.public_key().as_ref()].concat();
    let signature = pair.sign(&blake2::Blake2b512::digest(data));
    let line = [b"ED".as_slice(), &id, signature.as_ref()].concat();
    let global = pair.sign(&[signature.as_ref(), comment.as_bytes()].concat());
    let sig = format!("untrusted comment: a test's\n{}\ntrusted comment: {comment}\n{}\n", base64(&line), base64(global.as_ref()));
    (base64(&public), sig)
}

/// Signed updates (#233): built with a public key, a build takes its update once the release's
/// checksums are signed with that key for that release; signed with another key, for another
/// release, or not at all, the update isn't downloaded or offered.
#[test]
fn an_update_is_taken_only_with_its_checksums_signed() {
    let file = b"the new MailCraft".to_vec();
    for (case, signer, signed_tag, signature) in [
        ("signed", 7u8, "v0.33.0", true),
        ("another key", 9, "v0.33.0", true),
        ("another release", 7, "v0.32.0", true),
        ("unsigned", 7, "v0.33.0", false),
    ] {
        let dir = scratch(&format!("signed-{}", case.replace(' ', "-")));
        let kind = if cfg!(windows) { Kind::Portable(dir.join("MailCraft").join("mailcraft.exe")) } else { Kind::Tarball(dir.join("MailCraft")) };
        let name = format!("mailcraft-0.33.0{}", kind.suffix());
        let sums = format!("{}  {name}\n", mailcraft_net::updates::sha256_hex(&file));
        let (key, _) = minisign(7, b"", "");
        let (_, sig) = minisign(signer, sums.as_bytes(), &format!("MailCraft {signed_tag}"));
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://127.0.0.1:{}/", listener.local_addr().unwrap().port());
        let asset = |n: &str, body: &[u8]| format!(r#"{{"name":"{n}","size":{},"browser_download_url":"{base}download/v0.33.0/{n}"}}"#, body.len());
        let mut assets = vec![asset(&name, &file), asset("SHA256SUMS.txt", sums.as_bytes())];
        let mut files =
            vec![(format!("/download/v0.33.0/{name}"), file.clone()), ("/download/v0.33.0/SHA256SUMS.txt".to_string(), sums.clone().into_bytes())];
        if signature {
            assets.push(asset("SHA256SUMS.txt.minisig", sig.as_bytes()));
            files.push(("/download/v0.33.0/SHA256SUMS.txt.minisig".to_string(), sig.into_bytes()));
        }
        let list = format!(
            r#"[{{"tag_name":"v0.33.0","draft":false,"prerelease":false,"target_commitish":"0123456789abcdef0123456789abcdef01234567",
                 "html_url":"{base}tag/v0.33.0","assets":[{}]}}]"#,
            assets.join(",")
        );
        files.push(("/releases".to_string(), list.into_bytes()));
        let asked = serve(listener, files);
        let data = dir.join("data-folder");
        std::fs::create_dir_all(&data).unwrap();
        let mut u = Updater::new(kind, data.clone(), mailcraft_net::updates::Feed::local(&base).unwrap(), "0.32.0".into(), true);
        u.keys = Box::leak(Box::new([&*Box::leak(key.into_boxed_str())]));
        u.follow_choice(mailcraft_engine::policy::Updates::On);
        if case == "signed" {
            let u = u.launch(None, Some(Duration::ZERO)).unwrap();
            wait_for("the download", 60, || matches!(u.status(), Status::Ready { .. }));
            assert_eq!(std::fs::read(data.join(DOWNLOADS).join(&name)).unwrap(), file);
            u.closing();
        } else {
            // One check, run here to its end (not launched: its thread would check beside it):
            // refused, nothing downloaded, nothing offered.
            let refused = u.check(None, &mut None).expect_err(case);
            assert!(refused.contains("isn't installed"), "{case}: {refused}");
            assert!(!asked.lock().unwrap().iter().any(|p| p.ends_with(&name)), "{case}: downloaded: {:?}", asked.lock().unwrap());
            assert!(!matches!(u.status(), Status::Ready { .. }), "{case}: offered");
            assert!(!data.join(DOWNLOADS).join(&name).exists(), "{case}: downloaded");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Serve `files` (path → body) over HTTP on `listener` (bound beforehand, so the addresses in
/// the files could name its port), as many requests as come, on a thread of its own: the paths
/// asked for, as they come.
fn serve(listener: TcpListener, files: Vec<(String, Vec<u8>)>) -> std::sync::Arc<Mutex<Vec<String>>> {
    let asked = std::sync::Arc::new(Mutex::new(Vec::new()));
    let seen = asked.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut first = String::new();
            let _ = reader.read_line(&mut first);
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line.trim().is_empty() {
                    break;
                }
            }
            let path = first.split_whitespace().nth(1).unwrap_or("/").to_string();
            seen.lock().unwrap().push(path.clone());
            let mut out = stream;
            match files.iter().find(|(p, _)| *p == path) {
                Some((_, body)) => {
                    let _ = write!(out, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                    let _ = out.write_all(body);
                }
                None => {
                    let _ = out.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                }
            }
            let _ = out.flush();
            let mut rest = Vec::new();
            let _ = reader.read_to_end(&mut rest);
        }
    });
    asked
}

/// The package step run by a shell, `pkexec` and the package tools stood in for: the `.deb` goes
/// to `apt-get` after the password, and the file goes once installed; without `pkexec` the
/// package opens in the software installer and stays.
#[cfg(unix)]
#[test]
fn a_package_installs_with_the_system_tool_after_the_password() {
    use std::os::unix::fs::PermissionsExt;
    let dir = scratch("package");
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let said = dir.join("said");
    let tool = |name: &str, body: &str| {
        let p = bin.join(name);
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    };
    tool("pkexec", &format!("echo \"pkexec $*\" >> '{}'; \"$@\"", said.display()));
    tool("apt-get", &format!("echo \"apt-get $*\" >> '{}'", said.display()));
    tool("xdg-open", &format!("echo \"xdg-open $*\" >> '{}'", said.display()));
    let deb = dir.join("mailcraft-0.33.0-linux-x86_64.deb");
    let run = |path: &str| {
        let _ = std::fs::remove_file(&said);
        std::fs::write(&deb, b"a package").unwrap();
        let step = package_step(super::Package::Deb, &deb, None);
        assert!(std::process::Command::new("/bin/sh").arg("-c").arg(&step).env("PATH", path).status().unwrap().code().is_some());
        std::fs::read_to_string(&said).unwrap_or_default()
    };
    let with = format!("{}:/usr/bin:/bin", bin.display());
    let out = run(&with);
    assert!(
        out.contains("pkexec apt-get -o DPkg::Lock::Timeout=60 install -y --allow-downgrades") && out.contains(&deb.display().to_string()),
        "{out}"
    );
    assert!(!deb.exists(), "installed: the download goes");
    std::fs::remove_file(bin.join("pkexec")).unwrap();
    // Only the stand-ins: a real `pkexec` in /usr/bin must not be found.
    let out = run(&bin.display().to_string());
    assert!(out.starts_with("xdg-open ") && deb.exists(), "{out}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A damaged tarball changes nothing: the copy stays as it was, and nothing is left beside it.
#[cfg(unix)]
#[test]
fn a_damaged_tarball_changes_nothing() {
    let dir = scratch("tar-damaged");
    let root = dir.join("MailCraft");
    std::fs::create_dir_all(root.join("bin")).unwrap();
    std::fs::write(root.join("bin").join("mailcraft"), b"old").unwrap();
    let bad = dir.join("bad.tar.gz");
    std::fs::write(&bad, b"not a tarball").unwrap();
    std::process::Command::new("sh").arg("-c").arg(tarball_step(&bad, &root)).status().unwrap();
    assert_eq!(std::fs::read(root.join("bin").join("mailcraft")).unwrap(), b"old");
    assert!(!root.join(".mailcraft-update").exists());
    let _ = std::fs::remove_dir_all(&dir);
}
