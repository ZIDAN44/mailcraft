//! Windows: Group Policy's key `Software\Policies\MailCraft` (the user's, then the computer's
//! over it), and `%ProgramData%\MailCraft\policies.json` when only an administrator can have
//! written it. Every user may make folders and files in `%ProgramData%` and owns what they make,
//! so a file there is taken only when Administrators, SYSTEM or TrustedInstaller own it and its
//! folder, and no one else may change, replace or delete it. Its owner and access list are read
//! through the same handle as its bytes, by PowerShell (the security API's own calls would need
//! `unsafe`): a way of asking learnt from PdfCraft's owner-only files.

use std::path::PathBuf;

use super::named::{Named, from_named};
use super::*;

/// The key Group Policy and Intune write MailCraft's policies under.
pub const KEY: &str = r"Software\Policies\MailCraft";

/// Windows' own folder, as the computer's registry has it: not `%SystemRoot%`, which whoever
/// starts MailCraft may set.
fn system_root() -> PathBuf {
    windows_registry::LOCAL_MACHINE
        .open(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion")
        .and_then(|k| k.get_string("SystemRoot"))
        .ok()
        .filter(|r| r.len() > 3 && r.get(1..3) == Some(":\\"))
        .map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from)
}

/// `ProgramData`, as the computer's registry has it (`%SystemDrive%\ProgramData`, the drive
/// Windows is on): not `%ProgramData%`, which whoever starts MailCraft may set, so as to read
/// no administrator's profile.
fn program_data() -> PathBuf {
    let root = system_root().display().to_string();
    let drive = root.get(..2).unwrap_or("C:").to_string();
    let named = windows_registry::LOCAL_MACHINE
        .open(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\ProfileList")
        .and_then(|k| k.get_string("ProgramData"))
        .ok()
        .map(|p| p.replace("%SystemDrive%", &drive).replace("%SYSTEMDRIVE%", &drive));
    named.filter(|p| !p.contains('%') && p.get(1..3) == Some(":\\")).map_or_else(|| PathBuf::from(format!(r"{drive}\ProgramData")), PathBuf::from)
}

pub(super) fn system(found: &mut Vec<Found>, ignored: &mut Vec<String>) {
    let path = program_data().join("MailCraft").join(FILE_NAME);
    if path.is_file() {
        match admin_file(&path).and_then(|b| parse(&b)) {
            Ok(v) => found.push((path.display().to_string(), v, true)),
            Err(e) => ignored.push(format!("{}: {e}", path.display())),
        }
    }
    for (hive, name) in [(windows_registry::CURRENT_USER, "HKCU"), (windows_registry::LOCAL_MACHINE, "HKLM")] {
        if let Ok(key) = hive.open(KEY) {
            from_key(&key, &format!(r"{name}\{KEY}"), found, ignored);
        }
    }
}

/// What the values under `key` (at `at`) set.
fn from_key(key: &windows_registry::Key, at: &str, found: &mut Vec<Found>, ignored: &mut Vec<String>) {
    let values = named_values(key);
    if !values.is_empty() {
        let (file, profile, set, mut bad) = from_named(&values, at);
        ignored.append(&mut bad);
        // A file an administrator named: the key itself is theirs.
        if let Some(f) = file {
            match read_capped(Path::new(&f)).and_then(|b| parse(&b)) {
                Ok(v) => found.push((f, v, true)),
                Err(e) => ignored.push(format!("{at}\\ProfileFile ({f}): {e}")),
            }
        }
        if let Some(p) = profile {
            match parse(p.as_bytes()) {
                Ok(v) => found.push((format!(r"{at}\Profile"), v, true)),
                Err(e) => ignored.push(format!(r"{at}\Profile: {e}")),
            }
        }
        if set != empty() {
            found.push((at.to_string(), set, true));
        }
    }
}

/// The values under `key` that MailCraft reads, as they're kept (a list of lines is one text).
fn named_values(key: &windows_registry::Key) -> Vec<(String, Named)> {
    use windows_registry::Type;
    let mut out = Vec::new();
    for (name, _) in super::NAMED {
        let v = match key.get_type(name) {
            Ok(Type::String | Type::ExpandString) => key.get_string(name).ok().map(Named::Text),
            Ok(Type::MultiString) => key.get_multi_string(name).ok().map(|l| Named::Text(l.join("\n"))),
            Ok(Type::U32) => key.get_u32(name).ok().map(|n| Named::Number(i64::from(n))),
            Ok(Type::U64) => key.get_u64(name).ok().and_then(|n| i64::try_from(n).ok()).map(Named::Number),
            _ => None,
        };
        if let Some(v) = v {
            out.push((name.to_string(), v));
        }
    }
    out
}

/// Accounts that may own and change the profile file: Administrators, SYSTEM, TrustedInstaller.
const ADMINS: [&str; 3] = ["S-1-5-32-544", "S-1-5-18", "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464"];
/// Rights that change a file or replace it: write, append, delete, change its access list or
/// owner, and the generic write and all.
const FILE_CHANGES: u64 = 0x2 | 0x4 | 0x1_0000 | 0x4_0000 | 0x8_0000 | 0x1000_0000 | 0x4000_0000;
/// Rights on its folder that let a file in it be replaced: delete a child, delete or rename the
/// folder, change its access list or owner, and all. (Adding files is everyone's in
/// `%ProgramData%`, and adds nothing read.)
const FOLDER_CHANGES: u64 = 0x40 | 0x1_0000 | 0x4_0000 | 0x8_0000 | 0x1000_0000;

/// `path`'s bytes when only an administrator can have written it; else why not.
fn admin_file(path: &Path) -> Result<Vec<u8>, String> {
    const SCRIPT: &str = concat!(
        "$ErrorActionPreference = 'Stop'; ",
        "function Describe($b) { $raw = New-Object Security.AccessControl.RawSecurityDescriptor -ArgumentList @($b, 0); ",
        "$s = 'O:' + $raw.Owner.Value; if ($raw.DiscretionaryAcl -ne $null) { $s += 'D:'; foreach ($a in $raw.DiscretionaryAcl) { ",
        "$t = 'A'; if ([string]$a.AceType -like 'AccessDenied*') { $t = 'D' }; ",
        "$f = ''; if ($a.AceFlags.HasFlag([Security.AccessControl.AceFlags]::InheritOnly)) { $f = 'IO' }; ",
        "$sid = ''; if ($a.SecurityIdentifier) { $sid = $a.SecurityIdentifier.Value }; ",
        "$s += '(' + $t + ';' + $f + ';' + $a.AccessMask + ';;;' + $sid + ')' } }; return $s }; ",
        "$p = $env:MAILCRAFT_POLICY_PATH; $fs = [IO.File]::Open($p, 'Open', 'Read', 'Read'); try { ",
        "$file = Describe $fs.GetAccessControl().GetSecurityDescriptorBinaryForm(); ",
        "$dir = Describe ([IO.Directory]::GetAccessControl([IO.Path]::GetDirectoryName($p)).GetSecurityDescriptorBinaryForm()); ",
        "$max = [int]$env:MAILCRAFT_POLICY_MAX + 1; $buf = New-Object byte[] $max; $total = 0; ",
        "while ($total -lt $max) { $n = $fs.Read($buf, $total, $max - $total); if ($n -le 0) { break }; $total += $n }; ",
        "[Console]::Out.Write($file + [char]10 + $dir + [char]10 + [Convert]::ToBase64String($buf, 0, $total)) } finally { $fs.Dispose() }"
    );
    use std::os::windows::process::CommandExt as _;
    /// `CREATE_NO_WINDOW`: no console window flashing up as MailCraft starts.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    // By its full path, in Windows' own folder: a `powershell.exe` found first elsewhere would
    // answer instead.
    let out = std::process::Command::new(system_root().join(r"System32\WindowsPowerShell\v1.0\powershell.exe"))
        .args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT])
        .env("MAILCRAFT_POLICY_PATH", path)
        .env("MAILCRAFT_POLICY_MAX", PROFILE_MAX.to_string())
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("its owner couldn't be read ({e})"))?;
    if !out.status.success() {
        return Err("its owner couldn't be read".into());
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut lines = text.split('\n');
    let (Some(file), Some(dir), Some(data)) = (lines.next(), lines.next(), lines.next()) else {
        return Err("its owner couldn't be read".into());
    };
    if let Some(why) = foreign(dir, FOLDER_CHANGES) {
        return Err(format!("not only an administrator's: its folder {why}"));
    }
    if let Some(why) = foreign(file, FILE_CHANGES) {
        return Err(format!("not only an administrator's: it {why}"));
    }
    base64(data.trim()).ok_or_else(|| "its contents couldn't be read".into())
}

/// Why the object whose descriptor is `sddl` (`O:<owner>D:(<A|D>;<IO?>;<mask>;;;<sid>)…`, every
/// account a full SID) isn't only an administrator's: an owner who isn't one, no access list, or
/// another account allowed any of `changes`. `None`: it's theirs alone.
fn foreign(sddl: &str, changes: u64) -> Option<String> {
    let sddl = sddl.trim();
    let Some(rest) = sddl.strip_prefix("O:") else {
        return Some("has an owner that can't be read".into());
    };
    let (owner, acl) = rest.split_once("D:").unwrap_or((rest, ""));
    if !ADMINS.contains(&owner.trim()) {
        return Some(format!("is owned by another account ({})", owner.trim()));
    }
    if !sddl.contains("D:") {
        return Some("has no access list, so anyone may change it".into());
    }
    for ace in acl.split('(').skip(1) {
        let parts: Vec<&str> = ace.trim_end_matches(')').split(';').collect();
        let (Some(kind), Some(flags), Some(mask), Some(sid)) = (parts.first(), parts.get(1), parts.get(2), parts.get(5)) else {
            return Some("has an access list entry that can't be read".into());
        };
        // Deny entries take rights away; inherit-only ones are for what's made inside.
        if *kind != "A" || flags.contains("IO") {
            continue;
        }
        let Ok(mask) = mask.parse::<u64>() else {
            return Some("has an access list entry that can't be read".into());
        };
        if mask & changes != 0 && !ADMINS.contains(sid) {
            return Some(format!("may be changed by another account ({sid})"));
        }
    }
    None
}

/// Standard base64, as `[Convert]::ToBase64String` writes it.
fn base64(s: &str) -> Option<Vec<u8>> {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let mut acc = 0u32;
    let mut bits = 0u32;
    for b in s.bytes().filter(|b| *b != b'=') {
        let v = u32::try_from(T.iter().position(|c| *c == b)?).ok()?;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(u8::try_from((acc >> bits) & 0xFF).ok()?);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const USER: &str = "S-1-5-21-1-2-3-1001";

    /// Where the profile and PowerShell are looked for comes from the computer's registry, and
    /// is where this computer's own environment says (a test can't change the environment).
    #[test]
    fn its_folders_are_the_computers() {
        let same = |a: &std::path::Path, env: &str| {
            let want = std::env::var_os(env).map(PathBuf::from).unwrap_or_default();
            assert!(a.display().to_string().eq_ignore_ascii_case(&want.display().to_string()), "{a:?} for %{env}% {want:?}");
        };
        same(&system_root(), "SystemRoot");
        same(&program_data(), "ProgramData");
        assert!(system_root().join(r"System32\WindowsPowerShell\v1.0\powershell.exe").is_file());
    }

    #[test]
    fn only_a_file_administrators_alone_may_change_is_taken() {
        // As an administrator makes it under %ProgramData%: everyone reads it, no one else writes.
        let file = "O:S-1-5-32-544D:(A;;2032127;;;S-1-5-18)(A;;2032127;;;S-1-5-32-544)(A;;1179817;;;S-1-5-32-545)";
        let folder = "O:S-1-5-32-544D:(A;;2032127;;;S-1-5-18)(A;;2032127;;;S-1-5-32-544)(A;;1179817;;;S-1-5-32-545)(A;;6;;;S-1-5-32-545)(A;IO;268435456;;;S-1-3-0)";
        assert_eq!(foreign(file, FILE_CHANGES), None);
        assert_eq!(foreign(folder, FOLDER_CHANGES), None, "users may add files beside it");
        // Made by a user, or one a user may write, delete, or whose folder they may empty.
        assert!(foreign(&format!("O:{USER}D:(A;;2032127;;;{USER})"), FILE_CHANGES).unwrap().contains("owned by another"));
        assert!(foreign(&format!("O:S-1-5-32-544D:(A;;1180063;;;{USER})"), FILE_CHANGES).unwrap().contains(USER));
        assert!(foreign("O:S-1-5-32-544D:(A;;65536;;;S-1-5-11)", FILE_CHANGES).is_some(), "delete");
        assert!(foreign("O:S-1-5-32-544D:(A;;64;;;S-1-5-32-545)", FOLDER_CHANGES).is_some(), "delete a child");
        assert!(foreign("O:S-1-5-32-544", FILE_CHANGES).unwrap().contains("no access list"));
        // A deny entry, or one only for what's made inside, gives nothing.
        assert_eq!(foreign(&format!("O:S-1-5-18D:(D;;2032127;;;{USER})(A;IO;2032127;;;{USER})"), FILE_CHANGES), None);
        assert!(foreign("O:S-1-5-18D:(A;;junk;;;S-1-5-11)", FILE_CHANGES).is_some());
        // What PowerShell didn't write as it should is never taken as an administrator's.
        for odd in ["nonsense", "", "D:(A;;1;;;S-1-5-18)", "O:"] {
            assert!(foreign(odd, FILE_CHANGES).is_some(), "{odd:?}");
        }
    }

    /// On Windows itself, through PowerShell: a file a user made (as anyone may under
    /// `%ProgramData%`) is refused, its owner said.
    #[test]
    fn a_users_own_profile_file_is_refused() {
        let dir = std::env::temp_dir().join(format!("mailcraft-policy-owner-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(FILE_NAME);
        std::fs::write(&path, b"{\"organisation\": \"Theirs\"}").unwrap();
        let why = admin_file(&path).unwrap_err();
        assert!(why.contains("another account") || why.contains("may be changed"), "{why}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Named values as Group Policy writes them, read from a real key (a test's own, under the
    /// user's hive, removed after).
    #[test]
    fn a_policy_key_is_read() {
        let path = format!(r"Software\MailCraftTests\policy-{}", std::process::id());
        let key = windows_registry::CURRENT_USER.create(&path).unwrap();
        key.set_u32("RequireEncryption", 1).unwrap();
        key.set_string("Updates", "stable").unwrap();
        key.set_multi_string("Profile", &["{\"organisation\":", "\"Contoso\"}"]).unwrap();
        key.set_string("Unknown", "x").unwrap();
        let (mut found, mut ignored) = (Vec::new(), Vec::new());
        from_key(&key, "HKCU", &mut found, &mut ignored);
        let _ = windows_registry::CURRENT_USER.remove_tree(&path);
        let o = Organisation::from_sources(found);
        assert_eq!(o.top.organisation.as_deref(), Some("Contoso"));
        assert_eq!(o.top.require_encryption, Some(true));
        assert_eq!(o.top.updates, Some((super::super::super::Updates::Stable, true)));
        assert!(ignored.is_empty(), "values MailCraft doesn't read are left alone: {ignored:?}");
    }

    #[test]
    fn base64_as_powershell_writes_it() {
        assert_eq!(base64("e30="), Some(b"{}".to_vec()));
        assert_eq!(base64(""), Some(Vec::new()));
        assert_eq!(base64("e3!"), None);
    }
}
