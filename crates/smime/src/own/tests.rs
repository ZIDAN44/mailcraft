use super::*;

/// The fixtures' time: Riley's certificate was issued on 2026-10-24, last year's (riley-2025) on
/// 2025-10-01.
const NOW: i64 = 1_792_900_000;

fn data(name: &str) -> Vec<u8> {
    std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata").join(name)).unwrap()
}

fn entry(name: &str) -> Entry {
    let p12 = data(name);
    Entry { cert: read_p12(&p12, "secret").unwrap().chain[0].clone(), p12 }
}

/// A renewal in either order: the newer is in use, the older kept for reading; the same one
/// again changes nothing; use-older swaps them; forgetting one leaves the other.
#[test]
fn a_renewal_keeps_the_old_certificate() {
    let (old, new) = (entry("riley-2025.p12"), entry("riley-aes.p12"));
    for order in [[&old, &new], [&new, &old]] {
        let mut own = Own::default();
        assert_eq!(own.take(order[0].clone(), NOW).unwrap(), Taken::InUse);
        let second = own.take(order[1].clone(), NOW).unwrap();
        assert_eq!(second, if *order[1] == old { Taken::ForReading } else { Taken::InUse });
        assert_eq!(own.current.as_ref(), Some(&new));
        assert_eq!(own.older, vec![old.clone()]);
        assert_eq!(own.take(old.clone(), NOW).unwrap(), Taken::ForReading, "the old one again");
        assert_eq!(own.take(new.clone(), NOW).unwrap(), Taken::InUse, "the new one again");
        assert_eq!((own.current.as_ref(), own.older.len()), (Some(&new), 1));
        own.use_older(&old.fingerprint().to_lowercase()).unwrap();
        assert_eq!((own.current.as_ref(), own.older_certs()), (Some(&old), vec![new.cert.clone()]));
        assert!(own.use_older("00:11").is_err());
        assert!(own.forget_older(&new.fingerprint()) && !own.forget_older(&new.fingerprint()));
        assert_eq!(own.current.as_ref(), Some(&old));
    }
}

/// One password opens every file it's the password of; each message finds the key it was sent
/// to, and mail for a certificate not kept finds none.
#[test]
fn one_password_and_the_key_each_message_names() {
    let mut own = Own::default();
    own.take(entry("riley-aes.p12"), NOW).unwrap();
    own.take(entry("riley-2025.p12"), NOW).unwrap();
    assert_eq!(own.unlock("secret").len(), 2, "both share their password");
    assert!(own.unlock("wrong").is_empty());
    let mut rng = rand_core::OsRng;
    let body = b"Content-Type: text/plain\r\n\r\nLast year.\r\n";
    let old = entry("riley-2025.p12");
    let to_old = crate::encrypt(body, std::slice::from_ref(&old.cert), crate::Cipher::Aes256Gcm, &mut rng).unwrap();
    let found = own.for_message(&to_old).unwrap();
    assert_eq!(found.cert, old.cert);
    let id = read_p12(&found.p12, "secret").unwrap();
    assert_eq!(crate::decrypt(&to_old, &id, &mut rng).unwrap().0, body);
    let to_zoe = crate::encrypt(body, &[entry("zoe-aes.p12").cert], crate::Cipher::Aes256Cbc, &mut rng).unwrap();
    assert!(own.for_message(&to_zoe).is_none());
    assert!(own.for_message(b"junk").is_none());
}

/// At most so many older ones (here 2): one more is refused and nothing changes, but a renewal
/// on a full list still fits (the one leaving makes room). A file that isn't a certificate is
/// refused.
#[test]
fn the_cap_counts_what_stays() {
    let pem = |n: &str| Entry { cert: der::pem::decode_vec(&data(n)).unwrap().1, p12: Vec::new() };
    let full = Own { current: Some(entry("riley-aes.p12")), older: vec![pem("ana.pem"), pem("zoe.pem")] };
    let mut own = full.clone();
    assert!(own.take_within(entry("riley-2025.p12"), NOW, 2).is_err());
    assert_eq!(own, full, "nothing dropped unasked");
    // A renewal of Ana's (newer than Riley's in use, as far as the ranking goes) on a full list:
    // here, last year's Riley in use and this year's arriving.
    let mut own = Own { current: Some(entry("riley-2025.p12")), older: vec![pem("ana.pem"), entry("riley-aes.p12")] };
    assert_eq!(own.take_within(entry("riley-aes.p12"), NOW, 2).unwrap(), Taken::InUse);
    assert_eq!(own.current.as_ref().map(|e| e.cert.clone()), Some(entry("riley-aes.p12").cert));
    assert_eq!(own.older.len(), 2);
    assert!(own.take(Entry { cert: b"not a certificate".to_vec(), p12: Vec::new() }, NOW).is_err());
}
