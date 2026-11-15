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
        key: MyKey::Held { at: KeyRef::Card { module: "m.so".into(), token: "1".into(), id: "01".into() }, pin: Some(Secret("1234".into())) },
    };
    keep(&creds, &account, &card).unwrap();
    assert!(kept(&creds, &account).unwrap() == card);
    assert!(!creds.get_secret(&account, SECRET).unwrap().contains(&hex(&riley.key)));
    // A kept PIN the card refused is forgotten at once, the key's reference kept; any other
    // failure leaves it.
    forget_wrong_pin(&creds, &account, None, &Err::<(), _>(other("the card's module can't be loaded")));
    assert!(kept(&creds, &account).unwrap() == card);
    forget_wrong_pin(&creds, &account, None, &Err::<(), _>(KeyError::WrongKeptPin));
    let left = kept(&creds, &account).unwrap();
    assert!(matches!(&left.key, MyKey::Held { pin: None, .. }) && left.chain == card.chain);
    creds.delete_secret(&account, SECRET);
    assert!(kept(&creds, &account).is_err());
    assert_eq!(unhex("0aFf"), Some(vec![0x0a, 0xff]));
    assert_eq!(unhex("0"), None);
    assert_eq!(unhex("zz"), None);
}

/// Every byte kept in the keychain as the two lower-case digits it always was comes back.
#[test]
fn every_byte_kept_as_hex_comes_back() {
    let all: Vec<u8> = (0..=255).collect();
    let written = hex(&all);
    assert_eq!(written, all.iter().map(|x| format!("{x:02x}")).collect::<String>());
    assert_eq!(unhex(&written), Some(all));
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
        let first = take(&creds, &a, order[0], NOW).unwrap();
        assert_eq!(first.chain.first(), Some(&order[0].certificate().to_vec()));
        assert!(first.older.is_empty() && !first.for_reading);
        let second = take(&creds, &a, order[1], NOW).unwrap();
        assert_eq!(second.chain.first(), Some(&new_cert), "the newer signs and encrypts");
        assert_eq!(second.older, vec![old_cert.clone()], "the older is kept for reading");
        assert_eq!(second.for_reading, order[1].certificate() == old_cert.as_slice(), "said only when the one just kept is the older");
        // Either again: the one in use stays, the older stays older (and is said to be).
        let again = take(&creds, &a, order[1], NOW).unwrap();
        assert_eq!((again.chain.first(), again.older.len()), (Some(&new_cert), 1));
        let old_again = take(&creds, &a, &old, NOW).unwrap();
        assert!(old_again.for_reading && old_again.chain.first() == Some(&new_cert));

        let mut rng = rand_core::OsRng;
        let body = b"Content-Type: text/plain\r\n\r\nSent before the renewal.\r\n";
        let to_old = smime::encrypt(body, std::slice::from_ref(&old_cert), smime::Cipher::Aes256Gcm, &mut rng).unwrap();
        let to_new = smime::encrypt(body, std::slice::from_ref(&new_cert), smime::Cipher::Aes256Cbc, &mut rng).unwrap();
        assert_eq!(decrypt(&creds, &a, &to_old).unwrap(), (body.to_vec(), true));
        assert_eq!(decrypt(&creds, &a, &to_new).unwrap(), (body.to_vec(), false));
        let to_ana = smime::encrypt(body, &[testdata_pem("ana.pem")], smime::Cipher::Aes256Cbc, &mut rng).unwrap();
        assert!(decrypt(&creds, &a, &to_ana).unwrap_err().to_string().contains("doesn't have"));

        // The older one made the one in use again: the newer is kept for reading; both still open.
        let swapped = use_older(&creds, &a, &fingerprint(&old_cert)).unwrap();
        assert_eq!((swapped.chain.first(), swapped.older), (Some(&old_cert), vec![new_cert.clone()]));
        assert!(decrypt(&creds, &a, &to_new).is_ok() && decrypt(&creds, &a, &to_old).is_ok());
        assert!(use_older(&creds, &a, "00:11").is_err(), "one that isn't kept");
        // Forgetting the one in use leaves the older readable; forgetting that one too, nothing is.
        creds.delete_secret(&a, SECRET);
        assert!(decrypt(&creds, &a, &to_new).is_ok());
        assert!(decrypt(&creds, &a, &to_old).unwrap_err().to_string().contains("import that one"));
        forget_older(&creds, &a, &fingerprint(&new_cert));
        assert!(older_list(&creds, &a).is_empty() && creds.get_secret(&a, OLDER).is_none());
        assert!(decrypt(&creds, &a, &to_new).unwrap_err().to_string().contains("isn't in the keychain"));
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
    // Full, a swap still works: the one made current leaves its place to the one it replaces.
    let full: Vec<String> = (0..OLDER_MAX - 1).map(|i| format!("{i:02X}")).collect();
    creds.set_secret(&a, OLDER, &serde_json::to_string(&full).unwrap()).unwrap();
    take(&creds, &a, &p12("riley-2025.p12"), NOW).unwrap();
    assert_eq!(older_list(&creds, &a).len(), OLDER_MAX);
    let old_fp = fingerprint(p12("riley-2025.p12").certificate());
    let swapped = use_older(&creds, &a, &old_fp).unwrap();
    assert_eq!(swapped.chain.first().map(Vec::as_slice), Some(p12("riley-2025.p12").certificate()));
    assert_eq!(older_list(&creds, &a).len(), OLDER_MAX);
    // And a renewal on top of a full list: the one it replaces takes the renewal's old place.
    use_older(&creds, &a, &fingerprint(p12("riley-aes.p12").certificate())).unwrap();
    creds.set_secret(&a, OLDER, "not json").unwrap();
    assert!(older_list(&creds, &a).is_empty() && older_certs(&creds, &a).is_empty());
    let too_many: Vec<String> = (0..OLDER_MAX * 3).map(|i| format!("{i:02X}")).collect();
    creds.set_secret(&a, OLDER, &serde_json::to_string(&too_many).unwrap()).unwrap();
    assert_eq!(older_list(&creds, &a).len(), OLDER_MAX);
}

fn testdata_pem(name: &str) -> Vec<u8> {
    der::pem::decode_vec(&testdata(name)).unwrap().1
}

/// Three certificates imported at once, on threads of their own as the pool runs them: none is
/// lost, and last year's doesn't end up signing.
#[test]
fn imports_at_once_lose_nothing() {
    for _ in 0..20 {
        let creds = std::sync::Arc::new(MemoryCredentials::default());
        let a = account();
        let threads: Vec<_> = ["riley-2025.p12", "riley-aes.p12", "zoe-aes.p12"]
            .into_iter()
            .map(|name| {
                let (creds, a, me) = (creds.clone(), a.clone(), p12(name));
                std::thread::spawn(move || take(creds.as_ref(), &a, &me, NOW).unwrap())
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        assert_eq!(older_list(creds.as_ref(), &a).len(), 2);
        // Riley's and Zoe's test certificates were issued in the same second: either may win the
        // tie, never last year's.
        assert_ne!(kept(creds.as_ref(), &a).unwrap().certificate(), p12("riley-2025.p12").certificate());
    }
}

/// Issue #81: a message is signed with the certificate named for it (the one of the address
/// it's from), whether that's the one in use or one kept beside it; with none named, or one
/// MailCraft doesn't have, as before.
#[test]
fn a_message_is_signed_with_the_certificate_named() {
    let creds = MemoryCredentials::default();
    let account = account();
    let (riley, zoe) = (p12("riley-aes.p12"), p12("zoe-aes.p12"));
    take(&creds, &account, &riley, NOW).unwrap();
    take(&creds, &account, &zoe, NOW).unwrap();
    let in_use = kept(&creds, &account).unwrap();
    let beside = if in_use == riley { zoe.clone() } else { riley.clone() };
    // None named: the one in use.
    let (me, older) = signing(&creds, &account, None);
    assert!(me.unwrap() == in_use && older.is_none());
    // The one in use named: itself, not looked for among the others.
    let (me, older) = signing(&creds, &account, Some(in_use.certificate()));
    assert!(me.unwrap() == in_use && older.is_none());
    // The other named: it, with its key, from where it's kept.
    let (me, older) = signing(&creds, &account, Some(beside.certificate()));
    assert!(me.unwrap() == beside);
    assert_eq!(older, Some(fingerprint(beside.certificate())));
    // One MailCraft doesn't have: said, nothing signed with another.
    let stranger = p12("riley-2025.p12");
    assert!(signing(&creds, &account, Some(stranger.certificate())).0.is_err());
    // What can't be read as a certificate names none.
    assert!(signing(&creds, &account, Some(b"junk")).0.unwrap() == in_use);
}

/// A key on the tests' stand-in card, which refuses every PIN, with `me`'s certificate.
fn on_refusing_card(me: &Me, pin: Option<&str>) -> Me {
    let at = KeyRef::Card { module: REFUSES_PIN.into(), token: "1".into(), id: "01".into() };
    Me { chain: me.chain.clone(), key: MyKey::Held { at, pin: pin.map(|p| Secret(p.into())) } }
}

fn pin_of(me: std::result::Result<Me, String>) -> Option<String> {
    match me.unwrap().key {
        MyKey::Held { pin, .. } => pin.map(|p| p.0),
        MyKey::Software(_) => None,
    }
}

/// A kept PIN the card refuses is told by its kind, not by its words, through the MIME layer
/// and every wrapper on the way: signing and decrypting forget it from the entry it came from
/// (the one in use, or one kept beside it), the key's reference kept. A PIN just typed, or none
/// at all, isn't a kept one refused, and the PIN itself is never in what the user is told.
#[test]
fn a_kept_pin_the_card_refuses_is_forgotten_wherever_it_was_used() {
    const RAW: &[u8] = b"From: Riley <riley@studio.example>\r\nTo: riley@studio.example\r\nSubject: Plan\r\n\
MIME-Version: 1.0\r\nContent-Type: text/plain\r\n\r\nThe plan.\r\n";
    let creds = MemoryCredentials::default();
    let account = account();
    let (riley, zoe) = (p12("riley-aes.p12"), p12("zoe-aes.p12"));
    let card = on_refusing_card(&riley, Some("4711"));
    keep(&creds, &account, &card).unwrap();
    assert!(creds.get_secret(&account, SECRET).unwrap().contains(r#""pin":"4711""#), "kept as it always was");

    // Signed, and signed and encrypted, with the one in use.
    for encrypt_to in [Vec::new(), vec![riley.certificate().to_vec()]] {
        keep(&creds, &account, &card).unwrap();
        let (me, older) = signing(&creds, &account, None);
        let done = me.map_err(KeyError::Other).and_then(|me| protect(RAW, &me, true, &encrypt_to, true, true));
        assert_eq!(done, Err(KeyError::WrongKeptPin));
        let told = done.as_ref().map_err(ToString::to_string).unwrap_err();
        assert!(told == PIN_WRONG && !told.contains("4711"), "{told}");
        forget_wrong_pin(&creds, &account, older.as_deref(), &done);
        assert_eq!(pin_of(kept(&creds, &account)), None);
        assert!(kept(&creds, &account).unwrap().chain == card.chain, "the key's reference stays");
    }
    // With its PIN forgotten the card asks for one: not a kept PIN refused.
    let done = protect(RAW, &kept(&creds, &account).unwrap(), true, &[], false, false);
    assert!(matches!(&done, Err(KeyError::Other(e)) if e.contains("needs its PIN")), "{done:?}");

    // Signed with one kept beside the one in use (issue #81): only its own PIN is forgotten.
    keep(&creds, &account, &on_refusing_card(&zoe, Some("1111"))).unwrap();
    keep_older(&creds, &account, &card, None).unwrap();
    let riley_fp = fingerprint(riley.certificate());
    let (me, older) = signing(&creds, &account, Some(riley.certificate()));
    assert_eq!(older.as_deref(), Some(riley_fp.as_str()));
    let done = me.map_err(KeyError::Other).and_then(|me| protect(RAW, &me, true, &[], false, false));
    assert_eq!(done, Err(KeyError::WrongKeptPin));
    forget_wrong_pin(&creds, &account, older.as_deref(), &done);
    assert_eq!(pin_of(kept_older(&creds, &account, &riley_fp)), None);
    assert_eq!(pin_of(kept(&creds, &account)).as_deref(), Some("1111"));

    // Decrypting: mail to the older one forgets its PIN, mail to the one in use that one's.
    keep_older(&creds, &account, &card, None).unwrap();
    let mut rng = rand_core::OsRng;
    let body = b"Content-Type: text/plain\r\n\r\nFor the card.\r\n";
    let to_riley = smime::encrypt(body, std::slice::from_ref(&riley.chain[0]), smime::Cipher::Aes256Gcm, &mut rng).unwrap();
    assert_eq!(decrypt(&creds, &account, &to_riley), Err(KeyError::WrongKeptPin));
    assert_eq!(pin_of(kept_older(&creds, &account, &riley_fp)), None);
    assert_eq!(pin_of(kept(&creds, &account)).as_deref(), Some("1111"));
    let to_zoe = smime::encrypt(body, std::slice::from_ref(&zoe.chain[0]), smime::Cipher::Aes256Cbc, &mut rng).unwrap();
    assert_eq!(decrypt(&creds, &account, &to_zoe), Err(KeyError::WrongKeptPin));
    assert_eq!(pin_of(kept(&creds, &account)), None);

    // A PIN just typed that the card refuses is the user's to try again: not a kept one.
    let MyKey::Held { at, .. } = &card.key else { panic!() };
    let typed = with_held(at, Some("4711"), mailcraft_keys::pkcs11::PinFrom::Typed, |_| Ok::<(), smime::Error>(()));
    assert_eq!(typed, Err(KeyError::Other(mailcraft_keys::Error::Pin.to_string())));
}
