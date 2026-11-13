//! A backup of what MailCraft keeps in the browser (issue #83): its accounts and their
//! settings, rules, signatures, calendars, contacts, the Outbox, and the mail downloaded when
//! asked for, in one file to save and to bring into another browser, or back after the
//! browser's data for the site was cleared. Never a password: there's none in what's kept.
//!
//! The file: a line that names it, a version, whether it's locked, then its content, deflated;
//! locked, that is encrypted with AES-256-GCM under a key made from the password (PBKDF2 with
//! HMAC-SHA-256, a salt and a count the file states). The content: the kept files (path and
//! bytes), the interface's settings as JSON, and when it was made.
//!
//! A backup read is a file from anywhere: every length is checked against what's left, the
//! content's size once inflated is capped, and a path that would leave the cache is refused.

use std::io::{Read, Write};

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};

/// What the file begins with.
const MAGIC: &[u8] = b"MAILCRAFT-BACKUP\n";
const VERSION: u8 = 1;
/// PBKDF2 rounds for a new backup (OWASP's for HMAC-SHA-256), and the most a file may ask for
/// (one asking for billions would hold the page for hours).
const ROUNDS: u32 = 600_000;
const MAX_ROUNDS: u32 = 10_000_000;
/// The most a backup's content may be once inflated, the most files in it and the longest path.
pub const MAX_CONTENT: u64 = 1 << 30;
const MAX_FILES: u32 = 2_000_000;
const MAX_PATH: u32 = 1024;
/// What's never in a backup: files set aside as damaged, and a certificate kept unlocked for
/// this browser alone (sealed with a key the browser doesn't give out).
const LEFT_OUT: [&str; 2] = [".bad", "smime-keep.bin"];

/// What a backup holds.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Backup {
    /// The kept files, by their path in the cache (`a1/folders.json`).
    pub files: Vec<(String, Vec<u8>)>,
    /// The interface's settings (the window's saved state), as it gave them.
    pub state: serde_json::Value,
    /// When it was made (Unix seconds).
    pub made: i64,
}

/// Why a file can't be read as a backup.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    #[error("this isn't a MailCraft backup")]
    NotBackup,
    #[error("this backup was made by a newer MailCraft")]
    Newer,
    #[error("this backup is locked: give its password")]
    NeedsPassword,
    #[error("the password is wrong, or the backup is damaged")]
    WrongPassword,
    #[error("the backup is damaged ({0})")]
    Damaged(&'static str),
    #[error("the backup is too big to bring in")]
    TooBig,
}

/// Whether a kept file goes into a backup: `mail` says whether downloaded messages and the
/// search index (which is made again from them) do.
pub fn keeps(path: &str, mail: bool) -> bool {
    if LEFT_OUT.iter().any(|end| path.ends_with(end)) {
        return false;
    }
    let parts: Vec<&str> = path.split('/').collect();
    let downloaded = matches!(parts.as_slice(), [_, folder, _, file] if *folder != "search" && file.ends_with(".eml"));
    let index = parts.get(1) == Some(&"search");
    mail || !(downloaded || index)
}

/// Whether a path is one inside the cache: relative, of plain segments.
fn inside(path: &str) -> bool {
    !path.is_empty() && !path.contains('\\') && !path.contains(':') && path.split('/').all(|seg| !seg.is_empty() && seg != "." && seg != "..")
}

fn key(password: &str, salt: &[u8], rounds: u32) -> [u8; 32] {
    let mut key = [0u8; 32];
    pbkdf2::pbkdf2_hmac::<sha2::Sha256>(password.as_bytes(), salt, rounds, &mut key);
    key
}

/// Whether `bytes` are a backup, and whether it's locked with a password.
pub fn locked(bytes: &[u8]) -> Option<bool> {
    let rest = bytes.strip_prefix(MAGIC)?;
    Some(rest.get(1)? & 1 == 1)
}

/// `backup` as a file; with a password, locked with it. `random` is 28 bytes no other backup
/// had (the salt and the nonce).
pub fn write(backup: &Backup, password: Option<&str>, random: [u8; 28]) -> Result<Vec<u8>, Error> {
    write_with(backup, password, random, ROUNDS)
}

/// [`write`], its key made in `rounds` rounds (the tests' are few).
fn write_with(backup: &Backup, password: Option<&str>, random: [u8; 28], rounds: u32) -> Result<Vec<u8>, Error> {
    let mut content = Vec::new();
    let count = u32::try_from(backup.files.len()).ok().filter(|n| *n <= MAX_FILES).ok_or(Error::TooBig)?;
    content.extend(count.to_le_bytes());
    for (path, data) in &backup.files {
        let len = u32::try_from(path.len()).ok().filter(|n| *n <= MAX_PATH).ok_or(Error::TooBig)?;
        content.extend(len.to_le_bytes());
        content.extend(path.as_bytes());
        content.extend((data.len() as u64).to_le_bytes());
        content.extend(data);
    }
    let state = serde_json::to_vec(&backup.state).map_err(|_| Error::Damaged("its settings"))?;
    content.extend((state.len() as u64).to_le_bytes());
    content.extend(state);
    content.extend(backup.made.to_le_bytes());
    if content.len() as u64 > MAX_CONTENT {
        return Err(Error::TooBig);
    }
    let mut deflater = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::fast());
    deflater.write_all(&content).map_err(|_| Error::Damaged("it couldn't be packed"))?;
    let packed = deflater.finish().map_err(|_| Error::Damaged("it couldn't be packed"))?;
    drop(content);
    let mut out = MAGIC.to_vec();
    let Some(password) = password.filter(|p| !p.is_empty()) else {
        out.extend([VERSION, 0]);
        out.extend(packed);
        return Ok(out);
    };
    out.extend([VERSION, 1]);
    let (salt, nonce) = random.split_at(16);
    out.extend(salt);
    out.extend(rounds.to_le_bytes());
    out.extend(nonce);
    let cipher = Aes256Gcm::new_from_slice(&key(password, salt, rounds)).map_err(|_| Error::Damaged("its key"))?;
    // What comes before it is bound to it: a locked backup can't be passed off as an open one.
    let sealed =
        cipher.encrypt(Nonce::from_slice(nonce), Payload { msg: &packed, aad: &out }).map_err(|_| Error::Damaged("it couldn't be locked"))?;
    out.extend(sealed);
    Ok(out)
}

/// The backup in `bytes`, unlocked with `password` when it's locked.
pub fn read(bytes: &[u8], password: Option<&str>) -> Result<Backup, Error> {
    let rest = bytes.strip_prefix(MAGIC).ok_or(Error::NotBackup)?;
    let (&version, rest) = rest.split_first().ok_or(Error::NotBackup)?;
    let (&flags, rest) = rest.split_first().ok_or(Error::NotBackup)?;
    if version != VERSION || flags > 1 {
        return Err(Error::Newer);
    }
    let opened;
    let packed = if flags & 1 == 1 {
        let salt = rest.get(..16).ok_or(Error::Damaged("it's cut short"))?;
        let rounds = rest.get(16..20).and_then(|b| b.try_into().ok()).map(u32::from_le_bytes).ok_or(Error::Damaged("it's cut short"))?;
        let nonce = rest.get(20..32).ok_or(Error::Damaged("it's cut short"))?;
        let sealed = rest.get(32..).ok_or(Error::Damaged("it's cut short"))?;
        if rounds == 0 || rounds > MAX_ROUNDS {
            return Err(Error::Damaged("its key"));
        }
        let password = password.filter(|p| !p.is_empty()).ok_or(Error::NeedsPassword)?;
        let head = bytes.get(..bytes.len().saturating_sub(sealed.len())).unwrap_or_default();
        let cipher = Aes256Gcm::new_from_slice(&key(password, salt, rounds)).map_err(|_| Error::Damaged("its key"))?;
        opened = cipher.decrypt(Nonce::from_slice(nonce), Payload { msg: sealed, aad: head }).map_err(|_| Error::WrongPassword)?;
        opened.as_slice()
    } else {
        rest
    };
    let mut content = Vec::new();
    flate2::read::DeflateDecoder::new(packed).take(MAX_CONTENT + 1).read_to_end(&mut content).map_err(|_| Error::Damaged("it can't be unpacked"))?;
    if content.len() as u64 > MAX_CONTENT {
        return Err(Error::TooBig);
    }
    let mut at = Reader { rest: &content };
    let count = at.u32()?;
    if count > MAX_FILES {
        return Err(Error::TooBig);
    }
    let mut files = Vec::new();
    for _ in 0..count {
        let len = at.u32()?;
        if len > MAX_PATH {
            return Err(Error::Damaged("a file's name"));
        }
        let path = std::str::from_utf8(at.take(u64::from(len))?).map_err(|_| Error::Damaged("a file's name"))?.to_string();
        let len = at.u64()?;
        let data = at.take(len)?.to_vec();
        // One that would land outside the cache, or that no backup has, is left where it is.
        if inside(&path) && keeps(&path, true) {
            files.push((path, data));
        }
    }
    let len = at.u64()?;
    let state = serde_json::from_slice(at.take(len)?).map_err(|_| Error::Damaged("its settings"))?;
    let made = at.u64().map(|t| t as i64)?;
    Ok(Backup { files, state, made })
}

/// What's left of a backup's content, read a piece at a time: never past its end.
struct Reader<'a> {
    rest: &'a [u8],
}

impl<'a> Reader<'a> {
    fn take(&mut self, len: u64) -> Result<&'a [u8], Error> {
        let len = usize::try_from(len).map_err(|_| Error::Damaged("it's cut short"))?;
        let piece = self.rest.get(..len).ok_or(Error::Damaged("it's cut short"))?;
        self.rest = self.rest.get(len..).unwrap_or_default();
        Ok(piece)
    }

    fn u32(&mut self) -> Result<u32, Error> {
        self.take(4)?.try_into().map(u32::from_le_bytes).map_err(|_| Error::Damaged("it's cut short"))
    }

    fn u64(&mut self) -> Result<u64, Error> {
        self.take(8)?.try_into().map(u64::from_le_bytes).map_err(|_| Error::Damaged("it's cut short"))
    }
}

impl crate::Session {
    /// Whether what MailCraft keeps can be saved to a backup file: where it's kept in the
    /// browser. (On a computer it's in data files there, which are files to copy already.)
    pub fn backs_up(&self) -> bool {
        self.store.as_ref().is_some_and(mailcraft_store::Store::is_memory)
    }

    /// A backup of what's kept, as a file and the name to give it: with a password, locked;
    /// `mail`, with the mail downloaded. `state` is the interface's settings; `random`, 28
    /// bytes from the system when it has them (else the session's own).
    pub fn make_backup(
        &mut self,
        password: Option<&str>,
        mail: bool,
        state: serde_json::Value,
        random: Option<[u8; 28]>,
    ) -> Result<(String, Vec<u8>), crate::CmdError> {
        let store = self.store.clone().filter(mailcraft_store::Store::is_memory);
        let store = store.ok_or_else(|| crate::CmdError::Disabled("a backup is of what MailCraft keeps in the browser".into()))?;
        let random = random.unwrap_or_else(|| {
            let mut bytes = [0u8; 28];
            for piece in bytes.chunks_mut(8) {
                let more = self.random().to_le_bytes();
                piece.copy_from_slice(more.get(..piece.len()).unwrap_or_default());
            }
            bytes
        });
        let now = self.clock.now;
        let backup = Backup { files: store.snapshot(|path| keeps(path, mail)), state, made: now };
        let file = write(&backup, password, random).map_err(|e| crate::CmdError::Failed(e.to_string()))?;
        let (y, m, d) = mailcraft_model::dates::civil_from_days(now.saturating_add(i64::from(self.clock.offset)).div_euclid(86_400));
        Ok((format!("MailCraft backup {y:04}-{m:02}-{d:02}.mailcraft-backup"), file))
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn sample() -> Backup {
        Backup {
            files: vec![
                ("accounts.json".into(), br#"[{"id":"a1"}]"#.to_vec()),
                ("a1/folders.json".into(), b"[]".to_vec()),
                ("a1/f12ab/3/12.eml".into(), b"Subject: Hi\r\n\r\nHello.\r\n".to_vec()),
                ("a1/outbox/7.eml".into(), b"Subject: Waiting\r\n\r\n".to_vec()),
                ("a1/search/v1/seg".into(), vec![0, 1, 2]),
            ],
            state: serde_json::json!({"dark": true, "language": "de"}),
            made: 1_794_500_000,
        }
    }

    #[test]
    fn a_backup_comes_back_as_it_went() {
        let b = sample();
        let open = write(&b, None, [7; 28]).unwrap();
        assert_eq!(locked(&open), Some(false));
        assert_eq!(read(&open, None).unwrap(), b);
        assert_eq!(read(&open, Some("ignored")).unwrap(), b, "a password given for one that has none");
        let sealed = write(&b, Some("correct horse"), [9; 28]).unwrap();
        assert_eq!(locked(&sealed), Some(true));
        assert_eq!(read(&sealed, Some("correct horse")).unwrap(), b);
        assert_eq!(read(&sealed, None), Err(Error::NeedsPassword));
        assert_eq!(read(&sealed, Some("")), Err(Error::NeedsPassword));
        assert_eq!(read(&sealed, Some("wrong")), Err(Error::WrongPassword));
        // Nothing of it is readable while it's locked.
        assert!(!sealed.windows(8).any(|w| w == b"accounts"));
        // An empty password locks nothing.
        assert_eq!(locked(&write(&b, Some(""), [1; 28]).unwrap()), Some(false));
    }

    #[test]
    fn what_goes_in_and_what_never_does() {
        for (path, with_mail, without) in [
            ("accounts.json", true, true),
            ("rules.json", true, true),
            ("a1/pim.json", true, true),
            ("a1/outbox/7.eml", true, true),
            ("a1/outbox/7.json", true, true),
            ("a1/f12ab/state.json", true, true),
            ("a1/f12ab/3/12.eml", true, false),
            ("a1/search/v1/seg", true, false),
            ("a1/smime-keep.bin", false, false),
            ("a1/folders.json.bad", false, false),
        ] {
            assert_eq!((keeps(path, true), keeps(path, false)), (with_mail, without), "{path}");
        }
    }

    #[test]
    fn a_file_from_anywhere_is_read_with_care() {
        assert_eq!(read(b"", None), Err(Error::NotBackup));
        assert_eq!(read(b"PK\x03\x04 a zip", None), Err(Error::NotBackup));
        assert_eq!(locked(b"MAILCRAFT-BACKUP\n"), None);
        assert_eq!(read(b"MAILCRAFT-BACKUP\n\x02\x00", None), Err(Error::Newer));
        assert_eq!(read(b"MAILCRAFT-BACKUP\n\x01\x07", None), Err(Error::Newer));
        // Cut short anywhere: an error, never a panic.
        for file in [write(&sample(), None, [3; 28]).unwrap(), write_with(&sample(), Some("pw"), [3; 28], 16).unwrap()] {
            for cut in 0..file.len() {
                assert!(read(&file[..cut], Some("pw")).is_err(), "cut at {cut}");
            }
            // Changed anywhere after its first line: refused when it's locked.
            if locked(&file) == Some(true) {
                for at in MAGIC.len()..file.len() {
                    let mut changed = file.clone();
                    changed[at] ^= 0x20;
                    assert!(read(&changed, Some("pw")).is_err(), "changed at {at}");
                }
            }
        }
        // Paths that would leave the cache, and files no backup has, are left out.
        let odd = Backup {
            files: vec![
                ("../../outside.json".into(), vec![1]),
                ("/etc/passwd".into(), vec![1]),
                ("C:\\Users\\x".into(), vec![1]),
                ("a1//x.json".into(), vec![1]),
                ("a1/smime-keep.bin".into(), vec![1]),
                ("a1/ok.json".into(), vec![2]),
            ],
            ..Default::default()
        };
        assert_eq!(read(&write(&odd, None, [0; 28]).unwrap(), None).unwrap().files, [("a1/ok.json".to_string(), vec![2])]);
        // A file that asks for more key rounds than any MailCraft uses is refused before any.
        let mut greedy = write_with(&sample(), Some("pw"), [3; 28], 16).unwrap();
        let at = MAGIC.len() + 2 + 16;
        greedy[at..at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(read(&greedy, Some("pw")), Err(Error::Damaged("its key")));
    }

    /// The session's own: what a browser keeps, the downloaded mail left out unless asked for,
    /// under a name with the day; not where MailCraft keeps data files.
    #[test]
    fn a_session_backs_up_what_the_browser_keeps() {
        let store = mailcraft_store::Store::memory();
        crate::sample::seed_if_empty_at(&store, 1_794_500_000, 0).unwrap();
        let mut s = crate::Session::open(Some(store.clone()), mailcraft_model::dates::Clock::new(1_794_500_000, 0));
        assert!(s.backs_up());
        let (name, small) = s.make_backup(None, false, serde_json::json!({"dark": true}), None).unwrap();
        assert_eq!(name, "MailCraft backup 2026-11-12.mailcraft-backup");
        let (_, whole) = s.make_backup(None, true, serde_json::json!({"dark": true}), Some([5; 28])).unwrap();
        let (small, whole) = (read(&small, None).unwrap(), read(&whole, None).unwrap());
        assert!(small.files.iter().any(|(p, _)| p == "accounts.json") && !small.files.iter().any(|(p, _)| p.ends_with(".eml")));
        assert!(whole.files.iter().any(|(p, _)| p.ends_with(".eml")) && whole.files.len() > small.files.len());
        assert_eq!((small.state["dark"].as_bool(), small.made), (Some(true), 1_794_500_000));
        // Brought into a fresh cache: the same accounts and mail lists.
        let mut again =
            crate::Session::open(Some(mailcraft_store::Store::memory_from(whole.files)), mailcraft_model::dates::Clock::new(1_794_500_000, 0));
        assert_eq!(again.accounts.len(), s.accounts.len());
        assert_eq!(again.folder_state("sample", "INBOX").messages.len(), s.folder_state("sample", "INBOX").messages.len());
        // Two made one after the other don't share what locks them.
        let (_, a) = s.make_backup(Some("pw"), false, serde_json::Value::Null, None).unwrap();
        let (_, b) = s.make_backup(Some("pw"), false, serde_json::Value::Null, None).unwrap();
        assert_ne!(a.get(MAGIC.len() + 2..MAGIC.len() + 18), b.get(MAGIC.len() + 2..MAGIC.len() + 18));
        // Kept in files on a computer: nothing to back up this way.
        let dir = std::env::temp_dir().join(format!("mailcraft-backup-{}", std::process::id()));
        let on_disk = crate::Session::open(mailcraft_store::Store::open(&dir).ok(), mailcraft_model::dates::Clock::new(0, 0));
        assert!(!on_disk.backs_up());
        drop(on_disk);
        let _ = std::fs::remove_dir_all(dir);
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(24))]
        #[test]
        fn any_backup_round_trips(
            files in proptest::collection::vec(("[a-z0-9]{1,8}(/[a-z0-9.]{1,8}){0,3}", proptest::collection::vec(any::<u8>(), 0..300)), 0..12),
            made in any::<i64>(),
            lock in any::<bool>(),
            random in any::<[u8; 28]>(),
        ) {
            let files: Vec<(String, Vec<u8>)> = files.into_iter().filter(|(p, _)| inside(p) && keeps(p, true)).collect();
            let b = Backup { files, state: serde_json::json!({"n": made}), made };
            let password = lock.then_some("pw");
            prop_assert_eq!(read(&write_with(&b, password, random, 16).unwrap(), password).unwrap(), b);
        }

        #[test]
        fn junk_is_never_a_panic(junk in proptest::collection::vec(any::<u8>(), 0..400), lock in any::<bool>()) {
            let mut file = MAGIC.to_vec();
            file.extend([VERSION, u8::from(lock)]);
            file.extend(&junk);
            let _ = read(&file, Some("pw"));
            let _ = read(&junk, None);
        }
    }
}
