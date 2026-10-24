//! The pool: S/MIME's work with an account's private key (issue #6), each job on a thread of
//! its own so it never waits behind a sync. The key and its chain live in the system's keychain
//! only (`Credentials::set_secret`, under `smime`); the engine sees the certificate, never the key.

use mailcraft_model::Private;
use mailcraft_smime as smime;
use serde::{Deserialize, Serialize};

use super::*;

/// The keychain entry, next to the account's password.
const SECRET: &str = "smime";

/// What the keychain keeps: the key (PKCS #8) and the chain, as hex.
#[derive(Serialize, Deserialize)]
struct Kept {
    key: String,
    chain: Vec<String>,
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| s.get(i..i + 2).and_then(|p| u8::from_str_radix(p, 16).ok())).collect()
}

fn keep(creds: &dyn Credentials, account: &AccountConfig, id: &smime::Identity) -> std::result::Result<(), String> {
    let kept = Kept { key: hex(&id.key), chain: id.chain.iter().map(|c| hex(c)).collect() };
    let json = serde_json::to_string(&kept).map_err(|e| e.to_string())?;
    creds.set_secret(account, SECRET, &json)
}

fn kept(creds: &dyn Credentials, account: &AccountConfig) -> std::result::Result<smime::Identity, String> {
    let gone = || "the certificate's key isn't in the keychain: import the certificate again (File › Info › Security)".to_string();
    let json = creds.get_secret(account, SECRET).ok_or_else(gone)?;
    let k: Kept = serde_json::from_str(&json).map_err(|_| gone())?;
    let key = unhex(&k.key).ok_or_else(gone)?;
    let chain = k.chain.iter().map(|c| unhex(c)).collect::<Option<Vec<_>>>().ok_or_else(gone)?;
    Ok(smime::Identity { key, chain })
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| i64::try_from(d.as_secs()).unwrap_or(0)).unwrap_or(0)
}

/// `raw` signed and/or encrypted with `id`, as it will be sent.
pub(crate) fn protect(raw: &[u8], id: &smime::Identity, sign: bool, encrypt_to: &[Vec<u8>], gcm: bool) -> std::result::Result<Vec<u8>, String> {
    use rand_core::RngCore as _;
    let mut rng = rand_core::OsRng;
    let (outer, mut entity) = mailcraft_mime::smime::entity_of(raw);
    let mut message = mailcraft_mime::smime::with_entity(raw, &entity);
    if sign {
        if !mailcraft_mime::smime::safe_to_sign(&entity) {
            return Err("a part of it couldn't be sent byte for byte, so its signature wouldn't hold".into());
        }
        let signature = smime::sign(&entity, id, now(), &mut rng).map_err(|e| e.to_string())?;
        let boundary = format!("=_mcs{:016x}", rng.next_u64());
        message = mailcraft_mime::smime::signed_message(&outer, &entity, &signature, &boundary);
        entity = mailcraft_mime::smime::entity_of(&message).1;
    }
    if !encrypt_to.is_empty() {
        let cipher = if gcm { smime::Cipher::Aes256Gcm } else { smime::Cipher::Aes256Cbc };
        let cms = smime::encrypt(&entity, encrypt_to, cipher, &mut rng).map_err(|e| e.to_string())?;
        message = mailcraft_mime::smime::encrypted_message(&outer, &cms, gcm);
    }
    Ok(message)
}

impl Pool {
    /// A draft encrypted to the account's own certificate, at once (it may be the last thing
    /// before MailCraft closes). Without the key it stays as it is, and the user is told.
    pub(super) fn seal(&self, account: &AccountConfig, raw: Vec<u8>) -> Vec<u8> {
        let sealed = kept(self.creds.as_ref(), account).and_then(|me| {
            let own = me.chain.first().cloned().ok_or_else(|| "the certificate is missing".to_string())?;
            protect(&raw, &me, false, &[own], true)
        });
        match sealed {
            Ok(s) => s,
            Err(e) => {
                self.sink.emit(Event::Notice {
                    account: account.id.clone(),
                    message: format!("the draft couldn't be encrypted, so it's saved as it is: {e}"),
                });
                raw
            }
        }
    }

    pub(super) fn smime(&mut self, job: Job) {
        let creds = self.creds.clone();
        let sink = self.sink.clone();
        let spawned = std::thread::Builder::new().name("mail-smime".into()).spawn(move || match job {
            Job::ImportIdentity { account, data, password } => {
                let done = smime::read_p12(&data, &password.0).map_err(|e| e.to_string()).and_then(|id| {
                    keep(creds.as_ref(), &account, &id)?;
                    Ok(id.chain)
                });
                let (chain, error) = match done {
                    Ok(chain) => (chain, None),
                    Err(e) => (Vec::new(), Some(e)),
                };
                sink.emit(Event::Identity { account: account.id, chain, error });
            }
            Job::ForgetIdentity { account } => creds.delete_secret(&account, SECRET),
            Job::Protect { account, id, raw, sign, encrypt_to, gcm } => {
                let done = kept(creds.as_ref(), &account).and_then(|me| protect(&raw.0, &me, sign, &encrypt_to, gcm));
                let (raw, error) = match done {
                    Ok(r) => (Some(Private(r)), None),
                    Err(e) => (None, Some(e)),
                };
                sink.emit(Event::Protected { account: account.id, id, raw, error });
            }
            Job::Decrypt { account, key, cms } => {
                let done = kept(creds.as_ref(), &account).and_then(|me| smime::decrypt(&cms, &me, &mut rand_core::OsRng).map_err(|e| e.to_string()));
                let event = match done {
                    Ok((content, authenticated)) => Event::Decrypted { key, content: Some(Private(content)), authenticated, error: None },
                    Err(e) => Event::Decrypted { key, content: None, authenticated: false, error: Some(e) },
                };
                sink.emit(event);
            }
            _ => {}
        });
        if let Err(e) = spawned {
            log::warn!("S/MIME: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn testdata(name: &str) -> Vec<u8> {
        std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../smime/testdata").join(name)).unwrap()
    }

    /// A message signed and encrypted here reads back whole: decrypted with the recipient's key,
    /// its signature holding over exactly what was written.
    #[test]
    fn protected_mail_reads_back() {
        let riley = smime::read_p12(&testdata("riley-aes.p12"), "secret").unwrap();
        let raw = b"From: Riley <riley@studio.example>\r\nTo: riley@studio.example\r\nSubject: Plan\r\nDate: Sat, 24 Oct 2026 09:00:00 +0000\r\n\
MIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\nThe plan =E2=9C=93\r\n";
        for gcm in [false, true] {
            let sent = protect(raw, &riley, true, &[riley.chain[0].clone()], gcm).unwrap();
            let text = String::from_utf8_lossy(&sent);
            assert!(text.contains("Subject: Plan") && !text.contains("The plan"), "the subject shows, the text doesn't");
            let mailcraft_mime::smime::Protection::Encrypted(cms) = mailcraft_mime::smime::protection(&sent) else { panic!("{text}") };
            let (entity, authenticated) = smime::decrypt(&cms, &riley, &mut rand_core::OsRng).unwrap();
            assert_eq!(authenticated, gcm);
            let inner = mailcraft_mime::smime::with_entity(&sent, &entity);
            let mailcraft_mime::smime::Protection::Signed { content, signature } = mailcraft_mime::smime::protection(&inner) else { panic!() };
            let (signed, _) = smime::verify(&signature, Some(&content)).unwrap();
            assert_eq!(signed.signer, riley.chain[0]);
            assert!(mailcraft_mime::parse(&mailcraft_mime::smime::with_entity(&inner, &content)).unwrap().text.contains("The plan \u{2713}"));
        }
        // Signed only: readable by anyone, the signature over its text.
        let signed = protect(raw, &riley, true, &[], false).unwrap();
        assert!(matches!(mailcraft_mime::smime::protection(&signed), mailcraft_mime::smime::Protection::Signed { .. }));
    }

    #[test]
    fn the_key_is_kept_in_the_keychain_and_comes_back() {
        let creds = MemoryCredentials::default();
        let account = AccountConfig { id: "a1".into(), ..Default::default() };
        assert!(kept(&creds, &account).is_err());
        let riley = smime::read_p12(&testdata("riley-aes.p12"), "secret").unwrap();
        keep(&creds, &account, &riley).unwrap();
        assert_eq!(kept(&creds, &account).unwrap(), riley);
        creds.delete_secret(&account, SECRET);
        assert!(kept(&creds, &account).is_err());
        assert_eq!(unhex("0aFf"), Some(vec![0x0a, 0xff]));
        assert_eq!(unhex("0"), None);
        assert_eq!(unhex("zz"), None);
    }
}
