//! macOS, Linux and the BSDs: a `policies.json` that only root can have written (owned by root,
//! and neither it nor its folder writable by anyone else), and on macOS the configuration profile
//! an MDM installs for `io.github.zidan44.MailCraft` (`/Library/Managed Preferences/`, the user's
//! then the computer's). Inside a Flatpak, the computer's `/etc` is at `/run/host/etc` once its
//! administrator allows it (`flatpak override --filesystem=host-etc:ro`), where root's files show
//! as owned by the sandbox's overflow user.

use std::os::unix::fs::MetadataExt;

#[cfg(any(target_os = "macos", test))]
use super::named::{Named, from_named};
use super::*;

/// The overflow user a Flatpak sandbox shows the computer's root files as owned by.
const OVERFLOW_UID: u32 = 65534;

pub(super) fn system(found: &mut Vec<Found>, ignored: &mut Vec<String>) {
    let flatpak = std::env::var_os("FLATPAK_ID").is_some() || Path::new("/.flatpak-info").exists();
    let mut places: Vec<&str> = Vec::new();
    if cfg!(target_os = "macos") {
        places.push("/Library/Application Support/MailCraft");
    } else {
        places.push("/etc/mailcraft");
        if cfg!(target_os = "freebsd") {
            places.push("/usr/local/etc/mailcraft");
        }
        if flatpak {
            places.push("/run/host/etc/mailcraft");
        }
    }
    for dir in places {
        let path = Path::new(dir).join(FILE_NAME);
        if !path.exists() {
            continue;
        }
        match root_file(&path, flatpak).and_then(|b| parse(&b)) {
            Ok(v) => found.push((path.display().to_string(), v, true)),
            Err(e) => ignored.push(format!("{}: {e}", path.display())),
        }
    }
    #[cfg(target_os = "macos")]
    managed(found, ignored);
}

/// Whether `uid` is root's (or, in a Flatpak, the computer's root seen from inside).
fn root(uid: u32, flatpak: bool) -> bool {
    uid == 0 || flatpak && uid == OVERFLOW_UID
}

/// `path`'s bytes when only root can have written it: it and its folder owned by root, and
/// writable by no group or other user. The file's facts are read from the opened file itself.
fn root_file(path: &Path, flatpak: bool) -> Result<Vec<u8>, String> {
    let f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let m = f.metadata().map_err(|e| e.to_string())?;
    if !m.is_file() {
        return Err("not a file".into());
    }
    if !root(m.uid(), flatpak) || m.mode() & 0o022 != 0 {
        return Err("not only root's: someone else may have written it (owned by root, mode 644, is what's taken)".into());
    }
    let dir = path.parent().ok_or("no folder")?.metadata().map_err(|e| e.to_string())?;
    if !root(dir.uid(), flatpak) || dir.mode() & 0o022 != 0 {
        return Err("not only root's: its folder may be changed by someone else".into());
    }
    capped(f)
}

/// The configuration profile for MailCraft, the user's then the computer's (later wins).
#[cfg(target_os = "macos")]
fn managed(found: &mut Vec<Found>, ignored: &mut Vec<String>) {
    const DOMAIN: &str = "io.github.zidan44.MailCraft.plist";
    let base = Path::new("/Library/Managed Preferences");
    // The login name of the user MailCraft runs as, from the system: not `$USER`, which whoever
    // starts it may set, so as to read another user's profile, or none.
    let user = std::process::Command::new("/usr/bin/id")
        .arg("-un")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|u| !u.is_empty() && !u.contains('/') && u != "..");
    let mut paths: Vec<std::path::PathBuf> = user.map(|u| base.join(u).join(DOMAIN)).into_iter().collect();
    paths.push(base.join(DOMAIN));
    for path in paths {
        if !path.exists() {
            continue;
        }
        let at = path.display().to_string();
        let bytes = match root_file(&path, false) {
            Ok(b) => b,
            Err(e) => {
                ignored.push(format!("{at}: {e}"));
                continue;
            }
        };
        let parsed = if bytes.starts_with(b"bplist") { craft_plist::parse_binary(&bytes) } else { craft_plist::parse_xml(&bytes) };
        let Ok(craft_plist::Value::Dict(entries)) = parsed else {
            ignored.push(format!("{at}: not a property list of settings"));
            continue;
        };
        from_plist(&entries, &at, found, ignored);
    }
}

/// A configuration profile's keys: the named ones (`Profile`, `RequireEncryption`…), and a
/// profile written out as keys of its own (`organisation`, `settings`…).
#[cfg(any(target_os = "macos", test))]
fn from_plist(entries: &[(String, craft_plist::Value)], at: &str, found: &mut Vec<Found>, ignored: &mut Vec<String>) {
    let mut named = Vec::new();
    let mut own = serde_json::Map::new();
    for (k, v) in entries {
        if k.starts_with(|c: char| c.is_ascii_uppercase()) {
            let n = match v {
                craft_plist::Value::String(s) => Some(Named::Text(s.clone())),
                craft_plist::Value::Integer(i) => i64::try_from(*i).ok().map(Named::Number),
                craft_plist::Value::Bool(b) => Some(Named::Bool(*b)),
                _ => None,
            };
            match n {
                Some(n) => named.push((k.clone(), n)),
                None => ignored.push(format!("{at}: {k}: not a value it takes")),
            }
        } else if let Some(j) = json_of(v, 0) {
            own.insert(k.clone(), j);
        }
    }
    let (file, profile, set, mut bad) = from_named(&named, at);
    ignored.append(&mut bad);
    if let Some(f) = file {
        match read_capped(Path::new(&f)).and_then(|b| parse(&b)) {
            Ok(v) => found.push((f, v, true)),
            Err(e) => ignored.push(format!("{at}: ProfileFile ({f}): {e}")),
        }
    }
    if let Some(p) = profile {
        match parse(p.as_bytes()) {
            Ok(v) => found.push((format!("{at}: Profile"), v, true)),
            Err(e) => ignored.push(format!("{at}: Profile: {e}")),
        }
    }
    if !own.is_empty() {
        found.push((at.to_string(), Value::Object(own), true));
    }
    if set != empty() {
        found.push((at.to_string(), set, true));
    }
}

/// A property-list value as JSON (dates and data have none), 32 levels deep at most.
#[cfg(any(target_os = "macos", test))]
fn json_of(v: &craft_plist::Value, depth: usize) -> Option<Value> {
    use craft_plist::Value as P;
    if depth > 32 {
        return None;
    }
    Some(match v {
        P::Dict(d) => Value::Object(d.iter().filter_map(|(k, v)| Some((k.clone(), json_of(v, depth + 1)?))).collect()),
        P::Array(a) => Value::Array(a.iter().filter_map(|v| json_of(v, depth + 1)).collect()),
        P::String(s) => Value::String(s.clone()),
        P::Integer(i) => Value::from(i64::try_from(*i).ok()?),
        P::Real(r) => serde_json::Number::from_f64(*r).map(Value::Number)?,
        P::Bool(b) => Value::Bool(*b),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_plist::Value as P;

    #[test]
    fn a_configuration_profile_is_read_as_a_profile() {
        let entries = vec![
            ("RequireEncryption".to_string(), P::Bool(true)),
            ("PayloadUUID".to_string(), P::String("x".into())),
            ("organisation".to_string(), P::String("Contoso".into())),
            (
                "settings".to_string(),
                P::Dict(vec![("autoPictures".into(), P::Dict(vec![("value".into(), P::Bool(false)), ("locked".into(), P::Bool(true))]))]),
            ),
            ("Updates".to_string(), P::Data(vec![1])),
        ];
        let (mut found, mut ignored) = (Vec::new(), Vec::new());
        from_plist(&entries, "managed", &mut found, &mut ignored);
        let o = Organisation::from_sources(found);
        assert_eq!(o.top.organisation.as_deref(), Some("Contoso"));
        assert_eq!(o.top.require_encryption, Some(true));
        assert!(o.top.settings["autoPictures"].locked);
        assert_eq!(ignored.len(), 1, "{ignored:?}");
    }

    /// The sample configuration profile (`packaging/macos/MailCraft.mobileconfig`) reads as the
    /// profile it says: its payload is what macOS keeps in Managed Preferences.
    #[test]
    fn the_sample_configuration_profile_is_read() {
        let xml = include_bytes!("../../../../../packaging/macos/MailCraft.mobileconfig");
        let Ok(P::Dict(top)) = craft_plist::parse_xml(xml) else { panic!("not a property list") };
        let Some((_, P::Array(content))) = top.iter().find(|(k, _)| k == "PayloadContent") else { panic!("no payload") };
        let Some(P::Dict(payload)) = content.first() else { panic!("no payload") };
        let (mut found, mut ignored) = (Vec::new(), Vec::new());
        from_plist(payload, "managed", &mut found, &mut ignored);
        assert!(ignored.is_empty(), "{ignored:?}");
        let o = Organisation::from_sources(found);
        assert!(o.ignored.is_empty(), "{:?}", o.ignored);
        assert_eq!(o.top.organisation.as_deref(), Some("Contoso"));
        assert_eq!((o.top.require_encryption, o.top.certificate_exceptions), (Some(true), Some(false)));
        assert!(o.top.settings["autoPictures"].locked && o.top.settings["agentsRun"].locked && o.top.settings["plainTopBar"].locked);
        assert_eq!(o.servers_for("a@contoso.example").and_then(|s| s.smtp.as_ref()).map(|s| s.port), Some(587));
        assert!(o.signature_for("a@contoso.example").is_some_and(|r| r.locked && r.text.contains("{title}")));
    }

    #[test]
    fn only_roots_file_is_taken() {
        let dir = std::env::temp_dir().join(format!("mailcraft-policy-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(FILE_NAME);
        std::fs::write(&path, b"{}").unwrap();
        // The tests run as a user, so their file isn't root's (as root, in a container, it is).
        if std::fs::metadata(&path).unwrap().uid() != 0 {
            assert!(root_file(&path, false).unwrap_err().contains("not only root's"));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
