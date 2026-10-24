//! S/MIME as it arrives: BER rewritten as DER, a signature checked (detached, and carrying its
//! content), encrypted mail opened with a test key, a certificate read and checked for a sender.
#![no_main]

use libfuzzer_sys::fuzz_target;
use mailcraft_smime as smime;

/// A small RSA test identity (the crate's test CA's Riley), read once.
fn riley() -> Option<&'static smime::Identity> {
    static ID: std::sync::OnceLock<Option<smime::Identity>> = std::sync::OnceLock::new();
    ID.get_or_init(|| smime::read_p12(include_bytes!("../../crates/smime/testdata/riley-aes.p12"), "secret").ok()).as_ref()
}

fuzz_target!(|input: &[u8]| {
    let _ = smime::ber::to_der(input);
    let _ = smime::verify(input, Some(b"content"));
    let _ = smime::verify(input, None);
    let _ = smime::cert::info(input);
    let _ = smime::cert::check(input, &[input.to_vec()], &[input.to_vec()], 1_800_000_000, "ana@posteo.example");
    if let Some(id) = riley() {
        let _ = smime::decrypt(input, id, &mut rand_core::OsRng);
    }
    let _ = smime::read_p12(input, "secret");
});
