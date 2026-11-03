//! Trying to break it: what decompresses to too much, packets nested without end, a user ID
//! someone else certified, and keys by the hundred.

use pgp::types::CompressionAlgorithm;

use super::*;

/// A new-format packet: `tag`, a length, the body.
fn packet(tag: u8, body: &[u8]) -> Vec<u8> {
    let mut out = vec![0xC0 | tag, 0xFF];
    out.extend_from_slice(&u32::try_from(body.len()).unwrap().to_be_bytes());
    out.extend_from_slice(body);
    out
}

/// A literal data packet holding `data`.
fn literal(data: &[u8]) -> Vec<u8> {
    let mut body = vec![b'b', 0, 0, 0, 0, 0];
    body.extend_from_slice(data);
    packet(11, &body)
}

#[test]
fn what_decompresses_to_too_much_is_refused() {
    let ana = unlocked("Ana <ana@posteo.example>");
    let big = vec![0u8; MAX + 1024];
    let mut b = pgp::composed::MessageBuilder::from_bytes("", big).seipd_v1(rand_core::OsRng, pgp::crypto::sym::SymmetricKeyAlgorithm::AES256);
    let key: pgp::composed::SignedPublicKey = pgp::composed::Deserializable::from_bytes(&ana.public[..]).unwrap();
    b.encrypt_to_key(rand_core::OsRng, &key.public_subkeys[0]).unwrap();
    b.compression(CompressionAlgorithm::ZLIB);
    let sealed = b.to_vec(rand_core::OsRng).unwrap();
    assert!(sealed.len() < 1 << 20, "a small message: {}", sealed.len());
    assert_eq!(decrypt(&sealed, &[&ana.secret], &[]).unwrap_err(), Error::TooBig);
}

#[test]
fn packets_nested_without_end_dont_overflow() {
    // Thousands of compressed packets ("uncompressed" algorithm) around a literal one.
    let mut m = literal(b"deep");
    for _ in 0..5000 {
        let mut body = vec![0u8];
        body.extend_from_slice(&m);
        m = packet(8, &body);
    }
    assert!(open(&m, &[]).is_err());
    let _ = is_encrypted(&m);
    let _ = recipients(&m);
    // A few levels read.
    let mut m = literal(b"shallow");
    for _ in 0..2 {
        let mut body = vec![0u8];
        body.extend_from_slice(&m);
        m = packet(8, &body);
    }
    assert_eq!(open(&m, &[]).unwrap().data, b"shallow");
    // Thousands of one-pass signature packets, none ended.
    let ops = packet(4, &[3, 0, 8, 22, 1, 2, 3, 4, 5, 6, 7, 8, 0]);
    let many: Vec<u8> = std::iter::repeat_n(ops, 5000).flatten().chain(literal(b"x")).collect();
    assert!(open(&many, &[]).is_err() || open(&many, &[]).is_ok_and(|d| d.check.is_some()));
}

/// Eve's user ID, with her own signature, put on Ana's key: not Ana's, so mail to Eve's
/// address isn't encrypted to Ana's key, nor Ana's signatures taken as Eve's.
#[test]
fn a_user_id_someone_else_certified_doesnt_count() {
    let (ana, eve) = (unlocked("Ana <ana@posteo.example>"), unlocked("Eve <eve@posteo.example>"));
    let mut a: pgp::composed::SignedPublicKey = pgp::composed::Deserializable::from_bytes(&ana.public[..]).unwrap();
    let e: pgp::composed::SignedPublicKey = pgp::composed::Deserializable::from_bytes(&eve.public[..]).unwrap();
    a.details.users.extend(e.details.users.clone());
    let forged = pgp::ser::Serialize::to_bytes(&a).unwrap();
    let i = info(&forged).unwrap();
    assert_eq!(i.emails, ["ana@posteo.example"]);
    assert!(!i.is_for("eve@posteo.example"));
}

#[test]
fn a_file_of_many_keys_is_read_so_far() {
    let ana = unlocked("Ana <ana@posteo.example>");
    let many: Vec<u8> = std::iter::repeat_n(ana.public.clone(), KEYS_MAX * 3).flatten().collect();
    assert_eq!(public_keys(&many).unwrap().len(), KEYS_MAX);
    // Armor with mixed line ends and trailing spaces around it still reads.
    let p: pgp::composed::SignedPublicKey = pgp::composed::Deserializable::from_bytes(&ana.public[..]).unwrap();
    let armored = p.to_armored_string(Default::default()).unwrap().replace('\n', "\r\n");
    assert_eq!(public_keys(format!("  \r\n{armored}   \n").as_bytes()).unwrap().len(), 1);
}
