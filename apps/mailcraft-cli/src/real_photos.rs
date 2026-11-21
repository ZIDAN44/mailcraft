//! Issue #223 (people's photos) through a real Exchange server on this computer (never a real
//! account), run by hand:
//!
//! ```text
//! MAILCRAFT_REAL_PHOTOS="http://127.0.0.1:18080/EWS/Exchange.asmx user@ews.test password colleague@ews.test" \
//!   cargo test -p mailcraft-cli real_photos -- --ignored --nocapture
//! ```
//!
//! The whole app's way: a colleague shown, their photo asked of the account's server by the
//! worker, and what it answered kept. With a photo the server has, it's shown; with none (Gromox
//! answers HTTP 404 with no picture), the initials stay and it isn't asked again at once.

use super::*;

#[test]
#[ignore = "needs an Exchange server in MAILCRAFT_REAL_PHOTOS"]
fn real_photos_are_asked_of_the_server() {
    let Some(v) = std::env::var("MAILCRAFT_REAL_PHOTOS").ok() else {
        eprintln!("MAILCRAFT_REAL_PHOTOS not set: skipped");
        return;
    };
    let p: Vec<&str> = v.split_whitespace().collect();
    let [url, user, password, colleague] = p[..] else { panic!("MAILCRAFT_REAL_PHOTOS: url user password colleague") };
    let dir = std::env::temp_dir().join(format!("mailcraft-real-photos-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = Store::open(&dir).unwrap();
    let mut s = Session::open(Some(store.clone()), clock());
    let id = s.run("account.add", &json!({"email": user, "username": user, "ews": url, "deferSignIn": true})).unwrap()["account"]
        .as_str()
        .unwrap()
        .to_string();
    let config = s.account(&id).unwrap().config.clone();
    let mut pool = Pool::new(store, Arc::new(MemoryCredentials::default()));
    s.take_jobs();
    pool.submit(Job::Start { account: config, password: Some(Secret(password.into())) });
    drive(&mut s, &mut pool, Duration::from_secs(90), |e| matches!(e, Event::Status { status: SyncStatus::Online { .. }, .. }).then_some(Ok(())))
        .unwrap();

    s.want_photos(&[colleague.to_string()]);
    let mut answer = None;
    drive(&mut s, &mut pool, Duration::from_secs(30), |e| match e {
        Event::Photo { email, photo, error, .. } => {
            answer = Some((email.clone(), photo.as_ref().map(Vec::len), error.clone()));
            Some(Ok(()))
        }
        _ => None,
    })
    .unwrap();
    eprintln!("answer: {answer:?}");
    let (email, size, error) = answer.unwrap();
    assert_eq!((email.as_str(), error), (colleague.to_lowercase().as_str(), None), "asked and answered, not failed");
    assert_eq!(s.photo(colleague).map(|p| p.len()), size, "what the server answered is what's kept");
    // Shown again: not asked again.
    s.want_photos(&[colleague.to_string()]);
    assert!(!s.take_jobs().iter().any(|j| matches!(j, Job::FetchPhoto { .. })));
    drop(pool);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A contact's own photo, in its card on a CardDAV server (`MAILCRAFT_REAL_CARDDAV`: the
/// server's address, the user, and the address of a contact whose card has a `PHOTO`): shown
/// for them; and kept on the server when the contact is changed here.
#[test]
#[ignore = "needs a CardDAV server in MAILCRAFT_REAL_CARDDAV"]
fn real_contact_photos_come_from_carddav() {
    let Some(v) = std::env::var("MAILCRAFT_REAL_CARDDAV").ok() else {
        eprintln!("MAILCRAFT_REAL_CARDDAV not set: skipped");
        return;
    };
    let p: Vec<&str> = v.split_whitespace().collect();
    let [url, user, contact] = p[..] else { panic!("MAILCRAFT_REAL_CARDDAV: url user contact") };
    let open = |name: &str| {
        let dir = std::env::temp_dir().join(format!("mailcraft-real-card-photos-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store::open(&dir).unwrap();
        let mut s = Session::open(Some(store.clone()), clock());
        let p = json!({"email": format!("{user}@studio.test"), "username": user, "dav": url, "deferSignIn": true});
        let id = s.run("account.add", &p).unwrap()["account"].as_str().unwrap().to_string();
        let config = s.account(&id).unwrap().config.clone();
        let mut pool = Pool::new(store, Arc::new(MemoryCredentials::default()));
        s.take_jobs();
        pool.submit(Job::Start { account: config, password: Some(Secret("pw".into())) });
        drive(&mut s, &mut pool, Duration::from_secs(60), |e| matches!(e, Event::DavSynced { .. }).then_some(Ok(()))).unwrap();
        (s, pool, dir)
    };
    let (mut s, mut pool, dir) = open("writer");
    s.want_photos(&[contact.to_string()]);
    let photo = s.photo(contact).expect("the card's photo");
    eprintln!("photo: {} bytes", photo.len());
    assert!(photo.starts_with(&[0xFF, 0xD8]) || photo.starts_with(b"\x89PNG"), "a picture");
    // Changed here: the photo stays in the card on the server.
    let uid = s
        .pim
        .values()
        .flat_map(|p| p.cards.iter())
        .find(|c| c.card.emails.iter().any(|e| e.value.eq_ignore_ascii_case(contact)))
        .map(|c| c.card.uid.clone())
        .unwrap();
    s.run("contact.update", &json!({"uid": uid, "title": "Editor"})).unwrap();
    drive(&mut s, &mut pool, Duration::from_secs(60), |e| matches!(e, Event::DavSynced { sync, .. } if sync.errors.is_empty()).then_some(Ok(())))
        .unwrap();
    drop(pool);
    let (mut r, rpool, rdir) = open("reader");
    let card = r.pim.values().flat_map(|p| p.cards.iter()).find(|c| c.card.uid == uid).map(|c| c.card.clone()).expect("on the server");
    assert_eq!(card.title, "Editor", "the change reached the server");
    r.want_photos(&[contact.to_string()]);
    assert_eq!(r.photo(contact).as_deref(), Some(&photo[..]), "and the photo with it");
    drop(rpool);
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&rdir);
}
