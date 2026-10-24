//! The pool: S/MIME's work with an account's private key (issue #6), each job on a thread of
//! its own so it never waits behind a sync. The key and its chain live in the system's keychain
//! only (`Credentials::set_secret`, under `smime`); the engine sees the certificate, never the key.
//! A key on a smart card or in the macOS Keychain stays there: what's kept is where it is (and
//! the card's PIN), and signing and decrypting happen there ([`mailcraft_keys`]).

use mailcraft_keys::KeyRef;
use mailcraft_model::{Private, Revocation};
use mailcraft_smime as smime;
use serde::{Deserialize, Serialize};
use smime::revocation::{self as rev, Status};

use super::*;

/// The keychain entry, next to the account's password.
const SECRET: &str = "smime";

/// What the keychain keeps: the key (PKCS #8) and the chain, as hex; or, for a key held
/// elsewhere, where it is and the card's PIN.
#[derive(Serialize, Deserialize)]
struct Kept {
    #[serde(default)]
    key: String,
    chain: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    held: Option<KeyRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pin: Option<String>,
}

/// An account's own certificate and its private key.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Me {
    /// The certificate first, then the ones that issued it (DER).
    pub(crate) chain: Vec<Vec<u8>>,
    key: MyKey,
}

#[derive(Clone, PartialEq, Eq)]
enum MyKey {
    /// PKCS #8, from a `.p12` file.
    Software(Vec<u8>),
    /// On a smart card, or in the system's keychain.
    Held { at: KeyRef, pin: Option<String> },
}

impl std::fmt::Debug for Me {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Me").field("chain", &self.chain.len()).finish_non_exhaustive()
    }
}

impl From<smime::Identity> for Me {
    fn from(id: smime::Identity) -> Me {
        Me { chain: id.chain, key: MyKey::Software(id.key) }
    }
}

impl Me {
    pub(crate) fn certificate(&self) -> &[u8] {
        self.chain.first().map(Vec::as_slice).unwrap_or_default()
    }

    /// `f` done with the private key, wherever it is.
    fn with_key<T>(&self, f: impl FnOnce(&mut dyn smime::PrivateKey) -> std::result::Result<T, smime::Error>) -> std::result::Result<T, String> {
        match &self.key {
            MyKey::Software(key) => f(&mut smime::key::Software { key, rng: &mut rand_core::OsRng }).map_err(|e| e.to_string()),
            MyKey::Held { at, pin } => with_held(at, pin.as_deref(), mailcraft_keys::pkcs11::PinFrom::Kept, f),
        }
    }
}

fn with_held<T>(
    at: &KeyRef,
    pin: Option<&str>,
    from: mailcraft_keys::pkcs11::PinFrom,
    f: impl FnOnce(&mut dyn smime::PrivateKey) -> std::result::Result<T, smime::Error>,
) -> std::result::Result<T, String> {
    match at {
        KeyRef::Card { .. } => {
            let mut k = mailcraft_keys::pkcs11::open(at, pin, from).map_err(|e| match e {
                // The kept PIN is wrong: said so, and the pool forgets it (`forget_wrong_pin`).
                mailcraft_keys::Error::Pin if pin.is_some() && from == mailcraft_keys::pkcs11::PinFrom::Kept => PIN_WRONG.to_string(),
                e => e.to_string(),
            })?;
            f(&mut k).map_err(|e| e.to_string())
        }
        #[cfg(target_os = "macos")]
        KeyRef::Keychain { .. } => {
            let mut k = mailcraft_keys::macos::open(at).map_err(|e| e.to_string())?;
            f(&mut k).map_err(|e| e.to_string())
        }
        #[cfg(not(target_os = "macos"))]
        KeyRef::Keychain { .. } => Err("the certificate is in another computer's keychain".into()),
        #[cfg(windows)]
        KeyRef::WindowsStore { .. } => {
            let mut k = mailcraft_keys::windows::open(at).map_err(|e| e.to_string())?;
            f(&mut k).map_err(|e| e.to_string())
        }
        #[cfg(not(windows))]
        KeyRef::WindowsStore { .. } => Err("the certificate is in another computer's certificate store".into()),
    }
}

/// What a kept card PIN that the card refused says. It's forgotten at once, so it's never tried
/// again: a card locks after a few wrong ones.
const PIN_WRONG: &str = "the card refused its PIN, so MailCraft forgot it: the card would lock if it were tried again (give it again in File › Info › Security › Use a smart card…)";

/// After `error`, a kept PIN the card refused is forgotten (the reference to the key stays).
fn forget_wrong_pin<T>(creds: &dyn Credentials, account: &AccountConfig, done: &std::result::Result<T, String>) {
    if !matches!(done, Err(e) if e.contains(PIN_WRONG)) {
        return;
    }
    let Some(json) = creds.get_secret(account, SECRET) else { return };
    let Ok(mut k) = serde_json::from_str::<Kept>(&json) else { return };
    k.pin = None;
    if let Ok(json) = serde_json::to_string(&k) {
        let _ = creds.set_secret(account, SECRET, &json);
    }
}

/// The certificates whose keys are held elsewhere: on the cards `module` sees, or in the
/// system's own store (the macOS Keychain, the Windows certificate store).
fn find_held(module: Option<&str>, pin: Option<&str>) -> std::result::Result<Vec<mailcraft_model::HeldKey>, String> {
    let found = match module {
        Some(m) => mailcraft_keys::pkcs11::certificates(m, pin).map_err(|e| e.to_string())?,
        #[cfg(target_os = "macos")]
        None => mailcraft_keys::macos::certificates().map_err(|e| e.to_string())?,
        #[cfg(windows)]
        None => mailcraft_keys::windows::certificates().map_err(|e| e.to_string())?,
        #[cfg(not(any(target_os = "macos", windows)))]
        None => return Err("this system's keychain can't sign for MailCraft: use a smart card's module, or import a .p12 file".into()),
    };
    Ok(found
        .into_iter()
        .filter_map(|f| Some(mailcraft_model::HeldKey { label: f.label, cert: f.cert, at: serde_json::to_string(&f.key).ok()? }))
        .collect())
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

fn keep(creds: &dyn Credentials, account: &AccountConfig, me: &Me) -> std::result::Result<(), String> {
    let chain = me.chain.iter().map(|c| hex(c)).collect();
    let kept = match &me.key {
        MyKey::Software(key) => Kept { key: hex(key), chain, held: None, pin: None },
        MyKey::Held { at, pin } => Kept { key: String::new(), chain, held: Some(at.clone()), pin: pin.clone() },
    };
    let json = serde_json::to_string(&kept).map_err(|e| e.to_string())?;
    creds.set_secret(account, SECRET, &json)
}

fn kept(creds: &dyn Credentials, account: &AccountConfig) -> std::result::Result<Me, String> {
    let gone = || "the certificate's key isn't in the keychain: import the certificate again (File › Info › Security)".to_string();
    let json = creds.get_secret(account, SECRET).ok_or_else(gone)?;
    let k: Kept = serde_json::from_str(&json).map_err(|_| gone())?;
    let chain = k.chain.iter().map(|c| unhex(c)).collect::<Option<Vec<_>>>().ok_or_else(gone)?;
    let key = match k.held {
        Some(at) => MyKey::Held { at, pin: k.pin },
        None => MyKey::Software(unhex(&k.key).ok_or_else(gone)?),
    };
    Ok(Me { chain, key })
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| i64::try_from(d.as_secs()).unwrap_or(0)).unwrap_or(0)
}

/// `raw` signed and/or encrypted with `id`, as it will be sent.
pub(crate) fn protect(
    raw: &[u8],
    id: &Me,
    sign: bool,
    encrypt_to: &[Vec<u8>],
    gcm: bool,
    hide_subject: bool,
) -> std::result::Result<Vec<u8>, String> {
    use rand_core::RngCore as _;
    let boundary = format!("=_mcs{:016x}", rand_core::OsRng.next_u64());
    let mut sign_with = |entity: &[u8]| id.with_key(|k| smime::sign_with(entity, &id.chain, k, now()));
    let cipher = if gcm { smime::Cipher::Aes256Gcm } else { smime::Cipher::Aes256Cbc };
    let mut encrypt_with = |entity: &[u8]| smime::encrypt(entity, encrypt_to, cipher, &mut rand_core::OsRng).map_err(|e| e.to_string());
    let signer: Option<mailcraft_mime::smime::Crypto<'_>> = if sign { Some(&mut sign_with) } else { None };
    let encrypter: Option<mailcraft_mime::smime::Crypto<'_>> = if encrypt_to.is_empty() { None } else { Some(&mut encrypt_with) };
    mailcraft_mime::smime::protect(raw, signer, encrypter, gcm, hide_subject, &boundary)
}

impl Pool {
    /// A draft encrypted to the account's own certificate, at once (it may be the last thing
    /// before MailCraft closes). Without the key it stays as it is, and the user is told.
    pub(super) fn seal(&self, account: &AccountConfig, raw: Vec<u8>) -> Vec<u8> {
        let sealed = kept(self.creds.as_ref(), account).and_then(|me| {
            let own = me.chain.first().cloned().ok_or_else(|| "the certificate is missing".to_string())?;
            protect(&raw, &me, false, &[own], true, false)
        });
        forget_wrong_pin(self.creds.as_ref(), account, &sealed);
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
                    let me = Me::from(id);
                    keep(creds.as_ref(), &account, &me)?;
                    Ok(me.chain)
                });
                let (chain, error) = match done {
                    Ok(chain) => (chain, None),
                    Err(e) => (Vec::new(), Some(e)),
                };
                sink.emit(Event::Identity { account: account.id, chain, error });
            }
            Job::ForgetIdentity { account } => creds.delete_secret(&account, SECRET),
            Job::Protect { account, id, raw, sign, encrypt_to, gcm, hide_subject } => {
                let done = kept(creds.as_ref(), &account).and_then(|me| protect(&raw.0, &me, sign, &encrypt_to, gcm, hide_subject));
                forget_wrong_pin(creds.as_ref(), &account, &done);
                let (raw, error) = match done {
                    Ok(r) => (Some(Private(r)), None),
                    Err(e) => (None, Some(e)),
                };
                sink.emit(Event::Protected { account: account.id, id, raw, error });
            }
            Job::Decrypt { account, key, cms } => {
                let done = kept(creds.as_ref(), &account).and_then(|me| me.with_key(|k| smime::decrypt_with(&cms, me.certificate(), k)));
                forget_wrong_pin(creds.as_ref(), &account, &done);
                let event = match done {
                    Ok((content, authenticated)) => Event::Decrypted { key, content: Some(Private(content)), authenticated, error: None },
                    Err(e) => Event::Decrypted { key, content: None, authenticated: false, error: Some(e) },
                };
                sink.emit(event);
            }
            Job::FindHeldKeys { module, pin } => {
                let (keys, error) = match find_held(module.as_deref(), pin.as_ref().map(|p| p.0.as_str())) {
                    Ok(k) => (k, None),
                    Err(e) => (Vec::new(), Some(e)),
                };
                sink.emit(Event::HeldKeys { keys, error });
            }
            Job::UseHeldKey { account, at, cert, pin } => {
                let done = serde_json::from_str::<KeyRef>(&at).map_err(|_| "the key can't be found again".to_string()).and_then(|at| {
                    // The issuers Windows knows go with it, as a .p12's do, so readers can check it.
                    #[cfg(windows)]
                    let chain = [vec![cert], mailcraft_keys::windows::issuers(&at)].concat();
                    #[cfg(not(windows))]
                    let chain = vec![cert];
                    let me = Me { chain, key: MyKey::Held { at, pin: pin.map(|p| p.0) } };
                    // It's used once here, so a wrong PIN or a card taken out says so now (the PIN
                    // was just typed: the user's to try).
                    let MyKey::Held { at, pin } = &me.key else { return Err("the key can't be found again".to_string()) };
                    with_held(at, pin.as_deref(), mailcraft_keys::pkcs11::PinFrom::Typed, |k| smime::sign_with(b"MailCraft", &me.chain, k, now()))?;
                    keep(creds.as_ref(), &account, &me)?;
                    Ok(me.chain)
                });
                let (chain, error) = match done {
                    Ok(chain) => (chain, None),
                    Err(e) => (Vec::new(), Some(e)),
                };
                sink.emit(Event::Identity { account: account.id, chain, error });
            }
            Job::CheckRevocation { chain } => {
                let status = revocation(&chain, now(), &|r| crate::http::send_public(r).ok().filter(|a| a.ok()).map(|a| a.body));
                sink.emit(Event::Revocation { cert: chain.first().cloned().unwrap_or_default(), status });
            }
            _ => {}
        });
        if let Err(e) = spawned {
            log::warn!("S/MIME: {e}");
        }
    }
}

/// The largest list of withdrawn certificates fetched (a big authority's runs to megabytes).
const CRL_MAX: usize = 16 << 20;
const OCSP_MAX: usize = 64 << 10;
/// Certificates of a chain asked about (the signer's and its issuers', not the root's).
const ASKED_MAX: usize = 4;

/// Whether a chain's certificates were withdrawn: each one's authority asked by OCSP, else by its
/// CRL. The signer's answer is the chain's, unless one of its issuers was withdrawn. `fetch` makes
/// the request (on the internet only) and gives back a successful answer's body.
fn revocation(chain: &[Vec<u8>], now: i64, fetch: &dyn Fn(&crate::http::Request) -> Option<Vec<u8>>) -> Revocation {
    let mut signer = None;
    for pair in chain.windows(2).take(ASKED_MAX) {
        let [cert, issuer] = pair else { continue };
        match status_of(cert, issuer, now, fetch) {
            Status::Revoked { at } => return Revocation::Revoked { at },
            s if signer.is_none() => signer = Some(s),
            _ => {}
        }
    }
    match signer {
        Some(Status::Good { until }) => Revocation::Good { until },
        Some(Status::Unknown(why)) => Revocation::Unknown { why },
        _ => Revocation::Unknown { why: "the certificate's authority isn't known".into() },
    }
}

fn status_of(cert: &[u8], issuer: &[u8], now: i64, fetch: &dyn Fn(&crate::http::Request) -> Option<Vec<u8>>) -> Status {
    let (crls, responders) = rev::points(cert);
    let mut last = Status::Unknown("its certificate doesn't say where to ask".into());
    if let Ok(req) = rev::ocsp_request(cert, issuer) {
        for url in responders.iter().take(2) {
            let r = crate::http::Request::post(url, "application/ocsp-request", req.clone(), OCSP_MAX).header("Accept", "application/ocsp-response");
            last = match fetch(&r) {
                Some(answer) => rev::ocsp_status(&answer, cert, issuer, now),
                None => Status::Unknown("its authority's responder didn't answer".into()),
            };
            if !matches!(last, Status::Unknown(_)) {
                return last;
            }
        }
    }
    for url in crls.iter().take(2) {
        last = match fetch(&crate::http::Request::get(url, CRL_MAX)) {
            Some(list) => rev::crl_status(&list, cert, issuer, now),
            None => Status::Unknown("its authority's list of withdrawn certificates couldn't be had".into()),
        };
        if !matches!(last, Status::Unknown(_)) {
            return last;
        }
    }
    last
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pem(name: &str) -> Vec<u8> {
        der::pem::decode_vec(&testdata(name)).unwrap().1
    }

    /// The authority's answers as OpenSSL made them, served by a stand-in for the web.
    #[test]
    fn revocation_asks_the_responder_then_the_list() {
        const NOW: i64 = 1_792_900_000;
        let (ca, kim, lee) = (pem("ca.pem"), pem("rev-kim.pem"), pem("rev-lee.pem"));
        let answers = |ocsp: bool, crl: bool| {
            move |r: &crate::http::Request| -> Option<Vec<u8>> {
                if r.url.starts_with("http://ocsp.") && ocsp {
                    let lee_req = rev::ocsp_request(&pem("rev-lee.pem"), &pem("ca.pem")).unwrap();
                    return Some(testdata(if r.body == lee_req { "rev-lee-ocsp.der" } else { "rev-kim-ocsp.der" }));
                }
                (r.url.starts_with("http://crl.") && crl).then(|| testdata("rev-ca-crl.der"))
            }
        };
        for (ocsp, crl) in [(true, false), (false, true), (true, true)] {
            let f = answers(ocsp, crl);
            assert_eq!(revocation(&[lee.clone(), ca.clone()], NOW, &f), Revocation::Revoked { at: 1_792_850_401 }, "{ocsp} {crl}");
            assert!(matches!(revocation(&[kim.clone(), ca.clone()], NOW, &f), Revocation::Good { .. }), "{ocsp} {crl}");
        }
        // Nothing answers: unknown, never withdrawn; a chain of one asks no one.
        let none = answers(false, false);
        assert!(matches!(revocation(&[lee.clone(), ca.clone()], NOW, &none), Revocation::Unknown { .. }));
        assert!(matches!(revocation(std::slice::from_ref(&lee), NOW, &none), Revocation::Unknown { .. }));
        // Kim's answer for anyone who asks: Lee's request gets "good" for Kim, which doesn't count.
        let replay = |r: &crate::http::Request| r.url.starts_with("http://ocsp.").then(|| testdata("rev-kim-ocsp.der"));
        assert!(matches!(revocation(&[lee, ca], NOW, &replay), Revocation::Unknown { .. }));
    }

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
            let sent = protect(raw, &Me::from(riley.clone()), true, &[riley.chain[0].clone()], gcm, true).unwrap();
            let text = String::from_utf8_lossy(&sent);
            assert!(
                text.contains("Subject: [...]") && !text.contains("Subject: Plan") && !text.contains("The plan"),
                "neither the subject nor the text shows"
            );
            let mailcraft_mime::smime::Protection::Encrypted(cms) = mailcraft_mime::smime::protection(&sent) else { panic!("{text}") };
            let (entity, authenticated) = smime::decrypt(&cms, &riley, &mut rand_core::OsRng).unwrap();
            assert_eq!(authenticated, gcm);
            let inner = mailcraft_mime::smime::with_entity(&sent, &entity);
            let mailcraft_mime::smime::Protection::Signed { content, signature } = mailcraft_mime::smime::protection(&inner) else { panic!() };
            let (signed, _) = smime::verify(&signature, Some(&content)).unwrap();
            // The subject is inside the signed part, protected (RFC 9788's cryptographic payload).
            assert!(mailcraft_mime::smime::with_entity_protected(&inner, &content).1);
            assert_eq!(signed.signer, riley.chain[0]);
            let read = mailcraft_mime::parse(&mailcraft_mime::smime::with_entity(&inner, &content)).unwrap();
            assert!(read.text.contains("The plan \u{2713}"), "{}", read.text);
            assert_eq!(read.subject, "Plan", "the real subject, from inside");
        }
        // Signed only: readable by anyone, the signature over its text.
        let signed = protect(raw, &Me::from(riley.clone()), true, &[], false, true).unwrap();
        assert!(matches!(mailcraft_mime::smime::protection(&signed), mailcraft_mime::smime::Protection::Signed { .. }));
    }

    /// The app's whole path with a certificate in the Windows store (the test identities imported
    /// by hand, as `mailcraft-keys`' store test says): found, kept as a reference only, then a
    /// message signed and encrypted to it and decrypted, all in the store.
    #[cfg(windows)]
    #[test]
    #[ignore = "needs the test identities imported into Current User › Personal"]
    fn a_windows_store_certificate_signs_encrypts_and_decrypts() {
        let found = find_held(None, None).unwrap();
        let account = AccountConfig { id: "store".into(), ..Default::default() };
        for who in ["riley@studio.example", "zoe@posteo.example"] {
            let held = found.iter().find(|h| h.label.contains(who)).unwrap_or_else(|| panic!("{who}: {found:?}"));
            let at: KeyRef = serde_json::from_str(&held.at).unwrap();
            let creds = MemoryCredentials::default();
            keep(&creds, &account, &Me { chain: vec![held.cert.clone()], key: MyKey::Held { at, pin: None } }).unwrap();
            let me = kept(&creds, &account).unwrap();
            assert!(!creds.get_secret(&account, SECRET).unwrap().contains("PRIVATE"), "only where the key is");
            let raw = format!(
                "From: <{who}>
To: <{who}>
Subject: Store
Date: Sat, 24 Oct 2026 09:00:00 +0000
MIME-Version: 1.0
Content-Type: text/plain; charset=utf-8

Signed and sealed by Windows.
"
            );
            for gcm in [false, true] {
                let sent = protect(raw.as_bytes(), &me, true, std::slice::from_ref(&held.cert), gcm, true).unwrap_or_else(|e| panic!("{who}: {e}"));
                let mailcraft_mime::smime::Protection::Encrypted(cms) = mailcraft_mime::smime::protection(&sent) else { panic!("{who}") };
                let (entity, _) = me.with_key(|k| smime::decrypt_with(&cms, me.certificate(), k)).unwrap_or_else(|e| panic!("{who}: {e}"));
                let inner = mailcraft_mime::smime::with_entity(&sent, &entity);
                let mailcraft_mime::smime::Protection::Signed { content, signature } = mailcraft_mime::smime::protection(&inner) else { panic!() };
                assert_eq!(smime::verify(&signature, Some(&content)).unwrap().0.signer, held.cert, "{who}");
            }
        }
    }

    #[test]
    fn the_key_is_kept_in_the_keychain_and_comes_back() {
        let creds = MemoryCredentials::default();
        let account = AccountConfig { id: "a1".into(), ..Default::default() };
        assert!(kept(&creds, &account).is_err());
        let riley = smime::read_p12(&testdata("riley-aes.p12"), "secret").unwrap();
        keep(&creds, &account, &Me::from(riley.clone())).unwrap();
        assert!(kept(&creds, &account).unwrap() == Me::from(riley.clone()));
        // A card's key: only where it is, and its PIN.
        let card = Me {
            chain: riley.chain.clone(),
            key: MyKey::Held { at: KeyRef::Card { module: "m.so".into(), token: "1".into(), id: "01".into() }, pin: Some("1234".into()) },
        };
        keep(&creds, &account, &card).unwrap();
        assert!(kept(&creds, &account).unwrap() == card);
        assert!(!creds.get_secret(&account, SECRET).unwrap().contains(&hex(&riley.key)));
        // A kept PIN the card refused is forgotten at once, the key's reference kept; any other
        // failure leaves it.
        forget_wrong_pin(&creds, &account, &Err::<(), _>("the card's module can't be loaded".to_string()));
        assert!(kept(&creds, &account).unwrap() == card);
        forget_wrong_pin(&creds, &account, &Err::<(), _>(PIN_WRONG.to_string()));
        let left = kept(&creds, &account).unwrap();
        assert!(matches!(&left.key, MyKey::Held { pin: None, .. }) && left.chain == card.chain);
        creds.delete_secret(&account, SECRET);
        assert!(kept(&creds, &account).is_err());
        assert_eq!(unhex("0aFf"), Some(vec![0x0a, 0xff]));
        assert_eq!(unhex("0"), None);
        assert_eq!(unhex("zz"), None);
    }
}
