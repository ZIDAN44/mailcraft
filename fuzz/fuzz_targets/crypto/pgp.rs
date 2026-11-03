//! OpenPGP as it arrives: armor and packets read as keys (public, secret, Autocrypt's), a message
//! decrypted with a test key and its signature checked, signatures detached and cleartext, and
//! PGP/MIME and inline PGP found in a message.
#![no_main]

use libfuzzer_sys::fuzz_target;
use mailcraft_pgp as pgp;

/// The crate's GnuPG test key (Ana, Curve25519), unlocked once.
fn ana() -> Option<&'static pgp::Unlocked> {
    static KEY: std::sync::OnceLock<Option<pgp::Unlocked>> = std::sync::OnceLock::new();
    KEY.get_or_init(|| pgp::read_secret_key(include_bytes!("../../../crates/pgp/testdata/ana-secret.asc"), "secret").ok()).as_ref()
}

fuzz_target!(|input: &[u8]| {
    if let Some(ana) = ana() {
        let _ = pgp::decrypt(input, &[&ana.secret], &[&ana.public, input]);
        let _ = pgp::open(input, &[&ana.public, input]);
        let _ = pgp::verify(b"Content-Type: text/plain\r\n\r\nx\r\n", input, &[&ana.public, input]);
        let _ = pgp::verify_cleartext(input, &[&ana.public]);
    }
    let _ = pgp::recipients(input);
    let _ = pgp::is_encrypted(input);
    let _ = pgp::inline(input);
    if let Ok(keys) = pgp::public_keys(input) {
        for k in keys.iter().take(4) {
            let _ = pgp::info(k);
            let _ = pgp::minimal(k, "ana@posteo.example");
        }
    }
    let _ = pgp::info(input);
    let _ = pgp::read_secret_key(input, "secret");
    let text = String::from_utf8_lossy(input);
    let _ = pgp::autocrypt::parse(&text);
    let _ = mailcraft_mime::pgp::protection(input);
    let _ = mailcraft_mime::pgp::autocrypt(input);
    let _ = mailcraft_mime::pgp::key_parts(input);
});
