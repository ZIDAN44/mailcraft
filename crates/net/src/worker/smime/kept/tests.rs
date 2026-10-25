use super::*;

/// The fixtures' time: Riley's certificate was issued on 2026-10-24, the one before it
/// (`riley-2025.p12`) on 2025-10-01, running to 2026-11-01.
const NOW: i64 = 1_792_900_000;

fn testdata(name: &str) -> Vec<u8> {
    std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../smime/testdata").join(name)).unwrap()
}

fn p12(name: &str) -> Me {
    Me::from(smime::read_p12(&testdata(name), "secret").unwrap())
}

fn account() -> AccountConfig {
    AccountConfig { id: "a1".into(), ..Default::default() }
}

#[test]
fn the_key_is_kept_in_the_keychain_and_comes_back() {
    let creds = MemoryCredentials::default();
    let account = account();
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
    forget_wrong_pin(&creds, &account, None, &Err::<(), _>("the card's module can't be loaded".to_string()));
    assert!(kept(&creds, &account).unwrap() == card);
    forget_wrong_pin(&creds, &account, None, &Err::<(), _>(PIN_WRONG.to_string()));
    let left = kept(&creds, &account).unwrap();
    assert!(matches!(&left.key, MyKey::Held { pin: None, .. }) && left.chain == card.chain);
    creds.delete_secret(&account, SECRET);
    assert!(kept(&creds, &account).is_err());
    assert_eq!(unhex("0aFf"), Some(vec![0x0a, 0xff]));
    assert_eq!(unhex("0"), None);
    assert_eq!(unhex("zz"), None);
}

/// Issue #6: a certificate renewed. Whichever order they're imported in, the newer signs and
/// encrypts and the older is kept to read mail encrypted to it; each message opens with the key
/// it was sent to, and mail for neither says what's missing.
#[test]
fn a_renewed_certificate_keeps_the_old_one_for_reading() {
    let (old, new) = (p12("riley-2025.p12"), p12("riley-aes.p12"));
    let (old_cert, new_cert) = (old.certificate().to_vec(), new.certificate().to_vec());
    for order in [[&old, &new], [&new, &old]] {
        let creds = MemoryCredentials::default();
        let a = account();
        let (chain, older) = take(&creds, &a, order[0], NOW).unwrap();
        assert_eq!(chain.first(), Some(&order[0].certificate().to_vec()));
        assert!(older.is_empty());
        let (chain, older) = take(&creds, &a, order[1], NOW).unwrap();
        assert_eq!(chain.first(), Some(&new_cert), "the newer signs and encrypts");
        assert_eq!(older, vec![old_cert.clone()], "the older is kept for reading");
        // The same one again changes nothing.
        let again = take(&creds, &a, order[1], NOW).unwrap();
        assert_eq!((again.0.first(), again.1.len()), (Some(&new_cert), 1));

        let mut rng = rand_core::OsRng;
        let body = b"Content-Type: text/plain\r\n\r\nSent before the renewal.\r\n";
        let to_old = smime::encrypt(body, std::slice::from_ref(&old_cert), smime::Cipher::Aes256Gcm, &mut rng).unwrap();
        let to_new = smime::encrypt(body, std::slice::from_ref(&new_cert), smime::Cipher::Aes256Cbc, &mut rng).unwrap();
        assert_eq!(decrypt(&creds, &a, &to_old).unwrap(), (body.to_vec(), true));
        assert_eq!(decrypt(&creds, &a, &to_new).unwrap(), (body.to_vec(), false));
        let to_ana = smime::encrypt(body, &[testdata_pem("ana.pem")], smime::Cipher::Aes256Cbc, &mut rng).unwrap();
        assert!(decrypt(&creds, &a, &to_ana).unwrap_err().contains("doesn't have"));

        // The older one made the one in use again: the newer is kept for reading; both still open.
        let (chain, older) = use_older(&creds, &a, &fingerprint(&old_cert)).unwrap();
        assert_eq!((chain.first(), older), (Some(&old_cert), vec![new_cert.clone()]));
        assert!(decrypt(&creds, &a, &to_new).is_ok() && decrypt(&creds, &a, &to_old).is_ok());
        assert!(use_older(&creds, &a, "00:11").is_err(), "one that isn't kept");
        // Forgetting the one in use leaves the older readable; forgetting that one too, nothing is.
        creds.delete_secret(&a, SECRET);
        assert!(decrypt(&creds, &a, &to_new).is_ok());
        assert!(decrypt(&creds, &a, &to_old).unwrap_err().contains("import that one"));
        forget_older(&creds, &a, &fingerprint(&new_cert));
        assert!(older_list(&creds, &a).is_empty() && creds.get_secret(&a, OLDER).is_none());
        assert!(decrypt(&creds, &a, &to_new).unwrap_err().contains("isn't in the keychain"));
    }
}

/// The account removed: every key it had goes from the keychain, the older ones too.
#[test]
fn an_account_removed_takes_its_keys() {
    let creds = MemoryCredentials::default();
    let a = account();
    take(&creds, &a, &p12("riley-aes.p12"), NOW).unwrap();
    take(&creds, &a, &p12("riley-2025.p12"), NOW).unwrap();
    take(&creds, &a, &p12("zoe-aes.p12"), NOW).unwrap();
    assert_eq!(older_list(&creds, &a).len(), 2);
    let other = AccountConfig { id: "a2".into(), ..Default::default() };
    take(&creds, &other, &p12("riley-aes.p12"), NOW).unwrap();
    forget_all(&creds, &a);
    assert!(kept(&creds, &a).is_err() && older_list(&creds, &a).is_empty());
    for fp in [fingerprint(p12("riley-aes.p12").certificate()), fingerprint(p12("riley-2025.p12").certificate())] {
        assert!(creds.get_secret(&a, &older_entry(&fp)).is_none(), "{fp}");
    }
    assert!(kept(&creds, &other).is_ok(), "another account's stays");
}

/// At most [`OLDER_MAX`] older ones: one more is refused rather than an old one dropped unasked.
/// A damaged list reads as none.
#[test]
fn older_certificates_are_capped_and_a_damaged_list_is_none() {
    let creds = MemoryCredentials::default();
    let a = account();
    let list: Vec<String> = (0..OLDER_MAX).map(|i| format!("{i:02X}")).collect();
    creds.set_secret(&a, OLDER, &serde_json::to_string(&list).unwrap()).unwrap();
    take(&creds, &a, &p12("riley-aes.p12"), NOW).unwrap();
    assert!(take(&creds, &a, &p12("riley-2025.p12"), NOW).unwrap_err().contains("at most"));
    assert_eq!(kept(&creds, &a).unwrap().certificate(), p12("riley-aes.p12").certificate(), "the one in use stays");
    creds.set_secret(&a, OLDER, "not json").unwrap();
    assert!(older_list(&creds, &a).is_empty() && older_certs(&creds, &a).is_empty());
    let too_many: Vec<String> = (0..OLDER_MAX * 3).map(|i| format!("{i:02X}")).collect();
    creds.set_secret(&a, OLDER, &serde_json::to_string(&too_many).unwrap()).unwrap();
    assert_eq!(older_list(&creds, &a).len(), OLDER_MAX);
}

fn testdata_pem(name: &str) -> Vec<u8> {
    der::pem::decode_vec(&testdata(name)).unwrap().1
}
