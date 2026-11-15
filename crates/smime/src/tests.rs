//! Against OpenSSL 3's output (`testdata/`, made with `openssl cms`: a test CA, Ana and Riley
//! with RSA 2048 keys, Zoe with EC P-256), and round trips of MailCraft's own.

use super::*;

/// The fixtures were made on 2026-10-24; their certificates run for two years.
const NOW: i64 = 1_792_900_000;

fn data(name: &str) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata").join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn pem(name: &str) -> Vec<u8> {
    let text = String::from_utf8(data(name)).unwrap();
    let (_, der) = der::pem::decode_vec(text.as_bytes()).unwrap();
    der
}

fn rng() -> rand_core::OsRng {
    rand_core::OsRng
}

fn riley() -> Identity {
    read_p12(&data("riley-aes.p12"), "secret").unwrap()
}

/// Issue #89: Firefox and Thunderbird (NSS) write a `.p12` in BER, every length left open, and
/// name PBKDF2's HMAC without its NULL. The fixtures are Riley's, exported by NSS's `pk12util`
/// as Thunderbird exports today (AES, SHA-256, 600 000 rounds), as it did before (3DES, SHA-1),
/// and with a password that isn't ASCII. Each was refused as a wrong password.
#[test]
fn a_p12_from_thunderbird_opens() {
    let riley = riley();
    for (name, password) in [("riley-nss.p12", "secret"), ("riley-nss-3des.p12", "secret"), ("riley-nss-words.p12", "pässwörd €")] {
        let file = data(name);
        assert_eq!(file.get(..2), Some(&[0x30, 0x80][..]), "{name}: BER, its length left open");
        let id = read_p12(&file, password).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(id == riley, "{name}: the key and the chain OpenSSL's file has");
        // The password is still checked against the file, as it was written.
        assert_eq!(read_p12(&file, "wrong"), Err(Error::Password), "{name}");
        assert_eq!(read_p12(&file, ""), Err(Error::Password), "{name}");
        let mut changed = file.clone();
        if let Some(b) = changed.get_mut(file.len() / 2) {
            *b ^= 1;
        }
        assert_eq!(read_p12(&changed, password), Err(Error::Password), "{name}: changed since it was written");
        // Cut short, it isn't one.
        assert_eq!(read_p12(file.get(..file.len() / 2).unwrap(), password), Err(Error::NotCertificateFile), "{name}");
    }
    assert_eq!(Error::Password.to_string(), "the password is wrong");
}

const BODY: &[u8] = b"Content-Type: text/plain; charset=utf-8\r\n\r\nHello Riley, the proof is approved.\r\n";

#[test]
fn every_kind_of_p12_opens_and_a_wrong_password_doesnt() {
    for name in ["riley-aes.p12", "riley-legacy.p12", "riley-3des.p12", "zoe-aes.p12"] {
        let id = read_p12(&data(name), "secret").unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(id.chain.len(), 2, "{name}: the certificate and its CA");
        let me = cert::info(id.certificate()).unwrap();
        assert!(me.emails.iter().any(|e| e.ends_with(".example")), "{name}: the certificate with the key comes first");
        assert_eq!(cert::info(&id.chain[1]).unwrap().name, "MailCraft Test CA");
    }
    assert_eq!(read_p12(&data("riley-aes.p12"), "wrong"), Err(Error::Password));
    assert_eq!(read_p12(b"not a p12", "secret"), Err(Error::NotCertificateFile), "said as what it is, not as a wrong password");
    let info = cert::info(riley().certificate()).unwrap();
    assert_eq!(
        (info.name.as_str(), info.emails.as_slice(), info.issuer.as_str(), info.key.as_str()),
        ("riley", &["riley@studio.example".to_string()][..], "MailCraft Test CA", "RSA 2048")
    );
    assert!(info.not_before < NOW && NOW < info.not_after);
    assert_eq!(info.fingerprint.len(), 32 * 3 - 1);
    assert!(format!("{:?}", riley()).contains("<redacted>"), "the key never prints");
}

#[test]
fn openssl_signatures_verify() {
    for name in ["sig-rsa.der", "sig-rsa-ber.der", "sig-pss.der", "sig-ec.der"] {
        let (signed, content) = verify(&data(name), Some(BODY)).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(content, BODY);
        let who = cert::info(&signed.signer).unwrap();
        assert!(who.name == "ana" || who.name == "zoe", "{name}: {}", who.name);
        assert!(signed.signed_at.is_some_and(|t| (t - NOW).abs() < 86_400 * 30), "{name}");
        assert!(!signed.weak);
        assert!(signed.reads.contains(&oids::AES256_CBC), "{name}: OpenSSL says what it reads");
    }
    // The content carried in the signature comes out.
    let (signed, content) = verify(&data("opaque-ber.der"), None).unwrap();
    assert_eq!(content, BODY);
    assert_eq!(carried(&data("opaque-ber.der")).unwrap(), BODY);
    assert_eq!(cert::info(&signed.signer).unwrap().name, "ana");
}

#[test]
fn a_changed_message_or_signature_is_caught() {
    let changed = String::from_utf8_lossy(BODY).replace("approved", "rejected").into_bytes();
    assert_eq!(verify(&data("sig-rsa.der"), Some(&changed)).err(), Some(Error::BadSignature));
    // One byte of the signature itself changed.
    let mut sig = data("sig-rsa.der");
    let at = sig.len() - 10;
    sig[at] ^= 1;
    assert_eq!(verify(&sig, Some(BODY)).err(), Some(Error::BadSignature));
    // Not a signature at all, or cut short: an error, never a panic.
    let whole = data("sig-rsa.der");
    for cut in [0, 1, 10, whole.len() / 2, whole.len() - 1] {
        assert!(verify(&whole[..cut], Some(BODY)).is_err());
    }
    assert!(verify(&data("enc-cbc.der"), Some(BODY)).is_err());
}

#[test]
fn trust_needs_a_root_the_address_and_dates() {
    let ca = pem("ca.pem");
    let (signed, _) = verify(&data("sig-rsa.der"), Some(BODY)).unwrap();
    assert_eq!(cert::check(&signed.signer, &signed.certs, std::slice::from_ref(&ca), NOW, "ana@posteo.example").unwrap(), vec![]);
    assert_eq!(
        cert::check(&signed.signer, &signed.certs, std::slice::from_ref(&ca), NOW, "ANA@posteo.example ").unwrap(),
        vec![],
        "addresses compare without case"
    );
    // Unknown root, another sender, out of date.
    assert_eq!(cert::check(&signed.signer, &signed.certs, &[], NOW, "ana@posteo.example").unwrap(), vec![Problem::UnknownIssuer]);
    assert_eq!(
        cert::check(&signed.signer, &signed.certs, std::slice::from_ref(&ca), NOW, "boss@posteo.example").unwrap(),
        vec![Problem::OtherAddress(vec!["ana@posteo.example".into()])]
    );
    let later = NOW + 86_400 * 900;
    assert!(matches!(
        cert::check(&signed.signer, &signed.certs, std::slice::from_ref(&ca), later, "ana@posteo.example").unwrap().as_slice(),
        [Problem::Expired(_)]
    ));
    // A chain that doesn't reach the root: a certificate signed by a CA with the same name, but
    // another key, isn't trusted (the signature on it is checked, not only the name).
    let zoe_ca_pretender = pem("zoe.pem");
    assert_eq!(
        cert::check(&signed.signer, &[], std::slice::from_ref(&zoe_ca_pretender), NOW, "ana@posteo.example").unwrap(),
        vec![Problem::UnknownIssuer]
    );
}

#[test]
fn openssl_encryption_decrypts() {
    let id = riley();
    // AES-GCM of each key size: 256 (`enc-gcm`), 128 and 192.
    for (name, authenticated) in [
        ("enc-cbc.der", false),
        ("enc-cbc-ber.der", false),
        ("enc-3des.der", false),
        ("enc-gcm.der", true),
        ("enc-gcm128.der", true),
        ("enc-gcm192.der", true),
        ("enc-oaep.der", false),
    ] {
        let (plain, auth) = decrypt(&data(name), &id, &mut rng()).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(plain, BODY, "{name}");
        assert_eq!(auth, authenticated, "{name}");
    }
    // The same with the legacy and 3DES files' key.
    for p12 in ["riley-legacy.p12", "riley-3des.p12"] {
        let id = read_p12(&data(p12), "secret").unwrap();
        assert_eq!(decrypt(&data("enc-cbc.der"), &id, &mut rng()).unwrap().0, BODY);
    }
    // Mail for someone else.
    assert_eq!(decrypt(&data("enc-not-mine.der"), &id, &mut rng()), Err(Error::Decrypt));
}

/// OpenSSL's mail to Zoe's EC key: its default SHA-1 key derivation, SHA-256, and one sent to an
/// RSA and an EC recipient at once (AES-GCM).
#[test]
fn openssl_mail_to_an_ec_key_decrypts() {
    let zoe = read_p12(&data("zoe-aes.p12"), "secret").unwrap();
    for name in ["enc-ec-sha1.der", "enc-ec-sha256.der", "enc-mixed.der"] {
        assert_eq!(decrypt(&data(name), &zoe, &mut rng()).unwrap_or_else(|e| panic!("{name}: {e}")).0, BODY, "{name}");
    }
    assert_eq!(decrypt(&data("enc-mixed.der"), &riley(), &mut rng()).unwrap().0, BODY, "and the RSA recipient");
    // Every byte changed in turn: an error or (CBC) other bytes, never a panic.
    let m = data("enc-ec-sha256.der");
    for at in 0..m.len() {
        let mut b = m.clone();
        b[at] ^= 0x41;
        let _ = decrypt(&b, &zoe, &mut rng());
    }
}

#[test]
fn changed_encrypted_mail_fails_closed() {
    let id = riley();
    // GCM: any byte changed fails the tag.
    for name in ["enc-gcm.der", "enc-gcm128.der", "enc-gcm192.der"] {
        let gcm = data(name);
        for at in [gcm.len() - 1, gcm.len() - 20, gcm.len() - 40] {
            let mut m = gcm.clone();
            m[at] ^= 0x55;
            assert!(decrypt(&m, &id, &mut rng()).is_err(), "{name}: byte {at}");
        }
    }
    // Every cut and every flipped byte of a CBC message: an error or (CBC can't tell) other
    // bytes, never a panic.
    let cbc = data("enc-cbc.der");
    for at in 0..cbc.len() {
        let mut m = cbc.clone();
        m[at] ^= 0xFF;
        let _ = decrypt(&m, &id, &mut rng());
        let _ = decrypt(&cbc[..at], &id, &mut rng());
    }
}

#[test]
fn what_mailcraft_signs_verifies() {
    for p12 in ["riley-aes.p12", "zoe-aes.p12"] {
        let id = read_p12(&data(p12), "secret").unwrap();
        let sig = sign(BODY, &id, NOW, &mut rng()).unwrap();
        let (signed, content) = verify(&sig, Some(BODY)).unwrap_or_else(|e| panic!("{p12}: {e}"));
        assert_eq!(content, BODY);
        assert_eq!(signed.signer, id.chain[0]);
        assert_eq!(signed.signed_at, Some(NOW));
        assert!(signed.reads_gcm(), "it says it reads GCM");
        assert_eq!(signed.certs, vec![id.chain[0].clone()], "the root isn't sent");
        let ca = pem("ca.pem");
        let email = cert::info(&id.chain[0]).unwrap().emails[0].clone();
        assert_eq!(cert::check(&signed.signer, &signed.certs, &[ca], NOW, &email).unwrap(), vec![]);
        assert_eq!(verify(&sig, Some(b"other")).err(), Some(Error::BadSignature));
    }
}

#[test]
fn what_mailcraft_encrypts_decrypts_for_each_recipient() {
    let riley = riley();
    let ana = pem("ana.pem");
    for cipher in [Cipher::Aes256Cbc, Cipher::Aes256Gcm] {
        let sealed = encrypt(BODY, &[riley.chain[0].clone(), ana.clone()], cipher, &mut rng()).unwrap();
        let (plain, auth) = decrypt(&sealed, &riley, &mut rng()).unwrap();
        assert_eq!(plain, BODY);
        assert_eq!(auth, cipher == Cipher::Aes256Gcm);
        // Encrypted again, it isn't the same bytes (a fresh key and IV each time).
        assert_ne!(sealed, encrypt(BODY, &[riley.chain[0].clone()], cipher, &mut rng()).unwrap());
    }
    // EC and RSA recipients together: each reads it.
    let zoe = read_p12(&data("zoe-aes.p12"), "secret").unwrap();
    for cipher in [Cipher::Aes256Cbc, Cipher::Aes256Gcm] {
        let sealed = encrypt(BODY, &[riley.chain[0].clone(), zoe.chain[0].clone()], cipher, &mut rng()).unwrap();
        assert_eq!(decrypt(&sealed, &zoe, &mut rng()).unwrap().0, BODY, "Zoe (EC)");
        assert_eq!(decrypt(&sealed, &riley, &mut rng()).unwrap().0, BODY, "Riley (RSA)");
    }
    // Ana can't read what wasn't for her (an EC copy for Zoe is no copy for Ana).
    let for_zoe = encrypt(BODY, &[zoe.chain[0].clone()], Cipher::Aes256Cbc, &mut rng()).unwrap();
    assert_eq!(decrypt(&for_zoe, &riley, &mut rng()), Err(Error::Decrypt));
    assert!(encrypt(BODY, &[], Cipher::Aes256Cbc, &mut rng()).is_err());
}

/// A certificate named twice (the sender writing to themselves, their own copy added too) is one
/// recipient: one copy of the content key for it, RSA or EC.
#[test]
fn a_recipient_named_twice_is_one() {
    use cms::enveloped_data::EnvelopedData;
    use der::Decode as _;
    let (riley, zoe) = (pem("riley.pem"), pem("zoe.pem"));
    let infos = |recipients: &[Vec<u8>]| {
        let sealed = encrypt(BODY, recipients, Cipher::Aes256Cbc, &mut rng()).unwrap();
        let ci = cms::content_info::ContentInfo::from_der(&sealed).unwrap();
        ci.content.decode_as::<EnvelopedData>().unwrap().recip_infos.0.len()
    };
    assert_eq!(infos(&[riley.clone(), riley.clone()]), 1);
    assert_eq!(infos(&[zoe.clone(), riley.clone(), zoe.clone(), riley.clone()]), 2);
    let id = self::riley();
    let sealed = encrypt(BODY, &[riley.clone(), riley], Cipher::Aes256Gcm, &mut rng()).unwrap();
    assert_eq!(decrypt(&sealed, &id, &mut rng()).unwrap().0, BODY);
}

/// Which of several certificates a message is for, read from its recipient infos (RSA's and
/// EC's, OpenSSL's and MailCraft's, BER too), without a key.
#[test]
fn which_certificate_mail_is_for() {
    let (riley, zoe, ana) = (pem("riley.pem"), pem("zoe.pem"), pem("ana.pem"));
    for name in ["enc-cbc.der", "enc-cbc-ber.der", "enc-gcm.der", "enc-oaep.der", "enc-3des.der"] {
        assert!(addressed_to(&data(name), &riley), "{name}");
        assert!(!addressed_to(&data(name), &zoe), "{name}");
    }
    assert!(!addressed_to(&data("enc-not-mine.der"), &riley));
    assert!(addressed_to(&data("enc-ec-sha256.der"), &zoe) && !addressed_to(&data("enc-ec-sha256.der"), &riley));
    assert!(addressed_to(&data("enc-mixed.der"), &zoe) && addressed_to(&data("enc-mixed.der"), &riley));
    for cipher in [Cipher::Aes256Cbc, Cipher::Aes256Gcm] {
        let sealed = encrypt(BODY, &[zoe.clone(), ana.clone()], cipher, &mut rng()).unwrap();
        assert!(addressed_to(&sealed, &zoe) && addressed_to(&sealed, &ana) && !addressed_to(&sealed, &riley));
    }
    // Junk, a signature, a certificate that isn't one: for no one.
    assert!(!addressed_to(b"junk", &riley));
    assert!(!addressed_to(&data("sig-rsa.der"), &riley));
    assert!(!addressed_to(&data("enc-cbc.der"), b"not a certificate"));
}

#[test]
fn junk_never_panics() {
    let id = riley();
    let mut junk = vec![0x30, 0x80];
    for i in 0..300u32 {
        junk.push((i * 37 % 251) as u8);
        let _ = verify(&junk, Some(BODY));
        let _ = verify(&junk, None);
        let _ = decrypt(&junk, &id, &mut rng());
        let _ = addressed_to(&junk, &id.chain[0]);
        let _ = cert::info(&junk);
        let _ = read_p12(&junk, "secret");
    }
}

/// A certificate of `tbs` exactly as given, signed by `key` (P-256) over those bytes.
fn issued_over(tbs: &[u8], key: &p256::ecdsa::SigningKey) -> Vec<u8> {
    use signature::Signer as _;
    let signature: p256::ecdsa::Signature = key.sign(tbs);
    with_signature(tbs, &der::asn1::BitString::from_bytes(signature.to_der().as_bytes()).unwrap())
}

/// A certificate of `tbs` exactly as given, with `signature` (ECDSA with SHA-256).
fn with_signature(tbs: &[u8], signature: &der::asn1::BitString) -> Vec<u8> {
    use der::Encode as _;
    let alg = spki::AlgorithmIdentifierOwned { oid: oids::ECDSA_WITH_SHA256, parameters: None };
    let inner = [tbs.to_vec(), alg.to_der().unwrap(), signature.to_der().unwrap()].concat();
    der::Any::new(der::Tag::Sequence, inner).unwrap().to_der().unwrap()
}

/// The signature `sig` carrying `certs`, written as they are, in that order.
fn carrying(sig: &[u8], certs: &[Vec<u8>]) -> Vec<u8> {
    use der::{Decode as _, Encode as _};
    let ci = ::cms::content_info::ContentInfo::from_der(sig).unwrap();
    let sd: ::cms::signed_data::SignedData = ci.content.decode_as().unwrap();
    let set = der::Any::new(der::Tag::ContextSpecific { constructed: true, number: der::TagNumber::N0 }, certs.concat()).unwrap();
    let inner = [
        sd.version.to_der().unwrap(),
        sd.digest_algorithms.to_der().unwrap(),
        sd.encap_content_info.to_der().unwrap(),
        set.to_der().unwrap(),
        sd.signer_infos.to_der().unwrap(),
    ]
    .concat();
    let content = der::Any::new(der::Tag::Sequence, inner).unwrap();
    ::cms::content_info::ContentInfo { content_type: ci.content_type, content }.to_der().unwrap()
}

/// Riley's certificate issued again by a CA of its own (P-256), its extensions written with
/// `critical FALSE` spelt out: DER leaves a DEFAULT out, and `der` reads it and writes the
/// certificate again without it, so the CA's signature holds over it only as it was written.
struct SpeltOut {
    ca_key: p256::ecdsa::SigningKey,
    /// The CA's certificate (DER).
    ca: Vec<u8>,
    /// Riley's certificate, as the CA wrote and signed it.
    as_written: Vec<u8>,
    /// Its `tbsCertificate`, as written.
    tbs_as_written: Vec<u8>,
    /// The same certificate as `der` writes it again: the CA's signature doesn't hold over it.
    written_again: Vec<u8>,
}

fn spelt_out() -> SpeltOut {
    use der::{Decode as _, Encode as _};
    use x509_cert::ext::pkix::BasicConstraints;
    let riley = riley();
    let ca_key = p256::ecdsa::SigningKey::random(&mut rng());
    let ca_alg = spki::AlgorithmIdentifierOwned { oid: oids::ECDSA_WITH_SHA256, parameters: None };
    let ca_name: x509_cert::name::Name = "CN=Spelt Out CA".parse().unwrap();
    let leaf = x509_cert::Certificate::from_der(&riley.chain[0]).unwrap().tbs_certificate;
    let ca_spki = spki::SubjectPublicKeyInfoOwned {
        algorithm: spki::AlgorithmIdentifierOwned { oid: oids::EC_PUBLIC_KEY, parameters: Some(der::Any::encode_from(&oids::P256).unwrap()) },
        subject_public_key: der::asn1::BitString::from_bytes(ca_key.verifying_key().to_encoded_point(false).as_bytes()).unwrap(),
    };
    let basic = BasicConstraints { ca: true, path_len_constraint: None }.to_der().unwrap();
    let ca_tbs = x509_cert::TbsCertificate {
        version: x509_cert::Version::V3,
        serial_number: x509_cert::serial_number::SerialNumber::new(&[1]).unwrap(),
        signature: ca_alg.clone(),
        issuer: ca_name.clone(),
        validity: leaf.validity,
        subject: ca_name.clone(),
        subject_public_key_info: ca_spki,
        issuer_unique_id: None,
        subject_unique_id: None,
        extensions: Some(vec![x509_cert::ext::Extension {
            extn_id: oids::BASIC_CONSTRAINTS,
            critical: true,
            extn_value: der::asn1::OctetString::new(basic).unwrap(),
        }]),
    };
    let ca = issued_over(&ca_tbs.to_der().unwrap(), &ca_key);
    // Riley's, issued by the CA (its authority key identifier, the old CA's, left out).
    let extensions: Vec<_> =
        leaf.extensions.clone().unwrap().into_iter().filter(|e| e.extn_id != der::asn1::ObjectIdentifier::new_unwrap("2.5.29.35")).collect();
    assert!(extensions.iter().any(|e| !e.critical));
    let tbs = x509_cert::TbsCertificate { signature: ca_alg, issuer: ca_name, extensions: None, ..leaf };
    let fields = der::Any::from_der(&tbs.to_der().unwrap()).unwrap().value().to_vec();
    let spelt: Vec<u8> = extensions
        .iter()
        .flat_map(|e| {
            let critical: &[u8] = if e.critical { &[0x01, 0x01, 0xFF] } else { &[0x01, 0x01, 0x00] };
            let inner = [e.extn_id.to_der().unwrap(), critical.to_vec(), e.extn_value.to_der().unwrap()].concat();
            der::Any::new(der::Tag::Sequence, inner).unwrap().to_der().unwrap()
        })
        .collect();
    let spelt = der::Any::new(der::Tag::Sequence, spelt).unwrap().to_der().unwrap();
    let spelt = der::Any::new(der::Tag::ContextSpecific { constructed: true, number: der::TagNumber::N3 }, spelt).unwrap().to_der().unwrap();
    let tbs_as_written = der::Any::new(der::Tag::Sequence, [fields, spelt].concat()).unwrap().to_der().unwrap();
    let as_written = issued_over(&tbs_as_written, &ca_key);
    let written_again = x509_cert::Certificate::from_der(&as_written).unwrap().to_der().unwrap();
    assert_ne!(as_written, written_again, "der writes it again otherwise");
    assert_eq!(x509_cert::Certificate::from_der(&written_again), x509_cert::Certificate::from_der(&as_written));
    SpeltOut { ca_key, ca, as_written, tbs_as_written, written_again }
}

/// An issuer signs its certificate's bytes as it writes them, and a chain is checked over those
/// (RFC 5280 §4.1.1.3, as OpenSSL does). Riley's certificate issued again by a CA, its extensions
/// written with `critical FALSE` spelt out (DER leaves a DEFAULT out; `der` reads it and writes
/// the certificate again without it): its chain holds as it came, and its fingerprint is of
/// those bytes. Checked over the certificate written again, the chain was refused. One changed
/// after it was signed, if only in how it's written, isn't one its issuer signed.
#[test]
fn a_certificate_is_checked_as_it_came() {
    use der::Decode as _;
    use sha2::Digest as _;
    let riley = riley();
    let SpeltOut { ca_key, ca, as_written, tbs_as_written, written_again } = spelt_out();

    // Signed by Riley, with the CA's certificate after it or before.
    let id = Identity { key: riley.key.clone(), chain: vec![written_again.clone()] };
    let sig = sign(BODY, &id, NOW, &mut rng()).unwrap();
    let mut orders = Vec::new();
    for certs in [vec![as_written.clone(), ca.clone()], vec![ca.clone(), as_written.clone()]] {
        let (signed, content) = verify(&carrying(&sig, &certs), Some(BODY)).unwrap();
        assert_eq!(content, BODY);
        assert_eq!(signed.signer, as_written, "the certificate as it came");
        let fingerprint = sha2::Sha256::digest(&as_written).iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(":");
        assert_eq!(cert::info(&signed.signer).unwrap().fingerprint, fingerprint);
        assert_eq!(cert::check(&signed.signer, &signed.certs, std::slice::from_ref(&ca), NOW, "riley@studio.example").unwrap(), vec![]);
        orders.push(signed.certs);
    }
    assert_eq!(orders[0], orders[1], "tried in one order, whichever order they came in");
    assert!(orders[0].contains(&ca) && orders[0].contains(&as_written));

    // The CA's signature over the certificate as DER writes it, the certificate sent spelt out
    // (what was taken as its issuer's when it was checked written again).
    assert_eq!(cert::signed_part(&as_written), Some(&tbs_as_written[..]));
    let der_tbs = cert::signed_part(&written_again).unwrap();
    assert_ne!(der_tbs, &tbs_as_written[..]);
    let over_der = x509_cert::Certificate::from_der(&issued_over(der_tbs, &ca_key)).unwrap().signature;
    let mut changed = with_signature(&tbs_as_written, &over_der);
    let (signed, _) = verify(&carrying(&sig, &[changed.clone(), ca.clone()]), Some(BODY)).unwrap();
    assert_eq!(signed.signer, changed);
    assert_eq!(
        cert::check(&signed.signer, &signed.certs, std::slice::from_ref(&ca), NOW, "riley@studio.example").unwrap(),
        vec![Problem::UnknownIssuer]
    );
    // And the other way: a certificate that holds as written, one byte of its signature changed.
    let at = as_written.len() - 3;
    changed = as_written.clone();
    changed[at] ^= 1;
    let (signed, _) = verify(&carrying(&sig, &[changed, ca.clone()]), Some(BODY)).unwrap();
    assert_eq!(
        cert::check(&signed.signer, &signed.certs, std::slice::from_ref(&ca), NOW, "riley@studio.example").unwrap(),
        vec![Problem::UnknownIssuer]
    );
}

/// What MailCraft signs carries the signer's chain as it was kept, as its issuer wrote it: Riley's
/// certificate written again (as it was sent) didn't hold for its CA, here or in OpenSSL.
#[test]
fn a_certificate_is_sent_as_it_was_kept() {
    let riley = riley();
    let SpeltOut { ca, as_written, written_again, .. } = spelt_out();
    let id = Identity { key: riley.key.clone(), chain: vec![as_written.clone(), ca.clone()] };
    let sig = sign(BODY, &id, NOW, &mut rng()).unwrap();
    assert!(sig.windows(as_written.len()).any(|w| w == as_written), "sent as it was kept");
    assert!(!sig.windows(written_again.len()).any(|w| w == written_again));
    let (signed, _) = verify(&sig, Some(BODY)).unwrap();
    assert_eq!(signed.signer, as_written);
    assert_eq!(signed.certs, vec![as_written.clone()], "the root isn't sent");
    assert_eq!(cert::check(&signed.signer, &signed.certs, std::slice::from_ref(&ca), NOW, "riley@studio.example").unwrap(), vec![]);
    // Several sent: in DER's order for a SET OF, whatever order the chain has them in; one twice
    // can't be written.
    let (ana, zoe) = (pem("ana.pem"), pem("zoe.pem"));
    let sign_chain = |chain: Vec<Vec<u8>>| sign(BODY, &Identity { key: riley.key.clone(), chain }, NOW, &mut rng());
    let one = sign_chain(vec![as_written.clone(), ana.clone(), zoe.clone()]).unwrap();
    assert_eq!(sign_chain(vec![as_written.clone(), zoe.clone(), ana.clone()]).unwrap(), one, "RSA signs alike each time");
    let mut sent = verify(&one, Some(BODY)).unwrap().0.certs;
    assert_eq!(sent.len(), 3);
    let in_order = sent.clone();
    sent.sort();
    assert_eq!(in_order, sent);
    assert!(sign_chain(vec![as_written.clone(), ana.clone(), ana]).is_err());
}

/// How long a signature takes to check, and its certificate's chain (`--release`, by hand).
#[test]
#[ignore = "a measurement: run in release"]
fn measure_verify() {
    let ca = pem("ca.pem");
    for name in ["sig-rsa.der", "sig-ec.der"] {
        let sig = data(name);
        let n = 2000;
        let t = std::time::Instant::now();
        for _ in 0..n {
            let (signed, _) = verify(&sig, Some(BODY)).unwrap();
            assert!(cert::check(&signed.signer, &signed.certs, std::slice::from_ref(&ca), NOW, "").is_ok());
        }
        println!("MEASURE verify and check {name}: {:.1} us", t.elapsed().as_secs_f64() * 1e6 / f64::from(n));
    }
}

/// Writes MailCraft's signatures and encryption to `$MAILCRAFT_SMIME_OUT`, for OpenSSL to check
/// (`openssl cms -verify` / `-decrypt`): run by hand.
#[test]
#[ignore = "writes files for OpenSSL to check, where MAILCRAFT_SMIME_OUT says"]
fn write_for_openssl() {
    let Some(dir) = std::env::var_os("MAILCRAFT_SMIME_OUT") else { return };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap();
    let riley = riley();
    let zoe = read_p12(&data("zoe-aes.p12"), "secret").unwrap();
    std::fs::write(dir.join("mc-body.txt"), BODY).unwrap();
    std::fs::write(dir.join("mc-sig-rsa.der"), sign(BODY, &riley, NOW, &mut rng()).unwrap()).unwrap();
    std::fs::write(dir.join("mc-sig-ec.der"), sign(BODY, &zoe, NOW, &mut rng()).unwrap()).unwrap();
    // Riley's certificate with `critical FALSE` spelt out, and its CA (`mc-spelt-ca.der`).
    let spelt = spelt_out();
    let id = Identity { key: riley.key.clone(), chain: vec![spelt.as_written, spelt.ca.clone()] };
    std::fs::write(dir.join("mc-sig-spelt.der"), sign(BODY, &id, NOW, &mut rng()).unwrap()).unwrap();
    std::fs::write(dir.join("mc-spelt-ca.der"), spelt.ca).unwrap();
    std::fs::write(dir.join("mc-enc-cbc.der"), encrypt(BODY, &[riley.chain[0].clone()], Cipher::Aes256Cbc, &mut rng()).unwrap()).unwrap();
    std::fs::write(dir.join("mc-enc-gcm.der"), encrypt(BODY, &[riley.chain[0].clone()], Cipher::Aes256Gcm, &mut rng()).unwrap()).unwrap();
    std::fs::write(dir.join("mc-enc-ec.der"), encrypt(BODY, &[zoe.chain[0].clone()], Cipher::Aes256Cbc, &mut rng()).unwrap()).unwrap();
    std::fs::write(
        dir.join("mc-enc-ec-gcm.der"),
        encrypt(BODY, &[zoe.chain[0].clone(), riley.chain[0].clone()], Cipher::Aes256Gcm, &mut rng()).unwrap(),
    )
    .unwrap();
}

/// How long a signer's chain takes to check among a system's worth of roots (150, the CA last),
/// as the engine asks for each signed message shown (`--release`, by hand).
#[test]
#[ignore = "a measurement: run in release"]
fn measure_check_among_roots() {
    let ca = pem("ca.pem");
    let others = [pem("ana.pem"), pem("zoe.pem"), pem("rev-kim.pem"), pem("rev-lee.pem"), pem("riley.pem")];
    let mut roots: Vec<Vec<u8>> = others.iter().cycle().take(149).cloned().collect();
    roots.push(ca);
    let (signed, _) = verify(&data("sig-rsa.der"), Some(BODY)).unwrap();
    let n = 2000;
    let t = std::time::Instant::now();
    for _ in 0..n {
        assert_eq!(cert::check(&signed.signer, &signed.certs, &roots, NOW, "ana@posteo.example").unwrap(), vec![]);
    }
    println!("MEASURE check among {} roots: {:.1} us", roots.len(), t.elapsed().as_secs_f64() * 1e6 / f64::from(n));
}

/// How long a `.p12`'s key takes to be matched to its certificate: as files have it (the key's
/// first) and with it after five others (`--release`, by hand).
#[test]
#[ignore = "a measurement: run in release"]
fn measure_p12_key_match() {
    let riley = riley();
    let others: Vec<Vec<u8>> = ["ca.pem", "ana.pem", "zoe.pem", "rev-kim.pem", "rev-lee.pem"].into_iter().map(pem).collect();
    for (what, certs) in
        [("first", [vec![riley.chain[0].clone()], others.clone()].concat()), ("last", [others, vec![riley.chain[0].clone()]].concat())]
    {
        let n = 200;
        let t = std::time::Instant::now();
        for _ in 0..n {
            assert_eq!(p12::leaf_of(&riley.key, &certs), Some(&riley.chain[0]));
        }
        println!("MEASURE key matched, its certificate {what} of 6: {:.1} us", t.elapsed().as_secs_f64() * 1e6 / f64::from(n));
    }
    let file = data("riley-aes.p12");
    let n = 20;
    let t = std::time::Instant::now();
    for _ in 0..n {
        read_p12(&file, "secret").unwrap();
    }
    println!("MEASURE read_p12 riley-aes.p12: {:.1} us", t.elapsed().as_secs_f64() * 1e6 / f64::from(n));
}
