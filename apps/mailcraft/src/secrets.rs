//! Passwords and sign-ins in the system keychain, whatever their length.
//!
//! Windows' Credential Manager keeps at most 2560 bytes in an entry (CRED_MAX_CREDENTIAL_BLOB_SIZE),
//! and the keyring crate stores a password there as UTF-16, two bytes a character, so nothing
//! over 1280 characters could be kept. A Microsoft 365 sign-in (a refresh token of about 2,000
//! characters, more with more claims) failed with "password encoded as UTF-16 is longer than
//! platform limit of 2560 chars" (issue #5). On Windows a secret is kept as UTF-8 bytes, split
//! over as many entries as it needs: the first starts `MCK1:<parts>:`, the others are named after
//! it with `|part<n>`. Passwords kept by earlier versions (UTF-16, no header) still read. macOS's
//! Keychain and the Secret Service take long secrets as they are, and keep them as before.

/// What starts the first part of a secret kept in parts.
#[cfg(any(windows, test))]
const MARK: &str = "MCK1:";

/// The bytes of a secret each part holds, header included in the first (under Windows' 2560).
#[cfg(any(windows, test))]
const PART: usize = 2400;

/// The most parts a secret is kept in (77 KB).
#[cfg(any(windows, test))]
pub const MAX_PARTS: usize = 32;

/// A secret as the parts it's kept in: the first with the header.
#[cfg(any(windows, test))]
pub fn split(secret: &str) -> Result<Vec<Vec<u8>>, String> {
    let bytes = secret.as_bytes();
    let n = bytes.len().div_ceil(PART).max(1);
    if n > MAX_PARTS {
        return Err(format!("it's longer than the keychain can keep ({} bytes)", bytes.len()));
    }
    let mut parts: Vec<Vec<u8>> = bytes.chunks(PART).map(<[u8]>::to_vec).collect();
    if parts.is_empty() {
        parts.push(Vec::new());
    }
    if let Some(first) = parts.first_mut() {
        let mut head = format!("{MARK}{n}:").into_bytes();
        head.append(first);
        *first = head;
    }
    Ok(parts)
}

/// How many parts a first part says there are, and its own share of the secret; `None` for one
/// kept before parts (no header).
#[cfg(any(windows, test))]
pub fn header(first: &[u8]) -> Option<(usize, &[u8])> {
    let rest = first.strip_prefix(MARK.as_bytes())?;
    let colon = rest.iter().position(|b| *b == b':')?;
    let n: usize = std::str::from_utf8(rest.get(..colon)?).ok()?.parse().ok()?;
    (1..=MAX_PARTS).contains(&n).then_some((n, rest.get(colon + 1..)?))
}

/// A secret from its parts (the first with its header), when they're all there and whole.
#[cfg(any(windows, test))]
pub fn join(first: &[u8], rest: &[Vec<u8>]) -> Option<String> {
    let (n, mine) = header(first)?;
    if rest.len() + 1 != n {
        return None;
    }
    let mut all = mine.to_vec();
    for p in rest {
        all.extend_from_slice(p);
    }
    String::from_utf8(all).ok()
}

/// A password kept by an earlier version: UTF-16, as the keyring crate wrote it on Windows.
#[cfg(any(windows, test))]
pub fn legacy(bytes: &[u8]) -> Option<String> {
    let (pairs, odd) = bytes.as_chunks::<2>();
    if !odd.is_empty() {
        return None;
    }
    let units: Vec<u16> = pairs.iter().map(|p| u16::from_le_bytes(*p)).collect();
    String::from_utf16(&units).ok()
}

/// The entry of part `n` (2 and on) of the secret kept under `user`.
#[cfg(windows)]
fn part_name(user: &str, n: usize) -> String {
    format!("{user}|part{n}")
}

/// Keep `secret` under `user`.
pub fn store(service: &str, user: &str, secret: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        let parts = split(secret)?;
        let had = parts_kept(service, user);
        // The other parts first, the first last: a set half-written never reads as whole.
        for (i, p) in parts.iter().enumerate().skip(1) {
            keyring::Entry::new(service, &part_name(user, i + 1)).and_then(|e| e.set_secret(p)).map_err(|e| e.to_string())?;
        }
        let first = parts.first().ok_or("nothing to keep")?;
        keyring::Entry::new(service, user).and_then(|e| e.set_secret(first)).map_err(|e| e.to_string())?;
        // A longer secret before this one left parts this one hasn't.
        for n in parts.len() + 1..=had {
            if let Ok(e) = keyring::Entry::new(service, &part_name(user, n)) {
                let _ = e.delete_credential();
            }
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        keyring::Entry::new(service, user).and_then(|e| e.set_password(secret)).map_err(|e| e.to_string())
    }
}

/// The secret kept under `user`.
pub fn load(service: &str, user: &str) -> Option<String> {
    #[cfg(windows)]
    {
        let first = keyring::Entry::new(service, user).ok()?.get_secret().ok()?;
        let Some((n, _)) = header(&first) else { return legacy(&first) };
        let rest: Option<Vec<Vec<u8>>> = (2..=n).map(|i| keyring::Entry::new(service, &part_name(user, i)).ok()?.get_secret().ok()).collect();
        join(&first, &rest?)
    }
    #[cfg(not(windows))]
    {
        keyring::Entry::new(service, user).ok()?.get_password().ok()
    }
}

/// Forget the secret kept under `user`, every part of it.
pub fn remove(service: &str, user: &str) {
    #[cfg(windows)]
    for n in 2..=parts_kept(service, user) {
        if let Ok(e) = keyring::Entry::new(service, &part_name(user, n)) {
            let _ = e.delete_credential();
        }
    }
    if let Ok(e) = keyring::Entry::new(service, user) {
        let _ = e.delete_credential();
    }
}

/// How many parts the secret under `user` is kept in now (1 for none or one without parts).
#[cfg(windows)]
fn parts_kept(service: &str, user: &str) -> usize {
    keyring::Entry::new(service, user).ok().and_then(|e| e.get_secret().ok()).and_then(|b| header(&b).map(|(n, _)| n)).unwrap_or(1)
}

use mailcraft_engine::model::AccountConfig;
use mailcraft_net::worker::Credentials;

/// Passwords in the system keychain (Windows Credential Manager, macOS Keychain, Secret Service).
pub struct Keychain;

pub const SERVICE: &str = "MailCraft";

/// The keychain entry an account's password or sign-in is kept under.
fn entry(a: &AccountConfig) -> String {
    format!("{}|{}|{}", a.id, a.username, a.imap.host)
}

/// The outgoing server's own password, for an account whose SMTP sign-in is its own.
fn smtp_entry(a: &AccountConfig) -> String {
    format!("{}|smtp", a.id)
}

// Kept whatever their length (`secrets.rs`): a Microsoft 365 sign-in is longer than one Windows
// Credential Manager entry holds (issue #5).
impl Credentials for Keychain {
    fn get(&self, account: &AccountConfig) -> Option<String> {
        load(SERVICE, &entry(account))
    }
    fn set(&self, account: &AccountConfig, password: &str) -> Result<(), String> {
        store(SERVICE, &entry(account), password)
    }
    fn delete(&self, account: &AccountConfig) {
        remove(SERVICE, &entry(account));
    }
    fn get_smtp(&self, account: &AccountConfig) -> Option<String> {
        load(SERVICE, &smtp_entry(account))
    }
    fn set_smtp(&self, account: &AccountConfig, password: &str) -> Result<(), String> {
        store(SERVICE, &smtp_entry(account), password)
    }
    fn delete_smtp(&self, account: &AccountConfig) {
        remove(SERVICE, &smtp_entry(account));
    }
    fn get_secret(&self, account: &AccountConfig, what: &str) -> Option<String> {
        load(SERVICE, &format!("{}|{what}", account.id))
    }
    fn set_secret(&self, account: &AccountConfig, what: &str, secret: &str) -> Result<(), String> {
        store(SERVICE, &format!("{}|{what}", account.id), secret)
    }
    fn delete_secret(&self, account: &AccountConfig, what: &str) {
        remove(SERVICE, &format!("{}|{what}", account.id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Any length splits into parts under Windows' limit and joins back as it was; too long a one
    /// is refused rather than cut.
    #[test]
    fn secrets_split_and_join_whole() {
        for len in [0, 1, 1280, 1281, 2000, PART - 10, PART, PART + 1, 5 * PART + 7, MAX_PARTS * PART] {
            let s: String = (0..len).map(|i| char::from(b'a' + (i % 26) as u8)).collect();
            let parts = split(&s).unwrap();
            assert!(parts.iter().all(|p| p.len() <= 2560), "{len}: a part over Windows' 2560 bytes");
            let (first, rest) = parts.split_first().unwrap();
            assert_eq!(join(first, rest).as_deref(), Some(s.as_str()), "{len}");
        }
        assert!(split(&"x".repeat(MAX_PARTS * PART + 1)).is_err());
        // Other characters split mid-character still join (bytes, not characters).
        let s = "pässwörd—密码🔑".repeat(400);
        let parts = split(&s).unwrap();
        let (first, rest) = parts.split_first().unwrap();
        assert_eq!(join(first, rest).as_deref(), Some(s.as_str()));
        // A part missing, or one too many: not whole, so not used.
        let parts = split(&"y".repeat(3 * PART)).unwrap();
        assert_eq!(join(&parts[0], &parts[1..2]), None);
        assert_eq!(join(&parts[0], &[parts[1].clone(), parts[2].clone(), vec![b'z']]), None);
    }

    /// One kept by an earlier version (UTF-16, no header) still reads; a header can't be taken for
    /// one (an ASCII UTF-16 password has a zero in every other byte).
    #[test]
    fn earlier_passwords_still_read() {
        let old: Vec<u8> = "hunter2 é".encode_utf16().flat_map(u16::to_le_bytes).collect();
        assert_eq!(header(&old), None);
        assert_eq!(legacy(&old).as_deref(), Some("hunter2 é"));
        assert_eq!(legacy(&[0x41]), None, "odd length");
        for bad in [&b"MCK1:"[..], b"MCK1:x:abc", b"MCK1:0:abc", b"MCK1:99:abc", b"MCK1:3"] {
            assert_eq!(header(bad), None, "{bad:?}");
        }
    }
}
