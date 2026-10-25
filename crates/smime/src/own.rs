//! One's own certificates where there's no keychain to hold their keys (the browser): each kept
//! as the `.p12` file it came in, still encrypted with its password, beside its certificate (in
//! the clear: it's public). As every mail program that handles renewals does (Thunderbird's and
//! the system's certificate stores, Proton's older keys), a renewal keeps the old certificate:
//! the newer one signs and encrypts, the older ones decrypt mail that was sent to them.
//!
//! Nothing here keeps anything: the caller stores [`Own`] and opens the files with
//! [`Own::unlock`].

use crate::{Error, Identity, cert, read_p12};

/// The most older certificates kept: a renewal a year for a long time.
pub const OLDER_MAX: usize = 16;

/// A certificate and the `.p12` file its key is in.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    /// The certificate (DER).
    pub cert: Vec<u8>,
    /// The `.p12` file, as it came (encrypted with its password).
    pub p12: Vec<u8>,
}

impl Entry {
    /// Its SHA-256, as [`cert::CertInfo::fingerprint`] has it (`AB:CD:…`).
    pub fn fingerprint(&self) -> String {
        cert::info(&self.cert).map(|i| i.fingerprint).unwrap_or_default()
    }
}

/// An account's own certificates: the one in use, and the older ones, newest first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Own {
    pub current: Option<Entry>,
    pub older: Vec<Entry>,
}

/// What [`Own::take`] did with the certificate it was given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Taken {
    /// It's the one in use now (the first, a renewal, or the same one again).
    InUse,
    /// It's older than the one in use: kept to read mail sent to it.
    ForReading,
}

impl Own {
    /// Keep `entry`: the newer of it and the one in use signs and encrypts, the other is kept for
    /// reading. At most [`OLDER_MAX`] older ones: one more is refused rather than one dropped
    /// unasked.
    pub fn take(&mut self, entry: Entry, now: i64) -> Result<Taken, Error> {
        self.take_within(entry, now, OLDER_MAX)
    }

    fn take_within(&mut self, entry: Entry, now: i64, cap: usize) -> Result<Taken, Error> {
        let fp = entry.fingerprint();
        if fp.is_empty() {
            return Err(Error::Malformed("a certificate can't be read"));
        }
        let Some(cur) = self.current.clone() else {
            self.older.retain(|e| e.fingerprint() != fp);
            self.current = Some(entry);
            return Ok(Taken::InUse);
        };
        if cur.fingerprint() == fp {
            self.current = Some(entry);
            return Ok(Taken::InUse);
        }
        if !cert::newer(&entry.cert, &cur.cert, now) {
            self.keep_older(entry, None, cap)?;
            return Ok(Taken::ForReading);
        }
        self.keep_older(cur, Some(&fp), cap)?;
        self.current = Some(entry);
        Ok(Taken::InUse)
    }

    /// Put `entry` first among the older ones; `leaving` (one about to become the one in use)
    /// makes room.
    fn keep_older(&mut self, entry: Entry, leaving: Option<&str>, cap: usize) -> Result<(), Error> {
        let fp = entry.fingerprint();
        let mut older: Vec<Entry> = self
            .older
            .iter()
            .filter(|e| {
                let f = e.fingerprint();
                f != fp && Some(f.as_str()) != leaving
            })
            .cloned()
            .collect();
        if older.len() >= cap {
            return Err(Error::Unsupported(format!("keeping more than {cap} older certificates: remove one first")));
        }
        older.insert(0, entry);
        self.older = older;
        Ok(())
    }

    /// The older certificate `fingerprint` signs and encrypts again; the one in use is kept for
    /// reading.
    pub fn use_older(&mut self, fingerprint: &str) -> Result<(), Error> {
        let Some(at) = self.older.iter().position(|e| e.fingerprint().eq_ignore_ascii_case(fingerprint)) else {
            return Err(Error::Unsupported("that older certificate isn't kept".into()));
        };
        let chosen = self.older.remove(at);
        if let Some(cur) = self.current.take() {
            self.older.insert(0, cur);
        }
        self.current = Some(chosen);
        Ok(())
    }

    /// Forget the older certificate `fingerprint`; whether it was kept.
    pub fn forget_older(&mut self, fingerprint: &str) -> bool {
        let before = self.older.len();
        self.older.retain(|e| !e.fingerprint().eq_ignore_ascii_case(fingerprint));
        self.older.len() != before
    }

    /// The older certificates themselves, newest first.
    pub fn older_certs(&self) -> Vec<Vec<u8>> {
        self.older.iter().map(|e| e.cert.clone()).collect()
    }

    /// Every kept file `password` opens (renewals often share one): each with its key. Those it
    /// doesn't open stay locked.
    pub fn unlock(&self, password: &str) -> Vec<Identity> {
        self.current.iter().chain(&self.older).filter_map(|e| read_p12(&e.p12, password).ok()).filter(|id| !id.chain.is_empty()).collect()
    }

    /// The kept certificate `cms` (encrypted mail) was sent to: the one in use first.
    pub fn for_message(&self, cms: &[u8]) -> Option<&Entry> {
        self.current.iter().chain(&self.older).find(|e| crate::addressed_to(cms, &e.cert))
    }
}

#[cfg(test)]
mod tests;
