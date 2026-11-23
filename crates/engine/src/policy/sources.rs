//! Where an organisation's profile is on this computer (issue #245), read as MailCraft starts.
//! The earlier in this list, the more a later one overrides it:
//!
//! 1. `policies.json` beside a portable copy: defaults only, since its user can change it.
//! 2. A `policies.json` only an administrator can write: `%ProgramData%\MailCraft\` on Windows,
//!    `/Library/Application Support/MailCraft/` on macOS, `/etc/mailcraft/` on Linux and the BSDs
//!    (`/usr/local/etc/mailcraft/` too on FreeBSD; a Flatpak sees the computer's `/etc` once its
//!    administrator allows it). A file anyone else could have written or replaced is refused,
//!    and said: another user's file could send a domain's sign-ins to a server of theirs.
//! 3. Windows: `Software\Policies\MailCraft`, the user's then the computer's, as Group Policy and
//!    Intune write them (`packaging/windows/policy/`). macOS: the configuration profile for
//!    `io.github.zidan44.MailCraft`, the user's then the computer's.
//!
//! Nothing found, nothing changes. Nothing here runs a program unless a profile file is there
//! (on Windows, PowerShell reads the file's owner and access list: the safe way to ask needs no
//! `unsafe`).

use std::io::Read;
use std::path::Path;

use serde_json::Value;

use super::{Organisation, PROFILE_MAX};

mod named;
#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

pub use named::{NAMED, from_named};

/// The profile's file name, in each place it's looked for.
pub const FILE_NAME: &str = "policies.json";

/// One source found: where, its JSON, and whether its locks count.
pub(crate) type Found = (String, Value, bool);

/// The organisation's profile on this computer; `beside_program` is the folder a portable copy
/// runs from (its `policies.json` gives defaults only).
pub fn read(beside_program: Option<&Path>) -> Organisation {
    let mut found: Vec<Found> = Vec::new();
    let mut ignored = Vec::new();
    if let Some(dir) = beside_program {
        let path = dir.join(FILE_NAME);
        if path.is_file() {
            match read_capped(&path).and_then(|b| parse(&b)) {
                Ok(v) => found.push((path.display().to_string(), v, false)),
                Err(e) => ignored.push(format!("{}: {e}", path.display())),
            }
        }
    }
    #[cfg(unix)]
    unix::system(&mut found, &mut ignored);
    #[cfg(windows)]
    windows::system(&mut found, &mut ignored);
    let mut o = Organisation::from_sources(found);
    ignored.append(&mut o.ignored);
    ignored.truncate(100);
    o.ignored = ignored;
    o
}

/// A profile's JSON from its bytes (a UTF-8 byte-order mark is allowed, as Notepad writes one).
pub(crate) fn parse(bytes: &[u8]) -> Result<Value, String> {
    if bytes.len() > PROFILE_MAX {
        return Err(format!("larger than {PROFILE_MAX} bytes"));
    }
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let v: Value = serde_json::from_slice(bytes).map_err(|e| format!("not JSON ({e})"))?;
    if v.is_object() { Ok(v) } else { Err("not a JSON object".into()) }
}

/// A file's bytes, [`PROFILE_MAX`] at most (one byte more says it's too large).
pub(crate) fn read_capped(path: &Path) -> Result<Vec<u8>, String> {
    let f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    capped(f)
}

pub(crate) fn capped(f: impl Read) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    f.take(PROFILE_MAX as u64 + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    Ok(bytes)
}

/// A named source's JSON, `{}` when it set nothing (the registry's, a configuration profile's).
#[cfg(any(windows, target_os = "macos", test))]
pub(crate) fn empty() -> Value {
    Value::Object(serde_json::Map::new())
}

#[cfg(test)]
mod tests;
